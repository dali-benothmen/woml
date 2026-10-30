//! Schedule cursor registration and atomic occurrence/cursor advancement over the existing store owner.

use crate::durable::DurableStoreError;
use crate::durable::ScheduleCursor;
use crate::durable::ScheduleCursorRegistration;
use crate::durable::ScheduleCursorRegistrationOutcome;
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

fn validate_schedule_cursor_registration(
  registration: &ScheduleCursorRegistration,
) -> Result<(), DurableStoreError> {
  if registration.workflow_id.is_empty()
    || registration.trigger_id.is_empty()
    || registration.cron.is_empty()
    || registration.timezone.is_empty()
    || !matches!(registration.on_missed.as_str(), "skip" | "run-once")
    || !is_definition_hash(&registration.definition_hash)
  {
    return Err(DurableStoreError::Contract(
      "Schedule cursor registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn validate_schedule_registration_definition(
  transaction: &Transaction<'_>,
  registration: &ScheduleCursorRegistration,
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
  if trigger.handler != "trigger.schedule" {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }
  let crate::model::ValueExpression::Object { fields } = &trigger.config else {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  };
  let literal = |name: &str| match fields.get(name) {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  };
  if literal("cron") != Some(registration.cron.as_str())
    || literal("timezone") != Some(registration.timezone.as_str())
    || literal("onMissed") != Some(registration.on_missed.as_str())
  {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn load_schedule_cursor(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
) -> Result<Option<ScheduleCursor>, DurableStoreError> {
  let stored = connection
    .query_row(
      "SELECT workflow_id, trigger_id, definition_hash, cron, timezone, on_missed,
              next_scheduled_at, updated_at
       FROM woml_schedule_cursors
       WHERE workflow_id = ?1 AND trigger_id = ?2",
      params![workflow_id, trigger_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, String>(6)?,
          row.get::<_, String>(7)?,
        ))
      },
    )
    .optional()?;
  stored
    .map(|stored| {
      Ok(ScheduleCursor {
        workflow_id: stored.0,
        trigger_id: stored.1,
        definition_hash: stored.2,
        cron: stored.3,
        timezone: stored.4,
        on_missed: stored.5,
        next_scheduled_at: parse_durable_timestamp(&stored.6, "schedule next instant")?,
        updated_at: parse_durable_timestamp(&stored.7, "schedule update instant")?,
      })
    })
    .transpose()
}

fn update_schedule_cursor(
  transaction: &Transaction<'_>,
  workflow_id: &str,
  trigger_id: &str,
  expected_next_scheduled_at: DateTime<Utc>,
  next_scheduled_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  if next_scheduled_at <= expected_next_scheduled_at {
    return Err(DurableStoreError::Contract(
      "A schedule cursor must advance to a later planned instant.".to_string(),
    ));
  }
  let changed = transaction.execute(
    "UPDATE woml_schedule_cursors
     SET next_scheduled_at = ?1, updated_at = ?2
     WHERE workflow_id = ?3 AND trigger_id = ?4 AND next_scheduled_at = ?5",
    params![
      next_scheduled_at.to_rfc3339(),
      updated_at.to_rfc3339(),
      workflow_id,
      trigger_id,
      expected_next_scheduled_at.to_rfc3339(),
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::ScheduleCursorConflict);
  }
  Ok(())
}

impl DurableEventStore {
  pub fn register_schedule_cursor(
    &mut self,
    registration: &ScheduleCursorRegistration,
    initial_next_scheduled_at: DateTime<Utc>,
    registered_at: DateTime<Utc>,
  ) -> Result<ScheduleCursorRegistrationOutcome, DurableStoreError> {
    validate_schedule_cursor_registration(registration)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_schedule_registration_definition(&transaction, registration)?;
    let existing = load_schedule_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?;
    let initialized = existing.as_ref().is_none_or(|cursor| {
      cursor.definition_hash != registration.definition_hash
        || cursor.cron != registration.cron
        || cursor.timezone != registration.timezone
        || cursor.on_missed != registration.on_missed
    });
    if initialized {
      transaction.execute(
        "INSERT INTO woml_schedule_cursors(
           workflow_id, trigger_id, definition_hash, cron, timezone, on_missed,
           next_scheduled_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(workflow_id, trigger_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           cron = excluded.cron,
           timezone = excluded.timezone,
           on_missed = excluded.on_missed,
           next_scheduled_at = excluded.next_scheduled_at,
           updated_at = excluded.updated_at",
        params![
          registration.workflow_id,
          registration.trigger_id,
          registration.definition_hash,
          registration.cron,
          registration.timezone,
          registration.on_missed,
          initial_next_scheduled_at.to_rfc3339(),
          registered_at.to_rfc3339(),
        ],
      )?;
    }
    let cursor = load_schedule_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?
    .ok_or_else(|| DurableStoreError::Contract("Schedule cursor was not stored.".to_string()))?;
    transaction.commit()?;
    Ok(ScheduleCursorRegistrationOutcome {
      cursor,
      initialized,
    })
  }

  pub fn schedule_cursor(
    &self,
    workflow_id: &str,
    trigger_id: &str,
  ) -> Result<ScheduleCursor, DurableStoreError> {
    load_schedule_cursor(&self.connection, workflow_id, trigger_id)?.ok_or_else(|| {
      DurableStoreError::Contract(format!(
        "Schedule cursor for workflow {workflow_id:?}, trigger {trigger_id:?} does not exist."
      ))
    })
  }

  pub fn advance_schedule_cursor(
    &mut self,
    workflow_id: &str,
    trigger_id: &str,
    expected_next_scheduled_at: DateTime<Utc>,
    next_scheduled_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
  ) -> Result<ScheduleCursor, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    update_schedule_cursor(
      &transaction,
      workflow_id,
      trigger_id,
      expected_next_scheduled_at,
      next_scheduled_at,
      updated_at,
    )?;
    let cursor = load_schedule_cursor(&transaction, workflow_id, trigger_id)?
      .ok_or(DurableStoreError::ScheduleCursorConflict)?;
    transaction.commit()?;
    Ok(cursor)
  }

  pub fn claim_schedule_occurrence(
    &mut self,
    expected_next_scheduled_at: DateTime<Utc>,
    next_scheduled_at: DateTime<Utc>,
    request: TriggerAdmissionRequest,
  ) -> Result<(TriggerAdmissionOutcome, ScheduleCursor), DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    if request.trigger_handler != "trigger.schedule" {
      return Err(DurableStoreError::TriggerHandlerMismatch);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    update_schedule_cursor(
      &transaction,
      &request.workflow_id,
      &request.trigger_id,
      expected_next_scheduled_at,
      next_scheduled_at,
      request.received_at,
    )?;
    let cursor = load_schedule_cursor(&transaction, &request.workflow_id, &request.trigger_id)?
      .ok_or(DurableStoreError::ScheduleCursorConflict)?;
    transaction.commit()?;
    Ok((outcome, cursor))
  }
}
