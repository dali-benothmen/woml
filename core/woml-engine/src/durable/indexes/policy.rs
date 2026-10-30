//! Event-derived queue/start/binding indexes and startup rebuild detection.
//!
//! The existing caller controls transaction boundaries. These indexes are not
//! independent live scheduler ownership or permission to replay external work.

use crate::durable::{
  load_events, rebuild_run_summary_index, DurableEventStore, DurableStoreError,
};
use crate::event::RunAdmittedData;
use crate::fold_events;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::TransactionBehavior;

pub(in crate::durable) fn runtime_policy_indexes_need_rebuild(
  connection: &Connection,
) -> Result<bool, DurableStoreError> {
  let count = |sql: &str| -> Result<i64, DurableStoreError> {
    Ok(connection.query_row(sql, [], |row| row.get(0))?)
  };
  Ok(
    count("SELECT COUNT(*) FROM woml_runs")? != count("SELECT COUNT(*) FROM woml_run_summaries")?
      || count(
        "SELECT COUNT(*) FROM woml_run_events
         WHERE sequence = 1 AND event_schema_version = 11",
      )? != count("SELECT COUNT(*) FROM woml_runtime_policy_bindings")?
      || count("SELECT COUNT(*) FROM woml_run_summaries WHERE status = 'queued'")?
        != count("SELECT COUNT(*) FROM woml_runtime_policy_queue")?
      || count(
        "SELECT COUNT(*) FROM woml_run_summaries
         WHERE queue_name IS NOT NULL AND started_at IS NOT NULL",
      )? != count("SELECT COUNT(*) FROM woml_runtime_policy_starts")?,
  )
}

pub(in crate::durable) fn write_runtime_policy_indexes_for_run(
  connection: &Connection,
  events: &[RunEvent],
) -> Result<(), DurableStoreError> {
  let Some(RunEvent {
    run_id,
    payload: RunEventPayload::RunAdmitted(admission),
    ..
  }) = events.first()
  else {
    return Ok(());
  };
  let projection = fold_events(events)?;
  write_runtime_policy_indexes_from_projection(connection, run_id, admission, &projection)
}

pub(in crate::durable) fn write_runtime_policy_indexes_from_projection(
  connection: &Connection,
  run_id: &str,
  admission: &RunAdmittedData,
  projection: &RunProjection,
) -> Result<(), DurableStoreError> {
  let workflow_id: String = connection.query_row(
    "SELECT workflow_id FROM woml_runs WHERE run_id = ?1",
    [run_id],
    |row| row.get(0),
  )?;
  let occurrence_sequence = i64::try_from(admission.occurrence_sequence).map_err(|_| {
    DurableStoreError::Contract("occurrenceSequence exceeds SQLite integer range".to_string())
  })?;
  connection.execute(
    "INSERT INTO woml_runtime_policy_bindings(
       run_id, workflow_id, definition_hash, policy_hash, queue_name, admitted_at,
       occurrence_sequence
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
     ON CONFLICT(run_id) DO UPDATE SET
       workflow_id = excluded.workflow_id,
       definition_hash = excluded.definition_hash,
       policy_hash = excluded.policy_hash,
       queue_name = excluded.queue_name,
       admitted_at = excluded.admitted_at,
       occurrence_sequence = excluded.occurrence_sequence",
    params![
      run_id,
      workflow_id,
      admission.definition_hash,
      admission.policy_hash,
      admission.queue.name,
      admission.admitted_at.to_rfc3339(),
      occurrence_sequence,
    ],
  )?;
  connection.execute(
    "DELETE FROM woml_runtime_policy_queue WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute(
    "DELETE FROM woml_runtime_policy_starts WHERE run_id = ?1",
    [run_id],
  )?;
  if projection.status == RunStatus::Queued {
    connection.execute(
      "INSERT INTO woml_runtime_policy_queue(
         run_id, workflow_id, queue_name, admitted_at, occurrence_sequence
       ) VALUES (?1, ?2, ?3, ?4, ?5)",
      params![
        run_id,
        workflow_id,
        admission.queue.name,
        admission.admitted_at.to_rfc3339(),
        occurrence_sequence,
      ],
    )?;
  }
  if let Some(started_at) = projection.started_at {
    connection.execute(
      "INSERT INTO woml_runtime_policy_starts(run_id, workflow_id, started_at)
       VALUES (?1, ?2, ?3)",
      params![run_id, workflow_id, started_at.to_rfc3339()],
    )?;
  }
  Ok(())
}

pub(in crate::durable) fn rebuild_runtime_policy_indexes(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  connection.execute("DELETE FROM woml_runtime_policy_queue", [])?;
  connection.execute("DELETE FROM woml_runtime_policy_starts", [])?;
  connection.execute("DELETE FROM woml_runtime_policy_bindings", [])?;
  let run_ids = {
    let mut statement = connection.prepare("SELECT run_id FROM woml_runs ORDER BY run_id")?;
    let rows = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  for run_id in run_ids {
    let events = load_events(connection, &run_id)?;
    write_runtime_policy_indexes_for_run(connection, &events)?;
  }
  Ok(())
}

impl DurableEventStore {
  pub fn rebuild_runtime_policy_indexes(&mut self) -> Result<(), DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    rebuild_runtime_policy_indexes(&transaction)?;
    rebuild_run_summary_index(&transaction)?;
    transaction.commit()?;
    Ok(())
  }
}
