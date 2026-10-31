//! Event-derived run-summary writes and transactional full-index rebuilding.

use crate::durable::{load_events, DurableEventStore, DurableStoreError, PublicRunStatus};
use crate::fold_events;
use crate::RunEvent;
use crate::RunProjection;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::TransactionBehavior;

pub(in crate::durable) fn write_run_summary(
  connection: &Connection,
  events: &[RunEvent],
) -> Result<(), DurableStoreError> {
  let Some(first) = events.first() else {
    return Ok(());
  };
  let projection = fold_events(events)?;
  write_run_summary_from_projection(
    connection,
    first,
    events.last().unwrap().occurred_at,
    &projection,
  )
}

pub(in crate::durable) fn write_run_summary_from_projection(
  connection: &Connection,
  first: &RunEvent,
  updated_at: DateTime<Utc>,
  projection: &RunProjection,
) -> Result<(), DurableStoreError> {
  let workflow_id: String = connection.query_row(
    "SELECT workflow_id FROM woml_runs WHERE run_id = ?1",
    [&first.run_id],
    |row| row.get(0),
  )?;
  connection.execute(
    "INSERT INTO woml_run_summaries(
       run_id, workflow_id, status, admitted_at, started_at, updated_at,
       queue_name, waiting_for, eligible_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)
     ON CONFLICT(run_id) DO UPDATE SET
       workflow_id = excluded.workflow_id,
       status = excluded.status,
       admitted_at = excluded.admitted_at,
       started_at = excluded.started_at,
       updated_at = excluded.updated_at,
       queue_name = excluded.queue_name",
    params![
      first.run_id,
      workflow_id,
      PublicRunStatus::from(projection.status).as_str(),
      first.occurred_at.to_rfc3339(),
      projection.started_at.map(|value| value.to_rfc3339()),
      updated_at.to_rfc3339(),
      projection.queue,
    ],
  )?;
  Ok(())
}

pub(in crate::durable) fn rebuild_run_summary_index(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  connection.execute("DELETE FROM woml_run_summaries", [])?;
  let run_ids = {
    let mut statement = connection.prepare("SELECT run_id FROM woml_runs ORDER BY run_id")?;
    let rows = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  for run_id in run_ids {
    let events = load_events(connection, &run_id)?;
    write_run_summary(connection, &events)?;
  }
  Ok(())
}

impl DurableEventStore {
  pub fn rebuild_run_summaries(&mut self) -> Result<(), DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    rebuild_run_summary_index(&transaction)?;
    transaction.commit()?;
    Ok(())
  }
}
