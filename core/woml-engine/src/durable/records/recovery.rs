//! Durable records / recovery.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
  pub inspected_runs: usize,
  pub recovered_runs: usize,
  pub interrupted_attempts: usize,
  pub resumable_runs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkRecoveryBranchV1 {
  pub fork_id: String,
  pub branch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkRecoveryWorkV1 {
  pub profile: &'static str,
  pub openable_fork_ids: Vec<String>,
  pub pending_branches: Vec<ForkRecoveryBranchV1>,
  pub joinable_fork_ids: Vec<String>,
  pub pending_choice_ids: Vec<String>,
  pub ambiguous_active_node_ids: Vec<String>,
}
