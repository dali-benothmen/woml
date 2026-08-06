use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::RUN_EVENT_SCHEMA_VERSION;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEvent {
  pub event_schema_version: u32,
  pub event_id: String,
  pub run_id: String,
  pub sequence: u64,
  pub occurred_at: DateTime<Utc>,
  #[serde(flatten)]
  pub payload: RunEventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RunEventPayload {
  RunStarted(RunStartedData),
  StepAttemptStarted(StepAttemptStartedData),
  StepAttemptSucceeded(StepAttemptSucceededData),
  StepAttemptFailed(StepAttemptFailedData),
  RunSucceeded(RunSucceededData),
  RunFailed(RunFailedData),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunStartedData {
  pub workflow_id: String,
  pub definition_hash: String,
  pub trigger: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptStartedData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub handler: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptSucceededData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptFailedData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub failure: AttemptFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSucceededData {
  pub terminal_node_id: String,
  pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunFailedData {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub node_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub invocation_id: Option<String>,
  pub failure: AttemptFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptFailureKind {
  ScriptThrew,
  ScriptTimedOut,
  InvalidScriptResult,
  ContextTooLarge,
  ResultTooLarge,
  WorkerCrashed,
  HostCrashed,
  Interrupted,
}

impl AttemptFailureKind {
  pub const fn code(self) -> &'static str {
    match self {
      Self::ScriptThrew => "WOML_SCRIPT_THROWN",
      Self::ScriptTimedOut => "WOML_SCRIPT_TIMEOUT",
      Self::InvalidScriptResult => "WOML_SCRIPT_NON_JSON_RESULT",
      Self::ContextTooLarge => "WOML_SCRIPT_CONTEXT_TOO_LARGE",
      Self::ResultTooLarge => "WOML_SCRIPT_RESULT_TOO_LARGE",
      Self::WorkerCrashed => "WOML_SCRIPT_WORKER_CRASHED",
      Self::HostCrashed => "WOML_SCRIPT_HOST_CRASHED",
      Self::Interrupted => "WOML_STEP_INTERRUPTED",
    }
  }

  pub const fn accepts_size_details(self) -> bool {
    matches!(self, Self::ContextTooLarge | Self::ResultTooLarge)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureSizeDetails {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub actual_bytes: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptFailure {
  pub kind: AttemptFailureKind,
  pub code: String,
  pub message: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<FailureSizeDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventValidationError {
  #[error("unsupported run event schema version {0}")]
  UnsupportedSchemaVersion(u32),
  #[error("{0}")]
  Invalid(String),
}

fn valid_id(value: &str) -> bool {
  !value.is_empty() && value.chars().count() <= 256
}

fn validate_identity(
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Result<(), EventValidationError> {
  if !valid_id(node_id) || !valid_id(invocation_id) || attempt == 0 {
    return Err(EventValidationError::Invalid(
      "Attempt identity requires valid nodeId, invocationId, and attempt >= 1.".to_string(),
    ));
  }
  Ok(())
}

impl AttemptFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if self.code != self.kind.code() {
      return Err(EventValidationError::Invalid(format!(
        "Failure kind {:?} requires code {:?}, received {:?}.",
        self.kind,
        self.kind.code(),
        self.code
      )));
    }
    if self.message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Failure message must not be empty.".to_string(),
      ));
    }
    match (&self.details, self.kind.accepts_size_details()) {
      (Some(_), false) => Err(EventValidationError::Invalid(
        "Only size-limit failures may contain details.".to_string(),
      )),
      (Some(details), true) if details.actual_bytes.is_none() && details.limit_bytes.is_none() => {
        Err(EventValidationError::Invalid(
          "Failure size details must contain actualBytes or limitBytes.".to_string(),
        ))
      }
      _ => Ok(()),
    }
  }
}

impl RunEvent {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION {
      return Err(EventValidationError::UnsupportedSchemaVersion(
        self.event_schema_version,
      ));
    }
    if !valid_id(&self.event_id) || !valid_id(&self.run_id) || self.sequence == 0 {
      return Err(EventValidationError::Invalid(
        "Run events require valid eventId, runId, and sequence >= 1.".to_string(),
      ));
    }
    match &self.payload {
      RunEventPayload::RunStarted(data) => {
        if !valid_id(&data.workflow_id) || !is_definition_hash(&data.definition_hash) {
          return Err(EventValidationError::Invalid(
            "run_started requires a valid workflowId and definitionHash.".to_string(),
          ));
        }
      }
      RunEventPayload::StepAttemptStarted(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
        if data.handler.is_empty() || data.handler.chars().count() > 256 {
          return Err(EventValidationError::Invalid(
            "step_attempt_started requires a valid handler.".to_string(),
          ));
        }
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
      }
      RunEventPayload::StepAttemptFailed(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
        data.failure.validate()?;
      }
      RunEventPayload::RunSucceeded(data) => {
        if !valid_id(&data.terminal_node_id) {
          return Err(EventValidationError::Invalid(
            "run_succeeded requires a valid terminalNodeId.".to_string(),
          ));
        }
      }
      RunEventPayload::RunFailed(data) => {
        let identity_fields = [
          data.node_id.is_some(),
          data.attempt.is_some(),
          data.invocation_id.is_some(),
        ];
        if identity_fields.iter().any(|present| *present)
          && !identity_fields.iter().all(|present| *present)
        {
          return Err(EventValidationError::Invalid(
            "run_failed attempt identity must provide nodeId, attempt, and invocationId together."
              .to_string(),
          ));
        }
        if let (Some(node_id), Some(attempt), Some(invocation_id)) =
          (&data.node_id, data.attempt, &data.invocation_id)
        {
          validate_identity(node_id, attempt, invocation_id)?;
        }
        data.failure.validate()?;
      }
    }
    Ok(())
  }
}

pub fn is_definition_hash(value: &str) -> bool {
  let Some(hex) = value.strip_prefix("sha256:") else {
    return false;
  };
  hex.len() == 64
    && hex
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
