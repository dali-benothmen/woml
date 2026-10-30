//! Frozen Capability Call envelopes, descriptors, effects, constants, and result validation. Data validity is not durable invocation authority.

use crate::capability::failure;
use crate::capability::valid_digest;
use crate::capability::valid_name;
use crate::capability::validate_result_base;
use crate::capability::validate_safe_metadata;
use serde::de::Error as _;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

pub const CAPABILITY_CALL_CONTRACT: &str = "woml.capability-call";

pub const CAPABILITY_CALL_CONTRACT_VERSION: u32 = 1;

pub const DEFAULT_CAPABILITY_INPUT_BYTES: u64 = 1_048_576;

pub const DEFAULT_CAPABILITY_RESULT_BYTES: u64 = 4_194_304;

pub const DEFAULT_CAPABILITY_FRAME_BYTES: u64 = 8_388_608;

pub const DEFAULT_CAPABILITY_TIMEOUT_MS: u64 = 30_000;

pub const MAX_CAPABILITY_TIMEOUT_MS: u64 = 86_400_000;

pub const DEFAULT_CAPABILITY_IN_FLIGHT_PER_INVOCATION: usize = 32;

pub const DEFAULT_CAPABILITY_IN_FLIGHT_GLOBAL: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityIdentityMode {
  Automatic,
  Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallIdentity {
  pub mode: CapabilityIdentityMode,
  pub step_idempotency_key: String,
  pub operation_name: String,
  pub operation_key: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub provider_idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallLimits {
  pub input_bytes: u64,
  pub result_bytes: u64,
  pub timeout_ms: u64,
}

impl Default for CapabilityCallLimits {
  fn default() -> Self {
    Self {
      input_bytes: DEFAULT_CAPABILITY_INPUT_BYTES,
      result_bytes: DEFAULT_CAPABILITY_RESULT_BYTES,
      timeout_ms: DEFAULT_CAPABILITY_TIMEOUT_MS,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallRequest {
  pub contract: String,
  pub contract_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub run_id: String,
  pub node_id: String,
  pub attempt_number: u32,
  pub capability: String,
  pub operation: String,
  pub input_contract_version: u32,
  pub result_contract_version: u32,
  pub identity: CapabilityCallIdentity,
  pub limits: CapabilityCallLimits,
  pub input: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFailureKind {
  InvalidInput,
  InvalidResult,
  UnsupportedCapability,
  UnsupportedOperation,
  UnsupportedVersion,
  InputTooLarge,
  ResultTooLarge,
  FrameTooLarge,
  TimedOut,
  Cancelled,
  HandlerCrashed,
  WorkerCrashed,
  HostCrashed,
  TransportFailed,
  ServiceRejected,
  Interrupted,
  Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityFailure {
  pub kind: CapabilityFailureKind,
  pub code: String,
  pub message: String,
  pub retryable: bool,
  pub ambiguous: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallSucceeded {
  pub contract: String,
  pub contract_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub outcome: String,
  pub result_contract_version: u32,
  pub result_bytes: u64,
  pub duration_ms: f64,
  pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallFailed {
  pub contract: String,
  pub contract_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub outcome: String,
  pub duration_ms: f64,
  pub error: CapabilityFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CapabilityCallResult {
  Succeeded(CapabilityCallSucceeded),
  Failed(CapabilityCallFailed),
  Cancelled(CapabilityCallFailed),
}

impl<'de> Deserialize<'de> for CapabilityCallResult {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let value = Value::deserialize(deserializer)?;
    match value.get("outcome").and_then(Value::as_str) {
      Some("succeeded") => serde_json::from_value(value)
        .map(Self::Succeeded)
        .map_err(D::Error::custom),
      Some("failed") => serde_json::from_value(value)
        .map(Self::Failed)
        .map_err(D::Error::custom),
      Some("cancelled") => serde_json::from_value(value)
        .map(Self::Cancelled)
        .map_err(D::Error::custom),
      _ => Err(D::Error::custom(
        "Capability Call v1 result has an invalid outcome.",
      )),
    }
  }
}

impl CapabilityCallResult {
  pub fn invocation_id(&self) -> &str {
    match self {
      Self::Succeeded(result) => &result.invocation_id,
      Self::Failed(result) | Self::Cancelled(result) => &result.invocation_id,
    }
  }

  pub fn call_id(&self) -> &str {
    match self {
      Self::Succeeded(result) => &result.call_id,
      Self::Failed(result) | Self::Cancelled(result) => &result.call_id,
    }
  }

  pub fn validate(&self) -> Result<(), String> {
    match self {
      Self::Succeeded(result) => {
        validate_result_base(
          &result.contract,
          result.contract_version,
          &result.message_type,
          &result.invocation_id,
          &result.call_id,
          result.duration_ms,
        )?;
        if result.outcome != "succeeded"
          || result.result_contract_version == 0
          || result.result_bytes > DEFAULT_CAPABILITY_FRAME_BYTES
        {
          return Err("Capability success does not match Capability Call v1.".to_string());
        }
      }
      Self::Failed(result) | Self::Cancelled(result) => {
        validate_result_base(
          &result.contract,
          result.contract_version,
          &result.message_type,
          &result.invocation_id,
          &result.call_id,
          result.duration_ms,
        )?;
        let expected = if matches!(self, Self::Cancelled(_)) {
          "cancelled"
        } else {
          "failed"
        };
        if result.outcome != expected {
          return Err("Capability failure outcome does not match its result variant.".to_string());
        }
        result.error.validate()?;
      }
    }
    Ok(())
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityEffect {
  Read,
  IdempotentWrite,
  UnsafeWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDescriptor {
  pub capability: String,
  pub operation: String,
  pub input_contract_version: u32,
  pub result_contract_version: u32,
  pub effect: CapabilityEffect,
  pub supports_cancellation: bool,
  pub supports_provider_idempotency: bool,
}

impl CapabilityCallRequest {
  pub fn validate(&self) -> Result<(), CapabilityFailure> {
    let valid = self.contract == CAPABILITY_CALL_CONTRACT
      && self.contract_version == CAPABILITY_CALL_CONTRACT_VERSION
      && self.message_type == "request"
      && [
        &self.invocation_id,
        &self.call_id,
        &self.run_id,
        &self.node_id,
      ]
      .into_iter()
      .all(|value| !value.is_empty() && value.len() <= 256)
      && (1..=10).contains(&self.attempt_number)
      && valid_name(&self.capability)
      && valid_name(&self.operation)
      && self.input_contract_version > 0
      && self.result_contract_version > 0
      && valid_digest(&self.identity.step_idempotency_key)
      && valid_digest(&self.identity.operation_key)
      && !self.identity.operation_name.is_empty()
      && self.identity.operation_name.len() <= 256
      && self
        .identity
        .provider_idempotency_key
        .as_ref()
        .is_none_or(|value| !value.is_empty() && value.len() <= 256)
      && (1..=DEFAULT_CAPABILITY_FRAME_BYTES).contains(&self.limits.input_bytes)
      && (1..=DEFAULT_CAPABILITY_FRAME_BYTES).contains(&self.limits.result_bytes)
      && (1..=MAX_CAPABILITY_TIMEOUT_MS).contains(&self.limits.timeout_ms);
    if valid {
      Ok(())
    } else {
      Err(failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_CAPABILITY_REQUEST_INVALID",
        "The capability call does not match Capability Call v1.",
        false,
        false,
      ))
    }
  }
}

impl CapabilityFailure {
  pub fn validate(&self) -> Result<(), String> {
    let code_valid = !self.code.is_empty()
      && self.code.len() <= 128
      && self
        .code
        .starts_with(|character: char| character.is_ascii_uppercase())
      && self
        .code
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    if !code_valid || self.message.is_empty() || self.message.len() > 2048 {
      return Err("Capability failure has an invalid code or message.".to_string());
    }
    if let Some(details) = &self.details {
      validate_safe_metadata(details)?;
    }
    Ok(())
  }
}
