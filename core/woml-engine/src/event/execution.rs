//! event execution.

use super::AttemptFailure;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunStartedData {
  pub workflow_id: String,
  pub definition_hash: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub trigger_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub trigger_handler: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub trigger_occurrence_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ingress: Option<RunIngress>,
  pub trigger: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunIngress {
  WorkflowCall { call_key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptStartedData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub handler: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptSucceededData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptFailedData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub failure: AttemptFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepRetryScheduledData {
  pub node_id: String,
  pub failed_attempt: u32,
  pub next_attempt: u32,
  pub scheduled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchSelectedData {
  pub branch_id: String,
  pub arm_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChoiceSelectedData {
  pub choice_id: String,
  pub arm_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSucceededData {
  pub terminal_node_id: String,
  pub result: Value,
}
