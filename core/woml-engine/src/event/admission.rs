//! event admission.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAdmissionTrigger {
  pub id: String,
  pub handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAdmissionQueue {
  pub name: String,
  pub discipline: crate::model::QueueDiscipline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAdmittedData {
  pub definition_hash: String,
  pub policy_hash: String,
  pub trigger: RunAdmissionTrigger,
  pub payload: Map<String, Value>,
  pub queue: RunAdmissionQueue,
  pub admitted_at: DateTime<Utc>,
  pub occurrence_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunExecutionStartedData {
  pub started_at: DateTime<Utc>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunTimeoutReachedData {
  pub deadline_at: DateTime<Utc>,
  pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunCancellationRequestedData {
  pub request_id: String,
}
