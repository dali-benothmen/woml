//! Integrity-checked history loading and append helpers on caller-owned transactions.
//!
//! These helpers never commit independently. Indexed envelope identity, scoped
//! validation, compatibility expansion, fold, and row-write order remain intact.

use crate::durable::{
  definition_for_run, expand_model_v11_payload, generated_event_id, write_run_summary,
  write_run_summary_from_projection, write_runtime_policy_indexes_for_run,
  write_runtime_policy_indexes_from_projection, DurableEventStore, DurableStoreError,
};
use crate::fold_events;
use crate::projection::ProjectionFoldState;
use crate::RunEvent;
use crate::RunEventPayload;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::Transaction;

pub(in crate::durable) fn ensure_run_exists(
  connection: &Connection,
  run_id: &str,
) -> Result<(), DurableStoreError> {
  let exists: bool = connection.query_row(
    "SELECT EXISTS(SELECT 1 FROM woml_runs WHERE run_id = ?1)",
    [run_id],
    |row| row.get(0),
  )?;
  if !exists {
    return Err(DurableStoreError::RunNotFound(run_id.to_string()));
  }
  Ok(())
}

pub(in crate::durable) fn load_events(
  connection: &Connection,
  run_id: &str,
) -> Result<Vec<RunEvent>, DurableStoreError> {
  let mut statement = connection.prepare_cached(
    "SELECT sequence, event_schema_version, event_json
     FROM woml_run_events WHERE run_id = ?1 ORDER BY sequence",
  )?;
  let mut rows = statement.query([run_id])?;
  let mut events = Vec::new();
  while let Some(row) = rows.next()? {
    let stored_sequence: i64 = row.get(0)?;
    let stored_schema_version: i64 = row.get(1)?;
    let event_json: String = row.get(2)?;
    let event: RunEvent = serde_json::from_str(&event_json)?;
    if i64::try_from(event.sequence).ok() != Some(stored_sequence)
      || i64::from(event.event_schema_version) != stored_schema_version
      || event.run_id != run_id
    {
      return Err(DurableStoreError::InvalidStoredEvent(
        "indexed event columns do not match event_json".to_string(),
      ));
    }
    event
      .validate()
      .map_err(|error| DurableStoreError::InvalidStoredEvent(error.to_string()))?;
    events.push(event);
  }
  Ok(events)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::durable) fn append_to_history_scoped_incremental(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  state: &mut ProjectionFoldState,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
  update_read_models: bool,
) -> Result<RunEvent, DurableStoreError> {
  let payloads = if matches!(
    event_schema_version,
    crate::RUN_EVENT_SCHEMA_VERSION_V10
      | crate::RUN_EVENT_SCHEMA_VERSION_V11
      | crate::RUN_EVENT_SCHEMA_VERSION_V12
      | crate::RUN_EVENT_SCHEMA_VERSION_V13
      | crate::RUN_EVENT_SCHEMA_VERSION_V14
      | crate::RUN_EVENT_SCHEMA_VERSION_V15
      | crate::RUN_EVENT_SCHEMA_VERSION_V16
      | crate::RUN_EVENT_SCHEMA_VERSION_V17
      | crate::RUN_EVENT_SCHEMA_VERSION_V18
      | crate::RUN_EVENT_SCHEMA_VERSION_V19
      | crate::RUN_EVENT_SCHEMA_VERSION_V20
  ) && matches!(
    payload,
    RunEventPayload::RunSucceeded(_) | RunEventPayload::RunFailed(_)
  ) {
    expand_model_v11_payload(&definition_for_run(transaction, run_id)?, run_id, payload)?
  } else {
    vec![payload]
  };
  let mut first = None;
  for (index, payload) in payloads.into_iter().enumerate() {
    let event = append_single_to_history_incremental(
      transaction,
      events,
      state,
      run_id,
      if index == 0 {
        event_id.clone()
      } else {
        generated_event_id()
      },
      occurred_at,
      event_schema_version,
      iteration.clone(),
      payload,
      update_read_models,
    )?;
    first.get_or_insert(event);
  }
  Ok(first.unwrap())
}

#[allow(clippy::too_many_arguments)]
fn append_single_to_history_incremental(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  state: &mut ProjectionFoldState,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
  update_read_models: bool,
) -> Result<RunEvent, DurableStoreError> {
  let sequence = events.len() as u64 + 1;
  let event = RunEvent {
    event_schema_version,
    event_id,
    run_id: run_id.to_string(),
    sequence,
    occurred_at,
    iteration,
    payload,
  };
  state.apply(&event)?;
  let event_json = serde_json::to_string(&event)?;
  let stored_sequence = i64::try_from(sequence).map_err(|_| {
    DurableStoreError::Contract("event sequence exceeds SQLite integer range".to_string())
  })?;
  transaction.execute(
    "INSERT INTO woml_run_events(
       run_id, sequence, event_id, event_schema_version, event_json
     ) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![
      run_id,
      stored_sequence,
      event.event_id,
      i64::from(event.event_schema_version),
      event_json,
    ],
  )?;
  events.push(event.clone());
  if update_read_models {
    write_run_summary_from_projection(
      transaction,
      events.first().unwrap(),
      event.occurred_at,
      state.projection(),
    )?;
    if let RunEventPayload::RunAdmitted(admission) = &events.first().unwrap().payload {
      write_runtime_policy_indexes_from_projection(
        transaction,
        run_id,
        admission,
        state.projection(),
      )?;
    }
  }
  Ok(event)
}

pub(in crate::durable) fn append_to_history(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  append_to_history_scoped(
    transaction,
    events,
    run_id,
    event_id,
    occurred_at,
    event_schema_version,
    None,
    payload,
  )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::durable) fn append_to_history_scoped(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  let payloads = if matches!(
    event_schema_version,
    crate::RUN_EVENT_SCHEMA_VERSION_V10
      | crate::RUN_EVENT_SCHEMA_VERSION_V11
      | crate::RUN_EVENT_SCHEMA_VERSION_V12
      | crate::RUN_EVENT_SCHEMA_VERSION_V13
      | crate::RUN_EVENT_SCHEMA_VERSION_V14
      | crate::RUN_EVENT_SCHEMA_VERSION_V15
      | crate::RUN_EVENT_SCHEMA_VERSION_V16
      | crate::RUN_EVENT_SCHEMA_VERSION_V17
      | crate::RUN_EVENT_SCHEMA_VERSION_V18
      | crate::RUN_EVENT_SCHEMA_VERSION_V19
      | crate::RUN_EVENT_SCHEMA_VERSION_V20
  ) && matches!(
    payload,
    RunEventPayload::RunSucceeded(_) | RunEventPayload::RunFailed(_)
  ) {
    expand_model_v11_payload(&definition_for_run(transaction, run_id)?, run_id, payload)?
  } else {
    vec![payload]
  };
  let mut first = None;
  for (index, payload) in payloads.into_iter().enumerate() {
    let event = append_single_to_history(
      transaction,
      events,
      run_id,
      if index == 0 {
        event_id.clone()
      } else {
        generated_event_id()
      },
      occurred_at,
      event_schema_version,
      iteration.clone(),
      payload,
    )?;
    first.get_or_insert(event);
  }
  Ok(first.unwrap())
}

fn append_single_to_history(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  let sequence = events.len() as u64 + 1;
  let event = RunEvent {
    event_schema_version,
    event_id,
    run_id: run_id.to_string(),
    sequence,
    occurred_at,
    iteration,
    payload,
  };
  let mut candidate = events.clone();
  candidate.push(event.clone());
  fold_events(&candidate)?;
  let event_json = serde_json::to_string(&event)?;
  let stored_sequence = i64::try_from(sequence).map_err(|_| {
    DurableStoreError::Contract("event sequence exceeds SQLite integer range".to_string())
  })?;
  transaction.execute(
    "INSERT INTO woml_run_events(
       run_id, sequence, event_id, event_schema_version, event_json
     ) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![
      run_id,
      stored_sequence,
      event.event_id,
      i64::from(event.event_schema_version),
      event_json,
    ],
  )?;
  events.push(event.clone());
  write_run_summary(transaction, events)?;
  write_runtime_policy_indexes_for_run(transaction, events)?;
  Ok(event)
}

impl DurableEventStore {
  pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, DurableStoreError> {
    self.run_binding(run_id)?;
    load_events(&self.connection, run_id)
  }

  /// Count a run's durable events without allocating or decoding its history.
  /// Read models use this to reject pathological histories before projection.
  pub fn event_count(&self, run_id: &str) -> Result<usize, DurableStoreError> {
    self.run_binding(run_id)?;
    let mut statement = self
      .connection
      .prepare_cached("SELECT COUNT(*) FROM woml_run_events WHERE run_id = ?1")?;
    let count: i64 = statement.query_row([run_id], |row| row.get(0))?;
    usize::try_from(count)
      .map_err(|_| DurableStoreError::Contract("run event count is invalid".to_string()))
  }
}
