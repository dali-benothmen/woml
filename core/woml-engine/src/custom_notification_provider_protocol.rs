use std::collections::{BTreeMap, HashSet};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL: &str = "woml.custom-notification-provider";
pub const CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION: u32 = 1;
pub const CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES: usize = 1024 * 1024;

const HEADER_PREFIX: &str = "Content-Length: ";
const HEADER_TERMINATOR: &[u8] = b"\r\n\r\n";
const MAX_HEADER_BYTES: usize = 128;

fn valid_id(value: &str) -> bool {
  !value.is_empty() && value.chars().count() <= 320
}

fn valid_sha256(value: &str) -> bool {
  value.len() == 71
    && value.starts_with("sha256:")
    && value[7..]
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_code(value: &str) -> bool {
  value.strip_prefix("WOML_").is_some_and(|suffix| {
    !suffix.is_empty()
      && suffix
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
  })
}

fn valid_uri(value: &str) -> bool {
  if value.is_empty()
    || value.len() > 2048
    || value
      .chars()
      .any(|character| character.is_whitespace() || character.is_control())
  {
    return false;
  }
  let Some((scheme, remainder)) = value.split_once(':') else {
    return false;
  };
  !remainder.is_empty()
    && scheme
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_alphabetic())
    && scheme
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn valid_envelope(protocol: &str, protocol_version: u32) -> bool {
  protocol == CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL
    && protocol_version == CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderReadyMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub host_instance_id: String,
}

impl CustomProviderReadyMessage {
  pub fn validate(&self) -> Result<(), String> {
    if valid_envelope(&self.protocol, self.protocol_version)
      && self.message_type == "ready"
      && valid_id(&self.host_instance_id)
    {
      Ok(())
    } else {
      Err("The custom-provider host did not send a valid ready message.".to_string())
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomNotificationKind {
  Approval,
  Informational,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomNotificationAction {
  pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomNotificationActions {
  pub approve: CustomNotificationAction,
  pub reject: CustomNotificationAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomNotificationRequest {
  pub kind: CustomNotificationKind,
  pub message: String,
  pub delivery_id: String,
  pub idempotency_key: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub actions: Option<CustomNotificationActions>,
}

impl CustomNotificationRequest {
  fn validate(&self) -> bool {
    let actions_valid = match (self.kind, &self.actions) {
      (CustomNotificationKind::Approval, Some(actions)) => {
        valid_uri(&actions.approve.url) && valid_uri(&actions.reject.url)
      }
      (CustomNotificationKind::Informational, None) => true,
      _ => false,
    };
    (1..=16_384).contains(&self.message.chars().count())
      && valid_id(&self.delivery_id)
      && valid_sha256(&self.idempotency_key)
      && actions_valid
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProviderAttempt {
  pub number: u32,
  pub max: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderLimits {
  pub timeout_ms: u64,
  pub max_result_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderExecuteMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub definition_digest: String,
  pub script_artifact_id: String,
  pub props: BTreeMap<String, Value>,
  pub notification: CustomNotificationRequest,
  pub attempt: CustomProviderAttempt,
  pub limits: CustomProviderLimits,
}

impl CustomProviderExecuteMessage {
  pub fn validate(&self) -> Result<(), String> {
    if valid_envelope(&self.protocol, self.protocol_version)
      && self.message_type == "execute"
      && valid_id(&self.invocation_id)
      && valid_sha256(&self.definition_digest)
      && valid_id(&self.script_artifact_id)
      && self.props.len() <= 128
      && self.notification.validate()
      && self.attempt.number >= 1
      && self.attempt.max >= 1
      && self.attempt.number <= self.attempt.max
      && self.limits.timeout_ms >= 1
      && self.limits.max_result_bytes >= 1
    {
      Ok(())
    } else {
      Err("The custom-provider host received an invalid execute message.".to_string())
    }
  }

  pub fn validate_with_artifacts(
    &self,
    script_artifact_ids: &HashSet<String>,
  ) -> Result<(), String> {
    self.validate()?;
    if script_artifact_ids.contains(&self.script_artifact_id) {
      Ok(())
    } else {
      Err(format!(
        "Custom-provider script artifact {:?} is not registered.",
        self.script_artifact_id
      ))
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderCancelMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
}

impl CustomProviderCancelMessage {
  pub fn validate(&self) -> Result<(), String> {
    if valid_envelope(&self.protocol, self.protocol_version)
      && self.message_type == "cancel"
      && valid_id(&self.invocation_id)
    {
      Ok(())
    } else {
      Err("The custom-provider host received an invalid cancel message.".to_string())
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomProviderFailureKind {
  ScriptThrew,
  TimedOut,
  Cancelled,
  NonJson,
  WorkerCrashed,
  HostCrashed,
  ContextTooLarge,
  ResultTooLarge,
  DeliveryAmbiguous,
  ServiceFailed,
  RequestInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProviderFailure {
  pub kind: CustomProviderFailureKind,
  pub code: String,
  pub message: String,
  pub retryable: bool,
}

impl CustomProviderFailure {
  fn validate(&self) -> bool {
    valid_code(&self.code) && (1..=1024).contains(&self.message.chars().count())
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderReceipt {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub message_id: Option<String>,
}

impl CustomProviderReceipt {
  fn validate(&self) -> bool {
    self
      .message_id
      .as_ref()
      .is_none_or(|message_id| (1..=512).contains(&message_id.chars().count()))
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CustomProviderOutcome {
  Succeeded { receipt: CustomProviderReceipt },
  Failed { error: CustomProviderFailure },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderCompletedMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub duration_ms: f64,
  pub outcome: CustomProviderOutcome,
}

impl CustomProviderCompletedMessage {
  pub fn validate(&self) -> Result<(), String> {
    let outcome_valid = match &self.outcome {
      CustomProviderOutcome::Succeeded { receipt } => receipt.validate(),
      CustomProviderOutcome::Failed { error } => error.validate(),
    };
    if valid_envelope(&self.protocol, self.protocol_version)
      && self.message_type == "completed"
      && valid_id(&self.invocation_id)
      && self.duration_ms.is_finite()
      && self.duration_ms >= 0.0
      && outcome_valid
    {
      Ok(())
    } else {
      Err("The custom-provider host returned an invalid completion message.".to_string())
    }
  }
}

pub fn encode_custom_provider_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
  let body = serde_json::to_vec(message)
    .map_err(|error| format!("Custom-provider message is not JSON serializable: {error}"))?;
  let header = format!("{HEADER_PREFIX}{}\r\n\r\n", body.len());
  let mut frame = Vec::with_capacity(header.len() + body.len());
  frame.extend_from_slice(header.as_bytes());
  frame.extend_from_slice(&body);
  Ok(frame)
}

pub fn decode_custom_provider_frame<T: DeserializeOwned>(
  frame: &[u8],
  max_frame_bytes: usize,
) -> Result<T, String> {
  let Some(header_end) = frame
    .windows(HEADER_TERMINATOR.len())
    .position(|window| window == HEADER_TERMINATOR)
  else {
    return Err("Custom-provider frame is missing its header terminator.".to_string());
  };
  if header_end > MAX_HEADER_BYTES {
    return Err("Custom-provider frame header exceeds 128 bytes.".to_string());
  }
  let header = std::str::from_utf8(&frame[..header_end])
    .map_err(|_| "Custom-provider frame header is not ASCII.".to_string())?;
  let content_length = header
    .strip_prefix(HEADER_PREFIX)
    .and_then(|value| value.parse::<usize>().ok())
    .ok_or_else(|| "Custom-provider frame has an invalid Content-Length header.".to_string())?;
  if content_length > max_frame_bytes {
    return Err("Custom-provider frame exceeds the configured size limit.".to_string());
  }
  let body_start = header_end + HEADER_TERMINATOR.len();
  if frame.len() != body_start + content_length {
    return Err("Custom-provider frame length does not match Content-Length.".to_string());
  }
  serde_json::from_slice(&frame[body_start..])
    .map_err(|_| "Custom-provider frame body is not valid UTF-8 JSON.".to_string())
}
