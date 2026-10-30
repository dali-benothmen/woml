//! Occurrence deduplication, run admission, and internal-event delivery. The shared in-transaction helper borrows its caller transaction and never commits independently.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::InternalEventAdmissionOutcome;
use crate::durable::InternalEventAdmissionRequest;
use crate::durable::TriggerAdmissionOutcome;
use crate::durable::TriggerAdmissionRequest;
use crate::durable::TriggerOccurrence;
use crate::durable::{sha256_prefixed, DurableEventStore, RUNTIME_POLICY_QUEUE_CEILING};
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::event::is_definition_hash;
use crate::event::RunAdmissionQueue;
use crate::event::RunAdmissionTrigger;
use crate::event::RunAdmittedData;
use crate::event::RunStartedData;
use crate::run_event_schema_version_for_model;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RUN_EVENT_SCHEMA_VERSION_V11;
use crate::RUN_EVENT_SCHEMA_VERSION_V12;
use crate::RUN_EVENT_SCHEMA_VERSION_V13;
use crate::RUN_EVENT_SCHEMA_VERSION_V14;
use crate::RUN_EVENT_SCHEMA_VERSION_V7;
use crate::RUN_EVENT_SCHEMA_VERSION_V8;
use crate::RUN_EVENT_SCHEMA_VERSION_V9;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use serde_json::Map;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use uuid::Uuid;

pub(in crate::durable) fn validate_trigger_admission_request(
  request: &TriggerAdmissionRequest,
) -> Result<(), DurableStoreError> {
  if request.source_identity.is_empty() || request.source_identity.len() > 512 {
    return Err(DurableStoreError::Contract(
      "Trigger sourceIdentity must contain between 1 and 512 UTF-8 bytes.".to_string(),
    ));
  }
  if !is_definition_hash(&request.definition_hash) {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

pub(in crate::durable) fn admit_trigger_occurrence_in_transaction(
  transaction: &Transaction<'_>,
  request: &TriggerAdmissionRequest,
) -> Result<TriggerAdmissionOutcome, DurableStoreError> {
  let source_identity_hash = sha256_prefixed(request.source_identity.as_bytes());
  let payload_hash = canonical_payload_hash(&request.payload)?;
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&request.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(request.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V7
    || workflow.workflow_id != request.workflow_id
  {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger =
    workflow
      .trigger(&request.trigger_id)
      .ok_or_else(|| DurableStoreError::TriggerNotFound {
        workflow_id: request.workflow_id.clone(),
        trigger_id: request.trigger_id.clone(),
      })?;
  if trigger.handler != request.trigger_handler {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }

  let runtime_policy_identity =
    if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      let policy_hash = workflow.runtime_policy_hash().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy identity.".to_string())
      })?;
      let queue_name = workflow.runtime_policy_queue_name().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy queue.".to_string())
      })?;
      let conflict: bool = transaction.query_row(
        "SELECT EXISTS(
         SELECT 1
         FROM woml_runtime_policy_bindings AS bindings
         JOIN woml_run_summaries AS summaries ON summaries.run_id = bindings.run_id
         WHERE bindings.workflow_id = ?1
           AND summaries.status IN ('queued', 'running', 'waiting', 'cancelling', 'finalizing')
           AND (bindings.definition_hash != ?2 OR bindings.policy_hash != ?3)
       )",
        params![request.workflow_id, request.definition_hash, policy_hash],
        |row| row.get(0),
      )?;
      if conflict {
        return Err(DurableStoreError::RuntimePolicyDefinitionConflict(
          request.workflow_id.clone(),
        ));
      }
      Some((policy_hash, queue_name))
    } else {
      None
    };

  if let Some(existing) = load_trigger_occurrence_by_identity(
    transaction,
    &request.workflow_id,
    &request.trigger_id,
    &source_identity_hash,
  )? {
    if existing.payload_hash != payload_hash {
      return Err(DurableStoreError::TriggerIdempotencyConflict);
    }
    validate_trigger_occurrence_history(transaction, &existing)?;
    return Ok(TriggerAdmissionOutcome {
      occurrence_id: existing.occurrence_id,
      run_id: existing.run_id,
      duplicate: true,
    });
  }

  if runtime_policy_identity.is_some() {
    let queued: i64 = transaction.query_row(
      "SELECT COUNT(*) FROM woml_runtime_policy_queue",
      [],
      |row| row.get(0),
    )?;
    if queued >= RUNTIME_POLICY_QUEUE_CEILING {
      return Err(DurableStoreError::RuntimePolicyQueueFull);
    }
  }

  let occurrence_id = format!("occ_{}", Uuid::new_v4().simple());
  let run_id = format!("run_{}", Uuid::new_v4().simple());
  transaction.execute(
    "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
     VALUES (?1, ?2, ?3, ?4)",
    params![
      run_id,
      request.workflow_id,
      request.definition_hash,
      request.received_at.to_rfc3339(),
    ],
  )?;

  let payload = if let Some((policy_hash, queue_name)) = runtime_policy_identity {
    RunEventPayload::RunAdmitted(RunAdmittedData {
      definition_hash: request.definition_hash.clone(),
      policy_hash,
      trigger: RunAdmissionTrigger {
        id: request.trigger_id.clone(),
        handler: request.trigger_handler.clone(),
      },
      payload: request.payload.clone(),
      queue: RunAdmissionQueue {
        name: queue_name,
        discipline: crate::model::QueueDiscipline::WorkConservingFifo,
      },
      admitted_at: request.received_at,
      occurrence_sequence: 0,
    })
  } else {
    RunEventPayload::RunStarted(RunStartedData {
      workflow_id: request.workflow_id.clone(),
      definition_hash: request.definition_hash.clone(),
      trigger_id: Some(request.trigger_id.clone()),
      trigger_handler: Some(request.trigger_handler.clone()),
      trigger_occurrence_id: Some(occurrence_id.clone()),
      ingress: None,
      trigger: request.payload.clone(),
    })
  };
  validate_payload_against_definition(&workflow, &request.definition_hash, &payload)
    .map_err(DurableStoreError::Contract)?;
  let mut events = Vec::new();
  append_to_history(
    transaction,
    &mut events,
    &run_id,
    generated_event_id(),
    request.received_at,
    run_event_schema_version_for_model(workflow.schema_version),
    payload,
  )?;
  validate_event_history_against_definition(&workflow, &request.definition_hash, &events)
    .map_err(DurableStoreError::Contract)?;

  transaction.execute(
    "INSERT INTO woml_trigger_occurrences(
       occurrence_id, workflow_id, trigger_id, trigger_handler,
       definition_hash, source_identity_hash, payload_hash, received_at, run_id
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    params![
      occurrence_id,
      request.workflow_id,
      request.trigger_id,
      request.trigger_handler,
      request.definition_hash,
      source_identity_hash,
      payload_hash,
      request.received_at.to_rfc3339(),
      run_id,
    ],
  )?;
  Ok(TriggerAdmissionOutcome {
    occurrence_id,
    run_id,
    duplicate: false,
  })
}

type StoredTriggerOccurrence = (
  String,
  String,
  String,
  String,
  String,
  String,
  String,
  String,
  String,
);

fn trigger_occurrence_from_stored(
  stored: StoredTriggerOccurrence,
) -> Result<TriggerOccurrence, DurableStoreError> {
  let valid_id = |value: &str| !value.is_empty() && value.chars().count() <= 256;
  let valid_handler = matches!(
    stored.3.as_str(),
    "trigger.manual"
      | "trigger.webhook"
      | "trigger.slack"
      | "trigger.telegram"
      | "trigger.discord"
      | "trigger.whatsapp"
      | "trigger.schedule"
      | "trigger.interval"
      | "trigger.event"
  );
  if !valid_id(&stored.0)
    || !valid_id(&stored.1)
    || !valid_id(&stored.2)
    || !valid_handler
    || !is_definition_hash(&stored.4)
    || !is_definition_hash(&stored.5)
    || !is_definition_hash(&stored.6)
    || !valid_id(&stored.8)
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "trigger occurrence contains an invalid identity or hash".to_string(),
    ));
  }
  let received_at = DateTime::parse_from_rfc3339(&stored.7)
    .map_err(|_| {
      DurableStoreError::TriggerHistoryInvalid(
        "trigger occurrence receivedAt is not RFC 3339".to_string(),
      )
    })?
    .with_timezone(&Utc);
  Ok(TriggerOccurrence {
    occurrence_schema_version: 1,
    occurrence_id: stored.0,
    workflow_id: stored.1,
    trigger_id: stored.2,
    trigger_handler: stored.3,
    definition_hash: stored.4,
    source_identity_hash: stored.5,
    payload_hash: stored.6,
    received_at,
    run_id: stored.8,
  })
}

pub(in crate::durable) fn load_trigger_occurrence_by_id(
  connection: &Connection,
  occurrence_id: &str,
) -> Result<Option<TriggerOccurrence>, DurableStoreError> {
  let stored: Option<StoredTriggerOccurrence> = connection
    .query_row(
      "SELECT occurrence_id, workflow_id, trigger_id, trigger_handler,
              definition_hash, source_identity_hash, payload_hash, received_at, run_id
       FROM woml_trigger_occurrences WHERE occurrence_id = ?1",
      [occurrence_id],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
          row.get(7)?,
          row.get(8)?,
        ))
      },
    )
    .optional()?;
  stored.map(trigger_occurrence_from_stored).transpose()
}

fn load_trigger_occurrence_by_identity(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
  source_identity_hash: &str,
) -> Result<Option<TriggerOccurrence>, DurableStoreError> {
  let occurrence_id: Option<String> = connection
    .query_row(
      "SELECT occurrence_id FROM woml_trigger_occurrences
       WHERE workflow_id = ?1 AND trigger_id = ?2 AND source_identity_hash = ?3",
      params![workflow_id, trigger_id, source_identity_hash],
      |row| row.get(0),
    )
    .optional()?;
  occurrence_id
    .map(|occurrence_id| load_trigger_occurrence_by_id(connection, &occurrence_id))
    .transpose()
    .map(Option::flatten)
}

fn canonical_payload_hash(payload: &Map<String, Value>) -> Result<String, DurableStoreError> {
  let canonical = canonical_json(payload)?;
  Ok(sha256_prefixed(&canonical))
}

pub(in crate::durable) fn validate_trigger_occurrence_history(
  connection: &Connection,
  occurrence: &TriggerOccurrence,
) -> Result<Map<String, Value>, DurableStoreError> {
  let binding = run_binding_in_transaction(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence run binding is missing or invalid: {error}"
    ))
  })?;
  if binding.workflow_id != occurrence.workflow_id
    || binding.definition_hash != occurrence.definition_hash
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence does not match its immutable run binding".to_string(),
    ));
  }
  let workflow = definition_for_run(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence definition is missing or invalid: {error}"
    ))
  })?;
  let events = load_events(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence event history cannot be loaded: {error}"
    ))
  })?;
  validate_event_history_against_definition(&workflow, &occurrence.definition_hash, &events)
    .map_err(DurableStoreError::TriggerHistoryInvalid)?;
  let Some(first) = events.first() else {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence run has no admission event".to_string(),
    ));
  };
  let payload = match &first.payload {
    RunEventPayload::RunAdmitted(admission)
      if matches!(
        first.event_schema_version,
        RUN_EVENT_SCHEMA_VERSION_V11
          | RUN_EVENT_SCHEMA_VERSION_V12
          | RUN_EVENT_SCHEMA_VERSION_V13
          | RUN_EVENT_SCHEMA_VERSION_V14
          | crate::RUN_EVENT_SCHEMA_VERSION_V15
          | crate::RUN_EVENT_SCHEMA_VERSION_V16
          | crate::RUN_EVENT_SCHEMA_VERSION_V17
          | crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      ) && admission.definition_hash == occurrence.definition_hash
        && admission.trigger.id == occurrence.trigger_id
        && admission.trigger.handler == occurrence.trigger_handler
        && first.occurred_at == occurrence.received_at
        && admission.admitted_at == occurrence.received_at =>
    {
      &admission.payload
    }
    RunEventPayload::RunStarted(start)
      if matches!(
        first.event_schema_version,
        RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9
          | crate::RUN_EVENT_SCHEMA_VERSION_V10
      ) && start.workflow_id == occurrence.workflow_id
        && start.definition_hash == occurrence.definition_hash
        && start.trigger_id.as_deref() == Some(occurrence.trigger_id.as_str())
        && start.trigger_handler.as_deref() == Some(occurrence.trigger_handler.as_str())
        && start.trigger_occurrence_id.as_deref() == Some(occurrence.occurrence_id.as_str())
        && first.occurred_at == occurrence.received_at =>
    {
      &start.trigger
    }
    _ => {
      return Err(DurableStoreError::TriggerHistoryInvalid(
        "occurrence fields contradict its durable admission event".to_string(),
      ));
    }
  };
  if canonical_payload_hash(payload)? != occurrence.payload_hash {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence payload contradicts its durable admission event".to_string(),
    ));
  }
  Ok(payload.clone())
}

impl DurableEventStore {
  pub fn trigger_occurrence(
    &self,
    occurrence_id: &str,
  ) -> Result<TriggerOccurrence, DurableStoreError> {
    load_trigger_occurrence_by_id(&self.connection, occurrence_id)?.ok_or_else(|| {
      DurableStoreError::TriggerHistoryInvalid(format!(
        "trigger occurrence {occurrence_id:?} does not exist"
      ))
    })
  }

  pub fn admit_trigger_occurrence(
    &mut self,
    request: TriggerAdmissionRequest,
  ) -> Result<TriggerAdmissionOutcome, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.admit_trigger");
    span.count("transactions", 1);
    validate_trigger_admission_request(&request)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    transaction.commit()?;
    span.run_id(outcome.run_id.clone());
    span.count("duplicate", usize::from(outcome.duplicate));
    span.succeed();
    Ok(outcome)
  }

  /// Admits one subscriber delivery for a workflow-originated named event and
  /// stores its hidden lineage in the same transaction. Repeating the stable
  /// publication identity safely returns the original child run.
  pub fn admit_internal_event_occurrence(
    &mut self,
    request: InternalEventAdmissionRequest,
  ) -> Result<InternalEventAdmissionOutcome, DurableStoreError> {
    validate_trigger_admission_request(&request.trigger)?;
    if request.trigger.trigger_handler != "trigger.event"
      || !request.publication_id.starts_with("internal:v1:sha256:")
      || request.publication_id.len() != 83
      || request.event_name.is_empty()
      || request.event_name.len() > 256
    {
      return Err(DurableStoreError::Contract(
        "Internal event admission identity is invalid.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, &request.parent_run_id)?;
    let payload_hash = canonical_payload_hash(&request.trigger.payload)?;
    let parent_depth: Option<u32> = transaction
      .query_row(
        "SELECT publications.depth
         FROM woml_internal_event_deliveries AS deliveries
         JOIN woml_internal_event_publications AS publications
           ON publications.publication_id = deliveries.publication_id
         WHERE deliveries.run_id = ?1",
        [&request.parent_run_id],
        |row| row.get(0),
      )
      .optional()?;
    let depth = parent_depth.unwrap_or(0).saturating_add(1);
    if depth > 32 {
      return Err(DurableStoreError::InternalEventDepthExceeded);
    }

    let mut ancestor_run = Some(request.parent_run_id.clone());
    let mut inspected = 0;
    while let Some(run_id) = ancestor_run {
      inspected += 1;
      if inspected > 32 {
        return Err(DurableStoreError::InternalEventDepthExceeded);
      }
      let source: Option<(String, String)> = transaction
        .query_row(
          "SELECT workflow_id, trigger_id FROM woml_trigger_occurrences WHERE run_id = ?1",
          [&run_id],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
      if source.as_ref()
        == Some(&(
          request.trigger.workflow_id.clone(),
          request.trigger.trigger_id.clone(),
        ))
      {
        return Err(DurableStoreError::InternalEventCycle);
      }
      ancestor_run = transaction
        .query_row(
          "SELECT publications.parent_run_id
           FROM woml_internal_event_deliveries AS deliveries
           JOIN woml_internal_event_publications AS publications
             ON publications.publication_id = deliveries.publication_id
           WHERE deliveries.run_id = ?1",
          [&run_id],
          |row| row.get(0),
        )
        .optional()?;
    }

    let existing: Option<(String, String, String, u32)> = transaction
      .query_row(
        "SELECT parent_run_id, event_name, payload_hash, depth
         FROM woml_internal_event_publications WHERE publication_id = ?1",
        [&request.publication_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    if let Some(existing) = existing {
      if existing
        != (
          request.parent_run_id.clone(),
          request.event_name.clone(),
          payload_hash.clone(),
          depth,
        )
      {
        return Err(DurableStoreError::InternalEventIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_internal_event_publications(
           publication_id, parent_run_id, event_name, payload_hash, depth, emitted_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
          request.publication_id,
          request.parent_run_id,
          request.event_name,
          payload_hash,
          depth,
          request.emitted_at.to_rfc3339(),
        ],
      )?;
    }

    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request.trigger)?;
    let existing_delivery: Option<String> = transaction
      .query_row(
        "SELECT run_id FROM woml_internal_event_deliveries
         WHERE publication_id = ?1 AND workflow_id = ?2 AND trigger_id = ?3",
        params![
          request.publication_id,
          request.trigger.workflow_id,
          request.trigger.trigger_id,
        ],
        |row| row.get(0),
      )
      .optional()?;
    if let Some(run_id) = existing_delivery {
      if run_id != outcome.run_id {
        return Err(DurableStoreError::InternalEventIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_internal_event_deliveries(
           publication_id, workflow_id, trigger_id, run_id
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
          request.publication_id,
          request.trigger.workflow_id,
          request.trigger.trigger_id,
          outcome.run_id,
        ],
      )?;
    }
    transaction.commit()?;
    Ok(InternalEventAdmissionOutcome {
      depth,
      occurrence: outcome,
    })
  }
}
