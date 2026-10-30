//! protocol outcomes.

use super::{SCRIPT_HOST_PROTOCOL, SCRIPT_HOST_PROTOCOL_VERSION};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CapabilityFailure;
use crate::FailureSizeDetails;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

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
      return Err("The child sent an invalid Script Host v12 completion envelope.".to_string());
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
