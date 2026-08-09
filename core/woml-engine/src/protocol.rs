use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AttemptFailure, AttemptFailureKind, FailureSizeDetails, WorkflowContext};

pub const SCRIPT_HOST_PROTOCOL: &str = "woml.script-host";
pub const SCRIPT_HOST_PROTOCOL_VERSION: u32 = 3;

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
      return Err("The child did not send a valid script-host v3 ready message.".to_string());
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
  pub handler: &'static str,
  pub timeout_ms: u64,
  pub source: &'a str,
  pub context: &'a WorkflowContext,
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
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id,
      attempt,
      handler: "runtime.script",
      timeout_ms,
      source,
      context,
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
      return Err("The child sent an invalid script-host v3 completion envelope.".to_string());
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
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReportedFailure {
  pub kind: HostReportedFailureKind,
  pub code: String,
  pub message: String,
  #[serde(default)]
  pub details: Option<FailureSizeDetails>,
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
      ..AttemptFailure::legacy_defaults()
    }
  }
}
