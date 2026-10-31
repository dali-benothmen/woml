//! Correction read-model maintenance that preserves external-resolution evidence.
//!
//! Both ordinary writes and startup rebuilds retain externally_resolved operator
//! decisions. Those rows are mixed authority, not a wholly disposable event cache.

use crate::durable::{
  correction_invocation_id, load_events, DurableStoreError, CORRECTION_UNRESOLVED_STORE_LIMIT,
};
use crate::fold_events;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;

fn correction_action_kind_name(kind: crate::CorrectiveActionKind) -> &'static str {
  match kind {
    crate::CorrectiveActionKind::Compensation => "compensation",
    crate::CorrectiveActionKind::Cleanup => "cleanup",
  }
}

pub(in crate::durable) fn write_correction_indexes_from_projection(
  connection: &Connection,
  run_id: &str,
  definition_hash: &str,
  events: &[RunEvent],
  projection: &RunProjection,
) -> Result<(), DurableStoreError> {
  let externally_resolved = {
    let mut statement = connection.prepare(
      "SELECT saga_id, action_id, reason_code, created_at, resolved_at
       FROM woml_correction_manual_recovery
       WHERE run_id = ?1 AND status = 'externally_resolved'",
    )?;
    let rows = statement
      .query_map([run_id], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, Option<String>>(4)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  connection.execute(
    "DELETE FROM woml_correction_manual_recovery WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute(
    "DELETE FROM woml_correction_attempts WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute(
    "DELETE FROM woml_correction_actions WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute("DELETE FROM woml_saga_scopes WHERE run_id = ?1", [run_id])?;
  let updated_at = events
    .last()
    .map(|event| event.occurred_at)
    .unwrap_or_else(Utc::now)
    .to_rfc3339();
  for saga in projection.sagas.values() {
    let phase = match saga.phase {
      crate::SagaPhase::Forward => "forward",
      crate::SagaPhase::ForwardSucceeded => "forward_succeeded",
      crate::SagaPhase::Correcting => "correcting",
      crate::SagaPhase::CorrectionCompleted => "correction_completed",
      crate::SagaPhase::ManualRecovery => "manual_recovery",
    };
    let original_outcome = saga.reason.map(|reason| match reason {
      crate::SagaCorrectionReason::Success => "success",
      crate::SagaCorrectionReason::Failure => "failure",
      crate::SagaCorrectionReason::Cancel => "cancel",
    });
    connection.execute(
      "INSERT INTO woml_saga_scopes(
         run_id, saga_id, phase, original_outcome, started_sequence,
         committed_sequence, updated_at, model_digest
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
      params![
        run_id,
        saga.saga_id,
        phase,
        original_outcome,
        i64::try_from(saga.started_sequence).map_err(|_| DurableStoreError::Contract(
          "Saga sequence exceeds SQLite integer range.".to_string()
        ))?,
        saga
          .correction_completed_sequence
          .map(i64::try_from)
          .transpose()
          .map_err(|_| {
            DurableStoreError::Contract("Saga sequence exceeds SQLite integer range.".to_string())
          })?,
        updated_at,
        definition_hash,
      ],
    )?;
    for action in saga.actions.values() {
      let status = match action.status {
        crate::CorrectionActionStatus::Registered => "registered",
        crate::CorrectionActionStatus::Started => "started",
        crate::CorrectionActionStatus::RetryScheduled => "retry_scheduled",
        crate::CorrectionActionStatus::Succeeded => "succeeded",
        crate::CorrectionActionStatus::Failed => "failed",
        crate::CorrectionActionStatus::Interrupted => "interrupted",
        crate::CorrectionActionStatus::ManualRecovery => "manual_recovery",
      };
      connection.execute(
        "INSERT INTO woml_correction_actions(
           run_id, saga_id, action_id, action_kind, for_step_execution_id,
           completion_sequence, status, attempts, next_attempt_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
          run_id,
          saga.saga_id,
          action.action_id,
          correction_action_kind_name(action.action_kind),
          action.for_step_id,
          action
            .completion_sequence
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
              DurableStoreError::Contract(
                "Correction sequence exceeds SQLite integer range.".to_string(),
              )
            })?,
          status,
          i64::from(action.attempts),
          action.retry_at.map(|value| value.to_rfc3339()),
          updated_at,
        ],
      )?;
    }
  }
  for event in events {
    match &event.payload {
      RunEventPayload::CorrectionAttemptStarted(data) => {
        connection.execute(
          "INSERT INTO woml_correction_attempts(
             run_id, saga_id, action_id, attempt, invocation_id, status,
             started_at, finished_at, failure_code
           ) VALUES (?1, ?2, ?3, ?4, ?5, 'started', ?6, NULL, NULL)",
          params![
            run_id,
            data.saga_id,
            data.action_id,
            i64::from(data.attempt),
            correction_invocation_id(run_id, &data.saga_id, &data.action_id, data.attempt),
            event.occurred_at.to_rfc3339(),
          ],
        )?;
      }
      RunEventPayload::CorrectionAttemptSucceeded(data) => {
        connection.execute(
          "UPDATE woml_correction_attempts
           SET status = 'succeeded', finished_at = ?1
           WHERE run_id = ?2 AND saga_id = ?3 AND action_id = ?4 AND attempt = ?5",
          params![
            event.occurred_at.to_rfc3339(),
            run_id,
            data.saga_id,
            data.action_id,
            i64::from(data.attempt)
          ],
        )?;
      }
      RunEventPayload::CorrectionAttemptFailed(data) => {
        connection.execute(
          "UPDATE woml_correction_attempts
           SET status = 'failed', finished_at = ?1, failure_code = ?2
           WHERE run_id = ?3 AND saga_id = ?4 AND action_id = ?5 AND attempt = ?6",
          params![
            event.occurred_at.to_rfc3339(),
            data.code,
            run_id,
            data.saga_id,
            data.action_id,
            i64::from(data.attempt)
          ],
        )?;
      }
      RunEventPayload::CorrectionInterrupted(data) => {
        connection.execute(
          "UPDATE woml_correction_attempts
           SET status = 'interrupted', finished_at = ?1, failure_code = 'WOML_CORRECTION_INTERRUPTED'
           WHERE run_id = ?2 AND saga_id = ?3 AND action_id = ?4 AND attempt = ?5",
          params![event.occurred_at.to_rfc3339(), run_id, data.saga_id, data.action_id, i64::from(data.attempt)],
        )?;
      }
      RunEventPayload::ManualRecoveryRequired(data) => {
        let unresolved: i64 = connection.query_row(
          "SELECT COUNT(*) FROM woml_correction_manual_recovery WHERE status = 'required'",
          [],
          |row| row.get(0),
        )?;
        if unresolved >= i64::try_from(CORRECTION_UNRESOLVED_STORE_LIMIT).unwrap_or(i64::MAX) {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_LIMIT_EXCEEDED: the store reached 10000 unresolved recovery records."
              .to_string(),
          ));
        }
        connection.execute(
          "INSERT INTO woml_correction_manual_recovery(
             run_id, saga_id, action_id, status, reason_code, created_at, resolved_at
           ) VALUES (?1, ?2, ?3, 'required', ?4, ?5, NULL)",
          params![
            run_id,
            data.saga_id,
            data.action_id,
            data.code,
            event.occurred_at.to_rfc3339()
          ],
        )?;
      }
      RunEventPayload::CorrectionRetryScheduled(data) => {
        connection.execute(
          "DELETE FROM woml_correction_manual_recovery
           WHERE run_id = ?1 AND saga_id = ?2 AND action_id = ?3",
          params![run_id, data.saga_id, data.action_id],
        )?;
      }
      _ => {}
    }
  }
  for (saga_id, action_id, reason_code, created_at, resolved_at) in externally_resolved {
    connection.execute(
      "UPDATE woml_correction_manual_recovery
       SET status = 'externally_resolved', reason_code = ?1, created_at = ?2, resolved_at = ?3
       WHERE run_id = ?4 AND saga_id = ?5 AND action_id = ?6",
      params![
        reason_code,
        created_at,
        resolved_at,
        run_id,
        saga_id,
        action_id
      ],
    )?;
  }
  Ok(())
}

pub(in crate::durable) fn rebuild_correction_indexes(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  let externally_resolved = {
    let mut statement = connection.prepare(
      "SELECT run_id, saga_id, action_id, reason_code, created_at, resolved_at
       FROM woml_correction_manual_recovery
       WHERE status = 'externally_resolved'",
    )?;
    let rows = statement
      .query_map([], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, Option<String>>(5)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  connection.execute("DELETE FROM woml_correction_manual_recovery", [])?;
  connection.execute("DELETE FROM woml_correction_attempts", [])?;
  connection.execute("DELETE FROM woml_correction_actions", [])?;
  connection.execute("DELETE FROM woml_saga_scopes", [])?;
  let mut statement = connection
    .prepare("SELECT run_id, definition_hash FROM woml_runs ORDER BY created_at, run_id")?;
  let runs = statement
    .query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?
    .collect::<Result<Vec<_>, _>>()?;
  drop(statement);
  for (run_id, definition_hash) in runs {
    let events = load_events(connection, &run_id)?;
    if events.first().is_some_and(|event| {
      matches!(
        event.event_schema_version,
        crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      )
    }) {
      let projection = fold_events(&events)?;
      write_correction_indexes_from_projection(
        connection,
        &run_id,
        &definition_hash,
        &events,
        &projection,
      )?;
    }
  }
  for (run_id, saga_id, action_id, reason_code, created_at, resolved_at) in externally_resolved {
    connection.execute(
      "UPDATE woml_correction_manual_recovery
       SET status = 'externally_resolved', reason_code = ?1, created_at = ?2, resolved_at = ?3
       WHERE run_id = ?4 AND saga_id = ?5 AND action_id = ?6",
      params![
        reason_code,
        created_at,
        resolved_at,
        run_id,
        saga_id,
        action_id
      ],
    )?;
  }
  Ok(())
}
