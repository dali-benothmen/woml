//! event correction.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectiveActionKind {
  Compensation,
  Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SagaCorrectionReason {
  Failure,
  Cancel,
  Success,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SagaIdentityData {
  pub saga_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompensationRegisteredData {
  pub saga_id: String,
  pub action_id: String,
  pub for_step_id: String,
  pub completion_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SagaCorrectionStartedData {
  pub saga_id: String,
  pub reason: SagaCorrectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionAttemptIdentityData {
  pub saga_id: String,
  pub action_id: String,
  pub action_kind: CorrectiveActionKind,
  pub attempt: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionAttemptFailedData {
  pub saga_id: String,
  pub action_id: String,
  pub action_kind: CorrectiveActionKind,
  pub attempt: u32,
  pub code: String,
  pub retryable: bool,
  pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionRetryScheduledData {
  pub saga_id: String,
  pub action_id: String,
  pub action_kind: CorrectiveActionKind,
  pub attempt: u32,
  pub retry_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionInterruptedData {
  pub saga_id: String,
  pub action_id: String,
  pub action_kind: CorrectiveActionKind,
  pub attempt: u32,
  pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualRecoveryRequiredData {
  pub saga_id: String,
  pub action_id: String,
  pub action_kind: CorrectiveActionKind,
  pub code: String,
}
