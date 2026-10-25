//! Frozen Worker Protocol v1 messages and fail-closed decoding.
//!
//! The protocol authenticates worker sessions and carries one integrity-bound
//! node attempt per dispatch package. Workflow scheduling remains coordinator
//! authority and is never delegated to the worker.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{CompiledWorkflowNode, OperationalBodyV1, WorkflowContext};

pub const WORKER_PROTOCOL_PROFILE: &str = "woml.worker-protocol/v1";
pub const WORKER_PROTOCOL_VERSION: u32 = 1;
pub const WORKER_PROTOCOL_MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const WORKER_PROTOCOL_MAX_TARGETS: usize = 32;
pub const WORKER_PROTOCOL_MAX_CAPABILITIES: usize = 128;
pub const WORKER_PROTOCOL_MAX_CAPACITY: u32 = 64;
pub const REMOTE_DISPATCH_PACKAGE_PROFILE: &str = "woml.dispatch-package/v1";
pub const REMOTE_DISPATCH_PACKAGE_VERSION: u32 = 1;
pub const REMOTE_TERMINAL_RESULT_MAX_BYTES: usize = 1024 * 1024;
pub const REMOTE_OUTPUT_BATCH_MAX_RECORDS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerPlatformV1 {
  pub os: WorkerOperatingSystemV1,
  pub arch: WorkerArchitectureV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerOperatingSystemV1 {
  Linux,
  Darwin,
  Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerArchitectureV1 {
  X64,
  Arm64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerCapabilityV1 {
  pub id: String,
  pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteArtifactOperationV1 {
  Read,
  Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteArtifactGrantV1 {
  pub artifact_id: String,
  pub operation: RemoteArtifactOperationV1,
  pub expires_at: DateTime<Utc>,
  pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteCoordinatorAuthorityV1 {
  pub policy_digest: String,
  pub allowed_capabilities: Vec<WorkerCapabilityV1>,
  pub allowed_secret_names: Vec<String>,
}

/// One integrity-bound attempt. The package intentionally contains one node,
/// not a workflow graph, so a worker cannot schedule or select additional work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteDispatchPackageV1 {
  pub profile: String,
  pub version: u32,
  pub dispatch_id: String,
  pub run_id: String,
  pub node_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub iteration_key: Option<String>,
  pub attempt: u32,
  pub invocation_id: String,
  pub target: String,
  pub target_registry_digest: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub infrastructure_policy_digest: Option<String>,
  pub definition_package_root_hash: String,
  pub node: CompiledWorkflowNode,
  pub context: WorkflowContext,
  pub required_capabilities: Vec<String>,
  pub required_secrets: Vec<String>,
  pub artifact_grants: Vec<RemoteArtifactGrantV1>,
  pub deadline: DateTime<Utc>,
  pub package_digest: String,
}

impl RemoteDispatchPackageV1 {
  pub fn computed_digest(&self) -> Result<String, RemoteProtocolError> {
    let mut value =
      serde_json::to_value(self).map_err(|_| invalid("dispatch cannot be encoded"))?;
    let object = value
      .as_object_mut()
      .ok_or_else(|| invalid("dispatch package is not an object"))?;
    object.insert("packageDigest".to_string(), Value::String(String::new()));
    let bytes = canonical_json(&value).map_err(|_| invalid("dispatch cannot be canonicalized"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteEffectiveAuthorityV1 {
  pub coordinator_policy_digest: String,
  pub worker_policy_digest: String,
  pub capabilities: Vec<WorkerCapabilityV1>,
  pub secret_names: Vec<String>,
  pub artifact_grant_ids: Vec<String>,
  pub authority_digest: String,
}

impl RemoteEffectiveAuthorityV1 {
  pub fn computed_digest(&self) -> Result<String, RemoteProtocolError> {
    let mut value =
      serde_json::to_value(self).map_err(|_| invalid("authority cannot be encoded"))?;
    value
      .as_object_mut()
      .ok_or_else(|| invalid("authority is not an object"))?
      .insert("authorityDigest".to_string(), Value::String(String::new()));
    let bytes = canonical_json(&value).map_err(|_| invalid("authority cannot be canonicalized"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerHelloV1 {
  pub profile: String,
  pub protocol_version: u32,
  pub message_id: String,
  pub message_type: String,
  pub worker_id: String,
  pub sent_at: DateTime<Utc>,
  pub worker_version: String,
  pub platform: WorkerPlatformV1,
  pub targets: Vec<String>,
  pub capacity: u32,
  pub capabilities: Vec<WorkerCapabilityV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerPollV1 {
  pub profile: String,
  pub protocol_version: u32,
  pub message_id: String,
  pub message_type: String,
  pub worker_id: String,
  pub session_id: String,
  pub sent_at: DateTime<Utc>,
  pub target: String,
  pub dispatch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerShutdownV1 {
  pub profile: String,
  pub protocol_version: u32,
  pub message_id: String,
  pub message_type: String,
  pub worker_id: String,
  pub session_id: String,
  pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerDispatchFactV1 {
  pub profile: String,
  pub protocol_version: u32,
  pub message_id: String,
  pub message_type: String,
  pub worker_id: String,
  pub session_id: String,
  pub sent_at: DateTime<Utc>,
  pub target: String,
  pub dispatch_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerTerminalOutcomeV1 {
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerTerminalFailureV1 {
  pub code: String,
  pub message: String,
  pub retryable: bool,
  pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerTerminalV1 {
  pub profile: String,
  pub protocol_version: u32,
  pub message_id: String,
  pub message_type: String,
  pub worker_id: String,
  pub session_id: String,
  pub sent_at: DateTime<Utc>,
  pub target: String,
  pub dispatch_id: String,
  pub outcome: WorkerTerminalOutcomeV1,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failure: Option<WorkerTerminalFailureV1>,
  pub final_output_sequence: u64,
  pub journal_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteOperationalRecordV1 {
  pub sequence: u64,
  pub body: OperationalBodyV1,
  pub record_digest: String,
}

impl RemoteOperationalRecordV1 {
  pub fn computed_digest(&self) -> Result<String, RemoteProtocolError> {
    let mut value =
      serde_json::to_value(self).map_err(|_| invalid("output record cannot be encoded"))?;
    value
      .as_object_mut()
      .ok_or_else(|| invalid("output record is not an object"))?
      .insert("recordDigest".to_string(), Value::String(String::new()));
    let bytes =
      canonical_json(&value).map_err(|_| invalid("output record cannot be canonicalized"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerOutputBatchV1 {
  pub profile: String,
  pub protocol_version: u32,
  pub message_id: String,
  pub message_type: String,
  pub worker_id: String,
  pub session_id: String,
  pub sent_at: DateTime<Utc>,
  pub dispatch_id: String,
  pub records: Vec<RemoteOperationalRecordV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSessionV1 {
  pub profile: &'static str,
  pub protocol_version: u32,
  pub session_id: String,
  pub worker_id: String,
  pub targets: Vec<String>,
  pub expires_at: DateTime<Utc>,
  pub dispatch_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerPollResultV1 {
  pub profile: &'static str,
  pub session_id: String,
  pub status: &'static str,
  pub dispatch: Option<Value>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteProtocolError {
  #[error("the worker protocol message exceeds its size limit")]
  TooLarge,
  #[error("the worker protocol is incompatible")]
  Incompatible,
  #[error("the worker protocol message is invalid: {0}")]
  Invalid(String),
}

impl RemoteProtocolError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::TooLarge | Self::Invalid(_) => "WOML_REMOTE_PROTOCOL_INVALID",
      Self::Incompatible => "WOML_TARGET_INCOMPATIBLE",
    }
  }
}

pub fn decode_worker_hello(source: &[u8]) -> Result<WorkerHelloV1, RemoteProtocolError> {
  let message: WorkerHelloV1 = decode(source)?;
  validate_base(
    &message.profile,
    message.protocol_version,
    &message.message_id,
    &message.worker_id,
  )?;
  if message.message_type != "hello" || !valid_semver(&message.worker_version) {
    return Err(invalid("hello type or worker version is invalid"));
  }
  if message.targets.is_empty() || message.targets.len() > WORKER_PROTOCOL_MAX_TARGETS {
    return Err(invalid("hello targets exceed their bounds"));
  }
  unique_identifiers(&message.targets, false, "hello targets")?;
  if message.capacity == 0 || message.capacity > WORKER_PROTOCOL_MAX_CAPACITY {
    return Err(invalid("hello capacity is outside 1 through 64"));
  }
  if message.capabilities.is_empty()
    || message.capabilities.len() > WORKER_PROTOCOL_MAX_CAPABILITIES
  {
    return Err(invalid("hello capabilities exceed their bounds"));
  }
  let mut capabilities = BTreeSet::new();
  for capability in &message.capabilities {
    validate_identifier(&capability.id, true, "capability")?;
    if capability.version == 0 || capability.version > 65_535 {
      return Err(invalid("capability version is outside 1 through 65535"));
    }
    if !capabilities.insert(capability.id.as_str()) {
      return Err(invalid("hello contains a duplicate capability"));
    }
  }
  Ok(message)
}

pub fn decode_remote_dispatch_package(
  source: &[u8],
) -> Result<RemoteDispatchPackageV1, RemoteProtocolError> {
  let package: RemoteDispatchPackageV1 = decode(source)?;
  if package.profile != REMOTE_DISPATCH_PACKAGE_PROFILE
    || package.version != REMOTE_DISPATCH_PACKAGE_VERSION
    || package.package_digest != package.computed_digest()?
  {
    return Err(invalid("dispatch package version or digest is invalid"));
  }
  Ok(package)
}

pub fn decode_worker_poll(source: &[u8]) -> Result<WorkerPollV1, RemoteProtocolError> {
  let message: WorkerPollV1 = decode(source)?;
  validate_base(
    &message.profile,
    message.protocol_version,
    &message.message_id,
    &message.worker_id,
  )?;
  if message.message_type != "poll"
    || !valid_protocol_id(&message.session_id)
    || !valid_identifier(&message.target, false)
    || !valid_protocol_id(&message.dispatch_id)
  {
    return Err(invalid("poll identity is invalid"));
  }
  Ok(message)
}

pub fn decode_worker_shutdown(source: &[u8]) -> Result<WorkerShutdownV1, RemoteProtocolError> {
  let message: WorkerShutdownV1 = decode(source)?;
  validate_base(
    &message.profile,
    message.protocol_version,
    &message.message_id,
    &message.worker_id,
  )?;
  if message.message_type != "shutdown" || !valid_protocol_id(&message.session_id) {
    return Err(invalid("shutdown identity is invalid"));
  }
  Ok(message)
}

pub fn decode_worker_dispatch_fact(
  source: &[u8],
  expected_type: &str,
) -> Result<WorkerDispatchFactV1, RemoteProtocolError> {
  let message: WorkerDispatchFactV1 = decode(source)?;
  validate_dispatch_base(
    &message.profile,
    message.protocol_version,
    &message.message_id,
    &message.worker_id,
    &message.session_id,
    &message.target,
    &message.dispatch_id,
  )?;
  if message.message_type != expected_type
    || !matches!(expected_type, "accepted" | "started" | "cancel")
  {
    return Err(invalid("dispatch fact type is invalid"));
  }
  Ok(message)
}

pub fn decode_worker_terminal(source: &[u8]) -> Result<WorkerTerminalV1, RemoteProtocolError> {
  let message: WorkerTerminalV1 = decode(source)?;
  validate_dispatch_base(
    &message.profile,
    message.protocol_version,
    &message.message_id,
    &message.worker_id,
    &message.session_id,
    &message.target,
    &message.dispatch_id,
  )?;
  if message.message_type != "terminal" || !valid_digest(&message.journal_digest) {
    return Err(invalid("terminal identity is invalid"));
  }
  match message.outcome {
    WorkerTerminalOutcomeV1::Succeeded => {
      if message.result.is_none() || message.failure.is_some() {
        return Err(invalid("a successful terminal fact requires only a result"));
      }
    }
    WorkerTerminalOutcomeV1::Failed | WorkerTerminalOutcomeV1::Cancelled => {
      let failure = message
        .failure
        .as_ref()
        .ok_or_else(|| invalid("a failed terminal fact requires a failure"))?;
      if message.result.is_some()
        || failure.message.len() > 1024
        || !valid_failure_code(&failure.code)
        || failure.ambiguous
      {
        return Err(invalid("terminal failure is invalid or ambiguous"));
      }
    }
  }
  let encoded = canonical_json(&message).map_err(|_| invalid("terminal cannot be encoded"))?;
  if encoded.len() > REMOTE_TERMINAL_RESULT_MAX_BYTES {
    return Err(RemoteProtocolError::TooLarge);
  }
  Ok(message)
}

pub fn decode_worker_output_batch(
  source: &[u8],
) -> Result<WorkerOutputBatchV1, RemoteProtocolError> {
  let message: WorkerOutputBatchV1 = decode(source)?;
  validate_base(
    &message.profile,
    message.protocol_version,
    &message.message_id,
    &message.worker_id,
  )?;
  if message.message_type != "output"
    || !valid_protocol_id(&message.session_id)
    || !valid_dispatch_id(&message.dispatch_id)
    || message.records.is_empty()
    || message.records.len() > REMOTE_OUTPUT_BATCH_MAX_RECORDS
  {
    return Err(invalid("output batch identity or size is invalid"));
  }
  let mut previous = None;
  for record in &message.records {
    if record.sequence == 0
      || previous.is_some_and(|value| record.sequence != value + 1)
      || record.record_digest != record.computed_digest()?
    {
      return Err(invalid(
        "output records are unordered or fail integrity validation",
      ));
    }
    previous = Some(record.sequence);
  }
  Ok(message)
}

fn decode<T: for<'de> Deserialize<'de>>(source: &[u8]) -> Result<T, RemoteProtocolError> {
  if source.len() > WORKER_PROTOCOL_MAX_MESSAGE_BYTES {
    return Err(RemoteProtocolError::TooLarge);
  }
  serde_json::from_slice(source)
    .map_err(|_| invalid("JSON has malformed, duplicate, missing, or unknown fields"))
}

fn validate_base(
  profile: &str,
  version: u32,
  message_id: &str,
  worker_id: &str,
) -> Result<(), RemoteProtocolError> {
  if profile != WORKER_PROTOCOL_PROFILE || version != WORKER_PROTOCOL_VERSION {
    return Err(RemoteProtocolError::Incompatible);
  }
  if !valid_protocol_id(message_id) || !valid_identifier(worker_id, false) {
    return Err(invalid("message or worker identity is invalid"));
  }
  Ok(())
}

fn validate_dispatch_base(
  profile: &str,
  version: u32,
  message_id: &str,
  worker_id: &str,
  session_id: &str,
  target: &str,
  dispatch_id: &str,
) -> Result<(), RemoteProtocolError> {
  validate_base(profile, version, message_id, worker_id)?;
  if !valid_protocol_id(session_id)
    || !valid_identifier(target, false)
    || !valid_dispatch_id(dispatch_id)
  {
    return Err(invalid("dispatch identity is invalid"));
  }
  Ok(())
}

fn unique_identifiers(
  values: &[String],
  capability: bool,
  label: &str,
) -> Result<(), RemoteProtocolError> {
  let mut seen = BTreeSet::new();
  for value in values {
    validate_identifier(value, capability, label)?;
    if !seen.insert(value) {
      return Err(invalid(&format!("{label} contain a duplicate")));
    }
  }
  Ok(())
}

fn validate_identifier(
  value: &str,
  capability: bool,
  label: &str,
) -> Result<(), RemoteProtocolError> {
  if valid_identifier(value, capability) {
    Ok(())
  } else {
    Err(invalid(&format!("{label} contains an invalid identifier")))
  }
}

fn valid_identifier(value: &str, capability: bool) -> bool {
  let bytes = value.as_bytes();
  let separators: &[u8] = if capability { b".-" } else { b"-" };
  !bytes.is_empty()
    && bytes.len() <= 128
    && bytes[0].is_ascii_lowercase()
    && bytes
      .iter()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || separators.contains(byte))
    && !separators.contains(bytes.last().expect("non-empty"))
    && !bytes
      .windows(2)
      .any(|pair| separators.contains(&pair[0]) && separators.contains(&pair[1]))
}

fn valid_protocol_id(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 256
    && value
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

fn valid_dispatch_id(value: &str) -> bool {
  value.len() == 73
    && value.starts_with("dispatch_")
    && value[9..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_digest(value: &str) -> bool {
  value.len() == 71
    && value.starts_with("sha256:")
    && value[7..]
      .bytes()
      .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_failure_code(value: &str) -> bool {
  value.starts_with("WOML_")
    && value.len() <= 128
    && value
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(crate) fn valid_semver(value: &str) -> bool {
  if value.is_empty() || value.len() > 64 || !value.is_ascii() {
    return false;
  }
  let core = value.split_once('-').map_or(value, |(core, suffix)| {
    if suffix.is_empty()
      || !suffix
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    {
      return "";
    }
    core
  });
  let parts: Vec<_> = core.split('.').collect();
  parts.len() == 3
    && parts
      .iter()
      .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn invalid(message: &str) -> RemoteProtocolError {
  RemoteProtocolError::Invalid(message.to_string())
}
