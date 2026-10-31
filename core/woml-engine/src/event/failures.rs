//! event failures.

use super::{
  valid_capability_name, valid_id, ApprovalFailure, EventValidationError, NotificationRunFailure,
};
use crate::capability::CapabilityFailure;
use serde::Deserialize;
use serde::Serialize;

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
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV4 {
  Approval {
    #[serde(rename = "approvalId")]
    approval_id: String,
    #[serde(rename = "requestId")]
    request_id: String,
    failure: ApprovalFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV5 {
  Notification {
    #[serde(rename = "approvalId")]
    approval_id: String,
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "failedDeliveryIds")]
    failed_delivery_ids: Vec<String>,
    failure: NotificationRunFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunFailedData {
  V5(RunFailedDataV5),
  V4(RunFailedDataV4),
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
  ServiceFailed,
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
      Self::ServiceFailed => "WOML_SERVICE_FAILED",
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptFailure {
  pub kind: AttemptFailureKind,
  pub code: String,
  pub message: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<FailureSizeDetails>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub capability: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub operation: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub call_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retryable: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ambiguous: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cause: Option<CapabilityFailure>,
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

impl AttemptFailure {
  pub fn legacy_defaults() -> Self {
    Self {
      kind: AttemptFailureKind::Interrupted,
      code: AttemptFailureKind::Interrupted.code().to_string(),
      message: "legacy failure defaults".to_string(),
      details: None,
      capability: None,
      operation: None,
      call_id: None,
      retryable: None,
      ambiguous: None,
      cause: None,
    }
  }

  pub fn validate(&self) -> Result<(), EventValidationError> {
    let service_fields = (
      self.capability.as_deref(),
      self.operation.as_deref(),
      self.call_id.as_deref(),
      self.retryable,
      self.ambiguous,
      self.cause.as_ref(),
    );
    if self.kind == AttemptFailureKind::ServiceFailed {
      let valid = matches!(service_fields,
        (Some(capability), Some(operation), Some(call_id), Some(retryable), Some(ambiguous), Some(cause))
          if valid_capability_name(capability)
            && valid_capability_name(operation)
            && valid_id(call_id)
            && retryable == cause.retryable
            && ambiguous == cause.ambiguous
            && self.code == cause.code
            && cause.validate().is_ok()
      );
      if !valid || self.details.is_some() {
        return Err(EventValidationError::Invalid(
          "service_failed requires capability, operation, callId, retryable, ambiguous, and a matching safe cause.".to_string(),
        ));
      }
    } else if self.code != self.kind.code() {
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
    if self.kind != AttemptFailureKind::ServiceFailed
      && [
        self.capability.is_some(),
        self.operation.is_some(),
        self.call_id.is_some(),
        self.retryable.is_some(),
        self.ambiguous.is_some(),
        self.cause.is_some(),
      ]
      .into_iter()
      .any(|present| present)
    {
      return Err(EventValidationError::Invalid(
        "Only service_failed may contain service failure fields.".to_string(),
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
