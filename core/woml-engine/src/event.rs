use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
  RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V2, RUN_EVENT_SCHEMA_VERSION_V3,
};

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
  BranchSelected(BranchSelectedData),
  ParallelGroupStarted(ParallelGroupStartedData),
  ParallelGroupCompleted(ParallelGroupCompletedData),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchSelectedData {
  pub branch_id: String,
  pub arm_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParallelGroupStartedData {
  pub parallel_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelGroupOutcome {
  Succeeded,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParallelGroupCompletedData {
  pub parallel_id: String,
  pub outcome: ParallelGroupOutcome,
  pub failed_node_ids: Vec<String>,
  pub cancelled_node_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSucceededData {
  pub terminal_node_id: String,
  pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunFailedDataV1 {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub node_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub invocation_id: Option<String>,
  pub failure: AttemptFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV2 {
  Attempt {
    #[serde(rename = "nodeId")]
    node_id: String,
    attempt: u32,
    #[serde(rename = "invocationId")]
    invocation_id: String,
    failure: AttemptFailure,
  },
  Branch {
    #[serde(rename = "branchId")]
    branch_id: String,
    #[serde(rename = "armId", default, skip_serializing_if = "Option::is_none")]
    arm_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<Vec<String>>,
    failure: BranchFailure,
  },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParallelFailurePolicy {
  #[serde(rename = "fail-fast")]
  FailFast,
  #[serde(rename = "wait-all")]
  WaitAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
}

impl ParallelFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if self.kind != "parallel_child_failed" || self.code != "WOML_PARALLEL_CHILD_FAILED" {
      return Err(EventValidationError::Invalid(
        "Parallel failure requires kind parallel_child_failed and code WOML_PARALLEL_CHILD_FAILED."
          .to_string(),
      ));
    }
    if self.message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Parallel failure message must not be empty.".to_string(),
      ));
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV3 {
  Parallel {
    #[serde(rename = "parallelId")]
    parallel_id: String,
    policy: ParallelFailurePolicy,
    #[serde(rename = "primaryNodeId")]
    primary_node_id: String,
    #[serde(rename = "failedNodeIds")]
    failed_node_ids: Vec<String>,
    #[serde(rename = "cancelledNodeIds")]
    cancelled_node_ids: Vec<String>,
    failure: ParallelFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunFailedData {
  V3(RunFailedDataV3),
  V2(RunFailedDataV2),
  V1(RunFailedDataV1),
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
  InvocationCancelled,
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
      Self::InvocationCancelled => "WOML_SCRIPT_CANCELLED",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonValueType {
  Null,
  Number,
  String,
  Array,
  Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BranchFailure {
  BranchTestNotBoolean {
    code: String,
    message: String,
    #[serde(rename = "actualType")]
    actual_type: JsonValueType,
  },
  ReferenceNotAvailable {
    code: String,
    message: String,
  },
  BranchSelectionInvalid {
    code: String,
    message: String,
  },
}

impl BranchFailure {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::BranchTestNotBoolean { .. } => "WOML_BRANCH_TEST_NOT_BOOLEAN",
      Self::ReferenceNotAvailable { .. } => "WOML_REFERENCE_NOT_AVAILABLE",
      Self::BranchSelectionInvalid { .. } => "WOML_BRANCH_SELECTION_INVALID",
    }
  }

  pub fn validate(&self) -> Result<(), EventValidationError> {
    let (code, message) = match self {
      Self::BranchTestNotBoolean { code, message, .. }
      | Self::ReferenceNotAvailable { code, message }
      | Self::BranchSelectionInvalid { code, message } => (code, message),
    };
    if code != self.code() {
      return Err(EventValidationError::Invalid(format!(
        "Branch failure requires code {:?}, received {:?}.",
        self.code(),
        code
      )));
    }
    if message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Branch failure message must not be empty.".to_string(),
      ));
    }
    Ok(())
  }
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

fn valid_public_structural_id(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
    && characters.all(|character| character.is_ascii_alphanumeric())
    && value.chars().count() <= 256
}

fn valid_branch_arm_id(branch_id: &str, arm_id: &str) -> bool {
  let Some(suffix) = arm_id.strip_prefix(&format!("{branch_id}:")) else {
    return false;
  };
  if suffix == "otherwise" {
    return true;
  }
  let Some(index) = suffix.strip_prefix("when:") else {
    return false;
  };
  !index.is_empty()
    && index.bytes().all(|byte| byte.is_ascii_digit())
    && (index == "0" || !index.starts_with('0'))
    && arm_id.chars().count() <= 256
}

fn valid_ordered_id_lists(first: &[String], second: &[String]) -> bool {
  let mut seen = std::collections::HashSet::new();
  first
    .iter()
    .chain(second)
    .all(|node_id| valid_public_structural_id(node_id) && seen.insert(node_id))
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
    if !matches!(
      self.event_schema_version,
      RUN_EVENT_SCHEMA_VERSION_V1 | RUN_EVENT_SCHEMA_VERSION_V2 | RUN_EVENT_SCHEMA_VERSION_V3
    ) {
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
        if data.failure.kind == AttemptFailureKind::InvocationCancelled
          && self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V3
        {
          return Err(EventValidationError::Invalid(
            "invocation_cancelled is available only in run-event schema v3.".to_string(),
          ));
        }
      }
      RunEventPayload::BranchSelected(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V2 | RUN_EVENT_SCHEMA_VERSION_V3
        ) {
          return Err(EventValidationError::Invalid(
            "branch_selected is available only in run-event schema v2 or v3.".to_string(),
          ));
        }
        if !valid_public_structural_id(&data.branch_id)
          || !valid_branch_arm_id(&data.branch_id, &data.arm_id)
        {
          return Err(EventValidationError::Invalid(
            "branch_selected requires a valid branchId and matching canonical armId.".to_string(),
          ));
        }
      }
      RunEventPayload::ParallelGroupStarted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V3
          || !valid_public_structural_id(&data.parallel_id)
        {
          return Err(EventValidationError::Invalid(
            "parallel_group_started requires event schema v3 and a valid parallelId.".to_string(),
          ));
        }
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V3
          || !valid_public_structural_id(&data.parallel_id)
          || !valid_ordered_id_lists(&data.failed_node_ids, &data.cancelled_node_ids)
          || (data.outcome == ParallelGroupOutcome::Succeeded
            && (!data.failed_node_ids.is_empty() || !data.cancelled_node_ids.is_empty()))
          || (data.outcome == ParallelGroupOutcome::Failed
            && data.failed_node_ids.is_empty()
            && data.cancelled_node_ids.is_empty())
        {
          return Err(EventValidationError::Invalid(
            "parallel_group_completed contains an invalid outcome or node list.".to_string(),
          ));
        }
      }
      RunEventPayload::RunSucceeded(data) => {
        if !valid_id(&data.terminal_node_id) {
          return Err(EventValidationError::Invalid(
            "run_succeeded requires a valid terminalNodeId.".to_string(),
          ));
        }
      }
      RunEventPayload::RunFailed(data) => match (self.event_schema_version, data) {
        (RUN_EVENT_SCHEMA_VERSION_V1, RunFailedData::V1(data)) => {
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
        (
          RUN_EVENT_SCHEMA_VERSION_V2 | RUN_EVENT_SCHEMA_VERSION_V3,
          RunFailedData::V2(RunFailedDataV2::Attempt {
            node_id,
            attempt,
            invocation_id,
            failure,
          }),
        ) => {
          validate_identity(node_id, *attempt, invocation_id)?;
          failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V2 | RUN_EVENT_SCHEMA_VERSION_V3,
          RunFailedData::V2(RunFailedDataV2::Branch {
            branch_id,
            arm_id,
            path,
            failure,
          }),
        ) => {
          if !valid_public_structural_id(branch_id)
            || arm_id
              .as_deref()
              .is_some_and(|arm_id| !valid_branch_arm_id(branch_id, arm_id))
            || path
              .as_ref()
              .is_some_and(|path| path.is_empty() || path.iter().any(String::is_empty))
          {
            return Err(EventValidationError::Invalid(
              "Branch-scoped run_failed contains an invalid branchId, armId, or path.".to_string(),
            ));
          }
          match failure {
            BranchFailure::BranchTestNotBoolean { .. } if arm_id.is_none() || path.is_none() => {
              return Err(EventValidationError::Invalid(
                "branch_test_not_boolean requires armId and path.".to_string(),
              ));
            }
            BranchFailure::ReferenceNotAvailable { .. } if path.is_none() => {
              return Err(EventValidationError::Invalid(
                "reference_not_available requires path.".to_string(),
              ));
            }
            _ => {}
          }
          failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V3,
          RunFailedData::V3(RunFailedDataV3::Parallel {
            parallel_id,
            primary_node_id,
            failed_node_ids,
            cancelled_node_ids,
            failure,
            ..
          }),
        ) => {
          if !valid_public_structural_id(parallel_id)
            || !valid_public_structural_id(primary_node_id)
            || !valid_ordered_id_lists(failed_node_ids, cancelled_node_ids)
            || failed_node_ids.is_empty()
            || !failed_node_ids
              .iter()
              .any(|node_id| node_id == primary_node_id)
          {
            return Err(EventValidationError::Invalid(
              "Parallel-scoped run_failed contains an invalid group or child identity list."
                .to_string(),
            ));
          }
          failure.validate()?;
        }
        _ => {
          return Err(EventValidationError::Invalid(format!(
            "run_failed payload does not belong to event schema v{}.",
            self.event_schema_version
          )));
        }
      },
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
