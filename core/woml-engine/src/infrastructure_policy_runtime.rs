//! Trusted operation descriptors, evidence references, and the deterministic
//! Infrastructure Policy v1 evaluator.
//!
//! This module is intentionally side-effect free. It never reads a workflow
//! payload, filesystem, provider, database, or network resource while making a
//! decision. Capability handlers first validate requests and then expose only
//! the bounded metadata defined here.

use std::{
  collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
  sync::{Arc, Mutex},
  time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};

use crate::{
  CapabilityCallRequest, CapabilityDescriptor, CapabilityEffect, CapabilityFailure,
  CapabilityFailureKind, InfrastructurePolicyDecisionV1, InfrastructurePolicyDefaultV1,
  InfrastructurePolicyEffectV1, InfrastructurePolicyExecutionModeV1,
  InfrastructurePolicyRequirementsV1, InfrastructurePolicyRuleMatchV1, InfrastructurePolicyRuleV1,
  PreparedInfrastructurePolicyV1,
};

pub const INFRASTRUCTURE_POLICY_DESCRIPTOR_CONTRACT: &str = "woml.infrastructure-operation";
pub const INFRASTRUCTURE_POLICY_EVIDENCE_CONTRACT: &str = "woml.infrastructure-evidence";
pub const INFRASTRUCTURE_POLICY_DECISION_CONTRACT: &str = "woml.infrastructure-decision";
pub const INFRASTRUCTURE_POLICY_RUNTIME_CONTRACT_VERSION: u32 = 1;
pub const INFRASTRUCTURE_POLICY_MAX_METADATA_FIELDS: usize = 32;
pub const INFRASTRUCTURE_POLICY_MAX_METADATA_VALUE_BYTES: usize = 512;
pub const INFRASTRUCTURE_POLICY_MAX_EXPLANATION_BYTES: usize = 1024;
pub const INFRASTRUCTURE_POLICY_MAX_EVIDENCE: usize = 128;
const DEFAULT_MAX_EVALUATION_INSTRUCTIONS: u64 = 250_000;
const DEFAULT_MAX_EVALUATION_DURATION: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfrastructurePolicyDescriptorContextV1 {
  pub workflow_id: String,
  pub definition_hash: String,
  pub environment: String,
  pub effective_target: Option<String>,
  pub execution_mode: InfrastructurePolicyExecutionModeV1,
  pub review_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyOperationMetadataV1 {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub logical_target: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub executable: Option<String>,
  /// Raw arguments are required for exact prefix matching but are never
  /// serialized, persisted, or copied into a decision or explanation.
  #[serde(skip)]
  arguments: Vec<String>,
  pub argument_count: usize,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub argument_digest: Option<String>,
  pub facts: BTreeMap<String, String>,
}

impl InfrastructurePolicyOperationMetadataV1 {
  fn empty() -> Self {
    Self {
      logical_target: None,
      executable: None,
      arguments: Vec::new(),
      argument_count: 0,
      argument_digest: None,
      facts: BTreeMap::new(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyOperationDescriptorV1 {
  pub contract: String,
  pub contract_version: u32,
  pub descriptor_digest: String,
  pub run_id: String,
  pub workflow_id: String,
  pub definition_hash: String,
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub effect: InfrastructurePolicyEffectV1,
  pub environment: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub target: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub executable: Option<String>,
  #[serde(skip)]
  arguments: Vec<String>,
  pub argument_count: usize,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub argument_digest: Option<String>,
  pub execution_mode: InfrastructurePolicyExecutionModeV1,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub review_digest: Option<String>,
  pub facts: BTreeMap<String, String>,
}

impl InfrastructurePolicyOperationDescriptorV1 {
  pub fn build(
    request: &CapabilityCallRequest,
    handler: &CapabilityDescriptor,
    effective_effect: CapabilityEffect,
    context: InfrastructurePolicyDescriptorContextV1,
    metadata: InfrastructurePolicyOperationMetadataV1,
  ) -> Result<Self, CapabilityFailure> {
    if request.capability != handler.capability || request.operation != handler.operation {
      return Err(policy_input_unsupported(
        "The capability handler identity does not match the policy operation.",
      ));
    }
    validate_descriptor_context(&context)?;
    validate_policy_metadata(&metadata)?;
    let effect = effect_name(effective_effect);
    let target = context.effective_target.or(metadata.logical_target);
    let digest_payload = json!({
      "runId": request.run_id,
      "workflowId": context.workflow_id,
      "definitionHash": context.definition_hash,
      "nodeId": request.node_id,
      "attemptNumber": request.attempt_number,
      "invocationId": request.invocation_id,
      "operationKey": request.identity.operation_key,
      "capability": request.capability,
      "operation": request.operation,
      "effect": effect,
      "environment": context.environment,
      "target": target,
      "executable": metadata.executable,
      "argumentCount": metadata.argument_count,
      "argumentDigest": metadata.argument_digest,
      "executionMode": context.execution_mode,
      "reviewDigest": context.review_digest,
      "facts": metadata.facts,
    });
    let descriptor_digest = digest_json(&digest_payload)?;
    Ok(Self {
      contract: INFRASTRUCTURE_POLICY_DESCRIPTOR_CONTRACT.to_string(),
      contract_version: INFRASTRUCTURE_POLICY_RUNTIME_CONTRACT_VERSION,
      descriptor_digest,
      run_id: request.run_id.clone(),
      workflow_id: context.workflow_id,
      definition_hash: context.definition_hash,
      node_id: request.node_id.clone(),
      attempt_number: request.attempt_number,
      invocation_id: request.invocation_id.clone(),
      operation_key: request.identity.operation_key.clone(),
      capability: request.capability.clone(),
      operation: request.operation.clone(),
      effect,
      environment: context.environment,
      target,
      executable: metadata.executable,
      arguments: metadata.arguments,
      argument_count: metadata.argument_count,
      argument_digest: metadata.argument_digest,
      execution_mode: context.execution_mode,
      review_digest: context.review_digest,
      facts: metadata.facts,
    })
  }

  fn arguments_for_evaluation(&self) -> &[String] {
    &self.arguments
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfrastructurePolicyCoverageV1 {
  Supported,
  Unsupported,
}

pub fn infrastructure_policy_descriptor_coverage(
  capability: &str,
  operation: &str,
) -> InfrastructurePolicyCoverageV1 {
  if matches!(
    (capability, operation),
    ("process", "run")
      | ("http", "request")
      | (
        "db",
        "query" | "execute" | "read" | "insert" | "update" | "delete" | "transaction"
      )
      | ("storage", "put" | "get" | "head" | "list" | "delete")
      | (
        "cache",
        "get" | "set" | "delete" | "has" | "increment" | "set_if_absent"
      )
      | (
        "state",
        "get" | "has" | "set" | "delete" | "increment" | "set_if_absent"
      )
      | ("events", "emit")
      | ("workflows", "call" | "start")
      | ("artifacts", "put" | "head" | "materialize")
      | ("locks", "acquire" | "assert-current" | "release")
      | ("telegram" | "discord" | "whatsapp", "send")
  ) {
    InfrastructurePolicyCoverageV1::Supported
  } else {
    InfrastructurePolicyCoverageV1::Unsupported
  }
}

pub fn adapt_infrastructure_policy_metadata(
  handler: &CapabilityDescriptor,
  request: &CapabilityCallRequest,
) -> Result<InfrastructurePolicyOperationMetadataV1, CapabilityFailure> {
  if infrastructure_policy_descriptor_coverage(&handler.capability, &handler.operation)
    != InfrastructurePolicyCoverageV1::Supported
  {
    return Err(policy_input_unsupported(
      "No trusted Infrastructure Policy adapter covers this capability operation.",
    ));
  }
  let input = &request.input;
  let mut metadata = InfrastructurePolicyOperationMetadataV1::empty();
  match (handler.capability.as_str(), handler.operation.as_str()) {
    ("process", "run") => {
      metadata.executable = string_at(input, "/executable");
      metadata.arguments = strings_at(input, "/args")?;
      metadata.argument_count = metadata.arguments.len();
      metadata.argument_digest = Some(digest_strings(&metadata.arguments));
      insert_fact(&mut metadata, "cwdClass", cwd_class(input.pointer("/cwd")))?;
    }
    ("http", "request") => {
      let url = string_at(input, "/url")
        .ok_or_else(|| policy_input_unsupported("Managed HTTP URL metadata is unavailable."))?;
      let parsed = reqwest::Url::parse(&url)
        .map_err(|_| policy_input_unsupported("Managed HTTP URL metadata is invalid."))?;
      let origin = parsed.origin().ascii_serialization();
      metadata.logical_target = Some(http_target(&parsed));
      insert_fact(&mut metadata, "method", string_at(input, "/method"))?;
      insert_fact(&mut metadata, "origin", Some(origin))?;
      insert_fact(
        &mut metadata,
        "pathClass",
        Some(format!(
          "segments:{}",
          parsed.path_segments().map_or(0, Iterator::count)
        )),
      )?;
    }
    ("db", operation) => {
      let driver = string_at(input, "/driver");
      let connection = string_at(input, "/connection");
      insert_fact(&mut metadata, "driver", driver.clone())?;
      insert_fact(
        &mut metadata,
        "operationClass",
        Some(db_operation_class(operation)),
      )?;
      if let Some(connection) = connection {
        let digest = digest_text("database-connection", &connection);
        metadata.logical_target = Some(opaque_target("database", &connection));
        insert_fact(&mut metadata, "connectionDigest", Some(digest))?;
      }
    }
    ("storage", operation) => {
      insert_fact(&mut metadata, "action", Some(operation.to_string()))?;
      let key = string_at(input, "/input/key").or_else(|| string_at(input, "/input/prefix"));
      if let Some(key) = key {
        let digest = digest_text("storage-key", &key);
        metadata.logical_target = Some(opaque_target("storage", &key));
        insert_fact(&mut metadata, "keyDigest", Some(digest))?;
      }
    }
    ("cache", operation) => {
      insert_fact(&mut metadata, "action", Some(operation.to_string()))?;
      if let Some(key) = string_at(input, "/input/key") {
        let digest = digest_text("cache-key", &key);
        metadata.logical_target = Some(opaque_target("cache", &key));
        insert_fact(&mut metadata, "keyDigest", Some(digest))?;
      }
    }
    ("state", operation) => {
      let key = crate::durable_state::validated_policy_key(input, operation)?;
      metadata.logical_target = Some(opaque_target("state", &key));
      insert_fact(&mut metadata, "action", Some(operation.to_string()))?;
      insert_fact(&mut metadata, "scopeClass", Some("workflow".to_string()))?;
      insert_fact(
        &mut metadata,
        "keyDigest",
        Some(digest_text("state-key", &key)),
      )?;
    }
    ("events", "emit") => {
      let event_name = string_at(input, "/input/name");
      metadata.logical_target = event_name
        .as_ref()
        .map(|value| matcher_target("event", value));
      insert_fact(&mut metadata, "eventName", event_name)?;
    }
    ("workflows", operation) => {
      metadata.logical_target = string_at(input, "/workflowId");
      insert_fact(&mut metadata, "invocationMode", Some(operation.to_string()))?;
    }
    ("artifacts", "put") => {
      metadata.logical_target =
        string_at(input, "/input/name").map(|value| matcher_target("artifact", &value));
      insert_fact(&mut metadata, "action", Some("put".to_string()))?;
      insert_fact(
        &mut metadata,
        "contentType",
        string_at(input, "/input/contentType"),
      )?;
    }
    ("artifacts", operation @ ("head" | "materialize")) => {
      metadata.logical_target =
        string_at(input, "/input/artifact/id").map(|value| matcher_target("artifact", &value));
      insert_fact(&mut metadata, "action", Some(operation.to_string()))?;
      insert_fact(
        &mut metadata,
        "artifactDigest",
        string_at(input, "/input/artifact/sha256")
          .map(|value| format!("sha256:{}", value.trim_start_matches("sha256:"))),
      )?;
    }
    ("locks", "acquire") => {
      let key = string_at(input, "/key");
      if let Some(key) = key {
        let digest = digest_text("lock-key", &key);
        metadata.logical_target = Some(opaque_target("lock", &key));
        insert_fact(&mut metadata, "keyDigest", Some(digest))?;
      }
      insert_fact(&mut metadata, "action", Some("acquire".to_string()))?;
    }
    ("locks", operation @ ("assert-current" | "release")) => {
      if let Some(session) = string_at(input, "/sessionId") {
        let digest = digest_text("lock-session", &session);
        metadata.logical_target = Some(opaque_target("lock-session", &session));
        insert_fact(&mut metadata, "sessionDigest", Some(digest))?;
      }
      insert_fact(&mut metadata, "action", Some(operation.to_string()))?;
    }
    (provider @ ("telegram" | "discord" | "whatsapp"), "send") => {
      let destination = string_at(input, "/conversationId");
      if let Some(destination) = destination {
        let digest = digest_text("provider-destination", &destination);
        metadata.logical_target = Some(opaque_target(provider, &destination));
        insert_fact(&mut metadata, "destinationDigest", Some(digest))?;
      }
      insert_fact(&mut metadata, "provider", Some(provider.to_string()))?;
    }
    _ => {
      return Err(policy_input_unsupported(
        "No trusted Infrastructure Policy adapter covers this capability operation.",
      ));
    }
  }
  validate_policy_metadata(&metadata)?;
  validate_adapter_completeness(&handler.capability, &handler.operation, &metadata)?;
  Ok(metadata)
}

fn validate_adapter_completeness(
  capability: &str,
  operation: &str,
  metadata: &InfrastructurePolicyOperationMetadataV1,
) -> Result<(), CapabilityFailure> {
  let complete = match (capability, operation) {
    ("process", "run") => metadata.executable.is_some(),
    ("http", "request") => {
      metadata.logical_target.is_some()
        && metadata.facts.contains_key("method")
        && metadata.facts.contains_key("origin")
    }
    ("db", _) => {
      metadata.logical_target.is_some()
        && metadata.facts.contains_key("driver")
        && metadata.facts.contains_key("operationClass")
    }
    ("storage", "list") => metadata.facts.contains_key("action"),
    ("storage", _) | ("cache", _) => metadata.logical_target.is_some(),
    ("state", _) => {
      metadata.logical_target.is_some()
        && metadata.facts.contains_key("action")
        && metadata.facts.contains_key("keyDigest")
        && metadata
          .facts
          .get("scopeClass")
          .is_some_and(|scope| scope == "workflow")
    }
    ("events", "emit") | ("workflows", "call" | "start") => metadata.logical_target.is_some(),
    ("artifacts", _) | ("locks", _) | ("telegram" | "discord" | "whatsapp", "send") => {
      metadata.logical_target.is_some()
    }
    _ => false,
  };
  if complete {
    Ok(())
  } else {
    Err(policy_input_unsupported(
      "The trusted Infrastructure Policy adapter could not produce its required metadata.",
    ))
  }
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
  value
    .pointer(pointer)
    .and_then(Value::as_str)
    .map(str::to_string)
}

fn strings_at(value: &Value, pointer: &str) -> Result<Vec<String>, CapabilityFailure> {
  value
    .pointer(pointer)
    .and_then(Value::as_array)
    .ok_or_else(|| policy_input_unsupported("Process argument metadata is unavailable."))?
    .iter()
    .map(|value| {
      value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| policy_input_unsupported("Process argument metadata is invalid."))
    })
    .collect()
}

fn insert_fact(
  metadata: &mut InfrastructurePolicyOperationMetadataV1,
  key: &str,
  value: Option<String>,
) -> Result<(), CapabilityFailure> {
  if let Some(value) = value {
    metadata.facts.insert(key.to_string(), value);
  }
  Ok(())
}

fn cwd_class(value: Option<&Value>) -> Option<String> {
  value.and_then(Value::as_str).map(|cwd| {
    if cwd == "." || cwd.is_empty() {
      "project-root".to_string()
    } else {
      "project-relative".to_string()
    }
  })
}

fn db_operation_class(operation: &str) -> String {
  match operation {
    "query" | "read" => "read",
    "transaction" => "transaction",
    _ => "write",
  }
  .to_string()
}

fn http_target(url: &reqwest::Url) -> String {
  let scheme = url.scheme();
  let host = url.host_str().unwrap_or("unknown").to_ascii_lowercase();
  let candidate = if let Some(port) = url.port() {
    format!("{scheme}-{host}-{port}")
  } else {
    format!("{scheme}-{host}")
  };
  matcher_target("http", &candidate)
}

fn matcher_target(prefix: &str, value: &str) -> String {
  if valid_policy_identifier(value) {
    value.to_string()
  } else {
    opaque_target(prefix, value)
  }
}

fn opaque_target(prefix: &str, value: &str) -> String {
  format!("{prefix}-{}", hex::encode(Sha256::digest(value.as_bytes())))
}

fn valid_policy_identifier(value: &str) -> bool {
  if value.is_empty() || value.len() > 128 {
    return false;
  }
  let mut bytes = value.bytes();
  bytes.next().is_some_and(|first| first.is_ascii_lowercase())
    && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_descriptor_context(
  context: &InfrastructurePolicyDescriptorContextV1,
) -> Result<(), CapabilityFailure> {
  for value in [
    &context.workflow_id,
    &context.definition_hash,
    &context.environment,
  ] {
    if value.is_empty() || value.len() > 256 || value.contains('\0') {
      return Err(policy_input_unsupported(
        "The trusted operation context exceeds its frozen limits.",
      ));
    }
  }
  if context
    .effective_target
    .as_ref()
    .is_some_and(|value| !valid_policy_identifier(value))
    || context
      .review_digest
      .as_ref()
      .is_some_and(|value| !valid_sha256(value))
  {
    return Err(policy_input_unsupported(
      "The trusted target or review digest is invalid.",
    ));
  }
  Ok(())
}

fn validate_policy_metadata(
  metadata: &InfrastructurePolicyOperationMetadataV1,
) -> Result<(), CapabilityFailure> {
  if metadata.arguments.len() != metadata.argument_count
    || metadata.arguments.len() > 256
    || metadata.facts.len() > INFRASTRUCTURE_POLICY_MAX_METADATA_FIELDS
    || metadata
      .arguments
      .iter()
      .any(|value| value.len() > 32_768 || value.contains('\0'))
    || metadata.facts.iter().any(|(key, value)| {
      key.is_empty()
        || key.len() > 64
        || value.len() > INFRASTRUCTURE_POLICY_MAX_METADATA_VALUE_BYTES
        || key.contains('\0')
        || value.contains('\0')
    })
    || metadata
      .logical_target
      .as_ref()
      .is_some_and(|value| !valid_policy_identifier(value))
    || metadata
      .executable
      .as_ref()
      .is_some_and(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
  {
    return Err(policy_input_unsupported(
      "The trusted policy metadata exceeds its frozen limits.",
    ));
  }
  Ok(())
}

fn effect_name(effect: CapabilityEffect) -> InfrastructurePolicyEffectV1 {
  match effect {
    CapabilityEffect::Read => InfrastructurePolicyEffectV1::Read,
    CapabilityEffect::IdempotentWrite => InfrastructurePolicyEffectV1::IdempotentWrite,
    CapabilityEffect::UnsafeWrite => InfrastructurePolicyEffectV1::UnsafeWrite,
  }
}

fn valid_sha256(value: &str) -> bool {
  value
    .strip_prefix("sha256:")
    .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn digest_strings(values: &[String]) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.infrastructure-policy.arguments\0v1\0");
  for value in values {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
  }
  format!("sha256:{:x}", digest.finalize())
}

fn digest_text(domain: &str, value: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.infrastructure-policy\0v1\0");
  digest.update(domain.as_bytes());
  digest.update(b"\0");
  digest.update(value.as_bytes());
  format!("sha256:{:x}", digest.finalize())
}

fn digest_json(value: &Value) -> Result<String, CapabilityFailure> {
  canonical_json(value)
    .map(|encoded| format!("sha256:{:x}", Sha256::digest(encoded)))
    .map_err(|_| policy_input_unsupported("The trusted descriptor cannot be canonicalized."))
}

fn policy_input_unsupported(message: &str) -> CapabilityFailure {
  CapabilityFailure {
    kind: CapabilityFailureKind::ServiceRejected,
    code: "WOML_POLICY_INPUT_UNSUPPORTED".to_string(),
    message: message.to_string(),
    retryable: false,
    ambiguous: false,
    details: None,
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InfrastructurePolicyEvidenceKindV1 {
  Approval,
  Artifact,
  Backup,
  Attestation,
  Lock,
  OperatorIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyEvidenceReferenceV1 {
  contract: String,
  contract_version: u32,
  evidence_id: String,
  evidence_digest: String,
  kind: InfrastructurePolicyEvidenceKindV1,
  authority: String,
  issued_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  expires_at: Option<DateTime<Utc>>,
  run_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  operation_key: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  review_digest: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  principal_id: Option<String>,
  roles: BTreeSet<String>,
  verified_integrity: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  lock_key: Option<String>,
}

/// The only constructor for evidence references. Workflow JSON cannot
/// deserialize or instantiate this authority, so author-provided lookalikes
/// never become evaluator evidence.
#[derive(Debug, Clone, Copy)]
pub struct InfrastructurePolicyEvidenceAuthorityV1 {
  _private: (),
}

// IP6 wires these crate-private constructors to durable approval, artifact,
// lock, and operator authorities. Keeping them crate-private is the security
// boundary that prevents embedding callers from minting evidence.
#[allow(dead_code)]
impl InfrastructurePolicyEvidenceAuthorityV1 {
  pub(crate) const fn engine() -> Self {
    Self { _private: () }
  }

  #[allow(clippy::too_many_arguments)]
  pub(crate) fn issue(
    self,
    kind: InfrastructurePolicyEvidenceKindV1,
    evidence_id: String,
    authority: String,
    issued_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    run_id: String,
    node_id: Option<String>,
    operation_key: Option<String>,
    review_digest: Option<String>,
    principal_id: Option<String>,
    roles: BTreeSet<String>,
    verified_integrity: bool,
    lock_key: Option<String>,
  ) -> Result<InfrastructurePolicyEvidenceReferenceV1, String> {
    let fields = [evidence_id.as_str(), authority.as_str(), run_id.as_str()];
    if fields
      .iter()
      .any(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
      || node_id
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
      || operation_key
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
      || review_digest
        .as_ref()
        .is_some_and(|value| !valid_sha256(value))
      || principal_id
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
      || roles.len() > 64
      || roles
        .iter()
        .any(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
      || lock_key
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
      || expires_at.is_some_and(|expires| expires <= issued_at)
    {
      return Err("engine-owned evidence exceeds its frozen limits".to_string());
    }
    let digest_payload = json!({
      "kind": kind,
      "evidenceId": evidence_id,
      "authority": authority,
      "issuedAt": issued_at,
      "expiresAt": expires_at,
      "runId": run_id,
      "nodeId": node_id,
      "operationKey": operation_key,
      "reviewDigest": review_digest,
      "principalId": principal_id,
      "roles": roles,
      "verifiedIntegrity": verified_integrity,
      "lockKeyDigest": lock_key.as_ref().map(|key| digest_text("lock-key", key)),
    });
    let encoded = canonical_json(&digest_payload)
      .map_err(|_| "engine-owned evidence cannot be canonicalized".to_string())?;
    Ok(InfrastructurePolicyEvidenceReferenceV1 {
      contract: INFRASTRUCTURE_POLICY_EVIDENCE_CONTRACT.to_string(),
      contract_version: INFRASTRUCTURE_POLICY_RUNTIME_CONTRACT_VERSION,
      evidence_id,
      evidence_digest: format!("sha256:{:x}", Sha256::digest(encoded)),
      kind,
      authority,
      issued_at,
      expires_at,
      run_id,
      node_id,
      operation_key,
      review_digest,
      principal_id,
      roles,
      verified_integrity,
      lock_key,
    })
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfrastructurePolicyDecisionOutcomeV1 {
  Allow,
  Deny,
  RequirementsUnsatisfied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfrastructurePolicyReasonCodeV1 {
  ExplicitDeny,
  ExplicitAllow,
  DefaultAllow,
  DefaultDeny,
  EnvironmentMismatch,
  RequirementsUnsatisfied,
  EvaluationBudgetExhausted,
  InputUnsupported,
  EmergencyDeny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyEmergencyDenyV1 {
  pub policy_id: String,
  pub policy_version: String,
  pub policy_digest: String,
  pub matched_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfrastructurePolicyRequirementStatusV1 {
  Satisfied,
  Unsatisfied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyRequirementResultV1 {
  pub rule_id: String,
  pub requirement: String,
  pub status: InfrastructurePolicyRequirementStatusV1,
  pub required: u32,
  pub received: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicySafeOperationSummaryV1 {
  pub workflow_id: String,
  pub capability: String,
  pub operation: String,
  pub effect: InfrastructurePolicyEffectV1,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub target: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub executable: Option<String>,
  pub execution_mode: InfrastructurePolicyExecutionModeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyDecisionRecordV1 {
  pub contract: String,
  pub contract_version: u32,
  pub decision_id: String,
  pub policy_id: String,
  pub policy_version: String,
  pub policy_digest: String,
  pub descriptor_digest: String,
  pub outcome: InfrastructurePolicyDecisionOutcomeV1,
  pub matched_rule_ids: Vec<String>,
  pub operation: InfrastructurePolicySafeOperationSummaryV1,
  pub reason_codes: Vec<InfrastructurePolicyReasonCodeV1>,
  pub requirements: Vec<InfrastructurePolicyRequirementResultV1>,
  pub explanation: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub emergency_deny: Option<InfrastructurePolicyEmergencyDenyV1>,
  pub evaluated_at: DateTime<Utc>,
  /// Operational telemetry is returned to the caller but excluded from the
  /// canonical decision event so identical inputs serialize identically.
  #[serde(skip)]
  pub evaluation_duration_micros: u128,
  pub evaluation_instructions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfrastructurePolicyEvaluationBudgetV1 {
  pub max_instructions: u64,
  pub max_duration: Duration,
  pub max_evidence: usize,
}

impl Default for InfrastructurePolicyEvaluationBudgetV1 {
  fn default() -> Self {
    Self {
      max_instructions: DEFAULT_MAX_EVALUATION_INSTRUCTIONS,
      max_duration: DEFAULT_MAX_EVALUATION_DURATION,
      max_evidence: INFRASTRUCTURE_POLICY_MAX_EVIDENCE,
    }
  }
}

#[derive(Debug, Clone)]
pub struct InfrastructurePolicyPreparedEvaluatorV1 {
  policy: PreparedInfrastructurePolicyV1,
  rules: Vec<InfrastructurePolicyRuleV1>,
  budget: InfrastructurePolicyEvaluationBudgetV1,
}

impl InfrastructurePolicyPreparedEvaluatorV1 {
  pub fn prepare(policy: PreparedInfrastructurePolicyV1) -> Self {
    Self::prepare_with_budget(policy, InfrastructurePolicyEvaluationBudgetV1::default())
  }

  pub fn prepare_with_budget(
    policy: PreparedInfrastructurePolicyV1,
    budget: InfrastructurePolicyEvaluationBudgetV1,
  ) -> Self {
    let mut rules = policy.bundle.rules.clone();
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    Self {
      policy,
      rules,
      budget,
    }
  }

  pub fn policy_digest(&self) -> &str {
    &self.policy.digest
  }

  pub fn evaluate(
    &self,
    descriptor: &InfrastructurePolicyOperationDescriptorV1,
    evidence: &[InfrastructurePolicyEvidenceReferenceV1],
    evaluated_at: DateTime<Utc>,
  ) -> InfrastructurePolicyDecisionRecordV1 {
    let started = Instant::now();
    let mut meter = EvaluationMeter::new(self.budget, started);
    if descriptor.environment != self.policy.bundle.environment {
      return self.decision(
        descriptor,
        evaluated_at,
        started,
        meter.instructions,
        InfrastructurePolicyDecisionOutcomeV1::Deny,
        Vec::new(),
        vec![InfrastructurePolicyReasonCodeV1::EnvironmentMismatch],
        Vec::new(),
      );
    }
    if evidence.len() > self.budget.max_evidence {
      meter.exhausted = true;
    }
    let mut matching = Vec::new();
    for rule in &self.rules {
      if meter.tick(1).is_err() {
        break;
      }
      if rule_matches(&rule.matcher, descriptor, &mut meter) {
        matching.push(rule);
      }
    }
    if meter.exhausted {
      return self.budget_denied(descriptor, evaluated_at, started, meter.instructions);
    }
    let matched_rule_ids = matching
      .iter()
      .map(|rule| rule.id.clone())
      .collect::<Vec<_>>();
    if matching
      .iter()
      .any(|rule| rule.decision == InfrastructurePolicyDecisionV1::Deny)
    {
      return self.decision(
        descriptor,
        evaluated_at,
        started,
        meter.instructions,
        InfrastructurePolicyDecisionOutcomeV1::Deny,
        matched_rule_ids,
        vec![InfrastructurePolicyReasonCodeV1::ExplicitDeny],
        Vec::new(),
      );
    }
    let allows = matching
      .iter()
      .copied()
      .filter(|rule| rule.decision == InfrastructurePolicyDecisionV1::Allow)
      .collect::<Vec<_>>();
    if allows.is_empty() {
      let allowed = self.default_for(descriptor.effect) == InfrastructurePolicyDefaultV1::Allow;
      return self.decision(
        descriptor,
        evaluated_at,
        started,
        meter.instructions,
        if allowed {
          InfrastructurePolicyDecisionOutcomeV1::Allow
        } else {
          InfrastructurePolicyDecisionOutcomeV1::Deny
        },
        matched_rule_ids,
        vec![if allowed {
          InfrastructurePolicyReasonCodeV1::DefaultAllow
        } else {
          InfrastructurePolicyReasonCodeV1::DefaultDeny
        }],
        Vec::new(),
      );
    }
    let mut requirements = Vec::new();
    for rule in allows {
      if let Some(required) = &rule.requires {
        evaluate_requirements(
          &rule.id,
          required,
          descriptor,
          evidence,
          evaluated_at,
          &mut meter,
          &mut requirements,
        );
      }
    }
    if meter.exhausted {
      return self.budget_denied(descriptor, evaluated_at, started, meter.instructions);
    }
    requirements.sort_by(|left, right| {
      left
        .rule_id
        .cmp(&right.rule_id)
        .then(left.requirement.cmp(&right.requirement))
    });
    let unsatisfied = requirements
      .iter()
      .any(|result| result.status == InfrastructurePolicyRequirementStatusV1::Unsatisfied);
    self.decision(
      descriptor,
      evaluated_at,
      started,
      meter.instructions,
      if unsatisfied {
        InfrastructurePolicyDecisionOutcomeV1::RequirementsUnsatisfied
      } else {
        InfrastructurePolicyDecisionOutcomeV1::Allow
      },
      matched_rule_ids,
      vec![if unsatisfied {
        InfrastructurePolicyReasonCodeV1::RequirementsUnsatisfied
      } else {
        InfrastructurePolicyReasonCodeV1::ExplicitAllow
      }],
      requirements,
    )
  }

  /// Creates the bounded denial record used when a protected operation cannot
  /// be normalized into the frozen descriptor contract. Only engine-owned
  /// identity and the handler's declared effect enter this record; raw input
  /// is deliberately excluded.
  #[allow(clippy::too_many_arguments)]
  pub fn input_unsupported_decision(
    &self,
    run_id: &str,
    workflow_id: &str,
    definition_hash: &str,
    node_id: &str,
    attempt_number: u32,
    invocation_id: &str,
    operation_key: &str,
    capability: &str,
    operation: &str,
    effect: CapabilityEffect,
    execution_mode: InfrastructurePolicyExecutionModeV1,
    evaluated_at: DateTime<Utc>,
  ) -> InfrastructurePolicyDecisionRecordV1 {
    let descriptor_seed = json!({
      "runId": run_id,
      "workflowId": workflow_id,
      "definitionHash": definition_hash,
      "nodeId": node_id,
      "attemptNumber": attempt_number,
      "invocationId": invocation_id,
      "operationKey": operation_key,
      "capability": capability,
      "operation": operation,
      "effect": effect_name(effect),
      "environment": self.policy.bundle.environment,
      "executionMode": execution_mode,
      "normalization": "unsupported",
    });
    let descriptor_digest = canonical_json(&descriptor_seed)
      .map(|encoded| format!("sha256:{:x}", Sha256::digest(encoded)))
      .unwrap_or_else(|_| format!("sha256:{:x}", Sha256::digest(b"unsupported")));
    let decision_seed = json!({
      "policyDigest": self.policy.digest,
      "descriptorDigest": descriptor_digest,
      "evaluatedAt": evaluated_at,
      "reasonCodes": [InfrastructurePolicyReasonCodeV1::InputUnsupported],
      "outcome": InfrastructurePolicyDecisionOutcomeV1::Deny,
    });
    let decision_id = canonical_json(&decision_seed)
      .map(|encoded| format!("decision_{:x}", Sha256::digest(encoded)))
      .unwrap_or_else(|_| "decision_unavailable".to_string());
    InfrastructurePolicyDecisionRecordV1 {
      contract: INFRASTRUCTURE_POLICY_DECISION_CONTRACT.to_string(),
      contract_version: INFRASTRUCTURE_POLICY_RUNTIME_CONTRACT_VERSION,
      decision_id,
      policy_id: self.policy.bundle.id.clone(),
      policy_version: self.policy.bundle.version.clone(),
      policy_digest: self.policy.digest.clone(),
      descriptor_digest,
      outcome: InfrastructurePolicyDecisionOutcomeV1::Deny,
      matched_rule_ids: Vec::new(),
      operation: InfrastructurePolicySafeOperationSummaryV1 {
        workflow_id: workflow_id.to_string(),
        capability: capability.to_string(),
        operation: operation.to_string(),
        effect: effect_name(effect),
        target: None,
        executable: None,
        execution_mode,
      },
      reason_codes: vec![InfrastructurePolicyReasonCodeV1::InputUnsupported],
      requirements: Vec::new(),
      explanation:
        "The protected operation could not be normalized into a trusted policy descriptor."
          .to_string(),
      emergency_deny: None,
      evaluated_at,
      evaluation_duration_micros: 0,
      evaluation_instructions: 0,
    }
  }

  fn default_for(&self, effect: InfrastructurePolicyEffectV1) -> InfrastructurePolicyDefaultV1 {
    match effect {
      InfrastructurePolicyEffectV1::Read => self.policy.bundle.defaults.read,
      InfrastructurePolicyEffectV1::IdempotentWrite => self.policy.bundle.defaults.idempotent_write,
      InfrastructurePolicyEffectV1::UnsafeWrite => self.policy.bundle.defaults.unsafe_write,
    }
  }

  fn budget_denied(
    &self,
    descriptor: &InfrastructurePolicyOperationDescriptorV1,
    evaluated_at: DateTime<Utc>,
    started: Instant,
    instructions: u64,
  ) -> InfrastructurePolicyDecisionRecordV1 {
    self.decision(
      descriptor,
      evaluated_at,
      started,
      instructions,
      InfrastructurePolicyDecisionOutcomeV1::Deny,
      Vec::new(),
      vec![InfrastructurePolicyReasonCodeV1::EvaluationBudgetExhausted],
      Vec::new(),
    )
  }

  #[allow(clippy::too_many_arguments)]
  fn decision(
    &self,
    descriptor: &InfrastructurePolicyOperationDescriptorV1,
    evaluated_at: DateTime<Utc>,
    started: Instant,
    instructions: u64,
    outcome: InfrastructurePolicyDecisionOutcomeV1,
    mut matched_rule_ids: Vec<String>,
    mut reason_codes: Vec<InfrastructurePolicyReasonCodeV1>,
    requirements: Vec<InfrastructurePolicyRequirementResultV1>,
  ) -> InfrastructurePolicyDecisionRecordV1 {
    matched_rule_ids.sort();
    matched_rule_ids.dedup();
    reason_codes.sort();
    reason_codes.dedup();
    let decision_seed = json!({
      "policyDigest": self.policy.digest,
      "descriptorDigest": descriptor.descriptor_digest,
      "evaluatedAt": evaluated_at,
      "matchedRuleIds": matched_rule_ids,
      "reasonCodes": reason_codes,
      "requirements": requirements,
      "outcome": outcome,
    });
    let decision_id = canonical_json(&decision_seed)
      .map(|encoded| format!("decision_{:x}", Sha256::digest(encoded)))
      .unwrap_or_else(|_| "decision_unavailable".to_string());
    let explanation = bounded_explanation(outcome, &matched_rule_ids, &requirements);
    InfrastructurePolicyDecisionRecordV1 {
      contract: INFRASTRUCTURE_POLICY_DECISION_CONTRACT.to_string(),
      contract_version: INFRASTRUCTURE_POLICY_RUNTIME_CONTRACT_VERSION,
      decision_id,
      policy_id: self.policy.bundle.id.clone(),
      policy_version: self.policy.bundle.version.clone(),
      policy_digest: self.policy.digest.clone(),
      descriptor_digest: descriptor.descriptor_digest.clone(),
      outcome,
      matched_rule_ids,
      operation: InfrastructurePolicySafeOperationSummaryV1 {
        workflow_id: descriptor.workflow_id.clone(),
        capability: descriptor.capability.clone(),
        operation: descriptor.operation.clone(),
        effect: descriptor.effect,
        target: descriptor.target.clone(),
        executable: descriptor.executable.clone(),
        execution_mode: descriptor.execution_mode,
      },
      reason_codes,
      requirements,
      explanation,
      emergency_deny: None,
      evaluated_at,
      evaluation_duration_micros: started.elapsed().as_micros(),
      evaluation_instructions: instructions,
    }
  }
}

/// Applies a separately prepared deny-only overlay without changing the
/// admitted base policy identity.
pub fn apply_emergency_deny_overlay(
  mut base: InfrastructurePolicyDecisionRecordV1,
  overlay: InfrastructurePolicyDecisionRecordV1,
) -> InfrastructurePolicyDecisionRecordV1 {
  if overlay.outcome != InfrastructurePolicyDecisionOutcomeV1::Deny {
    return base;
  }
  base.outcome = InfrastructurePolicyDecisionOutcomeV1::Deny;
  base
    .reason_codes
    .push(InfrastructurePolicyReasonCodeV1::EmergencyDeny);
  base.reason_codes.sort();
  base.reason_codes.dedup();
  base.explanation =
    "An operator emergency-deny overlay blocked this protected effect.".to_string();
  base.emergency_deny = Some(InfrastructurePolicyEmergencyDenyV1 {
    policy_id: overlay.policy_id,
    policy_version: overlay.policy_version,
    policy_digest: overlay.policy_digest,
    matched_rule_ids: overlay.matched_rule_ids,
  });
  let seed = json!({
    "basePolicyDigest": base.policy_digest,
    "descriptorDigest": base.descriptor_digest,
    "evaluatedAt": base.evaluated_at,
    "emergencyDeny": base.emergency_deny,
    "outcome": base.outcome,
  });
  base.decision_id = canonical_json(&seed)
    .map(|encoded| format!("decision_{:x}", Sha256::digest(encoded)))
    .unwrap_or_else(|_| "decision_unavailable".to_string());
  base
}

struct EvaluationMeter {
  budget: InfrastructurePolicyEvaluationBudgetV1,
  started: Instant,
  instructions: u64,
  exhausted: bool,
}

impl EvaluationMeter {
  fn new(budget: InfrastructurePolicyEvaluationBudgetV1, started: Instant) -> Self {
    Self {
      budget,
      started,
      instructions: 0,
      exhausted: false,
    }
  }

  fn tick(&mut self, amount: u64) -> Result<(), ()> {
    self.instructions = self.instructions.saturating_add(amount);
    if self.instructions > self.budget.max_instructions
      || self.started.elapsed() > self.budget.max_duration
    {
      self.exhausted = true;
      Err(())
    } else {
      Ok(())
    }
  }
}

fn rule_matches(
  matcher: &InfrastructurePolicyRuleMatchV1,
  descriptor: &InfrastructurePolicyOperationDescriptorV1,
  meter: &mut EvaluationMeter,
) -> bool {
  let checks = [
    matcher
      .capabilities
      .as_ref()
      .is_none_or(|values| values.contains(&descriptor.capability)),
    matcher
      .operations
      .as_ref()
      .is_none_or(|values| values.contains(&descriptor.operation)),
    matcher
      .effects
      .as_ref()
      .is_none_or(|values| values.contains(&descriptor.effect)),
    matcher
      .workflows
      .as_ref()
      .is_none_or(|values| values.contains(&descriptor.workflow_id)),
    matcher.targets.as_ref().is_none_or(|values| {
      descriptor
        .target
        .as_ref()
        .is_some_and(|target| values.contains(target))
    }),
    matcher.executables.as_ref().is_none_or(|values| {
      descriptor
        .executable
        .as_ref()
        .is_some_and(|executable| values.contains(executable))
    }),
    matcher
      .argument_prefix
      .as_ref()
      .is_none_or(|prefix| descriptor.arguments_for_evaluation().starts_with(prefix)),
    matcher
      .execution_modes
      .as_ref()
      .is_none_or(|values| values.contains(&descriptor.execution_mode)),
  ];
  let _ = meter.tick(checks.len() as u64);
  !meter.exhausted && checks.into_iter().all(|matched| matched)
}

#[allow(clippy::too_many_arguments)]
fn evaluate_requirements(
  rule_id: &str,
  requirements: &InfrastructurePolicyRequirementsV1,
  descriptor: &InfrastructurePolicyOperationDescriptorV1,
  evidence: &[InfrastructurePolicyEvidenceReferenceV1],
  now: DateTime<Utc>,
  meter: &mut EvaluationMeter,
  results: &mut Vec<InfrastructurePolicyRequirementResultV1>,
) {
  if let Some(approval) = &requirements.approval {
    let mut principals = BTreeSet::new();
    let mut roles = BTreeSet::new();
    for item in evidence {
      if meter.tick(1).is_err() {
        return;
      }
      if evidence_in_scope(item, descriptor, now)
        && item.kind == InfrastructurePolicyEvidenceKindV1::Approval
        && now.signed_duration_since(item.issued_at).num_milliseconds() >= 0
        && now.signed_duration_since(item.issued_at).num_milliseconds() as u64
          <= approval.max_age_ms
        && (!approval.bind_review_digest
          || descriptor.review_digest.is_some() && item.review_digest == descriptor.review_digest)
      {
        if let Some(principal) = &item.principal_id {
          principals.insert(principal.clone());
        }
        roles.extend(item.roles.iter().cloned());
      }
    }
    let roles_satisfied = approval.roles.iter().all(|role| roles.contains(role));
    let received = principals.len() as u32;
    results.push(requirement_result(
      rule_id,
      "approval",
      approval.minimum,
      received,
      received >= approval.minimum && roles_satisfied,
    ));
  }
  if let Some(artifact) = &requirements.artifacts {
    let received = evidence
      .iter()
      .filter(|item| {
        let _ = meter.tick(1);
        !meter.exhausted
          && evidence_in_scope(item, descriptor, now)
          && matches!(
            item.kind,
            InfrastructurePolicyEvidenceKindV1::Artifact
              | InfrastructurePolicyEvidenceKindV1::Backup
              | InfrastructurePolicyEvidenceKindV1::Attestation
          )
          && (!artifact.verified_integrity || item.verified_integrity)
          && (!artifact.bind_review_digest
            || descriptor.review_digest.is_some() && item.review_digest == descriptor.review_digest)
      })
      .count() as u32;
    results.push(requirement_result(
      rule_id,
      "artifacts",
      artifact.minimum,
      received,
      received >= artifact.minimum,
    ));
  }
  if let Some(lock) = &requirements.lock {
    let minimum_expiry = now + chrono::Duration::milliseconds(lock.minimum_remaining_ms as i64);
    let received = u32::from(evidence.iter().any(|item| {
      let _ = meter.tick(1);
      !meter.exhausted
        && evidence_in_scope(item, descriptor, now)
        && item.kind == InfrastructurePolicyEvidenceKindV1::Lock
        && item.lock_key.as_deref() == Some(lock.key.as_str())
        && item
          .expires_at
          .is_some_and(|expires| expires >= minimum_expiry)
    }));
    results.push(requirement_result(
      rule_id,
      "lock",
      1,
      received,
      received == 1,
    ));
  }
  if let Some(window) = &requirements.valid_between {
    let not_before = DateTime::parse_from_rfc3339(&window.not_before)
      .ok()
      .map(|value| value.with_timezone(&Utc));
    let not_after = DateTime::parse_from_rfc3339(&window.not_after)
      .ok()
      .map(|value| value.with_timezone(&Utc));
    let valid = not_before
      .zip(not_after)
      .is_some_and(|(start, end)| now >= start && now <= end);
    let _ = meter.tick(2);
    results.push(requirement_result(
      rule_id,
      "valid_between",
      1,
      u32::from(valid),
      valid,
    ));
  }
}

fn evidence_in_scope(
  evidence: &InfrastructurePolicyEvidenceReferenceV1,
  descriptor: &InfrastructurePolicyOperationDescriptorV1,
  now: DateTime<Utc>,
) -> bool {
  evidence.contract == INFRASTRUCTURE_POLICY_EVIDENCE_CONTRACT
    && evidence.contract_version == INFRASTRUCTURE_POLICY_RUNTIME_CONTRACT_VERSION
    && evidence.run_id == descriptor.run_id
    && evidence
      .node_id
      .as_ref()
      .is_none_or(|value| value == &descriptor.node_id)
    && evidence
      .operation_key
      .as_ref()
      .is_none_or(|value| value == &descriptor.operation_key)
    && evidence.issued_at <= now
    && evidence.expires_at.is_none_or(|expires| expires > now)
}

fn requirement_result(
  rule_id: &str,
  requirement: &str,
  required: u32,
  received: u32,
  satisfied: bool,
) -> InfrastructurePolicyRequirementResultV1 {
  InfrastructurePolicyRequirementResultV1 {
    rule_id: rule_id.to_string(),
    requirement: requirement.to_string(),
    status: if satisfied {
      InfrastructurePolicyRequirementStatusV1::Satisfied
    } else {
      InfrastructurePolicyRequirementStatusV1::Unsatisfied
    },
    required,
    received,
  }
}

fn bounded_explanation(
  outcome: InfrastructurePolicyDecisionOutcomeV1,
  matched_rule_ids: &[String],
  requirements: &[InfrastructurePolicyRequirementResultV1],
) -> String {
  let rule_text = if matched_rule_ids.is_empty() {
    "the bundle default".to_string()
  } else {
    format!("rule(s) {}", matched_rule_ids.join(", "))
  };
  let missing = requirements
    .iter()
    .filter(|item| item.status == InfrastructurePolicyRequirementStatusV1::Unsatisfied)
    .map(|item| item.requirement.as_str())
    .collect::<Vec<_>>();
  let mut explanation = match outcome {
    InfrastructurePolicyDecisionOutcomeV1::Allow => {
      format!("The operation is allowed by {rule_text}.")
    }
    InfrastructurePolicyDecisionOutcomeV1::Deny => {
      format!("The operation is denied by {rule_text}.")
    }
    InfrastructurePolicyDecisionOutcomeV1::RequirementsUnsatisfied => format!(
      "The operation matched {rule_text}, but trusted evidence is missing or invalid for: {}.",
      missing.join(", ")
    ),
  };
  if explanation.len() > INFRASTRUCTURE_POLICY_MAX_EXPLANATION_BYTES {
    explanation.truncate(INFRASTRUCTURE_POLICY_MAX_EXPLANATION_BYTES);
    while !explanation.is_char_boundary(explanation.len()) {
      explanation.pop();
    }
  }
  explanation
}

#[derive(Debug)]
pub struct InfrastructurePolicyEvaluatorCacheV1 {
  maximum_entries: usize,
  entries: Mutex<HashMap<String, Arc<InfrastructurePolicyPreparedEvaluatorV1>>>,
  order: Mutex<VecDeque<String>>,
}

impl Default for InfrastructurePolicyEvaluatorCacheV1 {
  fn default() -> Self {
    Self::new(64)
  }
}

impl InfrastructurePolicyEvaluatorCacheV1 {
  pub fn new(maximum_entries: usize) -> Self {
    Self {
      maximum_entries: maximum_entries.max(1),
      entries: Mutex::new(HashMap::new()),
      order: Mutex::new(VecDeque::new()),
    }
  }

  pub fn prepare(
    &self,
    policy: PreparedInfrastructurePolicyV1,
  ) -> Arc<InfrastructurePolicyPreparedEvaluatorV1> {
    if let Some(existing) = self
      .entries
      .lock()
      .expect("infrastructure policy cache")
      .get(&policy.digest)
      .cloned()
    {
      return existing;
    }
    let digest = policy.digest.clone();
    let evaluator = Arc::new(InfrastructurePolicyPreparedEvaluatorV1::prepare(policy));
    let mut entries = self.entries.lock().expect("infrastructure policy cache");
    let mut order = self
      .order
      .lock()
      .expect("infrastructure policy cache order");
    if entries.len() >= self.maximum_entries {
      if let Some(oldest) = order.pop_front() {
        entries.remove(&oldest);
      }
    }
    entries.insert(digest.clone(), Arc::clone(&evaluator));
    order.push_back(digest);
    evaluator
  }

  pub fn len(&self) -> usize {
    self
      .entries
      .lock()
      .expect("infrastructure policy cache")
      .len()
  }

  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    capability::CapabilityIdentityMode,
    infrastructure_policy::{
      InfrastructurePolicyApprovalRequirementV1, InfrastructurePolicyArtifactRequirementV1,
      InfrastructurePolicyDefaultsV1, InfrastructurePolicyLockRequirementV1,
    },
    prepare_infrastructure_policy, CapabilityCallIdentity, CapabilityCallLimits,
    CapabilityRegistry, InfrastructurePolicyBundleV1, InfrastructurePolicyRuleMatchV1,
    InfrastructurePolicySignatureStatusV1, InfrastructurePolicyTrustV1, TestCapabilityHandler,
  };

  fn request(capability: &str, operation: &str, input: Value) -> CapabilityCallRequest {
    CapabilityCallRequest {
      contract: "woml.capability-call".to_string(),
      contract_version: 1,
      message_type: "request".to_string(),
      invocation_id: "invocation-policy-a".to_string(),
      call_id: "call-policy-a".to_string(),
      run_id: "run-policy-a".to_string(),
      node_id: "applyPlan".to_string(),
      attempt_number: 1,
      capability: capability.to_string(),
      operation: operation.to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      identity: CapabilityCallIdentity {
        mode: CapabilityIdentityMode::Named,
        step_idempotency_key:
          "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        operation_name: "apply-plan".to_string(),
        operation_key: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
          .to_string(),
        provider_idempotency_key: None,
      },
      limits: CapabilityCallLimits::default(),
      input,
    }
  }

  fn descriptor() -> InfrastructurePolicyOperationDescriptorV1 {
    descriptor_for_target("production-runner")
  }

  fn descriptor_for_target(target: &str) -> InfrastructurePolicyOperationDescriptorV1 {
    let call = request(
      "process",
      "run",
      json!({
        "contract": "woml.process",
        "contractVersion": 1,
        "kind": "request",
        "executable": "terraform",
        "args": ["apply", "secret-plan-token"],
        "cwd": ".",
        "env": { "TOKEN": "never-persist-this" },
        "timeoutMs": 30000,
        "acceptExitCodes": [0],
        "output": { "stdout": "capture", "stderr": "capture", "maxBytes": 4096 }
      }),
    );
    let handler = CapabilityDescriptor {
      capability: "process".to_string(),
      operation: "run".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    };
    let metadata = adapt_infrastructure_policy_metadata(&handler, &call).unwrap();
    InfrastructurePolicyOperationDescriptorV1::build(
      &call,
      &handler,
      CapabilityEffect::UnsafeWrite,
      InfrastructurePolicyDescriptorContextV1 {
        workflow_id: "deploy-production".to_string(),
        definition_hash: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
          .to_string(),
        environment: "production".to_string(),
        effective_target: Some(target.to_string()),
        execution_mode: InfrastructurePolicyExecutionModeV1::Normal,
        review_digest: Some(
          "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string(),
        ),
      },
      metadata,
    )
    .unwrap()
  }

  fn bundle(rules: Vec<InfrastructurePolicyRuleV1>) -> PreparedInfrastructurePolicyV1 {
    prepare_infrastructure_policy(
      InfrastructurePolicyBundleV1 {
        contract: "woml.infrastructure-policy".to_string(),
        contract_version: 1,
        id: "production".to_string(),
        version: "2026-10-22.1".to_string(),
        environment: "production".to_string(),
        defaults: InfrastructurePolicyDefaultsV1 {
          read: InfrastructurePolicyDefaultV1::Allow,
          idempotent_write: InfrastructurePolicyDefaultV1::Deny,
          unsafe_write: InfrastructurePolicyDefaultV1::Deny,
        },
        rules,
        signature: None,
      },
      &InfrastructurePolicyTrustV1::default(),
    )
    .unwrap()
  }

  fn allow_rule(id: &str) -> InfrastructurePolicyRuleV1 {
    InfrastructurePolicyRuleV1 {
      id: id.to_string(),
      description: None,
      matcher: InfrastructurePolicyRuleMatchV1 {
        capabilities: Some(vec!["process".to_string()]),
        operations: Some(vec!["run".to_string()]),
        executables: Some(vec!["terraform".to_string()]),
        argument_prefix: Some(vec!["apply".to_string()]),
        ..InfrastructurePolicyRuleMatchV1::default()
      },
      decision: InfrastructurePolicyDecisionV1::Allow,
      requires: None,
    }
  }

  fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")
      .unwrap()
      .with_timezone(&Utc)
  }

  #[test]
  fn descriptors_are_stable_and_serialize_no_raw_arguments_or_secrets() {
    let first = descriptor();
    let second = descriptor();
    assert_eq!(first.descriptor_digest, second.descriptor_digest);
    let encoded = serde_json::to_string(&first).unwrap();
    assert!(!encoded.contains("secret-plan-token"));
    assert!(!encoded.contains("never-persist-this"));
    assert!(encoded.contains("argumentDigest"));

    let changed = descriptor_for_target("staging-runner");
    assert_ne!(first.descriptor_digest, changed.descriptor_digest);

    let original_request = request(
      "process",
      "run",
      json!({
        "contract": "woml.process", "contractVersion": 1, "kind": "request",
        "executable": "terraform", "args": ["apply"], "cwd": "."
      }),
    );
    let mut retry = original_request.clone();
    retry.attempt_number = 2;
    retry.invocation_id = "invocation-policy-retry".to_string();
    let handler = CapabilityDescriptor {
      capability: "process".to_string(),
      operation: "run".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    };
    assert_eq!(
      adapt_infrastructure_policy_metadata(&handler, &original_request).unwrap(),
      adapt_infrastructure_policy_metadata(&handler, &retry).unwrap()
    );
  }

  #[test]
  fn coverage_is_closed_and_custom_capabilities_fail_closed() {
    for (capability, operation) in [
      ("process", "run"),
      ("http", "request"),
      ("db", "execute"),
      ("storage", "delete"),
      ("cache", "increment"),
      ("events", "emit"),
      ("workflows", "call"),
      ("artifacts", "put"),
      ("locks", "acquire"),
      ("telegram", "send"),
      ("discord", "send"),
      ("whatsapp", "send"),
    ] {
      assert_eq!(
        infrastructure_policy_descriptor_coverage(capability, operation),
        InfrastructurePolicyCoverageV1::Supported
      );
    }
    let handler = CapabilityDescriptor {
      capability: "custom-provider".to_string(),
      operation: "send".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    };
    let failure = adapt_infrastructure_policy_metadata(
      &handler,
      &request("custom-provider", "send", json!({ "token": "secret" })),
    )
    .unwrap_err();
    assert_eq!(failure.code, "WOML_POLICY_INPUT_UNSUPPORTED");

    let fixture: Value = serde_json::from_str(include_str!(
      "../../../woml/tests/fixtures/infrastructure-policies/operation-descriptors.v1.json"
    ))
    .unwrap();
    for capability in fixture["protectedCapabilities"].as_array().unwrap() {
      let name = capability["capability"].as_str().unwrap();
      for operation in capability["operations"].as_array().unwrap() {
        assert_eq!(
          infrastructure_policy_descriptor_coverage(name, operation.as_str().unwrap()),
          InfrastructurePolicyCoverageV1::Supported,
          "golden coverage is missing {name}.{}",
          operation.as_str().unwrap()
        );
      }
    }
  }

  #[test]
  fn registry_validates_first_and_refuses_a_missing_policy_adapter() {
    let registry = CapabilityRegistry::default();
    registry.register(Arc::new(TestCapabilityHandler)).unwrap();
    let failure = registry
      .infrastructure_policy_descriptor(
        &request("test", "control", json!({ "secret": "not-visible" })),
        InfrastructurePolicyDescriptorContextV1 {
          workflow_id: "test-workflow".to_string(),
          definition_hash:
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
          environment: "production".to_string(),
          effective_target: None,
          execution_mode: InfrastructurePolicyExecutionModeV1::Normal,
          review_digest: None,
        },
      )
      .unwrap_err();
    assert_eq!(failure.code, "WOML_POLICY_INPUT_UNSUPPORTED");
    assert!(!failure.message.contains("not-visible"));
  }

  #[test]
  fn deny_wins_and_rule_order_is_irrelevant() {
    let deny = InfrastructurePolicyRuleV1 {
      id: "deny-production".to_string(),
      description: Some("safe label".to_string()),
      matcher: InfrastructurePolicyRuleMatchV1 {
        targets: Some(vec!["production-runner".to_string()]),
        ..InfrastructurePolicyRuleMatchV1::default()
      },
      decision: InfrastructurePolicyDecisionV1::Deny,
      requires: None,
    };
    let timestamp = now();
    let left = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(vec![
      allow_rule("allow-terraform"),
      deny.clone(),
    ]))
    .evaluate(&descriptor(), &[], timestamp);
    let right = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(vec![
      deny,
      allow_rule("allow-terraform"),
    ]))
    .evaluate(&descriptor(), &[], timestamp);
    assert_eq!(left.outcome, InfrastructurePolicyDecisionOutcomeV1::Deny);
    assert_eq!(left.matched_rule_ids, right.matched_rule_ids);
    assert_eq!(left.reason_codes, right.reason_codes);
    assert_eq!(left.requirements, right.requirements);
  }

  #[test]
  fn requirements_accept_only_scoped_fresh_engine_evidence() {
    let mut rule = allow_rule("reviewed-terraform");
    rule.requires = Some(InfrastructurePolicyRequirementsV1 {
      approval: Some(InfrastructurePolicyApprovalRequirementV1 {
        minimum: 2,
        roles: vec!["production-owner".to_string()],
        bind_review_digest: true,
        max_age_ms: 3_600_000,
      }),
      artifacts: Some(InfrastructurePolicyArtifactRequirementV1 {
        minimum: 1,
        verified_integrity: true,
        bind_review_digest: true,
      }),
      lock: Some(InfrastructurePolicyLockRequirementV1 {
        key: "production/deploy".to_string(),
        minimum_remaining_ms: 30_000,
      }),
      valid_between: None,
    });
    let evaluator = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(vec![rule]));
    let operation = descriptor();
    let missing = evaluator.evaluate(&operation, &[], now());
    assert_eq!(
      missing.outcome,
      InfrastructurePolicyDecisionOutcomeV1::RequirementsUnsatisfied
    );

    let authority = InfrastructurePolicyEvidenceAuthorityV1::engine();
    let expires = now() + chrono::Duration::minutes(10);
    let evidence = vec![
      authority
        .issue(
          InfrastructurePolicyEvidenceKindV1::Approval,
          "approval-a".to_string(),
          "rich-approval-authority".to_string(),
          now() - chrono::Duration::minutes(2),
          Some(expires),
          operation.run_id.clone(),
          Some(operation.node_id.clone()),
          Some(operation.operation_key.clone()),
          operation.review_digest.clone(),
          Some("alice".to_string()),
          BTreeSet::from(["production-owner".to_string()]),
          false,
          None,
        )
        .unwrap(),
      authority
        .issue(
          InfrastructurePolicyEvidenceKindV1::Approval,
          "approval-b".to_string(),
          "rich-approval-authority".to_string(),
          now() - chrono::Duration::minutes(1),
          Some(expires),
          operation.run_id.clone(),
          Some(operation.node_id.clone()),
          Some(operation.operation_key.clone()),
          operation.review_digest.clone(),
          Some("bob".to_string()),
          BTreeSet::from(["production-owner".to_string()]),
          false,
          None,
        )
        .unwrap(),
      authority
        .issue(
          InfrastructurePolicyEvidenceKindV1::Artifact,
          "artifact-plan".to_string(),
          "artifact-authority".to_string(),
          now() - chrono::Duration::minutes(3),
          Some(expires),
          operation.run_id.clone(),
          None,
          Some(operation.operation_key.clone()),
          operation.review_digest.clone(),
          None,
          BTreeSet::new(),
          true,
          None,
        )
        .unwrap(),
      authority
        .issue(
          InfrastructurePolicyEvidenceKindV1::Lock,
          "lease-a".to_string(),
          "lock-authority".to_string(),
          now() - chrono::Duration::seconds(5),
          Some(expires),
          operation.run_id.clone(),
          Some(operation.node_id.clone()),
          Some(operation.operation_key.clone()),
          None,
          None,
          BTreeSet::new(),
          false,
          Some("production/deploy".to_string()),
        )
        .unwrap(),
    ];
    let allowed = evaluator.evaluate(&operation, &evidence, now());
    assert_eq!(
      allowed.outcome,
      InfrastructurePolicyDecisionOutcomeV1::Allow
    );
    assert!(allowed
      .requirements
      .iter()
      .all(|item| item.status == InfrastructurePolicyRequirementStatusV1::Satisfied));
  }

  #[test]
  fn decisions_are_stable_bounded_and_never_echo_sensitive_arguments() {
    let evaluator =
      InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(vec![allow_rule("allow-terraform")]));
    let first = evaluator.evaluate(&descriptor(), &[], now());
    let second = evaluator.evaluate(&descriptor(), &[], now());
    let first_json = serde_json::to_vec(&first).unwrap();
    let second_json = serde_json::to_vec(&second).unwrap();
    assert_eq!(first_json, second_json);
    assert!(!String::from_utf8(first_json)
      .unwrap()
      .contains("secret-plan-token"));
    assert!(first.explanation.len() <= INFRASTRUCTURE_POLICY_MAX_EXPLANATION_BYTES);
  }

  #[test]
  fn evaluation_budget_exhaustion_denies_and_cache_is_digest_keyed() {
    let prepared = bundle(vec![allow_rule("allow-terraform")]);
    assert_eq!(
      prepared.signature_status,
      InfrastructurePolicySignatureStatusV1::Unsigned
    );
    let evaluator = InfrastructurePolicyPreparedEvaluatorV1::prepare_with_budget(
      prepared.clone(),
      InfrastructurePolicyEvaluationBudgetV1 {
        max_instructions: 0,
        ..InfrastructurePolicyEvaluationBudgetV1::default()
      },
    );
    let decision = evaluator.evaluate(&descriptor(), &[], now());
    assert_eq!(
      decision.outcome,
      InfrastructurePolicyDecisionOutcomeV1::Deny
    );
    assert_eq!(
      decision.reason_codes,
      vec![InfrastructurePolicyReasonCodeV1::EvaluationBudgetExhausted]
    );

    let cache = InfrastructurePolicyEvaluatorCacheV1::new(2);
    let first = cache.prepare(prepared.clone());
    let second = cache.prepare(prepared);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(cache.len(), 1);
  }

  #[test]
  fn maximum_rule_bundle_stays_within_the_frozen_evaluation_budget() {
    for rule_count in [1, 100, 1_000, 1_024] {
      let rules = (0..rule_count)
        .map(|index| InfrastructurePolicyRuleV1 {
          id: format!("allow-{index:04}"),
          description: None,
          matcher: InfrastructurePolicyRuleMatchV1 {
            workflows: Some(vec!["other-workflow".to_string()]),
            ..InfrastructurePolicyRuleMatchV1::default()
          },
          decision: InfrastructurePolicyDecisionV1::Allow,
          requires: None,
        })
        .collect();
      let evaluator = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(rules));
      let decision = evaluator.evaluate(&descriptor(), &[], now());
      assert_eq!(
        decision.outcome,
        InfrastructurePolicyDecisionOutcomeV1::Deny
      );
      assert!(decision.evaluation_instructions <= DEFAULT_MAX_EVALUATION_INSTRUCTIONS);
    }
  }

  #[test]
  fn generated_rule_orders_and_concurrent_evaluations_do_not_diverge() {
    let mut rules = (0..64)
      .map(|index| InfrastructurePolicyRuleV1 {
        id: format!("allow-{index:02}"),
        description: None,
        matcher: InfrastructurePolicyRuleMatchV1 {
          workflows: Some(vec!["other-workflow".to_string()]),
          ..InfrastructurePolicyRuleMatchV1::default()
        },
        decision: InfrastructurePolicyDecisionV1::Allow,
        requires: None,
      })
      .collect::<Vec<_>>();
    rules.push(InfrastructurePolicyRuleV1 {
      id: "deny-production".to_string(),
      description: None,
      matcher: InfrastructurePolicyRuleMatchV1 {
        workflows: Some(vec!["deploy-production".to_string()]),
        ..InfrastructurePolicyRuleMatchV1::default()
      },
      decision: InfrastructurePolicyDecisionV1::Deny,
      requires: None,
    });

    let expected = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(rules.clone()))
      .evaluate(&descriptor(), &[], now());
    for offset in 0..rules.len() {
      rules.rotate_left(1);
      let actual = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(rules.clone()))
        .evaluate(&descriptor(), &[], now());
      assert_eq!(actual.outcome, expected.outcome, "rotation {offset}");
      assert_eq!(actual.matched_rule_ids, expected.matched_rule_ids);
      assert_eq!(actual.reason_codes, expected.reason_codes);
    }

    let evaluator = Arc::new(InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(
      rules,
    )));
    let handles = (0..8)
      .map(|_| {
        let evaluator = Arc::clone(&evaluator);
        std::thread::spawn(move || {
          for _ in 0..250 {
            let decision = evaluator.evaluate(&descriptor(), &[], now());
            assert_eq!(
              decision.outcome,
              InfrastructurePolicyDecisionOutcomeV1::Deny
            );
            assert!(decision
              .reason_codes
              .contains(&InfrastructurePolicyReasonCodeV1::ExplicitDeny));
          }
        })
      })
      .collect::<Vec<_>>();
    for handle in handles {
      handle.join().expect("policy evaluation thread");
    }
  }

  #[test]
  fn publication_budgets_cover_preparation_cache_and_per_effect_evaluation() {
    let small = bundle(vec![allow_rule("allow-terraform")]);
    let cache = InfrastructurePolicyEvaluatorCacheV1::new(16);
    let first = cache.prepare(small.clone());
    for _ in 0..1_000 {
      assert!(Arc::ptr_eq(&first, &cache.prepare(small.clone())));
    }
    assert_eq!(cache.len(), 1);

    let maximum_rules = (0..crate::INFRASTRUCTURE_POLICY_MAX_RULES)
      .map(|index| InfrastructurePolicyRuleV1 {
        id: format!("max-{index:04}"),
        description: None,
        matcher: InfrastructurePolicyRuleMatchV1 {
          workflows: Some(vec!["other-workflow".to_string()]),
          ..InfrastructurePolicyRuleMatchV1::default()
        },
        decision: InfrastructurePolicyDecisionV1::Allow,
        requires: None,
      })
      .collect::<Vec<_>>();
    let maximum = InfrastructurePolicyPreparedEvaluatorV1::prepare(bundle(maximum_rules));

    for evaluator in [&*first, &maximum] {
      for _ in 0..25 {
        let decision = evaluator.evaluate(&descriptor(), &[], now());
        assert!(decision.evaluation_instructions <= DEFAULT_MAX_EVALUATION_INSTRUCTIONS);
        assert!(decision.evaluation_duration_micros <= 100_000);
        assert!(serde_json::to_vec(&decision).unwrap().len() <= 65_536);
      }
    }
  }
}
