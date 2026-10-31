//! Private webhook intervals.

use crate::interval::IntervalProgress;
use crate::interval::IntervalProgressReason;
use crate::interval::WomlInterval;
use crate::interval::INTERVAL_PROGRESS_CONTRACT;
use crate::interval::INTERVAL_PROGRESS_CONTRACT_VERSION;
use crate::schedule::ScheduleMisfirePolicy;
use crate::webhook::dispatch_run;
use crate::webhook::literal_string;
use crate::webhook::literal_u64;
use crate::webhook::object_fields;
use crate::webhook::RunProgressIdentity;
use crate::webhook::WebhookDefinitionRegistration;
use crate::webhook::WebhookRuntimeError;
use crate::webhook::WebhookRuntimeState;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::IntervalCursorRegistration;
use crate::TriggerAdmissionRequest;
use actix_web::web;
use chrono::SecondsFormat;
use chrono::Utc;
use serde_json::Value;

#[derive(Clone)]
pub(in crate::webhook) struct IntervalRuntimeRegistration {
  pub(in crate::webhook) workflow_id: String,
  pub(in crate::webhook) definition_hash: String,
  pub(in crate::webhook) trigger_id: String,
  pub(in crate::webhook) interval: WomlInterval,
  pub(in crate::webhook) on_missed: ScheduleMisfirePolicy,
}

pub(in crate::webhook) fn compile_interval(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<IntervalRuntimeRegistration, WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let every_ms = literal_u64(fields.get("everyMs")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is missing everyMs",
      trigger.id
    ))
  })?;
  let on_missed = literal_string(fields.get("onMissed")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is missing onMissed",
      trigger.id
    ))
  })?;
  let interval = WomlInterval::new(every_ms).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  let on_missed = ScheduleMisfirePolicy::parse(on_missed).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  Ok(IntervalRuntimeRegistration {
    workflow_id: registration.workflow.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: trigger.id.clone(),
    interval,
    on_missed,
  })
}

pub(in crate::webhook) fn initialize_intervals(
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
  for registration in state.intervals.clone() {
    let mut store = DurableEventStore::open_ready(&state.database_path)?;
    let registered = store.register_interval_cursor(
      &IntervalCursorRegistration {
        workflow_id: registration.workflow_id.clone(),
        trigger_id: registration.trigger_id.clone(),
        definition_hash: registration.definition_hash.clone(),
        every_ms: registration.interval.every_ms(),
        on_missed: registration.on_missed.as_str().to_string(),
      },
      now,
      now,
    )?;
    let mut cursor = registered.cursor;
    let mut reason = if registered.initialized {
      IntervalProgressReason::Initialized
    } else {
      IntervalProgressReason::Restarted
    };

    if !registered.initialized && cursor.next_scheduled_at < now {
      let next_sequence = registration
        .interval
        .next_sequence_after(cursor.anchor_at, now)
        .map_err(|error| interval_registration_error(&registration, "recover its cursor", error))?;
      let next_scheduled_at = registration
        .interval
        .planned_at(cursor.anchor_at, next_sequence)
        .map_err(|error| interval_registration_error(&registration, "recover its cursor", error))?;
      match registration.on_missed {
        ScheduleMisfirePolicy::Skip => {
          cursor = store.advance_interval_cursor(
            &registration.workflow_id,
            &registration.trigger_id,
            cursor.next_sequence,
            cursor.next_scheduled_at,
            next_sequence,
            next_scheduled_at,
            now,
          )?;
          reason = IntervalProgressReason::MisfireSkipped;
        }
        ScheduleMisfirePolicy::RunOnce => {
          let planned_sequence = registration
            .interval
            .latest_sequence_at_or_before(cursor.anchor_at, now)
            .map_err(|error| {
              interval_registration_error(&registration, "recover its latest occurrence", error)
            })?;
          let planned_at = registration
            .interval
            .planned_at(cursor.anchor_at, planned_sequence)
            .map_err(|error| {
              interval_registration_error(&registration, "recover its latest occurrence", error)
            })?;
          let (outcome, advanced) = store.claim_interval_occurrence(
            cursor.next_sequence,
            cursor.next_scheduled_at,
            next_sequence,
            next_scheduled_at,
            interval_admission(
              &registration,
              cursor.anchor_at,
              planned_sequence,
              planned_at,
              now,
            ),
          )?;
          cursor = advanced;
          recovered_runs.push((
            interval_identity(&registration, &outcome),
            outcome.duplicate,
          ));
          reason = IntervalProgressReason::MisfireRunOnce;
        }
      }
    }
    report_interval_next_due(state.get_ref(), &registration, &cursor, reason, now);
    prepared.push(registration);
  }

  let tasks = prepared
    .into_iter()
    .map(|registration| {
      let task_state = state.clone();
      actix_web::rt::spawn(async move {
        run_interval_loop(task_state, registration).await;
      })
    })
    .collect();
  Ok((tasks, recovered_runs))
}

fn interval_registration_error(
  registration: &IntervalRuntimeRegistration,
  operation: &str,
  error: crate::IntervalError,
) -> WebhookRuntimeError {
  WebhookRuntimeError::InvalidRegistration(format!(
    "interval trigger {:?} could not {operation}: {error}",
    registration.trigger_id
  ))
}

async fn run_interval_loop(
  state: web::Data<WebhookRuntimeState>,
  registration: IntervalRuntimeRegistration,
) {
  loop {
    let cursor = match DurableEventStore::open_ready(&state.database_path)
      .and_then(|store| store.interval_cursor(&registration.workflow_id, &registration.trigger_id))
    {
      Ok(cursor) => cursor,
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
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

    let following_sequence = match cursor.next_sequence.checked_add(1) {
      Some(value) => value,
      None => {
        report_interval_error(
          state.get_ref(),
          &registration,
          "interval sequence exceeds the supported range",
        );
        return;
      }
    };
    let following_at = match registration
      .interval
      .planned_at(cursor.anchor_at, following_sequence)
    {
      Ok(value) => value,
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let multiple_elapsed = following_at <= now;
    let mut store = match DurableEventStore::open_ready(&state.database_path) {
      Ok(store) => store,
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };

    if multiple_elapsed && registration.on_missed == ScheduleMisfirePolicy::Skip {
      let next_sequence = match registration
        .interval
        .next_sequence_after(cursor.anchor_at, now)
      {
        Ok(value) => value,
        Err(error) => {
          report_interval_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      let next_scheduled_at = match registration
        .interval
        .planned_at(cursor.anchor_at, next_sequence)
      {
        Ok(value) => value,
        Err(error) => {
          report_interval_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      match store.advance_interval_cursor(
        &registration.workflow_id,
        &registration.trigger_id,
        cursor.next_sequence,
        cursor.next_scheduled_at,
        next_sequence,
        next_scheduled_at,
        now,
      ) {
        Ok(advanced) => report_interval_next_due(
          state.get_ref(),
          &registration,
          &advanced,
          IntervalProgressReason::MisfireSkipped,
          now,
        ),
        Err(DurableStoreError::IntervalCursorConflict) => continue,
        Err(error) => {
          report_interval_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      }
      continue;
    }

    let (planned_sequence, planned_at, next_sequence, next_scheduled_at, reason) =
      if multiple_elapsed {
        let planned_sequence = match registration
          .interval
          .latest_sequence_at_or_before(cursor.anchor_at, now)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        let planned_at = match registration
          .interval
          .planned_at(cursor.anchor_at, planned_sequence)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        let next_sequence = match registration
          .interval
          .next_sequence_after(cursor.anchor_at, now)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        let next_at = match registration
          .interval
          .planned_at(cursor.anchor_at, next_sequence)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        (
          planned_sequence,
          planned_at,
          next_sequence,
          next_at,
          IntervalProgressReason::MisfireRunOnce,
        )
      } else {
        (
          cursor.next_sequence,
          cursor.next_scheduled_at,
          following_sequence,
          following_at,
          IntervalProgressReason::Advanced,
        )
      };

    let (outcome, advanced) = match store.claim_interval_occurrence(
      cursor.next_sequence,
      cursor.next_scheduled_at,
      next_sequence,
      next_scheduled_at,
      interval_admission(
        &registration,
        cursor.anchor_at,
        planned_sequence,
        planned_at,
        now,
      ),
    ) {
      Ok(value) => value,
      Err(DurableStoreError::IntervalCursorConflict) => continue,
      Err(DurableStoreError::RuntimePolicyQueueFull) => {
        report_interval_policy_queue_full(state.get_ref(), &registration);
        return;
      }
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let identity = interval_identity(&registration, &outcome);
    state.report_accepted(&identity, outcome.duplicate);
    if !outcome.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    report_interval_next_due(state.get_ref(), &registration, &advanced, reason, now);
  }
}

fn interval_admission(
  registration: &IntervalRuntimeRegistration,
  anchor_at: chrono::DateTime<Utc>,
  sequence: u64,
  planned_at: chrono::DateTime<Utc>,
  triggered_at: chrono::DateTime<Utc>,
) -> TriggerAdmissionRequest {
  let anchor_text = anchor_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  let planned_text = planned_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  let triggered_text = triggered_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  TriggerAdmissionRequest {
    workflow_id: registration.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.interval".to_string(),
    source_identity: format!(
      "{}:{}:{anchor_text}:{sequence}",
      registration.workflow_id, registration.trigger_id
    ),
    payload: serde_json::Map::from_iter([
      ("scheduledAt".to_string(), Value::String(planned_text)),
      ("triggeredAt".to_string(), Value::String(triggered_text)),
    ]),
    received_at: triggered_at,
  }
}

fn interval_identity(
  registration: &IntervalRuntimeRegistration,
  outcome: &crate::TriggerAdmissionOutcome,
) -> RunProgressIdentity {
  RunProgressIdentity {
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.interval".to_string(),
    occurrence_id: outcome.occurrence_id.clone(),
    run_id: outcome.run_id.clone(),
  }
}

fn report_interval_next_due(
  state: &WebhookRuntimeState,
  registration: &IntervalRuntimeRegistration,
  cursor: &crate::IntervalCursor,
  reason: IntervalProgressReason,
  occurred_at: chrono::DateTime<Utc>,
) {
  state.execution.report_interval(IntervalProgress::NextDue {
    contract: INTERVAL_PROGRESS_CONTRACT,
    contract_version: INTERVAL_PROGRESS_CONTRACT_VERSION,
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    every_ms: registration.interval.every_ms(),
    anchor_at: cursor.anchor_at,
    next_sequence: cursor.next_sequence,
    next_scheduled_at: cursor.next_scheduled_at,
    reason,
    occurred_at,
  });
}

fn report_interval_error(
  state: &WebhookRuntimeState,
  registration: &IntervalRuntimeRegistration,
  message: &str,
) {
  state
    .execution
    .report_interval(IntervalProgress::SchedulerError {
      contract: INTERVAL_PROGRESS_CONTRACT,
      contract_version: INTERVAL_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_INTERVAL_RUNTIME_FAILED".to_string(),
      message: message.to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}

fn report_interval_policy_queue_full(
  state: &WebhookRuntimeState,
  registration: &IntervalRuntimeRegistration,
) {
  state
    .execution
    .report_interval(IntervalProgress::SchedulerError {
      contract: INTERVAL_PROGRESS_CONTRACT,
      contract_version: INTERVAL_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_POLICY_QUEUE_FULL".to_string(),
      message: "The durable WOML policy queue is full; the interval cursor was not advanced."
        .to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}
