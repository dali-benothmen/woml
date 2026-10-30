//! runtime outcomes.

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
