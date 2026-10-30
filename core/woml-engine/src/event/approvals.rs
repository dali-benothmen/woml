//! event approvals.

use super::EventValidationError;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutPolicy {
  Reject,
  Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalRequestedData {
  pub approval_id: String,
  pub request_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<DateTime<Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
  Approved,
  Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionSource {
  Human,
  Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApprovalResolution {
  Decision {
    decision: ApprovalDecision,
    source: ApprovalDecisionSource,
  },
  TimeoutFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalResolvedData {
  pub approval_id: String,
  pub request_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub resolution: Option<ApprovalResolution>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub review_digest: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub decision: Option<ApprovalDecision>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reason: Option<RichApprovalResolutionReason>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub required: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub received: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub required_roles: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub represented_roles: Option<Vec<String>>,
}

impl ApprovalResolvedData {
  pub fn basic(approval_id: String, request_id: String, resolution: ApprovalResolution) -> Self {
    Self {
      approval_id,
      request_id,
      resolution: Some(resolution),
      review_digest: None,
      decision: None,
      reason: None,
      required: None,
      received: None,
      required_roles: None,
      represented_roles: None,
    }
  }

  pub const fn is_rich(&self) -> bool {
    self.review_digest.is_some()
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RichApprovalResolutionReason {
  Quorum,
  Rejection,
  Timeout,
  Cancellation,
  ArtifactInvalidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalReviewRequestedData {
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub directory_version: String,
  pub required: u32,
  pub required_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalReviewerDecisionRecordedData {
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub principal_id: String,
  pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalQuorumProgressedData {
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub required: u32,
  pub received: u32,
  pub required_roles: Vec<String>,
  pub represented_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalInvalidatedData {
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub reason: RichApprovalResolutionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
}

impl ApprovalFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if self.kind != "approval_timeout" || self.code != "WOML_APPROVAL_TIMEOUT" {
      return Err(EventValidationError::Invalid(
        "Approval failure requires kind approval_timeout and code WOML_APPROVAL_TIMEOUT."
          .to_string(),
      ));
    }
    if self.message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Approval failure message must not be empty.".to_string(),
      ));
    }
    Ok(())
  }
}

impl ApprovalResolution {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if matches!(
      self,
      Self::Decision {
        decision: ApprovalDecision::Approved,
        source: ApprovalDecisionSource::Timeout,
      }
    ) {
      return Err(EventValidationError::Invalid(
        "A timeout approval resolution may reject only.".to_string(),
      ));
    }
    Ok(())
  }
}
