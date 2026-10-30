//! Run creation and atomic event-batch persistence on the existing store owner.
//!
//! The immediate transaction covers validation, sequencing, event rows, summaries,
//! and relevant indexes. Publish candidate cache state only after commit succeeds.

use crate::durable::{
  append_to_history_scoped_incremental, cache_projection_state, definition_for_run,
  ensure_run_exists, expand_model_v11_payload, load_events, projection_state_for_events,
  run_binding_in_transaction, write_correction_indexes_from_projection,
  write_run_summary_from_projection, write_runtime_policy_indexes_from_projection,
  DurableEventStore, DurableStoreError,
};
use crate::engine::ready_node_ids_for_projection;
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::engine::validate_scoped_payload_against_definition;
use crate::event::RunStartedData;
use crate::run_event_schema_version_for_model;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RUN_EVENT_SCHEMA_VERSION_V11;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::Map;
use serde_json::Value;
use uuid::Uuid;

pub(in crate::durable) fn generated_event_id() -> String {
  format!("evt_{}", Uuid::new_v4().simple())
}

impl DurableEventStore {
  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    workflow_id: impl Into<String>,
    definition_hash: impl Into<String>,
    trigger: Map<String, Value>,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    let workflow_id = workflow_id.into();
    let definition_hash = definition_hash.into();
    self.append_payload(
      run_id.into(),
      event_id.into(),
      occurred_at,
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id,
        definition_hash,
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        ingress: None,
        trigger,
      }),
    )
  }

  pub fn append_payload(
    &mut self,
    run_id: impl Into<String>,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    payload: RunEventPayload,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    self.append_payload_scoped(run_id, event_id, occurred_at, None, payload)
  }

  pub fn append_payload_scoped(
    &mut self,
    run_id: impl Into<String>,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    iteration: Option<crate::event::ForEachIterationScope>,
    payload: RunEventPayload,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    let run_id = run_id.into();
    let event_id = event_id.into();
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.append_events");
    span.run_id(run_id.clone());
    span.count("transactions", 1);
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;

    let event_schema_version = match &payload {
      RunEventPayload::RunAdmitted(data) => {
        let existing: Option<String> = transaction
          .query_row(
            "SELECT run_id FROM woml_runs WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
          )
          .optional()?;
        if existing.is_some() {
          return Err(DurableStoreError::RunAlreadyExists(run_id));
        }
        let registered: Option<(String, i64)> = transaction
          .query_row(
            "SELECT workflow_id, schema_version FROM woml_definitions WHERE definition_hash = ?1",
            [&data.definition_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
          )
          .optional()?;
        let (workflow_id, model_schema_version) = registered
          .ok_or_else(|| DurableStoreError::DefinitionNotFound(data.definition_hash.clone()))?;
        let model_schema_version = u32::try_from(model_schema_version).map_err(|_| {
          DurableStoreError::Contract(
            "Stored compiled-model schema version is invalid.".to_string(),
          )
        })?;
        let event_schema_version = run_event_schema_version_for_model(model_schema_version);
        if event_schema_version < RUN_EVENT_SCHEMA_VERSION_V11 {
          return Err(DurableStoreError::Contract(
            "run_admitted requires compiled Model v12 or later.".to_string(),
          ));
        }
        transaction.execute(
          "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
           VALUES (?1, ?2, ?3, ?4)",
          params![
            run_id,
            workflow_id,
            data.definition_hash,
            occurred_at.to_rfc3339(),
          ],
        )?;
        event_schema_version
      }
      RunEventPayload::RunStarted(data) => {
        let existing: Option<String> = transaction
          .query_row(
            "SELECT run_id FROM woml_runs WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
          )
          .optional()?;
        if existing.is_some() {
          return Err(DurableStoreError::RunAlreadyExists(run_id));
        }
        let registered_workflow: Option<(String, i64)> = transaction
          .query_row(
            "SELECT workflow_id, schema_version FROM woml_definitions WHERE definition_hash = ?1",
            [&data.definition_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
          )
          .optional()?;
        let (registered_workflow, model_schema_version) = registered_workflow
          .ok_or_else(|| DurableStoreError::DefinitionNotFound(data.definition_hash.clone()))?;
        if registered_workflow != data.workflow_id {
          return Err(DurableStoreError::Contract(
            "run_started workflowId does not match its registered definition.".to_string(),
          ));
        }
        transaction.execute(
          "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
           VALUES (?1, ?2, ?3, ?4)",
          params![
            run_id,
            data.workflow_id,
            data.definition_hash,
            occurred_at.to_rfc3339(),
          ],
        )?;
        let model_schema_version = u32::try_from(model_schema_version).map_err(|_| {
          DurableStoreError::Contract(
            "Stored compiled-model schema version is invalid.".to_string(),
          )
        })?;
        run_event_schema_version_for_model(model_schema_version)
      }
      _ => {
        ensure_run_exists(&transaction, &run_id)?;
        let existing = load_events(&transaction, &run_id)?;
        existing
          .first()
          .map(|event| event.event_schema_version)
          .ok_or_else(|| {
            DurableStoreError::Contract(
              "A non-start event cannot be appended before run_started.".to_string(),
            )
          })?
      }
    };

    let mut events = load_events(&transaction, &run_id)?;
    let mut projection_state =
      projection_state_for_events(&self.projection_cache, &run_id, &events)?;
    let existing_event_count = events.len();
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    let payloads = if iteration.is_some() {
      vec![payload]
    } else if matches!(
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
    ) {
      expand_model_v11_payload(&workflow, &run_id, payload)?
    } else {
      vec![payload]
    };
    let mut first_event = None;
    for (index, payload) in payloads.into_iter().enumerate() {
      if let Some(scope) = iteration.as_ref() {
        validate_scoped_payload_against_definition(
          &workflow,
          &binding.definition_hash,
          &run_id,
          scope,
          &payload,
        )
        .map_err(DurableStoreError::Contract)?;
      } else {
        validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
          .map_err(DurableStoreError::Contract)?;
      }
      if iteration.is_none() {
        if let RunEventPayload::BranchSelected(data) = &payload {
          let projection = projection_state.projection();
          if projection.branch_selections.contains_key(&data.branch_id) {
            return Err(DurableStoreError::Contract(format!(
              "Branch {:?} already has an immutable selection.",
              data.branch_id
            )));
          }
          let selector_id = format!("__woml_branch__{}__select", data.branch_id);
          let ready =
            ready_node_ids_for_projection(&workflow, &binding.definition_hash, projection)
              .map_err(DurableStoreError::Contract)?;
          if !ready.iter().any(|node_id| node_id == &selector_id) {
            return Err(DurableStoreError::Contract(format!(
              "Branch selector {selector_id:?} is not ready for selection."
            )));
          }
        }
      }
      let event = append_to_history_scoped_incremental(
        &transaction,
        &mut events,
        &mut projection_state,
        &run_id,
        if index == 0 {
          event_id.clone()
        } else {
          generated_event_id()
        },
        occurred_at,
        event_schema_version,
        iteration.clone(),
        payload,
        true,
      )?;
      first_event.get_or_insert(event);
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = projection_state.projection().clone();
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      write_correction_indexes_from_projection(
        &transaction,
        &run_id,
        &binding.definition_hash,
        &events,
        &projection,
      )?;
    }
    transaction.commit()?;
    cache_projection_state(&self.projection_cache, &run_id, projection_state);
    span.count("events", events.len().saturating_sub(existing_event_count));
    span.succeed();
    Ok((first_event.unwrap(), projection))
  }

  pub(crate) fn append_payloads_atomically(
    &mut self,
    run_id: &str,
    payloads: Vec<(String, DateTime<Utc>, RunEventPayload)>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.append_payloads_scoped_atomically(run_id, None, payloads)
  }

  pub(crate) fn append_payloads_scoped_atomically(
    &mut self,
    run_id: &str,
    iteration: Option<crate::event::ForEachIterationScope>,
    payloads: Vec<(String, DateTime<Utc>, RunEventPayload)>,
  ) -> Result<RunProjection, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.append_event_batch");
    span.run_id(run_id.to_string());
    span.count("events", payloads.len());
    span.count("transactions", 1);
    if payloads.is_empty() {
      return Err(DurableStoreError::Contract(
        "An atomic event batch must not be empty.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let mut projection_state =
      projection_state_for_events(&self.projection_cache, run_id, &events)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "An atomic event batch cannot be appended before run_started.".to_string(),
        )
      })?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    for (event_id, occurred_at, payload) in payloads {
      let translated = if matches!(
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
      ) {
        expand_model_v11_payload(&workflow, run_id, payload)?
      } else {
        vec![payload]
      };
      for (index, mut payload) in translated.into_iter().enumerate() {
        if let RunEventPayload::CompensationRegistered(registration) = &mut payload {
          if let Some(previous) = events.last() {
            if matches!(
              &previous.payload,
              RunEventPayload::StepAttemptSucceeded(succeeded)
                if succeeded.node_id == registration.for_step_id
            ) {
              // The store owns global sequencing. Concurrent loop iterations
              // cannot safely predict their success sequence before this
              // immediate transaction acquires the writer lock.
              registration.completion_sequence = previous.sequence;
            }
          }
        }
        if let Some(scope) = iteration.as_ref() {
          validate_scoped_payload_against_definition(
            &workflow,
            &binding.definition_hash,
            run_id,
            scope,
            &payload,
          )
          .map_err(DurableStoreError::Contract)?;
        } else {
          validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
            .map_err(DurableStoreError::Contract)?;
        }
        append_to_history_scoped_incremental(
          &transaction,
          &mut events,
          &mut projection_state,
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
          false,
        )?;
      }
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = projection_state.projection().clone();
    let last_occurred_at = events
      .last()
      .expect("an atomic event batch preserves a non-empty history")
      .occurred_at;
    write_run_summary_from_projection(
      &transaction,
      events
        .first()
        .expect("an atomic event batch preserves a non-empty history"),
      last_occurred_at,
      &projection,
    )?;
    if let RunEventPayload::RunAdmitted(admission) = &events
      .first()
      .expect("an atomic event batch preserves a non-empty history")
      .payload
    {
      write_runtime_policy_indexes_from_projection(&transaction, run_id, admission, &projection)?;
    }
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      write_correction_indexes_from_projection(
        &transaction,
        run_id,
        &binding.definition_hash,
        &events,
        &projection,
      )?;
    }
    transaction.commit()?;
    cache_projection_state(&self.projection_cache, run_id, projection_state);
    span.succeed();
    Ok(projection)
  }
}
