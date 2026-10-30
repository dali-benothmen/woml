//! Durable records / inspection.

use crate::durable::{InspectedBusinessOutcome, RichApprovalDecisionInspection};
use crate::ReviewPayloadV1;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicRunStatus {
  NotStarted,
  Queued,
  Running,
  Waiting,
  Cancelling,
  Finalizing,
  Succeeded,
  Failed,
  Cancelled,
}

impl PublicRunStatus {
  pub(in crate::durable) const fn as_str(self) -> &'static str {
    match self {
      Self::NotStarted => "not_started",
      Self::Queued => "queued",
      Self::Running => "running",
      Self::Waiting => "waiting",
      Self::Cancelling => "cancelling",
      Self::Finalizing => "finalizing",
      Self::Succeeded => "succeeded",
      Self::Failed => "failed",
      Self::Cancelled => "cancelled",
    }
  }

  pub(in crate::durable) fn parse(value: &str) -> Option<Self> {
    Some(match value {
      "not_started" => Self::NotStarted,
      "queued" => Self::Queued,
      "running" => Self::Running,
      "waiting" => Self::Waiting,
      "cancelling" => Self::Cancelling,
      "finalizing" => Self::Finalizing,
      "succeeded" => Self::Succeeded,
      "failed" => Self::Failed,
      "cancelled" => Self::Cancelled,
      _ => return None,
    })
  }
}

impl From<RunStatus> for PublicRunStatus {
  fn from(value: RunStatus) -> Self {
    match value {
      RunStatus::NotStarted => Self::NotStarted,
      RunStatus::Queued => Self::Queued,
      RunStatus::Running => Self::Running,
      RunStatus::Waiting => Self::Waiting,
      RunStatus::Cancelling => Self::Cancelling,
      RunStatus::Finalizing => Self::Finalizing,
      RunStatus::Succeeded => Self::Succeeded,
      RunStatus::Failed => Self::Failed,
      RunStatus::Cancelled => Self::Cancelled,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryV1 {
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub started_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListV1 {
  pub profile: &'static str,
  pub runs: Vec<RunSummaryV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyWaitingFor {
  Concurrency,
  RateLimit,
}

impl PolicyWaitingFor {
  pub(in crate::durable) fn parse(value: &str) -> Option<Self> {
    match value {
      "concurrency" => Some(Self::Concurrency),
      "rate_limit" => Some(Self::RateLimit),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryV2 {
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub admitted_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub started_at: Option<DateTime<Utc>>,
  pub updated_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub queue: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waiting_for: Option<PolicyWaitingFor>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub eligible_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListV2 {
  pub profile: &'static str,
  pub runs: Vec<RunSummaryV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObservationV1 {
  pub profile: &'static str,
  pub status_totals: BTreeMap<String, u64>,
  pub retrying_run_ids: Vec<String>,
  pub approval_waiting_run_ids: Vec<String>,
  pub retries_total: u64,
  pub triggers_total: u64,
  pub workflow_calls_active: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionHookV2 {
  pub hook_id: String,
  pub subject_kind: crate::event::LifecycleSubjectKind,
  pub subject_id: String,
  pub status: crate::projection::LifecycleHookStatus,
  pub failed_actions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionCancellationV2 {
  pub requested: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV2 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionPolicyV3 {
  pub queue: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waiting_for: Option<PolicyWaitingFor>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub eligible_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub timeout_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV3 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedForkStatusV4 {
  Active,
  Waiting,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedForkJoinModeV4 {
  All,
  Selected,
  None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForkCountsV4 {
  pub opened: u32,
  pub active: u32,
  pub waiting: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub cancelled: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForkJoinV4 {
  pub mode: InspectedForkJoinModeV4,
  pub branch_ids: Vec<String>,
  pub status: crate::projection::ForkJoinStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForkV4 {
  pub fork_id: String,
  pub status: InspectedForkStatusV4,
  pub join: RunInspectionForkJoinV4,
  pub branches: RunInspectionForkCountsV4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForksV4 {
  pub counts: RunInspectionForkCountsV4,
  pub items: Vec<RunInspectionForkV4>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV4 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectedReusableKindV5 {
  Step,
  NotificationProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedReusableStatusV5 {
  Pending,
  Running,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedReusableLifecycleStatusV5 {
  Idle,
  Running,
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionReusableItemV5 {
  pub invocation_id: String,
  pub alias: String,
  pub definition_digest_prefix: String,
  pub kind: InspectedReusableKindV5,
  pub status: InspectedReusableStatusV5,
  pub lifecycle_status: InspectedReusableLifecycleStatusV5,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionReusableCountsV5 {
  pub pending: u32,
  pub running: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub cancelled: u32,
  pub completed_with_warnings: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionReusableDefinitionsV5 {
  pub counts: RunInspectionReusableCountsV5,
  pub items: Vec<RunInspectionReusableItemV5>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV5 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
  pub reusable_definitions: RunInspectionReusableDefinitionsV5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedForEachStatusV6 {
  Active,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForEachCountsV6 {
  pub opened: u32,
  pub active: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub cancelled: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForEachItemV6 {
  pub for_each_id: String,
  pub status: InspectedForEachStatusV6,
  pub total: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub skipped: u32,
  pub active: u32,
  pub pending: u32,
  pub concurrency: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failed_index: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failed_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForEachV6 {
  pub counts: RunInspectionForEachCountsV6,
  pub items: Vec<RunInspectionForEachItemV6>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV6 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
  pub reusable_definitions: RunInspectionReusableDefinitionsV5,
  pub for_each: RunInspectionForEachV6,
}

pub const RUN_INSPECTION_RICH_APPROVAL_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionRichApprovalV11 {
  pub approval_id: String,
  pub request_id: String,
  pub status: String,
  pub review_digest: String,
  pub review: ReviewPayloadV1,
  pub required: u32,
  pub received: u32,
  pub required_roles: Vec<String>,
  pub represented_roles: Vec<String>,
  pub directory_version: String,
  pub requested_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub resolved_at: Option<DateTime<Utc>>,
  pub decisions: Vec<RichApprovalDecisionInspection>,
  pub total_decisions: u32,
  pub decisions_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionRichApprovalsV11 {
  pub pending: u32,
  pub items: Vec<RunInspectionRichApprovalV11>,
  pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV11 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
  pub reusable_definitions: RunInspectionReusableDefinitionsV5,
  pub for_each: RunInspectionForEachV6,
  pub approvals: RunInspectionRichApprovalsV11,
}
