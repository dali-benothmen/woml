//! projection control flow.

use super::{
  AttemptProjection, LifecycleHookProjection, LockProjection, OperationIdentity,
  OperationProjection, RetryScheduleProjection, ReusableLifecycleProjection, WorkflowContext,
};
use crate::event::ForkBranchOutcome;
use crate::event::ParallelGroupOutcome;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelGroupStatus {
  Started,
  Completed {
    outcome: ParallelGroupOutcome,
    failed_node_ids: Vec<String>,
    cancelled_node_ids: Vec<String>,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelGroupProjection {
  pub parallel_id: String,
  pub fork_context: WorkflowContext,
  pub status: ParallelGroupStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkJoinStatus {
  Pending,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkBranchProjection {
  pub branch_id: String,
  pub terminal_node_id: Option<String>,
  pub outcome: Option<ForkBranchOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkProjection {
  pub fork_id: String,
  pub branches: BTreeMap<String, ForkBranchProjection>,
  pub join_status: ForkJoinStatus,
  pub blocking_branch_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForEachStatus {
  Open,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForEachIterationStatus {
  Started,
  Succeeded { result: Option<Value> },
  Failed { failed_node_id: String },
  Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForEachProjection {
  pub for_each_id: String,
  pub total: u32,
  pub items_digest: String,
  pub concurrency: u32,
  pub status: ForEachStatus,
  pub iterations: BTreeMap<u32, ForEachIterationStatus>,
  pub executions: BTreeMap<u32, ForEachIterationExecutionProjection>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ForEachIterationExecutionProjection {
  pub context: WorkflowContext,
  pub attempts: Vec<AttemptProjection>,
  pub operations: BTreeMap<OperationIdentity, OperationProjection>,
  pub pending_retries: BTreeMap<String, RetryScheduleProjection>,
  pub branch_selections: BTreeMap<String, String>,
  pub choice_selections: BTreeMap<String, String>,
  pub parallel_groups: BTreeMap<String, ParallelGroupProjection>,
  pub lifecycle_hooks: BTreeMap<String, LifecycleHookProjection>,
  pub reusable_lifecycle_hooks: BTreeMap<String, ReusableLifecycleProjection>,
  pub locks: BTreeMap<String, LockProjection>,
}
