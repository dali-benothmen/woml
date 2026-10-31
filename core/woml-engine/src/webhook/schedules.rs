//! Private webhook schedules.

use crate::schedule::ScheduleMisfirePolicy;
use crate::schedule::ScheduleProgress;
use crate::schedule::ScheduleProgressReason;
use crate::schedule::WomlSchedule;
use crate::schedule::SCHEDULE_PROGRESS_CONTRACT;
use crate::schedule::SCHEDULE_PROGRESS_CONTRACT_VERSION;
use crate::webhook::dispatch_run;
use crate::webhook::literal_string;
use crate::webhook::object_fields;
use crate::webhook::RunProgressIdentity;
use crate::webhook::WebhookDefinitionRegistration;
use crate::webhook::WebhookRuntimeError;
use crate::webhook::WebhookRuntimeState;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::ScheduleCursorRegistration;
use crate::TriggerAdmissionRequest;
use actix_web::web;
use chrono::SecondsFormat;
use chrono::Utc;
use serde_json::Value;

#[derive(Clone)]
pub(in crate::webhook) struct ScheduleRuntimeRegistration {
  pub(in crate::webhook) workflow_id: String,
  pub(in crate::webhook) definition_hash: String,
  pub(in crate::webhook) trigger_id: String,
  pub(in crate::webhook) schedule: WomlSchedule,
  pub(in crate::webhook) on_missed: ScheduleMisfirePolicy,
}

pub(in crate::webhook) fn compile_schedule(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<ScheduleRuntimeRegistration, WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let cron = literal_string(fields.get("cron")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is missing cron",
      trigger.id
    ))
  })?;
  let timezone = literal_string(fields.get("timezone")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is missing timezone",
      trigger.id
    ))
  })?;
  let on_missed = literal_string(fields.get("onMissed")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is missing onMissed",
      trigger.id
    ))
  })?;
  let schedule = WomlSchedule::parse(cron, timezone).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  let on_missed = ScheduleMisfirePolicy::parse(on_missed).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  Ok(ScheduleRuntimeRegistration {
    workflow_id: registration.workflow.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: trigger.id.clone(),
    schedule,
    on_missed,
  })
}

pub(in crate::webhook) fn initialize_schedules(
  state: web::Data<WebhookRuntimeState>,
) -> Result<
  (
    Vec<actix_web::rt::task::JoinHandle<()>>,
    Vec<(RunProgressIdentity, bool)>,
  ),
  WebhookRuntimeError,
> {
  let now = state.execution.schedule_clock.now();
  let mut recovered_runs = Vec::new();
  let mut prepared = Vec::new();
  for registration in state.schedules.clone() {
    let initial_next = registration
      .schedule
      .next_at_or_after(now)
      .map_err(|error| {
        WebhookRuntimeError::InvalidRegistration(format!(
          "schedule trigger {:?} could not compute its first occurrence: {error}",
          registration.trigger_id
        ))
      })?;
    let mut store = DurableEventStore::open_ready(&state.database_path)?;
    let registered = store.register_schedule_cursor(
      &ScheduleCursorRegistration {
        workflow_id: registration.workflow_id.clone(),
        trigger_id: registration.trigger_id.clone(),
        definition_hash: registration.definition_hash.clone(),
        cron: registration.schedule.cron().to_string(),
        timezone: registration.schedule.timezone().to_string(),
        on_missed: registration.on_missed.as_str().to_string(),
      },
      initial_next,
      now,
    )?;
    let mut cursor = registered.cursor;
    let mut reason = if registered.initialized {
      ScheduleProgressReason::Initialized
    } else {
      ScheduleProgressReason::Restarted
    };

    if !registered.initialized && cursor.next_scheduled_at < now {
      let next = registration.schedule.next_after(now).map_err(|error| {
        WebhookRuntimeError::InvalidRegistration(format!(
          "schedule trigger {:?} could not recover its cursor: {error}",
          registration.trigger_id
        ))
      })?;
      match registration.on_missed {
        ScheduleMisfirePolicy::Skip => {
          cursor = store.advance_schedule_cursor(
            &registration.workflow_id,
            &registration.trigger_id,
            cursor.next_scheduled_at,
            next,
            now,
          )?;
          reason = ScheduleProgressReason::MisfireSkipped;
        }
        ScheduleMisfirePolicy::RunOnce => {
          let planned = registration
            .schedule
            .latest_at_or_before(now)
            .map_err(|error| {
              WebhookRuntimeError::InvalidRegistration(format!(
                "schedule trigger {:?} could not recover its latest occurrence: {error}",
                registration.trigger_id
              ))
            })?;
          let (outcome, advanced) = store.claim_schedule_occurrence(
            cursor.next_scheduled_at,
            next,
            schedule_admission(&registration, planned, now),
          )?;
          cursor = advanced;
          recovered_runs.push((
            schedule_identity(&registration, &outcome),
            outcome.duplicate,
          ));
          reason = ScheduleProgressReason::MisfireRunOnce;
        }
      }
    }
    report_schedule_next_due(state.get_ref(), &registration, &cursor, reason, now);
    prepared.push(registration);
  }

  let tasks = prepared
    .into_iter()
    .map(|registration| {
      let task_state = state.clone();
      actix_web::rt::spawn(async move {
        run_schedule_loop(task_state, registration).await;
      })
    })
    .collect();
  Ok((tasks, recovered_runs))
}

async fn run_schedule_loop(
  state: web::Data<WebhookRuntimeState>,
  registration: ScheduleRuntimeRegistration,
) {
  loop {
    let cursor = match DurableEventStore::open_ready(&state.database_path)
      .and_then(|store| store.schedule_cursor(&registration.workflow_id, &registration.trigger_id))
    {
      Ok(cursor) => cursor,
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    state
      .execution
      .schedule_clock
      .sleep_until(cursor.next_scheduled_at)
      .await;
    let now = state.execution.schedule_clock.now();
    if now < cursor.next_scheduled_at {
      continue;
    }

    let following = match registration.schedule.next_after(cursor.next_scheduled_at) {
      Ok(value) => value,
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let multiple_elapsed = following <= now;
    let mut store = match DurableEventStore::open_ready(&state.database_path) {
      Ok(store) => store,
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };

    if multiple_elapsed && registration.on_missed == ScheduleMisfirePolicy::Skip {
      let next = match registration.schedule.next_after(now) {
        Ok(value) => value,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      match store.advance_schedule_cursor(
        &registration.workflow_id,
        &registration.trigger_id,
        cursor.next_scheduled_at,
        next,
        now,
      ) {
        Ok(advanced) => report_schedule_next_due(
          state.get_ref(),
          &registration,
          &advanced,
          ScheduleProgressReason::MisfireSkipped,
          now,
        ),
        Err(DurableStoreError::ScheduleCursorConflict) => continue,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      }
      continue;
    }

    let (planned, next, reason) = if multiple_elapsed {
      let planned = match registration.schedule.latest_at_or_before(now) {
        Ok(value) => value,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      let next = match registration.schedule.next_after(now) {
        Ok(value) => value,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      (planned, next, ScheduleProgressReason::MisfireRunOnce)
    } else {
      (
        cursor.next_scheduled_at,
        following,
        ScheduleProgressReason::Advanced,
      )
    };
    let (outcome, advanced) = match store.claim_schedule_occurrence(
      cursor.next_scheduled_at,
      next,
      schedule_admission(&registration, planned, now),
    ) {
      Ok(value) => value,
      Err(DurableStoreError::ScheduleCursorConflict) => continue,
      Err(DurableStoreError::RuntimePolicyQueueFull) => {
        report_schedule_policy_queue_full(state.get_ref(), &registration);
        return;
      }
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let identity = schedule_identity(&registration, &outcome);
    state.report_accepted(&identity, outcome.duplicate);
    if !outcome.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    report_schedule_next_due(state.get_ref(), &registration, &advanced, reason, now);
  }
}

fn schedule_admission(
  registration: &ScheduleRuntimeRegistration,
  planned: chrono::DateTime<Utc>,
  triggered_at: chrono::DateTime<Utc>,
) -> TriggerAdmissionRequest {
  let planned_text = planned.to_rfc3339_opts(SecondsFormat::Millis, true);
  let triggered_text = triggered_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  TriggerAdmissionRequest {
    workflow_id: registration.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.schedule".to_string(),
    source_identity: format!(
      "{}:{}:{planned_text}",
      registration.workflow_id, registration.trigger_id
    ),
    payload: serde_json::Map::from_iter([
      ("scheduledAt".to_string(), Value::String(planned_text)),
      ("triggeredAt".to_string(), Value::String(triggered_text)),
    ]),
    received_at: triggered_at,
  }
}

fn schedule_identity(
  registration: &ScheduleRuntimeRegistration,
  outcome: &crate::TriggerAdmissionOutcome,
) -> RunProgressIdentity {
  RunProgressIdentity {
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.schedule".to_string(),
    occurrence_id: outcome.occurrence_id.clone(),
    run_id: outcome.run_id.clone(),
  }
}

fn report_schedule_next_due(
  state: &WebhookRuntimeState,
  registration: &ScheduleRuntimeRegistration,
  cursor: &crate::ScheduleCursor,
  reason: ScheduleProgressReason,
  occurred_at: chrono::DateTime<Utc>,
) {
  state.execution.report_schedule(ScheduleProgress::NextDue {
    contract: SCHEDULE_PROGRESS_CONTRACT,
    contract_version: SCHEDULE_PROGRESS_CONTRACT_VERSION,
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    timezone: registration.schedule.timezone().to_string(),
    next_scheduled_at: cursor.next_scheduled_at,
    reason,
    occurred_at,
  });
}

fn report_schedule_error(
  state: &WebhookRuntimeState,
  registration: &ScheduleRuntimeRegistration,
  message: &str,
) {
  state
    .execution
    .report_schedule(ScheduleProgress::SchedulerError {
      contract: SCHEDULE_PROGRESS_CONTRACT,
      contract_version: SCHEDULE_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_SCHEDULE_RUNTIME_FAILED".to_string(),
      message: message.to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}

fn report_schedule_policy_queue_full(
  state: &WebhookRuntimeState,
  registration: &ScheduleRuntimeRegistration,
) {
  state
    .execution
    .report_schedule(ScheduleProgress::SchedulerError {
      contract: SCHEDULE_PROGRESS_CONTRACT,
      contract_version: SCHEDULE_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_POLICY_QUEUE_FULL".to_string(),
      message: "The durable WOML policy queue is full; the schedule cursor was not advanced."
        .to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}
