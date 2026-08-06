use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::event::{
  is_definition_hash, RunEventPayload, RunFailedData, RunFailedDataV2, RunStartedData,
};
use crate::{
  model::{EdgeCondition, ValueExpression},
  run_event_schema_version_for_model, CompiledWorkflowDefinition, EventStoreError, FoldError,
  InMemoryEventStore, ModelValidationError, RunEvent, RunProjection, RunStatus, WorkflowContext,
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

  let active = active_node_ids(workflow, projection);
  let attempted = projection.attempted_node_ids();
  let mut incoming: HashMap<&str, Vec<&crate::model::CompiledWorkflowEdge>> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), Vec::new()))
    .collect();
  for edge in &workflow.graph.edges {
    if active.contains(edge.from.as_str()) && edge_is_active(edge, projection) {
      incoming.entry(edge.to.as_str()).or_default().push(edge);
    }
  }

  Ok(
    workflow
      .graph
      .nodes
      .iter()
      .filter(|node| {
        active.contains(node.id.as_str())
          && !node_is_complete(node, projection)
          && !attempted.contains(node.id.as_str())
          && incoming
            .get(node.id.as_str())
            .into_iter()
            .flatten()
            .all(|edge| {
              workflow
                .node(&edge.from)
                .is_some_and(|predecessor| node_is_complete(predecessor, projection))
            })
      })
      .map(|node| node.id.clone())
      .collect(),
  )
}

fn active_node_ids<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  projection: &RunProjection,
) -> HashSet<&'a str> {
  let mut active = HashSet::new();
  let mut queue: VecDeque<&str> = workflow
    .graph
    .entry_node_ids
    .iter()
    .map(String::as_str)
    .collect();
  while let Some(node_id) = queue.pop_front() {
    if !active.insert(node_id) {
      continue;
    }
    for edge in workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == node_id && edge_is_active(edge, projection))
    {
      queue.push_back(edge.to.as_str());
    }
  }
  active
}

fn edge_is_active(edge: &crate::model::CompiledWorkflowEdge, projection: &RunProjection) -> bool {
  match edge.branch_id.as_deref() {
    Some(branch_id) => projection
      .branch_selections
      .get(branch_id)
      .is_some_and(|arm_id| arm_id == &edge.id),
    None => true,
  }
}

fn node_is_complete(node: &crate::model::CompiledWorkflowNode, projection: &RunProjection) -> bool {
  if node.handler == "engine.branch-select" {
    return selector_branch_id(&node.id)
      .is_some_and(|branch_id| projection.branch_selections.contains_key(branch_id));
  }
  projection.context.steps.contains_key(&node.id)
}

pub(crate) fn selector_branch_id(selector_id: &str) -> Option<&str> {
  selector_id
    .strip_prefix("__woml_branch__")?
    .strip_suffix("__select")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextReferenceError {
  pub path: Vec<String>,
}

pub(crate) fn resolve_context_reference(
  expression: &ValueExpression,
  context: &WorkflowContext,
) -> Result<Value, ContextReferenceError> {
  let ValueExpression::ContextReference { path } = expression else {
    return Err(ContextReferenceError { path: Vec::new() });
  };
  let missing = || ContextReferenceError { path: path.clone() };
  let Some(root) = path.first().map(String::as_str) else {
    return Err(missing());
  };
  let (mut value, remaining): (Value, &[String]) = match root {
    "trigger" => (Value::Object(context.trigger.clone()), &path[1..]),
    "steps" => (Value::Object(context.steps.clone()), &path[1..]),
    _ => return Err(missing()),
  };
  for segment in remaining {
    value = match value {
      Value::Object(object) => object.get(segment).cloned().ok_or_else(missing)?,
      _ => return Err(missing()),
    };
  }
  Ok(value)
}

pub(crate) fn selected_branch_arm(
  workflow: &CompiledWorkflowDefinition,
  selector_id: &str,
  context: &WorkflowContext,
) -> Result<String, BranchEvaluationError> {
  let branch_id = selector_branch_id(selector_id).ok_or_else(|| BranchEvaluationError {
    branch_id: selector_id.to_string(),
    arm_id: None,
    path: None,
    kind: BranchEvaluationErrorKind::SelectionInvalid,
  })?;
  for edge in workflow
    .graph
    .edges
    .iter()
    .filter(|edge| edge.from == selector_id && edge.branch_id.as_deref() == Some(branch_id))
  {
    match &edge.condition {
      EdgeCondition::Boolean { value: expression } => {
        let condition_path = match expression {
          ValueExpression::ContextReference { path } => Some(path.clone()),
          _ => None,
        };
        let resolved = resolve_context_reference(expression, context).map_err(|error| {
          BranchEvaluationError {
            branch_id: branch_id.to_string(),
            arm_id: Some(edge.id.clone()),
            path: Some(error.path),
            kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
          }
        })?;
        match resolved {
          Value::Bool(true) => return Ok(edge.id.clone()),
          Value::Bool(false) => {}
          value => {
            return Err(BranchEvaluationError {
              branch_id: branch_id.to_string(),
              arm_id: Some(edge.id.clone()),
              path: condition_path,
              kind: BranchEvaluationErrorKind::NotBoolean(json_value_type(&value)),
            });
          }
        }
      }
      EdgeCondition::Always => return Ok(edge.id.clone()),
      _ => {
        return Err(BranchEvaluationError {
          branch_id: branch_id.to_string(),
          arm_id: Some(edge.id.clone()),
          path: None,
          kind: BranchEvaluationErrorKind::SelectionInvalid,
        });
      }
    }
  }
  Err(BranchEvaluationError {
    branch_id: branch_id.to_string(),
    arm_id: None,
    path: None,
    kind: BranchEvaluationErrorKind::SelectionInvalid,
  })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchEvaluationError {
  pub branch_id: String,
  pub arm_id: Option<String>,
  pub path: Option<Vec<String>>,
  pub kind: BranchEvaluationErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BranchEvaluationErrorKind {
  NotBoolean(crate::JsonValueType),
  ReferenceNotAvailable,
  SelectionInvalid,
}

fn json_value_type(value: &Value) -> crate::JsonValueType {
  match value {
    Value::Null => crate::JsonValueType::Null,
    Value::Number(_) => crate::JsonValueType::Number,
    Value::String(_) => crate::JsonValueType::String,
    Value::Array(_) => crate::JsonValueType::Array,
    Value::Object(_) => crate::JsonValueType::Object,
    Value::Bool(_) => unreachable!("boolean values are handled before type classification"),
  }
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
        return Err("The current executable profile requires attempt 1.".to_string());
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
