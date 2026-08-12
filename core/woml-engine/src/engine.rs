use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::event::{
  is_definition_hash, ApprovalTimeoutPolicy, AttemptFailureKind, ParallelFailurePolicy,
  ParallelGroupOutcome, RunEventPayload, RunFailedData, RunFailedDataV2, RunFailedDataV3,
  RunFailedDataV4, RunFailedDataV5, RunOutcomeDecidedData, RunStartedData,
};
use crate::{
  model::{EdgeCondition, ValueExpression},
  run_event_schema_version_for_model, CompiledWorkflowDefinition, EventStoreError, FoldError,
  InMemoryEventStore, ModelValidationError, ParallelGroupStatus, RunEvent, RunProjection,
  RunStatus, WorkflowContext,
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

pub(crate) fn ready_node_ids_for_projection(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  projection: &RunProjection,
) -> Result<Vec<String>, String> {
  ready_node_ids_for_projection_at(workflow, definition_hash, projection, Utc::now())
}

pub(crate) fn ready_node_ids_for_projection_at(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  projection: &RunProjection,
  now: DateTime<Utc>,
) -> Result<Vec<String>, String> {
  if projection.status != RunStatus::Running {
    return Ok(Vec::new());
  }
  let workflow_identity_matches = projection.workflow_id.as_deref()
    == Some(workflow.workflow_id.as_str())
    || (projection.event_schema_version == Some(crate::RUN_EVENT_SCHEMA_VERSION_V11)
      && projection.workflow_id.is_none());
  if !workflow_identity_matches || projection.definition_hash.as_deref() != Some(definition_hash) {
    return Err("The run is not bound to this compiled workflow definition.".to_string());
  }

  let active = active_node_ids(workflow, projection);
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
          && !node_is_complete(workflow, node, projection)
          && match projection.latest_attempt(&node.id) {
            None => true,
            Some(attempt)
              if matches!(attempt.status, crate::projection::AttemptStatus::Started) =>
            {
              false
            }
            Some(attempt)
              if matches!(
                attempt.status,
                crate::projection::AttemptStatus::Succeeded { .. }
              ) =>
            {
              false
            }
            Some(_) => projection
              .pending_retries
              .get(&node.id)
              .is_some_and(|schedule| schedule.scheduled_at <= now),
          }
          && incoming
            .get(node.id.as_str())
            .into_iter()
            .flatten()
            .all(|edge| {
              workflow
                .node(&edge.from)
                .is_some_and(|predecessor| node_is_complete(workflow, predecessor, projection))
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
  if let Some(approval_id) = edge.approval_id.as_deref() {
    let decision = projection
      .context
      .steps
      .get(approval_id)
      .and_then(Value::as_object)
      .and_then(|output| output.get("decision"))
      .and_then(Value::as_str);
    if edge.id == format!("{approval_id}:approved")
      || edge.id == format!("{approval_id}:approved:join")
    {
      return decision == Some("approved");
    }
    if edge.id == format!("{approval_id}:rejected")
      || edge.id == format!("{approval_id}:rejected:join")
    {
      return decision == Some("rejected");
    }
    return false;
  }
  match edge.branch_id.as_deref() {
    Some(branch_id) => projection
      .branch_selections
      .get(branch_id)
      .is_some_and(|arm_id| arm_id == &edge.id),
    None => true,
  }
}

pub(crate) fn node_is_complete(
  workflow: &CompiledWorkflowDefinition,
  node: &crate::model::CompiledWorkflowNode,
  projection: &RunProjection,
) -> bool {
  if node.handler == "engine.branch-select" {
    return selector_branch_id(&node.id)
      .is_some_and(|branch_id| projection.branch_selections.contains_key(branch_id));
  }
  if node.handler == "engine.parallel-start" {
    return node
      .id
      .strip_prefix("__woml_parallel__")
      .and_then(|id| id.strip_suffix("__start"))
      .is_some_and(|parallel_id| projection.parallel_groups.contains_key(parallel_id));
  }
  if node.handler == "engine.parallel-join" {
    return projection
      .parallel_groups
      .get(&node.id)
      .is_some_and(|group| {
        matches!(
          group.status,
          ParallelGroupStatus::Completed {
            outcome: ParallelGroupOutcome::Succeeded,
            ..
          }
        )
      });
  }
  if node.handler == "engine.approval-wait" {
    return projection.context.steps.contains_key(&node.id);
  }
  if node.handler == "engine.approval-join" {
    return workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.to == node.id && edge_is_active(edge, projection))
      .any(|edge| {
        workflow
          .node(&edge.from)
          .is_some_and(|predecessor| node_is_complete(workflow, predecessor, projection))
      });
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
    RunEventPayload::RunAdmitted(data) => {
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        || data.definition_hash != definition_hash
        || workflow.runtime_policy_hash().as_deref() != Some(data.policy_hash.as_str())
        || workflow.runtime_policy_queue_name().as_deref() != Some(data.queue.name.as_str())
      {
        return Err(
          "run_admitted does not match the compiled definition and immutable policy identity."
            .to_string(),
        );
      }
      let workflow_call_ingress =
        data.trigger.id == "workflow-call" && data.trigger.handler == "trigger.event";
      if !workflow_call_ingress {
        let trigger = workflow.trigger(&data.trigger.id).ok_or_else(|| {
          format!(
            "run_admitted references unknown trigger {:?}.",
            data.trigger.id
          )
        })?;
        if trigger.handler != data.trigger.handler {
          return Err("run_admitted trigger identity does not match the definition.".to_string());
        }
      }
    }
    RunEventPayload::RunExecutionStarted(data) => {
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
        return Err("Event v11 runtime-policy events require compiled Model v12.".to_string());
      }
      let expected_timeout = workflow
        .runtime_policy
        .as_ref()
        .and_then(|policy| policy.timeout_ms)
        .map(|timeout_ms| {
          data.started_at
            + chrono::Duration::milliseconds(i64::try_from(timeout_ms).unwrap_or(i64::MAX))
        });
      if data.timeout_at != expected_timeout {
        return Err(
          "run_execution_started timeoutAt does not match the compiled workflow timeout."
            .to_string(),
        );
      }
    }
    RunEventPayload::RunTimeoutReached(_) => {
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
        return Err("Event v11 runtime-policy events require compiled Model v12.".to_string());
      }
    }
    RunEventPayload::RunStarted(data) => {
      if data.workflow_id != workflow.workflow_id || data.definition_hash != definition_hash {
        return Err(
          "run_started does not match the engine's workflow ID and definition hash.".to_string(),
        );
      }
      if matches!(
        data.ingress.as_ref(),
        Some(crate::RunIngress::WorkflowCall { .. })
      ) {
        if data.trigger_id.is_some()
          || data.trigger_handler.is_some()
          || data.trigger_occurrence_id.is_some()
        {
          return Err("Workflow-call run_started requires no source trigger identity.".to_string());
        }
      } else if workflow.schema_version == crate::COMPILED_MODEL_SCHEMA_VERSION_V10 {
        return Err("Call-only Model v10 requires workflow_call ingress.".to_string());
      } else if workflow.schema_version == crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        && data.ingress.is_some()
      {
        if data.trigger_id.is_some()
          || data.trigger_handler.is_some()
          || data.trigger_occurrence_id.is_some()
        {
          return Err("Model v11 workflow-call ingress cannot carry trigger identity.".to_string());
        }
      } else if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V7 {
        let trigger_id = data
          .trigger_id
          .as_deref()
          .ok_or_else(|| "run_started v7 is missing its triggerId.".to_string())?;
        let trigger = workflow
          .trigger(trigger_id)
          .ok_or_else(|| format!("run_started references unknown trigger {trigger_id:?}."))?;
        if data.trigger_handler.as_deref() != Some(trigger.handler.as_str())
          || data.trigger_occurrence_id.is_none()
        {
          return Err(
            "run_started trigger identity does not match the compiled definition.".to_string(),
          );
        }
        if data.ingress.is_some() {
          return Err(
            "Source-triggered run_started cannot carry workflow-call ingress.".to_string(),
          );
        }
      } else if data.trigger_id.is_some()
        || data.trigger_handler.is_some()
        || data.trigger_occurrence_id.is_some()
        || data.ingress.is_some()
      {
        return Err("Pre-v7 run_started cannot carry production-trigger identity.".to_string());
      }
    }
    RunEventPayload::StepAttemptStarted(data) => {
      let node = workflow
        .node(&data.node_id)
        .ok_or_else(|| format!("Attempt references unknown node {:?}.", data.node_id))?;
      let maximum_attempts = node
        .retry_policy
        .as_ref()
        .map_or(1, |policy| policy.max_attempts);
      if data.attempt == 0
        || data.attempt > maximum_attempts
        || (workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 && data.attempt != 1)
      {
        return Err(format!(
          "Node {:?} attempt {} exceeds its compiled maximum of {maximum_attempts}.",
          data.node_id, data.attempt
        ));
      }
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
    RunEventPayload::StepRetryScheduled(data) => {
      if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
        return Err("step_retry_scheduled requires compiled model v6.".to_string());
      }
      let node = workflow
        .node(&data.node_id)
        .ok_or_else(|| format!("Retry schedule references unknown node {:?}.", data.node_id))?;
      let policy = node.retry_policy.as_ref().ok_or_else(|| {
        format!(
          "Retry schedule references node {:?} without a compiled retry policy.",
          data.node_id
        )
      })?;
      if node.handler != "runtime.script"
        || data.next_attempt != data.failed_attempt + 1
        || data.next_attempt > policy.max_attempts
      {
        return Err(format!(
          "Retry schedule for node {:?} exceeds its compiled policy.",
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
    RunEventPayload::ParallelGroupStarted(data) => {
      if workflow.parallel_group(&data.parallel_id).is_none() {
        return Err(format!(
          "parallel_group_started references unknown parallel group {:?}.",
          data.parallel_id
        ));
      }
    }
    RunEventPayload::ParallelGroupCompleted(data) => {
      let group = workflow.parallel_group(&data.parallel_id).ok_or_else(|| {
        format!(
          "parallel_group_completed references unknown parallel group {:?}.",
          data.parallel_id
        )
      })?;
      if data
        .failed_node_ids
        .iter()
        .chain(&data.cancelled_node_ids)
        .any(|node_id| !group.child_node_ids.contains(node_id))
      {
        return Err(format!(
          "parallel_group_completed for {:?} references a node outside that group.",
          data.parallel_id
        ));
      }
    }
    RunEventPayload::ApprovalRequested(data) => {
      let approval = workflow.approval(&data.approval_id).ok_or_else(|| {
        format!(
          "approval_requested references unknown approval {:?}.",
          data.approval_id
        )
      })?;
      let expected_policy = match approval.on_timeout.as_str() {
        "reject" => ApprovalTimeoutPolicy::Reject,
        "fail" => ApprovalTimeoutPolicy::Fail,
        _ => {
          return Err(format!(
            "Approval {:?} has an invalid policy.",
            data.approval_id
          ))
        }
      };
      if data.on_timeout != expected_policy
        || approval.timeout_ms.is_some() != data.expires_at.is_some()
      {
        return Err(format!(
          "approval_requested does not match approval {:?} timeout inputs.",
          data.approval_id
        ));
      }
    }
    RunEventPayload::ApprovalResolved(data) => {
      if workflow.approval(&data.approval_id).is_none() {
        return Err(format!(
          "approval_resolved references unknown approval {:?}.",
          data.approval_id
        ));
      }
    }
    RunEventPayload::NotificationDeliveryRequested(data) => {
      let approval = workflow.approval(&data.approval_id).ok_or_else(|| {
        format!(
          "Notification references unknown approval {:?}.",
          data.approval_id
        )
      })?;
      let delivery = approval
        .notifications
        .iter()
        .find(|delivery| delivery.delivery_id == data.delivery_id)
        .ok_or_else(|| {
          format!(
            "Notification references unknown delivery {:?}.",
            data.delivery_id
          )
        })?;
      if data.provider != delivery.provider || data.destination != delivery.destination {
        return Err(
          "Notification delivery intent does not match the compiled definition.".to_string(),
        );
      }
    }
    RunEventPayload::NotificationDeliveryAttemptStarted(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification attempt references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification attempt references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationDeliverySucceeded(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification success references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification success references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationDeliveryFailed(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification failure references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification failure references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationDecisionAccepted(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification decision references an unknown approval.".to_string())?;
      if !approval.notifications.iter().any(|delivery| {
        delivery.delivery_id == data.delivery_id && delivery.provider == data.provider
      }) {
        return Err("Notification decision references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationMessageUpdateRequested(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification update references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification update references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationMessageUpdateAttemptStarted(data)
    | RunEventPayload::NotificationMessageUpdated(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification update attempt references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err(
          "Notification update attempt references an unknown compiled delivery.".to_string(),
        );
      }
    }
    RunEventPayload::NotificationMessageUpdateFailed(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification update failure references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err(
          "Notification update failure references an unknown compiled delivery.".to_string(),
        );
      }
    }
    RunEventPayload::RunSucceeded(data) => {
      if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V11 {
        return Err("Model v11+ finalizes with run_outcome_decided and run_finalized.".to_string());
      }
      if workflow.terminal_node_id() != Some(data.terminal_node_id.as_str()) {
        return Err(format!(
          "run_succeeded names {:?}, which is not the workflow terminal node.",
          data.terminal_node_id
        ));
      }
    }
    RunEventPayload::OperationStarted(data) => {
      validate_operation_node(workflow, &data.node_id)?;
    }
    RunEventPayload::OperationSucceeded(data) => {
      validate_operation_node(workflow, &data.node_id)?;
    }
    RunEventPayload::OperationFailed(data) => {
      validate_operation_node(workflow, &data.node_id)?;
    }
    RunEventPayload::RunCancellationRequested(_)
    | RunEventPayload::RunOutcomeDecided(_)
    | RunEventPayload::RunFinalized(_) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11 | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      ) {
        return Err("Lifecycle and run-control events require compiled Model v11+.".to_string());
      }
    }
    RunEventPayload::LifecycleHookRequested(data) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11 | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      ) {
        return Err("Lifecycle hook events require compiled Model v11+.".to_string());
      }
      let hook = workflow.lifecycle_hook(&data.hook_id).ok_or_else(|| {
        format!(
          "Lifecycle request references unknown hook {:?}.",
          data.hook_id
        )
      })?;
      if hook.event != data.event
        || (data.event.is_step()
          && workflow
            .lifecycle_hook_for_step_event(data.event, &data.subject.id)
            .is_none())
      {
        return Err("Lifecycle hook request does not match its compiled binding.".to_string());
      }
    }
    RunEventPayload::LifecycleActionAttemptStarted(data)
    | RunEventPayload::LifecycleActionSucceeded(data) => {
      let action_exists = workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
        lifecycle.hooks.iter().any(|hook| {
          hook.hook_id == data.action_id.split(":action:").next().unwrap_or_default()
            && hook
              .actions
              .iter()
              .any(|action| action.action_id == data.action_id)
        })
      });
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11 | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      ) || !action_exists
      {
        return Err("Lifecycle action event references an unknown Model v11+ action.".to_string());
      }
    }
    RunEventPayload::LifecycleActionFailed(data) => {
      let action_exists = workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
        lifecycle
          .hooks
          .iter()
          .flat_map(|hook| &hook.actions)
          .any(|action| action.action_id == data.action_id)
      });
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11 | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      ) || !action_exists
      {
        return Err(
          "Lifecycle action failure references an unknown Model v11+ action.".to_string(),
        );
      }
    }
    RunEventPayload::LifecycleHookCompleted(data) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11 | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      ) || workflow.lifecycle.is_none()
        || !crate::event::is_definition_hash(&data.hook_invocation_id)
      {
        return Err(
          "Lifecycle hook completion requires a Model v11+ lifecycle binding.".to_string(),
        );
      }
    }
    RunEventPayload::RunFailed(data) => {
      if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V11 {
        return Err("Model v11+ finalizes with run_outcome_decided and run_finalized.".to_string());
      }
      if let RunFailedData::V5(RunFailedDataV5::Notification {
        approval_id,
        failed_delivery_ids,
        ..
      }) = data
      {
        let approval = workflow
          .approval(approval_id)
          .ok_or_else(|| format!("run_failed references unknown approval {approval_id:?}."))?;
        if failed_delivery_ids.iter().any(|id| {
          !approval
            .notifications
            .iter()
            .any(|delivery| delivery.delivery_id == *id)
        }) {
          return Err(
            "run_failed references a delivery outside its compiled approval.".to_string(),
          );
        }
      }
      let (node_id, branch_identity, parallel_identity, approval_identity) = match data {
        RunFailedData::V5(_) => (None, None, None, None),
        RunFailedData::V1(data) => (data.node_id.as_deref(), None, None, None),
        RunFailedData::V2(RunFailedDataV2::Attempt { node_id, .. }) => {
          (Some(node_id.as_str()), None, None, None)
        }
        RunFailedData::V2(RunFailedDataV2::Branch {
          branch_id, arm_id, ..
        }) => (
          None,
          Some((branch_id.as_str(), arm_id.as_deref())),
          None,
          None,
        ),
        RunFailedData::V3(RunFailedDataV3::Parallel {
          parallel_id,
          policy,
          primary_node_id,
          failed_node_ids,
          cancelled_node_ids,
          ..
        }) => (
          None,
          None,
          Some((
            parallel_id.as_str(),
            *policy,
            primary_node_id.as_str(),
            failed_node_ids,
            cancelled_node_ids,
          )),
          None,
        ),
        RunFailedData::V4(RunFailedDataV4::Approval {
          approval_id,
          request_id,
          ..
        }) => (
          None,
          None,
          None,
          Some((approval_id.as_str(), request_id.as_str())),
        ),
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
      if let Some((parallel_id, policy, primary_node_id, failed, cancelled)) = parallel_identity {
        let group = workflow.parallel_group(parallel_id).ok_or_else(|| {
          format!("run_failed references unknown parallel group {parallel_id:?}.")
        })?;
        let expected_policy = match group.on_error.as_str() {
          "fail-fast" => ParallelFailurePolicy::FailFast,
          "wait-all" => ParallelFailurePolicy::WaitAll,
          _ => {
            return Err(format!(
              "Parallel group {parallel_id:?} has an invalid policy."
            ))
          }
        };
        if policy != expected_policy
          || !failed.iter().any(|node_id| node_id == primary_node_id)
          || failed
            .iter()
            .chain(cancelled)
            .any(|node_id| !group.child_node_ids.contains(node_id))
        {
          return Err(format!(
            "run_failed does not match parallel group {parallel_id:?} and its compiled policy."
          ));
        }
      }
      if let Some((approval_id, _request_id)) = approval_identity {
        if workflow.approval(approval_id).is_none() {
          return Err(format!(
            "run_failed references unknown approval {approval_id:?}."
          ));
        }
      }
    }
  }

  Ok(())
}

pub fn step_effect_idempotency_key(run_id: &str, definition_hash: &str, node_id: &str) -> String {
  let run_id = serde_json::to_string(run_id).expect("serializing a string cannot fail");
  let definition_hash =
    serde_json::to_string(definition_hash).expect("serializing a string cannot fail");
  let node_id = serde_json::to_string(node_id).expect("serializing a string cannot fail");
  let canonical = format!(
    "{{\"contract\":\"woml.step-effect\",\"definitionHash\":{definition_hash},\"nodeId\":{node_id},\"runId\":{run_id},\"version\":1}}"
  );
  format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParallelChildState {
  Active,
  RetryPending,
  Succeeded,
  Failed(AttemptFailureKind),
}

pub(crate) fn validate_event_history_against_definition(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &[RunEvent],
) -> Result<(), String> {
  crate::fold_events(events).map_err(|error| error.to_string())?;
  let mut child_states: HashMap<String, ParallelChildState> = HashMap::new();

  for (index, event) in events.iter().enumerate() {
    validate_payload_against_definition(workflow, definition_hash, &event.payload)?;
    match &event.payload {
      RunEventPayload::ParallelGroupStarted(data) => {
        let group = workflow
          .parallel_group(&data.parallel_id)
          .ok_or_else(|| format!("Unknown parallel group {:?}.", data.parallel_id))?;
        let prefix = crate::fold_events(&events[..index]).map_err(|error| error.to_string())?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, &prefix)?;
        if !ready.iter().any(|node_id| node_id == &group.start_node_id) {
          return Err(format!(
            "Parallel group {:?} was started before its fork was ready.",
            data.parallel_id
          ));
        }
      }
      RunEventPayload::StepAttemptStarted(data) => {
        let prefix = crate::fold_events(&events[..index]).map_err(|error| error.to_string())?;
        let ready =
          ready_node_ids_for_projection_at(workflow, definition_hash, &prefix, event.occurred_at)?;
        if !ready.iter().any(|node_id| node_id == &data.node_id) {
          return Err(format!(
            "Node {:?} started before it was ready.",
            data.node_id
          ));
        }
        if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
          let expected_key =
            step_effect_idempotency_key(&event.run_id, definition_hash, &data.node_id);
          if data.idempotency_key.as_deref() != Some(expected_key.as_str()) {
            return Err(format!(
              "Node {:?} attempt does not carry its derived stable idempotency key.",
              data.node_id
            ));
          }
        }
        let Some(group) = workflow.parallel_group_for_child(&data.node_id) else {
          continue;
        };
        if !matches!(
          prefix
            .parallel_groups
            .get(&group.parallel_id)
            .map(|state| &state.status),
          Some(ParallelGroupStatus::Started)
        ) {
          return Err(format!(
            "Parallel child {:?} started outside its active group {:?}.",
            data.node_id, group.parallel_id
          ));
        }
        let active = group
          .child_node_ids
          .iter()
          .filter(|node_id| child_states.get(*node_id) == Some(&ParallelChildState::Active))
          .count();
        if active >= group.concurrency {
          return Err(format!(
            "Parallel group {:?} exceeds its concurrency cap of {}.",
            group.parallel_id, group.concurrency
          ));
        }
        child_states.insert(data.node_id.clone(), ParallelChildState::Active);
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(data.node_id.clone(), ParallelChildState::Succeeded);
        }
      }
      RunEventPayload::StepAttemptFailed(data) => {
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(
            data.node_id.clone(),
            ParallelChildState::Failed(data.failure.kind),
          );
        }
        if let Some(policy) = workflow
          .node(&data.node_id)
          .and_then(|node| node.retry_policy.as_ref())
        {
          let should_retry = data.failure.kind == AttemptFailureKind::ScriptThrew
            && data.attempt < policy.max_attempts;
          if should_retry
            && !matches!(
              events.get(index + 1).map(|event| &event.payload),
              Some(RunEventPayload::StepRetryScheduled(schedule))
                if schedule.node_id == data.node_id
                  && schedule.failed_attempt == data.attempt
                  && schedule.next_attempt == data.attempt + 1
            )
          {
            return Err(format!(
              "Retryable failure for node {:?} must be atomically followed by step_retry_scheduled.",
              data.node_id
            ));
          }
        }
      }
      RunEventPayload::StepRetryScheduled(data) => {
        let previous = events
          .get(index.wrapping_sub(1))
          .ok_or_else(|| "step_retry_scheduled cannot be the first run event.".to_string())?;
        let RunEventPayload::StepAttemptFailed(failed) = &previous.payload else {
          return Err(
            "step_retry_scheduled must immediately follow step_attempt_failed.".to_string(),
          );
        };
        let policy = workflow
          .node(&data.node_id)
          .and_then(|node| node.retry_policy.as_ref())
          .ok_or_else(|| format!("Node {:?} has no retry policy.", data.node_id))?;
        let delay_ms = policy
          .delay_before_attempt(data.next_attempt)
          .ok_or_else(|| "Compiled retry delay is invalid.".to_string())?;
        let delay = chrono::Duration::milliseconds(
          i64::try_from(delay_ms)
            .map_err(|_| "Retry delay exceeds the clock range.".to_string())?,
        );
        if failed.node_id != data.node_id
          || failed.attempt != data.failed_attempt
          || failed.failure.kind != AttemptFailureKind::ScriptThrew
          || data.scheduled_at != previous.occurred_at + delay
        {
          return Err(format!(
            "Retry schedule for node {:?} does not match its failed attempt and compiled backoff.",
            data.node_id
          ));
        }
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(data.node_id.clone(), ParallelChildState::RetryPending);
        }
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        let group = workflow
          .parallel_group(&data.parallel_id)
          .ok_or_else(|| format!("Unknown parallel group {:?}.", data.parallel_id))?;
        let failed = group
          .child_node_ids
          .iter()
          .filter(|node_id| {
            matches!(
              child_states.get(*node_id),
              Some(ParallelChildState::Failed(kind))
                if *kind != AttemptFailureKind::InvocationCancelled
            )
          })
          .cloned()
          .collect::<Vec<_>>();
        let cancelled = group
          .child_node_ids
          .iter()
          .filter(|node_id| {
            matches!(
              child_states.get(*node_id),
              Some(ParallelChildState::Failed(
                AttemptFailureKind::InvocationCancelled
              ))
            )
          })
          .cloned()
          .collect::<Vec<_>>();
        let active = group
          .child_node_ids
          .iter()
          .any(|node_id| child_states.get(node_id) == Some(&ParallelChildState::Active));
        let every_succeeded = group
          .child_node_ids
          .iter()
          .all(|node_id| child_states.get(node_id) == Some(&ParallelChildState::Succeeded));
        let every_terminal = group.child_node_ids.iter().all(|node_id| {
          matches!(
            child_states.get(node_id),
            Some(ParallelChildState::Succeeded | ParallelChildState::Failed(_))
          )
        });
        let valid_outcome = match data.outcome {
          ParallelGroupOutcome::Succeeded => every_succeeded,
          ParallelGroupOutcome::Failed if group.on_error == "wait-all" => every_terminal,
          ParallelGroupOutcome::Failed => !active,
        };
        if !valid_outcome || data.failed_node_ids != failed || data.cancelled_node_ids != cancelled
        {
          return Err(format!(
            "Parallel completion for {:?} does not match its durable child outcomes.",
            data.parallel_id
          ));
        }
      }
      RunEventPayload::ApprovalRequested(data) => {
        let approval = workflow
          .approval(&data.approval_id)
          .ok_or_else(|| format!("Unknown approval {:?}.", data.approval_id))?;
        let prefix = crate::fold_events(&events[..index]).map_err(|error| error.to_string())?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, &prefix)?;
        if !ready.iter().any(|node_id| node_id == &data.approval_id) {
          return Err(format!(
            "Approval {:?} was requested before its wait node was ready.",
            data.approval_id
          ));
        }
        let expected_expires_at = approval
          .timeout_ms
          .map(|milliseconds| {
            i64::try_from(milliseconds)
              .map(chrono::Duration::milliseconds)
              .map(|duration| event.occurred_at + duration)
          })
          .transpose()
          .map_err(|_| {
            format!(
              "Approval {:?} timeout exceeds the clock range.",
              data.approval_id
            )
          })?;
        if data.expires_at != expected_expires_at {
          return Err(format!(
            "Approval {:?} request deadline does not match its compiled timeout.",
            data.approval_id
          ));
        }
      }
      RunEventPayload::ApprovalResolved(_) => {}
      RunEventPayload::RunSucceeded(_) => {
        let projection =
          crate::fold_events(&events[..=index]).map_err(|error| error.to_string())?;
        if projection.parallel_groups.values().any(|group| {
          !matches!(
            group.status,
            ParallelGroupStatus::Completed {
              outcome: ParallelGroupOutcome::Succeeded,
              ..
            }
          )
        }) {
          return Err(
            "A run cannot succeed while a started parallel group is incomplete or failed."
              .to_string(),
          );
        }
      }
      RunEventPayload::LifecycleHookRequested(data) => {
        let expected = crate::derive_lifecycle_hook_invocation_id(
          &event.run_id,
          &data.hook_id,
          data.subject.kind,
          &data.subject.id,
        );
        if data.hook_invocation_id != expected
          || (matches!(data.subject.kind, crate::LifecycleSubjectKind::Workflow)
            && data.subject.id != event.run_id)
        {
          return Err(
            "Lifecycle hook request has a non-canonical invocation identity.".to_string(),
          );
        }
        let eligible = events
          .get(index.wrapping_sub(1))
          .is_some_and(|previous| match data.event {
            crate::LifecycleEventName::RunStart => {
              matches!(previous.payload, RunEventPayload::RunStarted(_))
            }
            crate::LifecycleEventName::RunCancel => matches!(
              previous.payload,
              RunEventPayload::RunCancellationRequested(_)
                | RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Cancelled { .. })
            ),
            crate::LifecycleEventName::RunSuccess => matches!(
              previous.payload,
              RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Succeeded { .. })
            ),
            crate::LifecycleEventName::RunFailure => matches!(
              previous.payload,
              RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Failed { .. })
            ),
            crate::LifecycleEventName::RunComplete => matches!(
              previous.payload,
              RunEventPayload::LifecycleHookCompleted(_) | RunEventPayload::RunOutcomeDecided(_)
            ),
            crate::LifecycleEventName::StepStart => matches!(
              previous.payload,
              RunEventPayload::StepAttemptStarted(ref step) if step.node_id == data.subject.id
            ),
            crate::LifecycleEventName::StepSuccess => matches!(
              previous.payload,
              RunEventPayload::StepAttemptSucceeded(ref step) if step.node_id == data.subject.id
            ),
            crate::LifecycleEventName::StepFailure => matches!(
              previous.payload,
              RunEventPayload::StepAttemptFailed(ref step) if step.node_id == data.subject.id
            ),
            crate::LifecycleEventName::StepComplete => matches!(
              previous.payload,
              RunEventPayload::LifecycleHookCompleted(_)
                | RunEventPayload::StepAttemptSucceeded(_)
                | RunEventPayload::StepAttemptFailed(_)
            ),
          });
        if !eligible {
          return Err(
            "Lifecycle hook request is not adjacent to its eligibility event.".to_string(),
          );
        }
      }
      RunEventPayload::LifecycleActionAttemptStarted(data) => {
        let prefix = crate::fold_events(&events[..index]).map_err(|error| error.to_string())?;
        let hook_projection = prefix
          .lifecycle_hooks
          .get(&data.hook_invocation_id)
          .ok_or_else(|| "Lifecycle action has no requested hook.".to_string())?;
        let hook = workflow
          .lifecycle_hook(&hook_projection.hook_id)
          .ok_or_else(|| "Lifecycle action hook is absent from Model v11.".to_string())?;
        let expected = hook
          .actions
          .get(hook_projection.actions.len())
          .ok_or_else(|| {
            "Lifecycle hook has no remaining action at this source-order position.".to_string()
          })?;
        if expected.action_id != data.action_id {
          return Err("Lifecycle actions must start in compiled source order.".to_string());
        }
      }
      RunEventPayload::LifecycleHookCompleted(data) => {
        let prefix = crate::fold_events(&events[..index]).map_err(|error| error.to_string())?;
        let hook_projection = prefix
          .lifecycle_hooks
          .get(&data.hook_invocation_id)
          .ok_or_else(|| "Lifecycle completion has no requested hook.".to_string())?;
        let hook = workflow
          .lifecycle_hook(&hook_projection.hook_id)
          .ok_or_else(|| "Lifecycle completion hook is absent from Model v11.".to_string())?;
        if hook_projection.actions.len() != hook.actions.len() {
          return Err(
            "Lifecycle hook cannot complete before every compiled action has an outcome."
              .to_string(),
          );
        }
      }
      RunEventPayload::RunAdmitted(_)
      | RunEventPayload::RunExecutionStarted(_)
      | RunEventPayload::RunTimeoutReached(_)
      | RunEventPayload::RunStarted(_)
      | RunEventPayload::BranchSelected(_)
      | RunEventPayload::NotificationDeliveryRequested(_)
      | RunEventPayload::NotificationDeliveryAttemptStarted(_)
      | RunEventPayload::NotificationDeliverySucceeded(_)
      | RunEventPayload::NotificationDeliveryFailed(_)
      | RunEventPayload::NotificationDecisionAccepted(_)
      | RunEventPayload::NotificationMessageUpdateRequested(_)
      | RunEventPayload::NotificationMessageUpdateAttemptStarted(_)
      | RunEventPayload::NotificationMessageUpdated(_)
      | RunEventPayload::NotificationMessageUpdateFailed(_)
      | RunEventPayload::OperationStarted(_)
      | RunEventPayload::OperationSucceeded(_)
      | RunEventPayload::OperationFailed(_)
      | RunEventPayload::RunCancellationRequested(_)
      | RunEventPayload::LifecycleActionSucceeded(_)
      | RunEventPayload::LifecycleActionFailed(_)
      | RunEventPayload::RunOutcomeDecided(_)
      | RunEventPayload::RunFinalized(_)
      | RunEventPayload::RunFailed(_) => {}
    }
  }
  Ok(())
}

fn validate_operation_node(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
) -> Result<(), String> {
  if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V8 {
    return Err("Operation events require compiled workflow Model v8.".to_string());
  }
  if workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
    lifecycle
      .hooks
      .iter()
      .flat_map(|hook| &hook.actions)
      .any(|action| {
        action.action_id == node_id
          && matches!(
            action.handler.as_str(),
            "runtime.lifecycle-script" | "notification.informational"
          )
      })
  }) {
    return Ok(());
  }
  let node = workflow
    .node(node_id)
    .ok_or_else(|| format!("Operation references unknown node or lifecycle action {node_id:?}."))?;
  if node.handler != "runtime.script" || node.script_runtime.is_none() {
    return Err(format!(
      "Operation node {node_id:?} is not a Model v8 runtime.script node."
    ));
  }
  Ok(())
}
