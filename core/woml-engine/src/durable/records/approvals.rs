//! Durable records / approvals.

use crate::event::ApprovalDecision;
use crate::event::ApprovalDecisionSource;
use crate::event::ApprovalResolution;
use crate::ReviewPayloadV1;
use crate::ReviewerIdentityClaimV1;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedApprovalToken {
  pub token: String,
  pub token_id: String,
  pub request_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub issued_at: DateTime<Utc>,
  pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalTokenBinding {
  pub token_id: String,
  pub request_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub issued_at: DateTime<Utc>,
  pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionOutcomeStatus {
  Accepted,
  AlreadyResolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionOutcome {
  pub status: ApprovalDecisionOutcomeStatus,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub decision: ApprovalDecision,
  pub source: ApprovalDecisionSource,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutSettlementStatus {
  Settled,
  AlreadyResolved,
  NotDue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalTimeoutSettlement {
  pub status: ApprovalTimeoutSettlementStatus,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub resolution: Option<ApprovalResolution>,
  pub settled_at: Option<DateTime<Utc>>,
}

pub const RICH_APPROVAL_INSPECTION_DECISION_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalDecisionAdmission {
  pub run_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub principal_id: String,
  pub decision: ApprovalDecision,
  pub provider: String,
  pub provider_account_id: String,
  pub provider_actor_id: String,
  pub roles: Vec<String>,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalIdentityDecision {
  pub run_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub decision: ApprovalDecision,
  pub identity: ReviewerIdentityClaimV1,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichApprovalDecisionStatus {
  Pending,
  Approved,
  Rejected,
  Idempotent,
  AlreadyResolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalDecisionOutcome {
  pub status: RichApprovalDecisionStatus,
  pub required: u32,
  pub received: u32,
  pub represented_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichApprovalDecisionInspection {
  pub principal_id: String,
  pub decision: ApprovalDecision,
  pub provider: String,
  pub provider_account_id: String,
  pub roles: Vec<String>,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichApprovalInspectionV1 {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub review: ReviewPayloadV1,
  pub required: u32,
  pub required_roles: Vec<String>,
  pub directory_version: String,
  pub status: String,
  pub created_at: DateTime<Utc>,
  pub resolved_at: Option<DateTime<Utc>>,
  pub decisions: Vec<RichApprovalDecisionInspection>,
  pub total_decisions: u32,
  pub decisions_truncated: bool,
}

#[derive(Debug, Clone)]
pub(in crate::durable) struct RichReviewRow {
  pub(in crate::durable) approval_id: String,
  pub(in crate::durable) request_id: String,
  pub(in crate::durable) review_digest: String,
  pub(in crate::durable) required: u32,
  pub(in crate::durable) required_roles: Vec<String>,
  pub(in crate::durable) directory_version: String,
  pub(in crate::durable) status: String,
}

#[derive(Debug, Clone)]
pub(in crate::durable) struct RichQuorumState {
  pub(in crate::durable) received: u32,
  pub(in crate::durable) represented_roles: Vec<String>,
}
