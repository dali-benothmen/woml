//! Private bridge contracts.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use woml_engine::ApprovalDecisionOutcome;
use woml_engine::CompiledWorkflowDefinition;
use woml_engine::RunStatus;
use woml_engine::RuntimeModuleArtifact;
use woml_engine::WorkflowCallRunRelations;

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum NativeTimerWaitSettlement {
  Resolved,
  AlreadyResolved,
  NotDue {
    #[serde(rename = "deadline")]
    deadline: DateTime<Utc>,
  },
}

#[derive(Serialize)]
pub(super) struct NativeApprovalDecisionOutcome {
  pub(super) contract: &'static str,
  pub(super) version: u32,
  #[serde(flatten)]
  pub(super) outcome: ApprovalDecisionOutcome,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeRichApprovalDecisionOutcome {
  pub(super) contract: &'static str,
  pub(super) version: u32,
  pub(super) status: &'static str,
  pub(super) run_id: String,
  pub(super) approval_id: String,
  pub(super) request_id: String,
  pub(super) review_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NativeWebhookRegistration {
  pub(super) workflow: CompiledWorkflowDefinition,
  pub(super) definition_hash: String,
  pub(super) resolved_secrets: BTreeMap<String, String>,
  #[serde(default)]
  pub(super) runtime_modules: Vec<RuntimeModuleArtifact>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeWebhookRuntimeStarted {
  pub(super) runtime_id: String,
  pub(super) host: String,
  pub(super) port: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeStoredRunRequirements {
  pub(super) contract: &'static str,
  pub(super) version: u32,
  pub(super) workflow_id: String,
  pub(super) definition_hash: String,
  pub(super) required_secrets: Vec<String>,
  pub(super) module_count: usize,
  pub(super) has_approval: bool,
  pub(super) has_notifications: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NativeTriggerIngressRequest {
  pub(super) contract: String,
  pub(super) contract_version: u32,
  pub(super) message_type: String,
  pub(super) request_id: String,
  pub(super) workflow_id: String,
  pub(super) definition_hash: String,
  pub(super) trigger_id: String,
  pub(super) trigger_handler: String,
  pub(super) source_identity: String,
  pub(super) payload: Map<String, Value>,
  pub(super) received_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeTriggerIngressAccepted {
  pub(super) contract: &'static str,
  pub(super) contract_version: u32,
  pub(super) message_type: &'static str,
  pub(super) request_id: String,
  pub(super) occurrence_id: String,
  pub(super) run_id: String,
  pub(super) duplicate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeTriggerIngressRejected {
  pub(super) contract: &'static str,
  pub(super) contract_version: u32,
  pub(super) message_type: &'static str,
  pub(super) request_id: String,
  pub(super) failure: NativeTriggerIngressFailure,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeTriggerIngressFailure {
  pub(super) code: &'static str,
  pub(super) message: &'static str,
  pub(super) retryable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NativeManualAdmissionRequest {
  pub(super) profile: String,
  #[serde(rename = "type")]
  pub(super) message_type: String,
  pub(super) request_id: String,
  pub(super) workflow_id: String,
  pub(super) trigger_id: String,
  pub(super) payload: Map<String, Value>,
  pub(super) requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeManualAdmissionAccepted {
  pub(super) profile: &'static str,
  #[serde(rename = "type")]
  pub(super) message_type: &'static str,
  pub(super) request_id: String,
  pub(super) occurrence_id: String,
  pub(super) run_id: String,
  pub(super) status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeManualAdmissionRejected {
  pub(super) profile: &'static str,
  #[serde(rename = "type")]
  pub(super) message_type: &'static str,
  pub(super) request_id: String,
  pub(super) code: &'static str,
  pub(super) message: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeRunInspection {
  pub(super) run_id: String,
  pub(super) workflow_id: String,
  pub(super) status: RunStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) terminal_node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) result: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) failure_code: Option<String>,
  pub(super) workflow_calls: WorkflowCallRunRelations,
}
