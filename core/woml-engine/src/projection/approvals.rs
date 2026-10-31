//! projection approvals.

use crate::event::ApprovalDecision;
use crate::event::ApprovalResolution;
use crate::event::ApprovalTimeoutPolicy;
use chrono::DateTime;
use chrono::Utc;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalRequestStatus {
  Waiting,
  Resolved {
    resolution: ApprovalResolution,
    resolved_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestProjection {
  pub approval_id: String,
  pub request_id: String,
  pub requested_at: DateTime<Utc>,
  pub expires_at: Option<DateTime<Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
  pub status: ApprovalRequestStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichApprovalRequestStatus {
  Waiting,
  Approved,
  Rejected,
  Expired,
  Cancelled,
  Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalReviewerDecisionProjection {
  pub principal_id: String,
  pub decision: ApprovalDecision,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalRequestProjection {
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub directory_version: String,
  pub required: u32,
  pub required_roles: Vec<String>,
  pub represented_roles: Vec<String>,
  pub requested_at: DateTime<Utc>,
  pub resolved_at: Option<DateTime<Utc>>,
  pub status: RichApprovalRequestStatus,
  pub decisions: BTreeMap<String, RichApprovalReviewerDecisionProjection>,
}
