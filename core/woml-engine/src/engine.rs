use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::event::{
  is_definition_hash, RunEventPayload, RunFailedData, RunFailedDataV2, RunStartedData,
};
use crate::{
  run_event_schema_version_for_model, CompiledWorkflowDefinition, EventStoreError, FoldError,
  InMemoryEventStore, ModelValidationError, RunEvent, RunProjection, RunStatus,
};

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
      payload: RunEventPayload::RunStarted(RunStartedData {
        workflow_id: self.workflow.workflow_id.clone(),
        definition_hash: self.definition_hash.clone(),
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

pub(crate) fn ready_node_ids_for_projection(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  projection: &RunProjection,
) -> Result<Vec<String>, String> {
  if projection.status != RunStatus::Running {
    return Ok(Vec::new());
  }
  if projection.workflow_id.as_deref() != Some(workflow.workflow_id.as_str())
    || projection.definition_hash.as_deref() != Some(definition_hash)
  {
    return Err("The run is not bound to this compiled workflow definition.".to_string());
  }

  let completed = projection.completed_node_ids();
  let attempted = projection.attempted_node_ids();
  let mut incoming: HashMap<&str, Vec<&str>> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), Vec::new()))
    .collect();
  for edge in &workflow.graph.edges {
    incoming
      .entry(edge.to.as_str())
      .or_default()
      .push(edge.from.as_str());
  }

  Ok(
    workflow
      .graph
      .nodes
      .iter()
      .filter(|node| {
        !completed.contains(node.id.as_str())
          && !attempted.contains(node.id.as_str())
          && incoming
            .get(node.id.as_str())
            .into_iter()
            .flatten()
            .all(|predecessor| completed.contains(predecessor))
      })
      .map(|node| node.id.clone())
      .collect(),
  )
}

pub(crate) fn validate_payload_against_definition(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  payload: &RunEventPayload,
) -> Result<(), String> {
  match payload {
    RunEventPayload::RunStarted(data) => {
      if data.workflow_id != workflow.workflow_id || data.definition_hash != definition_hash {
        return Err(
          "run_started does not match the engine's workflow ID and definition hash.".to_string(),
        );
      }
    }
    RunEventPayload::StepAttemptStarted(data) => {
      if data.attempt != 1 {
        return Err("R2 does not execute retries; attempt must be 1.".to_string());
      }
      let node = workflow
        .node(&data.node_id)
        .ok_or_else(|| format!("Attempt references unknown node {:?}.", data.node_id))?;
      if data.handler != node.handler {
        return Err(format!(
          "Attempt handler {:?} does not match node {:?} handler {:?}.",
          data.handler, data.node_id, node.handler
        ));
      }
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      if workflow.node(&data.node_id).is_none() {
        return Err(format!(
          "Attempt references unknown node {:?}.",
          data.node_id
        ));
      }
    }
    RunEventPayload::StepAttemptFailed(data) => {
      if workflow.node(&data.node_id).is_none() {
        return Err(format!(
          "Attempt references unknown node {:?}.",
          data.node_id
        ));
      }
    }
    RunEventPayload::BranchSelected(data) => {
      let selector_id = format!("__woml_branch__{}__select", data.branch_id);
      let selector = workflow.node(&selector_id).ok_or_else(|| {
        format!(
          "branch_selected references unknown branch {:?}.",
          data.branch_id
        )
      })?;
      if selector.handler != "engine.branch-select" {
        return Err(format!(
          "Branch {:?} does not have its canonical selector.",
          data.branch_id
        ));
      }
      let valid_arm = workflow.graph.edges.iter().any(|edge| {
        edge.from == selector_id
          && edge.id == data.arm_id
          && edge.branch_id.as_deref() == Some(data.branch_id.as_str())
      });
      if !valid_arm {
        return Err(format!(
          "Arm {:?} is not selectable for branch {:?}.",
          data.arm_id, data.branch_id
        ));
      }
    }
    RunEventPayload::RunSucceeded(data) => {
      if workflow.terminal_node_id() != Some(data.terminal_node_id.as_str()) {
        return Err(format!(
          "run_succeeded names {:?}, which is not the workflow terminal node.",
          data.terminal_node_id
        ));
      }
    }
    RunEventPayload::RunFailed(data) => {
      let (node_id, branch_identity) = match data {
        RunFailedData::V1(data) => (data.node_id.as_deref(), None),
        RunFailedData::V2(RunFailedDataV2::Attempt { node_id, .. }) => {
          (Some(node_id.as_str()), None)
        }
        RunFailedData::V2(RunFailedDataV2::Branch {
          branch_id, arm_id, ..
        }) => (None, Some((branch_id.as_str(), arm_id.as_deref()))),
      };
      if node_id.is_some_and(|node_id| workflow.node(node_id).is_none()) {
        return Err(format!("run_failed references unknown node {node_id:?}."));
      }
      if let Some((branch_id, arm_id)) = branch_identity {
        let selector_id = format!("__woml_branch__{branch_id}__select");
        if workflow.node(&selector_id).is_none() {
          return Err(format!(
            "run_failed references unknown branch {branch_id:?}."
          ));
        }
        if arm_id.is_some_and(|arm_id| {
          !workflow.graph.edges.iter().any(|edge| {
            edge.from == selector_id
              && edge.id == arm_id
              && edge.branch_id.as_deref() == Some(branch_id)
          })
        }) {
          return Err(format!(
            "run_failed references an unknown arm for branch {branch_id:?}."
          ));
        }
      }
    }
  }

  Ok(())
}
