//! Durable wait registration, due-work queries, timer and signal-timeout settlement. Eligible early signals are consumed atomically with registration.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::DurableWaitRecordV1;
use crate::durable::SignalWaitTimeoutSettlement;
use crate::durable::TimerWaitSettlement;
use crate::durable::{
  append_signal_resolution, derive_lifecycle_hook_invocation_id, parse_stored_timestamp,
  resolve_signal_correlation, DurableEventStore,
};
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::fold_events;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::Value;
use std::collections::HashSet;

impl DurableEventStore {
  pub fn register_durable_wait_atomically(
    &mut self,
    run_id: &str,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "A durable wait cannot be registered before run_started.".to_string(),
        )
      })?;
    if !matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      return Err(DurableStoreError::Contract(
        "Durable wait registration requires Event v16 or a supported successor.".to_string(),
      ));
    }
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let payload = RunEventPayload::WaitRegistered(registration.clone());
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let event_id = event_id.into();
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id,
      occurred_at,
      event_schema_version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    if projection.status != RunStatus::Waiting
      || !matches!(
        projection.durable_waits.get(&registration.wait_id),
        Some(wait) if wait.status == crate::DurableWaitStatus::Registered
      )
    {
      return Err(DurableStoreError::Contract(
        "The wait registration did not produce the matching durable waiting state.".to_string(),
      ));
    }
    let kind = match registration.kind {
      crate::DurableWaitKind::Duration => "duration",
      crate::DurableWaitKind::Until => "until",
      crate::DurableWaitKind::Signal => "signal",
    };
    let signals_json = registration
      .signals
      .as_ref()
      .map(serde_json::to_string)
      .transpose()?;
    transaction.execute(
      "INSERT INTO woml_durable_waits(
         run_id, wait_id, iteration_identity, node_id, kind, registered_at,
         deadline, signals_json, correlation_digest, state
       ) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7, ?8, 'registered')",
      params![
        run_id,
        registration.wait_id,
        registration.node_id,
        kind,
        registration.registered_at.to_rfc3339(),
        registration.deadline.map(|value| value.to_rfc3339()),
        signals_json,
        registration.correlation_digest,
      ],
    )?;
    if registration.kind == crate::DurableWaitKind::Signal {
      let signals = registration.signals.as_deref().unwrap_or_default();
      let correlation_digest = registration
        .correlation_digest
        .as_deref()
        .unwrap_or_default();
      let correlation = match workflow.durable_wait(&registration.wait_id) {
        Some(crate::CompiledWait::Signal { correlation, .. }) => {
          resolve_signal_correlation(correlation, &projection.context).ok_or_else(|| {
            DurableStoreError::Contract(
              "The registered signal wait has no resolvable correlation value.".to_string(),
            )
          })?
        }
        _ => {
          return Err(DurableStoreError::Contract(
            "The registered signal wait has no compiled signal descriptor.".to_string(),
          ));
        }
      };
      let early: Option<(String, String, String, String, String, String)> = transaction
        .prepare(
          "SELECT signal_id, signal, received_at, idempotency_digest,
                  payload_json, authentication_class
           FROM woml_external_signals
           WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''
             AND correlation_digest = ?3 AND consumed_at IS NULL
           ORDER BY received_at, signal_id",
        )?
        .query_map(
          params![run_id, registration.wait_id, correlation_digest],
          |row| {
            Ok((
              row.get::<_, String>(0)?,
              row.get::<_, String>(1)?,
              row.get::<_, String>(2)?,
              row.get::<_, String>(3)?,
              row.get::<_, String>(4)?,
              row.get::<_, String>(5)?,
            ))
          },
        )?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(_, signal, _, _, _, _)| signals.iter().any(|accepted| accepted == signal));
      if let Some((signal_id, signal, received_at, idempotency, payload, authentication_class)) =
        early
      {
        let received_at = parse_stored_timestamp(&received_at)?;
        let payload: Value = serde_json::from_str(&payload)?;
        let projection = append_signal_resolution(
          &transaction,
          &workflow,
          &binding.definition_hash,
          &mut events,
          run_id,
          &registration.wait_id,
          &signal_id,
          &signal,
          &correlation,
          &idempotency,
          &authentication_class,
          received_at,
          registration.registered_at,
          payload,
        )?;
        transaction.commit()?;
        return Ok(projection);
      }
    }
    transaction.commit()?;
    Ok(projection)
  }

  pub fn due_timer_waits(
    &self,
    now: DateTime<Utc>,
    limit: usize,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    self.query_due_waits(now, limit, None, false)
  }

  pub(crate) fn due_timer_waits_for_workflows(
    &self,
    now: DateTime<Utc>,
    limit: usize,
    workflow_ids: &HashSet<String>,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    if workflow_ids.is_empty() {
      return Ok(Vec::new());
    }
    self.query_due_waits(now, limit, Some(workflow_ids), false)
  }

  pub(crate) fn due_signal_waits_for_workflows(
    &self,
    now: DateTime<Utc>,
    limit: usize,
    workflow_ids: &HashSet<String>,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    if workflow_ids.is_empty() {
      return Ok(Vec::new());
    }
    self.query_due_waits(now, limit, Some(workflow_ids), true)
  }

  fn query_due_waits(
    &self,
    now: DateTime<Utc>,
    limit: usize,
    workflow_ids: Option<&HashSet<String>>,
    signal_waits: bool,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    if limit == 0 || limit > 1_000 {
      return Err(DurableStoreError::Contract(
        "Due-wait scans require a limit between 1 and 1000.".to_string(),
      ));
    }
    let mut parameters = vec![rusqlite::types::Value::Text(now.to_rfc3339())];
    let workflow_filter = workflow_ids.map(|workflow_ids| {
      let mut workflow_ids = workflow_ids.iter().cloned().collect::<Vec<_>>();
      workflow_ids.sort();
      let placeholders = workflow_ids
        .iter()
        .map(|workflow_id| {
          parameters.push(rusqlite::types::Value::Text(workflow_id.clone()));
          format!("?{}", parameters.len())
        })
        .collect::<Vec<_>>()
        .join(", ");
      format!(
        " AND EXISTS (
           SELECT 1 FROM woml_runs r
           WHERE r.run_id = w.run_id AND r.workflow_id IN ({placeholders})
         )"
      )
    });
    parameters.push(rusqlite::types::Value::Integer(
      i64::try_from(limit).unwrap_or(1_000),
    ));
    let limit_parameter = parameters.len();
    let kind_filter = if signal_waits {
      "w.kind = 'signal'"
    } else {
      "w.kind IN ('duration', 'until')"
    };
    let query = format!(
      "SELECT w.run_id, w.wait_id, w.node_id, w.kind, w.registered_at,
              w.deadline, w.state, w.resolved_at
       FROM woml_durable_waits w
       WHERE w.state = 'registered'
         AND {kind_filter}
         AND w.deadline <= ?1{}
       ORDER BY w.deadline, w.run_id, w.wait_id
       LIMIT ?{limit_parameter}",
      workflow_filter.as_deref().unwrap_or_default()
    );
    let mut statement = self.connection.prepare(&query)?;
    let records = statement
      .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
        let kind: String = row.get(3)?;
        let registered_at: String = row.get(4)?;
        let deadline: Option<String> = row.get(5)?;
        let resolved_at: Option<String> = row.get(7)?;
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          kind,
          registered_at,
          deadline,
          row.get::<_, String>(6)?,
          resolved_at,
        ))
      })?
      .map(|row| {
        let (run_id, wait_id, node_id, kind, registered_at, deadline, state, resolved_at) = row?;
        Ok(DurableWaitRecordV1 {
          run_id,
          wait_id,
          node_id,
          kind: match kind.as_str() {
            "duration" => crate::DurableWaitKind::Duration,
            "until" => crate::DurableWaitKind::Until,
            "signal" => crate::DurableWaitKind::Signal,
            _ => {
              return Err(DurableStoreError::Contract(
                "Stored durable wait has an invalid kind.".to_string(),
              ))
            }
          },
          registered_at: parse_stored_timestamp(&registered_at)?,
          deadline: deadline
            .as_deref()
            .map(parse_stored_timestamp)
            .transpose()?,
          state,
          resolved_at: resolved_at
            .as_deref()
            .map(parse_stored_timestamp)
            .transpose()?,
        })
      })
      .collect();
    records
  }

  pub fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: DateTime<Utc>,
  ) -> Result<TimerWaitSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let row: Option<(String, String, Option<String>, String)> = transaction
      .query_row(
        "SELECT node_id, kind, deadline, state FROM woml_durable_waits
         WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''",
        params![run_id, wait_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    let Some((node_id, kind, deadline, state)) = row else {
      return Err(DurableStoreError::Contract(
        "Timer settlement references an unknown durable wait.".to_string(),
      ));
    };
    let events = load_events(&transaction, run_id)?;
    if state != "registered" {
      return Ok(TimerWaitSettlement::AlreadyResolved {
        projection: fold_events(&events)?,
      });
    }
    if !matches!(kind.as_str(), "duration" | "until") {
      return Err(DurableStoreError::Contract(
        "Timer settlement requires a duration or until wait.".to_string(),
      ));
    }
    let deadline = deadline
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .ok_or_else(|| DurableStoreError::Contract("Timer wait has no deadline.".to_string()))?;
    if now < deadline {
      return Ok(TimerWaitSettlement::NotDue { deadline });
    }
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let mut events = events;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| DurableStoreError::Contract("Timer wait has no run history.".to_string()))?;
    let event_id = generated_event_id();
    let result = serde_json::json!({
      "status": "elapsed",
      "scheduledAt": deadline.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
      "resumedAt": now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    let payload = RunEventPayload::WaitSatisfied(crate::WaitSatisfiedData {
      wait_id: wait_id.to_string(),
      winner: crate::DurableWaitWinner::Timer,
      result,
      resolved_at: now,
    });
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id.clone(),
      now,
      event_schema_version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let changed = transaction.execute(
      "UPDATE woml_durable_waits
       SET state = 'satisfied', resolution_event_id = ?1, resolved_at = ?2
       WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = '' AND state = 'registered'",
      params![event_id, now.to_rfc3339(), run_id, wait_id],
    )?;
    if changed != 1 {
      return Ok(TimerWaitSettlement::AlreadyResolved {
        projection: fold_events(&load_events(&transaction, run_id)?)?,
      });
    }
    let projection = fold_events(&events)?;
    if projection.status != RunStatus::Running || !projection.context.steps.contains_key(&node_id) {
      return Err(DurableStoreError::Contract(
        "Timer settlement did not publish the wait result into context.".to_string(),
      ));
    }
    transaction.commit()?;
    Ok(TimerWaitSettlement::Resolved { projection })
  }

  pub fn settle_signal_wait_timeout(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: DateTime<Utc>,
  ) -> Result<SignalWaitTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let Some(crate::CompiledWait::Signal { on_timeout, .. }) = workflow.durable_wait(wait_id)
    else {
      return Err(DurableStoreError::Contract(
        "Signal timeout references an unknown signal wait.".to_string(),
      ));
    };
    let row: Option<(Option<String>, String)> = transaction
      .query_row(
        "SELECT deadline, state FROM woml_durable_waits
         WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''",
        params![run_id, wait_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    let Some((deadline, state)) = row else {
      return Err(DurableStoreError::Contract(
        "Signal timeout references an unregistered wait.".to_string(),
      ));
    };
    let events = load_events(&transaction, run_id)?;
    if state != "registered" {
      return Ok(SignalWaitTimeoutSettlement::AlreadyResolved {
        projection: fold_events(&events)?,
      });
    }
    let deadline = deadline
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .ok_or_else(|| DurableStoreError::Contract("Signal wait has no timeout.".to_string()))?;
    if now < deadline {
      return Ok(SignalWaitTimeoutSettlement::NotDue { deadline });
    }
    let mut events = events;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| DurableStoreError::Contract("Signal wait has no run history.".to_string()))?;
    let behavior = if on_timeout == "continue" {
      crate::DurableWaitTimeoutBehavior::Continue
    } else {
      crate::DurableWaitTimeoutBehavior::Fail
    };
    let resolution_event_id = generated_event_id();
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      resolution_event_id.clone(),
      now,
      event_schema_version,
      RunEventPayload::WaitTimedOut(crate::WaitTimedOutData {
        wait_id: wait_id.to_string(),
        deadline,
        resolved_at: now,
        behavior,
      }),
    )?;
    if behavior == crate::DurableWaitTimeoutBehavior::Fail {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::RunOutcomeDecided(crate::RunOutcomeDecidedData::Failed {
          failure: crate::LifecycleFailure {
            kind: crate::LifecycleFailureKind::TimedOut,
            code: "WOML_WAIT_TIMED_OUT".to_string(),
            message: format!("Durable wait {wait_id:?} reached its signal timeout."),
          },
        }),
      )?;
      if let Some(hook) = workflow
        .lifecycle_hook_for_event(crate::model::LifecycleEventName::RunFailure)
        .or_else(|| {
          workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete)
        })
      {
        let subject = LifecycleSubject {
          kind: LifecycleSubjectKind::Workflow,
          id: run_id.to_string(),
        };
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
            hook_invocation_id: derive_lifecycle_hook_invocation_id(
              run_id,
              &hook.hook_id,
              subject.kind,
              &subject.id,
            ),
            hook_id: hook.hook_id.clone(),
            event: hook.event,
            subject,
          }),
        )?;
      }
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let changed = transaction.execute(
      "UPDATE woml_durable_waits
       SET state = 'timed_out', resolution_event_id = ?1, resolved_at = ?2
       WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = '' AND state = 'registered'",
      params![resolution_event_id, now.to_rfc3339(), run_id, wait_id],
    )?;
    if changed != 1 {
      return Ok(SignalWaitTimeoutSettlement::AlreadyResolved {
        projection: fold_events(&load_events(&transaction, run_id)?)?,
      });
    }
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(if behavior == crate::DurableWaitTimeoutBehavior::Continue {
      SignalWaitTimeoutSettlement::Continued { projection }
    } else {
      SignalWaitTimeoutSettlement::Failed { projection }
    })
  }
}
