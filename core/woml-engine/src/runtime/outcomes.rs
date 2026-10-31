//! Runtime outcomes, recorded execution order, and final-result reconstruction.

use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::RuntimeDagEngine;
use crate::RunEventPayload;

use crate::event::ApprovalTimeoutPolicy;
use crate::ReviewPayloadV1;
use crate::RunEvent;
use crate::WorkflowContext;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionResult {
  pub workflow_id: String,
  pub run_id: String,
  pub terminal_node_id: String,
  pub result: Value,
  pub context: WorkflowContext,
  pub execution_order: Vec<String>,
  pub events: Vec<RunEvent>,
}

pub const RUNTIME_OUTCOME_CONTRACT: &str = "woml.runtime-outcome";

pub const RUNTIME_OUTCOME_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum WorkflowRuntimeOutcome {
  Succeeded {
    contract: &'static str,
    version: u32,
    execution: WorkflowExecutionResult,
  },
  Waiting {
    contract: &'static str,
    version: u32,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    #[serde(rename = "runId")]
    run_id: String,
    approval: WaitingWorkflowApproval,
  },
  #[serde(rename = "waiting_time")]
  WaitingTime {
    contract: &'static str,
    version: u32,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    #[serde(rename = "runId")]
    run_id: String,
    wait: WaitingWorkflowTimer,
  },
  #[serde(rename = "waiting_signal")]
  WaitingSignal {
    contract: &'static str,
    version: u32,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    #[serde(rename = "runId")]
    run_id: String,
    wait: WaitingWorkflowSignal,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingWorkflowApproval {
  pub approval_id: String,
  pub request_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub token: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub credential_expires_at: Option<chrono::DateTime<chrono::Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub review_digest: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub review: Option<ReviewPayloadV1>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub authority: Option<WaitingRichApprovalAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingRichApprovalAuthority {
  pub required: u32,
  pub received: u32,
  pub required_roles: Vec<String>,
  pub represented_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingWorkflowTimer {
  pub wait_id: String,
  pub node_id: String,
  pub kind: crate::DurableWaitKind,
  pub registered_at: chrono::DateTime<chrono::Utc>,
  pub deadline: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingWorkflowSignal {
  pub wait_id: String,
  pub node_id: String,
  pub registered_at: chrono::DateTime<chrono::Utc>,
  pub signals: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<chrono::DateTime<chrono::Utc>>,
}

impl WorkflowRuntimeOutcome {
  pub(super) fn succeeded(execution: WorkflowExecutionResult) -> Self {
    Self::Succeeded {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      execution,
    }
  }
}

pub(in crate::runtime) fn succeeded_execution(
  outcome: WorkflowRuntimeOutcome,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  match outcome {
    WorkflowRuntimeOutcome::Succeeded { execution, .. } => Ok(execution),
    WorkflowRuntimeOutcome::Waiting { .. } => Err(RuntimeExecutionError::Stalled(
      "workflow is durably waiting for approval; use the runtime outcome API".to_string(),
    )),
    WorkflowRuntimeOutcome::WaitingTime { .. } => Err(RuntimeExecutionError::Stalled(
      "workflow is durably waiting for time; use the runtime outcome API".to_string(),
    )),
    WorkflowRuntimeOutcome::WaitingSignal { .. } => Err(RuntimeExecutionError::Stalled(
      "workflow is durably waiting for an external signal; use the runtime outcome API".to_string(),
    )),
  }
}

pub(in crate::runtime) fn recorded_execution_order(events: &[RunEvent]) -> Vec<String> {
  events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptSucceeded(data) => Some(data.node_id.clone()),
      RunEventPayload::WaitSatisfied(data) => Some(data.wait_id.clone()),
      RunEventPayload::WaitTimedOut(data)
        if data.behavior == crate::DurableWaitTimeoutBehavior::Continue =>
      {
        Some(data.wait_id.clone())
      }
      _ => None,
    })
    .collect()
}

pub(in crate::runtime) fn final_result<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  execution_order: Vec<String>,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let terminal_node_id = projection
    .terminal_node_id
    .or_else(|| engine.workflow().terminal_node_id().map(str::to_string))
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled("successful run has no terminal node".to_string())
    })?;
  let result = projection
    .result
    .ok_or_else(|| RuntimeExecutionError::Stalled("successful run has no result".to_string()))?;
  Ok(WorkflowExecutionResult {
    workflow_id: engine.workflow().workflow_id.clone(),
    run_id: run_id.to_string(),
    terminal_node_id,
    result,
    context: projection.context,
    execution_order,
    events: engine.events(run_id)?,
  })
}
