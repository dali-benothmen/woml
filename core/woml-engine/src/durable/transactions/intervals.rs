//! Fixed-grid interval cursor registration and atomic occurrence/cursor advancement.

use crate::durable::DurableStoreError;
use crate::durable::IntervalCursor;
use crate::durable::IntervalCursorRegistration;
use crate::durable::IntervalCursorRegistrationOutcome;
use crate::durable::TriggerAdmissionOutcome;
use crate::durable::TriggerAdmissionRequest;
use crate::durable::{
  admit_trigger_occurrence_in_transaction, parse_durable_timestamp,
  validate_trigger_admission_request, DurableEventStore,
};
use crate::event::is_definition_hash;
use crate::CompiledWorkflowDefinition;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;

fn validate_interval_cursor_registration(
  registration: &IntervalCursorRegistration,
) -> Result<(), DurableStoreError> {
  if registration.workflow_id.is_empty()
    || registration.trigger_id.is_empty()
    || !matches!(registration.on_missed.as_str(), "skip" | "run-once")
    || !is_definition_hash(&registration.definition_hash)
    || crate::interval::WomlInterval::new(registration.every_ms).is_err()
  {
    return Err(DurableStoreError::Contract(
      "Interval cursor registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn validate_interval_registration_definition(
  transaction: &Transaction<'_>,
  registration: &IntervalCursorRegistration,
) -> Result<(), DurableStoreError> {
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&registration.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(registration.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  if workflow.workflow_id != registration.workflow_id {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger = workflow.trigger(&registration.trigger_id).ok_or_else(|| {
    DurableStoreError::TriggerNotFound {
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
    }
  })?;
  if trigger.handler != "trigger.interval" {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }
  let crate::model::ValueExpression::Object { fields } = &trigger.config else {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  };
  let every_ms = match fields.get("everyMs") {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_u64(),
    _ => None,
  };
  let on_missed = match fields.get("onMissed") {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  };
  if every_ms != Some(registration.every_ms) || on_missed != Some(registration.on_missed.as_str()) {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn load_interval_cursor(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
) -> Result<Option<IntervalCursor>, DurableStoreError> {
  let stored = connection
    .query_row(
      "SELECT workflow_id, trigger_id, definition_hash, every_ms, on_missed,
              anchor_at, next_sequence, next_scheduled_at, updated_at
       FROM woml_interval_cursors
       WHERE workflow_id = ?1 AND trigger_id = ?2",
      params![workflow_id, trigger_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, i64>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, i64>(6)?,
          row.get::<_, String>(7)?,
          row.get::<_, String>(8)?,
        ))
      },
    )
    .optional()?;
  stored
    .map(|stored| {
      Ok(IntervalCursor {
        workflow_id: stored.0,
        trigger_id: stored.1,
        definition_hash: stored.2,
        every_ms: u64::try_from(stored.3).map_err(|_| {
          DurableStoreError::Contract("Stored interval duration is invalid.".to_string())
        })?,
        on_missed: stored.4,
        anchor_at: parse_durable_timestamp(&stored.5, "interval anchor")?,
        next_sequence: u64::try_from(stored.6).map_err(|_| {
          DurableStoreError::Contract("Stored interval sequence is invalid.".to_string())
        })?,
        next_scheduled_at: parse_durable_timestamp(&stored.7, "interval next instant")?,
        updated_at: parse_durable_timestamp(&stored.8, "interval update instant")?,
      })
    })
    .transpose()
}

#[allow(clippy::too_many_arguments)]
fn update_interval_cursor(
  transaction: &Transaction<'_>,
  workflow_id: &str,
  trigger_id: &str,
  expected_sequence: u64,
  expected_scheduled_at: DateTime<Utc>,
  next_sequence: u64,
  next_scheduled_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  if next_sequence <= expected_sequence || next_scheduled_at <= expected_scheduled_at {
    return Err(DurableStoreError::Contract(
      "An interval cursor must advance to a later sequence and planned instant.".to_string(),
    ));
  }
  let expected_sequence = i64::try_from(expected_sequence).map_err(|_| {
    DurableStoreError::Contract("Interval sequence exceeds the durable integer range.".to_string())
  })?;
  let next_sequence = i64::try_from(next_sequence).map_err(|_| {
    DurableStoreError::Contract("Interval sequence exceeds the durable integer range.".to_string())
  })?;
  let changed = transaction.execute(
    "UPDATE woml_interval_cursors
     SET next_sequence = ?1, next_scheduled_at = ?2, updated_at = ?3
     WHERE workflow_id = ?4 AND trigger_id = ?5
       AND next_sequence = ?6 AND next_scheduled_at = ?7",
    params![
      next_sequence,
      next_scheduled_at.to_rfc3339(),
      updated_at.to_rfc3339(),
      workflow_id,
      trigger_id,
      expected_sequence,
      expected_scheduled_at.to_rfc3339(),
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::IntervalCursorConflict);
  }
  Ok(())
}

impl DurableEventStore {
  pub fn register_interval_cursor(
    &mut self,
    registration: &IntervalCursorRegistration,
    anchor_at: DateTime<Utc>,
    registered_at: DateTime<Utc>,
  ) -> Result<IntervalCursorRegistrationOutcome, DurableStoreError> {
    validate_interval_cursor_registration(registration)?;
    let interval = crate::interval::WomlInterval::new(registration.every_ms)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let anchor_at = crate::interval::WomlInterval::normalize_anchor(anchor_at)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let initial_next_scheduled_at = interval
      .planned_at(anchor_at, 1)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_interval_registration_definition(&transaction, registration)?;
    let existing = load_interval_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?;
    let initialized = existing.as_ref().is_none_or(|cursor| {
      cursor.definition_hash != registration.definition_hash
        || cursor.every_ms != registration.every_ms
        || cursor.on_missed != registration.on_missed
    });
    if initialized {
      transaction.execute(
        "INSERT INTO woml_interval_cursors(
           workflow_id, trigger_id, definition_hash, every_ms, on_missed,
           anchor_at, next_sequence, next_scheduled_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
         ON CONFLICT(workflow_id, trigger_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           every_ms = excluded.every_ms,
           on_missed = excluded.on_missed,
           anchor_at = excluded.anchor_at,
           next_sequence = excluded.next_sequence,
           next_scheduled_at = excluded.next_scheduled_at,
           updated_at = excluded.updated_at",
        params![
          registration.workflow_id,
          registration.trigger_id,
          registration.definition_hash,
          i64::try_from(registration.every_ms).map_err(|_| DurableStoreError::Contract(
            "Interval duration exceeds the durable integer range.".to_string()
          ))?,
          registration.on_missed,
          anchor_at.to_rfc3339(),
          initial_next_scheduled_at.to_rfc3339(),
          registered_at.to_rfc3339(),
        ],
      )?;
    }
    let cursor = load_interval_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?
    .ok_or_else(|| DurableStoreError::Contract("Interval cursor was not stored.".to_string()))?;
    transaction.commit()?;
    Ok(IntervalCursorRegistrationOutcome {
      cursor,
      initialized,
    })
  }

  pub fn interval_cursor(
    &self,
    workflow_id: &str,
    trigger_id: &str,
  ) -> Result<IntervalCursor, DurableStoreError> {
    load_interval_cursor(&self.connection, workflow_id, trigger_id)?.ok_or_else(|| {
      DurableStoreError::Contract(format!(
        "Interval cursor for workflow {workflow_id:?}, trigger {trigger_id:?} does not exist."
      ))
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub fn advance_interval_cursor(
    &mut self,
    workflow_id: &str,
    trigger_id: &str,
    expected_sequence: u64,
    expected_scheduled_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
  ) -> Result<IntervalCursor, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    update_interval_cursor(
      &transaction,
      workflow_id,
      trigger_id,
      expected_sequence,
      expected_scheduled_at,
      next_sequence,
      next_scheduled_at,
      updated_at,
    )?;
    let cursor = load_interval_cursor(&transaction, workflow_id, trigger_id)?
      .ok_or(DurableStoreError::IntervalCursorConflict)?;
    transaction.commit()?;
    Ok(cursor)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn claim_interval_occurrence(
    &mut self,
    expected_sequence: u64,
    expected_scheduled_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    request: TriggerAdmissionRequest,
  ) -> Result<(TriggerAdmissionOutcome, IntervalCursor), DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    if request.trigger_handler != "trigger.interval" {
      return Err(DurableStoreError::TriggerHandlerMismatch);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    update_interval_cursor(
      &transaction,
      &request.workflow_id,
      &request.trigger_id,
      expected_sequence,
      expected_scheduled_at,
      next_sequence,
      next_scheduled_at,
      request.received_at,
    )?;
    let cursor = load_interval_cursor(&transaction, &request.workflow_id, &request.trigger_id)?
      .ok_or(DurableStoreError::IntervalCursorConflict)?;
    transaction.commit()?;
    Ok((outcome, cursor))
  }
}
