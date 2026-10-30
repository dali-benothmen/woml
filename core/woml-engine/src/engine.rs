//! Stable in-memory DAG adapter and graph-decision facade.
//!
//! Definitions and every candidate history prefix are validated before the
//! event store publishes history or derived context. Private children separate
//! readiness, routing, reference resolution, and definition-bound validation.

mod context;
mod history;
mod identity;
mod readiness;
mod routing;
mod validation;

pub(crate) use context::resolve_context_reference;
pub(crate) use history::validate_event_history_against_definition;
use history::{validate_for_each_id, validate_operation_node, validate_reusable_lifecycle_binding};
pub use identity::step_effect_idempotency_key;
pub(crate) use readiness::{
  node_is_complete, ready_node_ids_for_projection, ready_node_ids_for_projection_at,
};
pub(crate) use routing::{
  selected_branch_arm, selected_choice_arm, selector_branch_id, BranchEvaluationError,
  BranchEvaluationErrorKind,
};
pub(crate) use validation::{
  validate_payload_against_definition, validate_scoped_payload_against_definition,
};

use crate::event::is_definition_hash;
use crate::event::RunEventPayload;
use crate::event::RunStartedData;
use crate::run_event_schema_version_for_model;
use crate::CompiledWorkflowDefinition;
use crate::EventStoreError;
use crate::FoldError;
use crate::InMemoryEventStore;
use crate::ModelValidationError;
use crate::RunEvent;
use crate::RunProjection;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use serde_json::Map;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error(transparent)]
  EventStore(#[from] EventStoreError),
  #[error(transparent)]
  Fold(#[from] FoldError),
  #[error("{0}")]
  Contract(String),
}

#[derive(Debug)]
pub struct InMemoryDagEngine {
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  store: InMemoryEventStore,
}

impl InMemoryDagEngine {
  pub fn new(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
  ) -> Result<Self, EngineError> {
    workflow.validate_for_execution()?;
    let definition_hash = definition_hash.into();
    if !is_definition_hash(&definition_hash) {
      return Err(EngineError::Contract(
        "The engine requires a valid RFC 8785 SHA-256 definition hash.".to_string(),
      ));
    }
    Ok(Self {
      workflow,
      definition_hash,
      store: InMemoryEventStore::default(),
    })
  }

  pub fn new_for_event_history(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
  ) -> Result<Self, EngineError> {
    workflow.validate_structure()?;
    let definition_hash = definition_hash.into();
    if !is_definition_hash(&definition_hash) {
      return Err(EngineError::Contract(
        "The engine requires a valid RFC 8785 SHA-256 definition hash.".to_string(),
      ));
    }
    Ok(Self {
      workflow,
      definition_hash,
      store: InMemoryEventStore::default(),
    })
  }

  pub fn workflow(&self) -> &CompiledWorkflowDefinition {
    &self.workflow
  }

  pub fn definition_hash(&self) -> &str {
    &self.definition_hash
  }

  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    trigger: Map<String, Value>,
  ) -> Result<RunProjection, EngineError> {
    let event = RunEvent {
      event_schema_version: run_event_schema_version_for_model(self.workflow.schema_version),
      event_id: event_id.into(),
      run_id: run_id.into(),
      sequence: 1,
      occurred_at,
      iteration: None,
      payload: RunEventPayload::RunStarted(RunStartedData {
        workflow_id: self.workflow.workflow_id.clone(),
        definition_hash: self.definition_hash.clone(),
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        ingress: None,
        trigger,
      }),
    };
    self.append_event(event)
  }

  pub fn append_event(&mut self, event: RunEvent) -> Result<RunProjection, EngineError> {
    let expected_version = run_event_schema_version_for_model(self.workflow.schema_version);
    if event.event_schema_version != expected_version {
      return Err(EngineError::Contract(format!(
        "Compiled model v{} requires run-event schema v{expected_version}, received v{}.",
        self.workflow.schema_version, event.event_schema_version
      )));
    }
    validate_payload_against_definition(&self.workflow, &self.definition_hash, &event.payload)
      .map_err(EngineError::Contract)?;
    if let RunEventPayload::StepAttemptStarted(data) = &event.payload {
      let ready = self.ready_node_ids(&event.run_id)?;
      if !ready.iter().any(|node_id| node_id == &data.node_id) {
        return Err(EngineError::Contract(format!(
          "Node {:?} is not ready for execution.",
          data.node_id
        )));
      }
    }
    if let RunEventPayload::BranchSelected(data) = &event.payload {
      let projection = self.projection(&event.run_id)?;
      if projection.branch_selections.contains_key(&data.branch_id) {
        return Err(EngineError::Contract(format!(
          "Branch {:?} already has an immutable selection.",
          data.branch_id
        )));
      }
      let selector_id = format!("__woml_branch__{}__select", data.branch_id);
      let ready = self.ready_node_ids(&event.run_id)?;
      if !ready.iter().any(|node_id| node_id == &selector_id) {
        return Err(EngineError::Contract(format!(
          "Branch selector {selector_id:?} is not ready for selection."
        )));
      }
    }
    let mut candidate = self.store.events(&event.run_id).to_vec();
    candidate.push(event.clone());
    validate_event_history_against_definition(&self.workflow, &self.definition_hash, &candidate)
      .map_err(EngineError::Contract)?;
    Ok(self.store.append(event)?)
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, EngineError> {
    Ok(self.store.projection(run_id)?)
  }

  pub fn events(&self, run_id: &str) -> &[RunEvent] {
    self.store.events(run_id)
  }

  pub fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, EngineError> {
    let projection = self.projection(run_id)?;
    if projection.status != RunStatus::Running {
      return Ok(Vec::new());
    }
    if projection.workflow_id.as_deref() != Some(self.workflow.workflow_id.as_str())
      || projection.definition_hash.as_deref() != Some(self.definition_hash.as_str())
    {
      return Err(EngineError::Contract(
        "The run is not bound to this compiled workflow definition.".to_string(),
      ));
    }

    ready_node_ids_for_projection(&self.workflow, &self.definition_hash, &projection)
      .map_err(EngineError::Contract)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextReferenceError {
  pub path: Vec<String>,
}
