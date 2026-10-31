//! event waits.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableWaitKind {
  Duration,
  Until,
  Signal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitRegisteredData {
  pub wait_id: String,
  pub node_id: String,
  pub kind: DurableWaitKind,
  pub registered_at: DateTime<Utc>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub signals: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub correlation_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalSignalReceivedData {
  pub signal_id: String,
  pub wait_id: String,
  pub signal: String,
  pub received_at: DateTime<Utc>,
  pub idempotency_digest: String,
  pub payload: Value,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub correlation: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub authentication_class: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableWaitWinner {
  Timer,
  Signal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitSatisfiedData {
  pub wait_id: String,
  pub winner: DurableWaitWinner,
  pub result: Value,
  pub resolved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableWaitTimeoutBehavior {
  Fail,
  Continue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitTimedOutData {
  pub wait_id: String,
  pub deadline: DateTime<Utc>,
  pub resolved_at: DateTime<Utc>,
  pub behavior: DurableWaitTimeoutBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitCancelledData {
  pub wait_id: String,
  pub cancelled_at: DateTime<Utc>,
  pub cancellation_id: String,
}
