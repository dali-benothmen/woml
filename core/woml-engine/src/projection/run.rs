//! projection run.

use super::{
  ApprovalRequestProjection, AttemptProjection, AttemptStatus, DurableWaitProjection,
  ForEachProjection, ForkProjection, LifecycleHookProjection, LifecycleStatus, LockProjection,
  NotificationDeliveryProjection, NotificationMessageUpdateProjection, OperationIdentity,
  OperationProjection, OperationStatus, ParallelGroupProjection, RetryScheduleProjection,
  ReusableLifecycleProjection, RichApprovalRequestProjection, SagaProjection, WorkflowContext,
};
use crate::event::ApprovalFailure;
use crate::event::AttemptFailure;
use crate::event::BranchFailure;
use crate::event::BusinessOutcome;
use crate::event::LifecycleFailure;
use crate::event::LifecycleWarning;
use crate::event::OperationExecutionMode;
use crate::event::ParallelFailure;
use crate::event::ParallelFailurePolicy;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
  #[default]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
  Attempt(AttemptFailure),
  Branch(BranchFailure),
  Parallel {
    parallel_id: String,
    policy: ParallelFailurePolicy,
    primary_node_id: String,
    failed_node_ids: Vec<String>,
    cancelled_node_ids: Vec<String>,
    failure: ParallelFailure,
  },
  Approval {
    approval_id: String,
    request_id: String,
    failure: ApprovalFailure,
  },
  Notification {
    approval_id: String,
    request_id: String,
    failed_delivery_ids: Vec<String>,
    failure: crate::event::NotificationRunFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProjection {
  pub event_schema_version: Option<u32>,
  pub run_id: Option<String>,
  pub workflow_id: Option<String>,
  pub definition_hash: Option<String>,
  pub policy_hash: Option<String>,
  pub queue: Option<String>,
  pub occurrence_sequence: Option<u64>,
  pub admitted_at: Option<DateTime<Utc>>,
  pub started_at: Option<DateTime<Utc>>,
  pub timeout_at: Option<DateTime<Utc>>,
  pub timeout_reached_at: Option<DateTime<Utc>>,
  pub trigger_id: Option<String>,
  pub trigger_handler: Option<String>,
  pub trigger_occurrence_id: Option<String>,
  pub ingress: Option<crate::RunIngress>,
  pub status: RunStatus,
  pub business_outcome: Option<BusinessOutcome>,
  pub lifecycle_status: LifecycleStatus,
  pub cancellation_request_id: Option<String>,
  pub lifecycle_hooks: BTreeMap<String, LifecycleHookProjection>,
  pub reusable_lifecycle_hooks: BTreeMap<String, ReusableLifecycleProjection>,
  pub lifecycle_warnings: Vec<LifecycleWarning>,
  pub context: WorkflowContext,
  pub attempts: Vec<AttemptProjection>,
  pub infrastructure_policy_decisions: Vec<crate::event::InfrastructurePolicyDecisionRecordedData>,
  pub operations: BTreeMap<OperationIdentity, OperationProjection>,
  pub pending_retries: BTreeMap<String, RetryScheduleProjection>,
  pub branch_selections: BTreeMap<String, String>,
  pub choice_selections: BTreeMap<String, String>,
  pub parallel_groups: BTreeMap<String, ParallelGroupProjection>,
  pub forks: BTreeMap<String, ForkProjection>,
  pub for_each: BTreeMap<String, ForEachProjection>,
  pub approval_requests: BTreeMap<String, ApprovalRequestProjection>,
  pub rich_approval_requests: BTreeMap<String, RichApprovalRequestProjection>,
  pub durable_waits: BTreeMap<String, DurableWaitProjection>,
  pub locks: BTreeMap<String, LockProjection>,
  pub sagas: BTreeMap<String, SagaProjection>,
  pub external_signals: Vec<crate::ExternalSignalReceivedData>,
  pub notification_deliveries: BTreeMap<String, NotificationDeliveryProjection>,
  pub notification_updates: BTreeMap<String, NotificationMessageUpdateProjection>,
  pub notification_decisions: Vec<crate::event::NotificationDecisionAcceptedData>,
  pub terminal_node_id: Option<String>,
  pub result: Option<Value>,
  pub failure: Option<RunFailure>,
  pub lifecycle_failure: Option<LifecycleFailure>,
  pub last_sequence: u64,
}

impl RunProjection {
  pub fn completed_node_ids(&self) -> HashSet<&str> {
    self.context.steps.keys().map(String::as_str).collect()
  }

  pub fn attempted_node_ids(&self) -> HashSet<&str> {
    self
      .attempts
      .iter()
      .map(|attempt| attempt.identity.node_id.as_str())
      .collect()
  }

  pub fn active_attempt_node_ids(&self) -> HashSet<&str> {
    self
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
      .map(|attempt| attempt.identity.node_id.as_str())
      .collect()
  }

  pub fn latest_attempt(&self, node_id: &str) -> Option<&AttemptProjection> {
    self
      .attempts
      .iter()
      .rev()
      .find(|attempt| attempt.identity.node_id == node_id)
  }

  pub fn active_managed_operations(&self, invocation_id: &str) -> Vec<&OperationProjection> {
    self
      .operations
      .values()
      .filter(|operation| {
        operation.identity.invocation_id == invocation_id
          && operation.execution_mode == OperationExecutionMode::Managed
          && operation.status == OperationStatus::Started
      })
      .collect()
  }
}
