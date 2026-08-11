use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

use serde_json::Value;

use crate::{
  AttemptFailure, AttemptFailureKind, CapabilityCallRequest, CapabilityCallResult,
  CapabilityFailure, FailureSizeDetails, NativeFetchObservation, WorkflowContext,
};

pub const SCRIPT_HOST_PROTOCOL: &str = "woml.script-host";
pub const SCRIPT_HOST_PROTOCOL_VERSION: u32 = 7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadyMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub host_instance_id: String,
}

impl ReadyMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "ready"
      || self.host_instance_id.is_empty()
      || self.host_instance_id.chars().count() > 256
    {
      return Err("The child did not send a valid script-host v7 ready message.".to_string());
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub reason: &'static str,
}

impl<'a> CancelMessage<'a> {
  pub fn parallel_fail_fast(invocation_id: &'a str) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "cancel",
      invocation_id,
      reason: "parallel_fail_fast",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub run_id: &'a str,
  pub node_id: &'a str,
  pub attempt: ScriptAttempt<'a>,
  pub mode: ScriptExecutionMode,
  pub handler: &'static str,
  pub timeout_ms: u64,
  pub source: &'a str,
  pub context: &'a WorkflowContext,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub lifecycle: Option<&'a LifecycleBindingV1>,
  pub bindings: ScriptBindings<'a>,
  pub modules: &'a [RuntimeModuleBinding],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptExecutionMode {
  Step,
  Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleBindingV1 {
  pub event: crate::model::LifecycleEventName,
  pub workflow: LifecycleWorkflowBindingV1,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub step: Option<LifecycleStepBindingV1>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failure: Option<LifecycleFailureBindingV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleWorkflowBindingV1 {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub outcome: Option<crate::event::BusinessOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleStepBindingV1 {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub outcome: Option<crate::event::BusinessOutcome>,
  pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleFailureBindingV1 {
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeModuleBinding {
  pub name: String,
  pub bundle_digest: String,
  pub exports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterModuleMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub bundle_digest: &'a str,
  pub bundle: &'a str,
  pub source_map_digest: &'a str,
  pub source_map: &'a str,
}

impl<'a> RegisterModuleMessage<'a> {
  pub fn new(
    bundle_digest: &'a str,
    bundle: &'a str,
    source_map_digest: &'a str,
    source_map: &'a str,
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "register_module",
      bundle_digest,
      bundle,
      source_map_digest,
      source_map,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleRegisteredMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub bundle_digest: String,
  pub source_map_digest: String,
  pub accepted: bool,
  #[serde(default)]
  pub code: Option<String>,
  #[serde(default)]
  pub message: Option<String>,
}

impl ModuleRegisteredMessage {
  pub fn validate(
    &self,
    expected_digest: &str,
    expected_source_map_digest: &str,
  ) -> Result<(), String> {
    let envelope = self.protocol == SCRIPT_HOST_PROTOCOL
      && self.protocol_version == SCRIPT_HOST_PROTOCOL_VERSION
      && self.message_type == "module_registered"
      && self.bundle_digest == expected_digest
      && self.source_map_digest == expected_source_map_digest;
    let outcome = if self.accepted {
      self.code.is_none() && self.message.is_none()
    } else {
      matches!(
        self.code.as_deref(),
        Some("WOML_MODULE_DIGEST_MISMATCH" | "WOML_MODULE_CACHE_LIMIT_EXCEEDED")
      ) && self
        .message
        .as_ref()
        .is_some_and(|message| !message.is_empty())
    };
    if envelope && outcome {
      Ok(())
    } else {
      Err("The child sent an invalid script-host v7 module registration response.".to_string())
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptBindings<'a> {
  pub binding_version: u32,
  pub services_version: u32,
  pub secrets: &'a BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptAttempt<'a> {
  pub number: u32,
  pub max_attempts: u32,
  pub idempotency_key: &'a str,
}

impl<'a> ScriptAttempt<'a> {
  pub fn new(number: u32, max_attempts: u32, idempotency_key: &'a str) -> Result<Self, String> {
    if number == 0 || max_attempts == 0 || max_attempts > 10 || number > max_attempts {
      return Err(
        "Script attempt numbers must satisfy 1 <= number <= maxAttempts <= 10.".to_string(),
      );
    }
    if !crate::event::is_definition_hash(idempotency_key) {
      return Err("Script attempts require a canonical sha256 idempotency key.".to_string());
    }
    Ok(Self {
      number,
      max_attempts,
      idempotency_key,
    })
  }
}

impl<'a> ExecuteMessage<'a> {
  pub fn runtime_script(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
  ) -> Self {
    static EMPTY_SECRETS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    Self::runtime_script_with_secrets(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      EMPTY_SECRETS.get_or_init(BTreeMap::new),
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn runtime_script_with_secrets(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    secrets: &'a BTreeMap<String, String>,
  ) -> Self {
    Self::runtime_script_with_modules(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      secrets,
      &[],
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn runtime_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id,
      attempt,
      mode: ScriptExecutionMode::Step,
      handler: "runtime.script",
      timeout_ms,
      source,
      context,
      lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn lifecycle_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    action_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    lifecycle: &'a LifecycleBindingV1,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id: action_id,
      attempt,
      mode: ScriptExecutionMode::Lifecycle,
      handler: "runtime.lifecycle-script",
      timeout_ms,
      source,
      context,
      lifecycle: Some(lifecycle),
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletedMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub outcome: HostOutcome,
  pub duration_ms: f64,
}

impl CompletedMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "completed"
      || self.invocation_id.is_empty()
      || self.invocation_id.chars().count() > 256
      || !self.duration_ms.is_finite()
      || self.duration_ms < 0.0
    {
      return Err("The child sent an invalid script-host v7 completion envelope.".to_string());
    }
    if let HostOutcome::Failure { error } = &self.outcome {
      error.validate()?;
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostOutcome {
  Success { value: Value },
  Failure { error: HostReportedFailure },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostReportedFailureKind {
  ScriptThrew,
  ScriptTimedOut,
  InvalidScriptResult,
  ContextTooLarge,
  ResultTooLarge,
  WorkerCrashed,
  InvocationCancelled,
  ServiceFailed,
}

impl HostReportedFailureKind {
  const fn canonical_kind(self) -> AttemptFailureKind {
    match self {
      Self::ScriptThrew => AttemptFailureKind::ScriptThrew,
      Self::ScriptTimedOut => AttemptFailureKind::ScriptTimedOut,
      Self::InvalidScriptResult => AttemptFailureKind::InvalidScriptResult,
      Self::ContextTooLarge => AttemptFailureKind::ContextTooLarge,
      Self::ResultTooLarge => AttemptFailureKind::ResultTooLarge,
      Self::WorkerCrashed => AttemptFailureKind::WorkerCrashed,
      Self::InvocationCancelled => AttemptFailureKind::InvocationCancelled,
      Self::ServiceFailed => AttemptFailureKind::ServiceFailed,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostReportedFailure {
  pub kind: HostReportedFailureKind,
  pub code: String,
  pub message: String,
  #[serde(default)]
  pub details: Option<FailureSizeDetails>,
  #[serde(default)]
  pub capability: Option<String>,
  #[serde(default)]
  pub operation: Option<String>,
  #[serde(default)]
  pub call_id: Option<String>,
  #[serde(default)]
  pub retryable: Option<bool>,
  #[serde(default)]
  pub ambiguous: Option<bool>,
  #[serde(default)]
  pub cause: Option<CapabilityFailure>,
}

impl HostReportedFailure {
  pub fn validate(&self) -> Result<(), String> {
    let failure = self.clone().into_attempt_failure();
    failure.validate().map_err(|error| error.to_string())
  }

  pub fn into_attempt_failure(self) -> AttemptFailure {
    AttemptFailure {
      kind: self.kind.canonical_kind(),
      code: self.code,
      message: self.message,
      details: self.details,
      capability: self.capability,
      operation: self.operation,
      call_id: self.call_id,
      retryable: self.retryable,
      ambiguous: self.ambiguous,
      cause: self.cause,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub call: CapabilityCallRequest,
}

impl CapabilityCallMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "capability_call"
      || self.invocation_id != self.call.invocation_id
      || self.call_id != self.call.call_id
    {
      return Err("The child sent an invalid script-host v4 capability call envelope.".to_string());
    }
    self.call.validate().map_err(|error| error.message)
  }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityResultMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub call_id: &'a str,
  pub result: &'a CapabilityCallResult,
}

impl<'a> CapabilityResultMessage<'a> {
  pub fn new(result: &'a CapabilityCallResult) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "capability_result",
      invocation_id: result.invocation_id(),
      call_id: result.call_id(),
      result,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchObservationMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub request_id: String,
  pub observation: NativeFetchObservation,
}

impl FetchObservationMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "fetch_observation"
      || self.invocation_id != self.observation.invocation_id()
      || self.request_id != self.observation.request_id()
    {
      return Err("The child sent an invalid native-Fetch observation envelope.".to_string());
    }
    self.observation.validate()
  }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchObservationAckMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub request_id: &'a str,
  pub accepted: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<&'a CapabilityFailure>,
}

impl<'a> FetchObservationAckMessage<'a> {
  pub fn accepted(invocation_id: &'a str, request_id: &'a str) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "fetch_observation_ack",
      invocation_id,
      request_id,
      accepted: true,
      error: None,
    }
  }

  pub fn rejected(
    invocation_id: &'a str,
    request_id: &'a str,
    error: &'a CapabilityFailure,
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "fetch_observation_ack",
      invocation_id,
      request_id,
      accepted: false,
      error: Some(error),
    }
  }
}
