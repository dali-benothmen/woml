//! event notifications.

use super::{ApprovalDecision, EventValidationError};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliveryRequestedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliveryAttemptStartedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProviderMessageIdentity {
  Slack(SlackProviderMessageIdentity),
  Communication(CommunicationProviderMessageIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlackProviderMessageIdentity {
  pub workspace_id: String,
  pub channel_id: String,
  pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunicationProviderMessageIdentity {
  pub provider: String,
  pub account_id: String,
  pub conversation_id: String,
  pub message_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliverySucceededData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  pub provider_message: ProviderMessageIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationSafeFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
  pub retryable: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliveryFailedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  #[serde(rename = "final")]
  pub final_: bool,
  pub failure: NotificationSafeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDecisionAcceptedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub provider_actor_id: String,
  pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationResolution {
  Approved,
  Rejected,
  TimeoutFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationMessageUpdateRequestedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub resolution: NotificationResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationMessageUpdateAttemptStartedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub attempt: u32,
  pub attempt_id: String,
}

pub type NotificationMessageUpdatedData = NotificationMessageUpdateAttemptStartedData;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationMessageUpdateFailedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  #[serde(rename = "final")]
  pub final_: bool,
  pub failure: NotificationSafeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationRunFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
}

impl NotificationSafeFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    const KINDS: &[&str] = &[
      "secret_not_found",
      "provider_auth_failed",
      "destination_invalid",
      "rate_limited",
      "provider_unavailable",
      "delivery_ambiguous",
      "host_crashed",
      "size_limit_exceeded",
      "request_invalid",
      "update_failed",
    ];
    if !KINDS.contains(&self.kind.as_str())
      || !self.code.starts_with("WOML_")
      || !self
        .code
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
      || self.message.is_empty()
      || self.message.len() > 1024
      || self.retry_after_ms.is_some_and(|value| value > 86_400_000)
    {
      return Err(EventValidationError::Invalid(
        "Notification failure contains an invalid safe failure contract.".to_string(),
      ));
    }
    Ok(())
  }
}
