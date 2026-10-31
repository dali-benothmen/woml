//! Durable records / policy.

use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerClaimV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub owner_id: String,
  pub claim_id: String,
  pub claimed_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyClaimWaitReason {
  Concurrency,
  RateLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PolicyExecutionClaimResult {
  Claimed {
    claim: SchedulerClaimV1,
    #[serde(rename = "firstStart")]
    first_start: bool,
  },
  Waiting {
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    queue: String,
    reason: PolicyClaimWaitReason,
    #[serde(rename = "eligibleAt", skip_serializing_if = "Option::is_none")]
    eligible_at: Option<DateTime<Utc>>,
  },
}
