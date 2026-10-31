//! event parallel.

use serde::Deserialize;
use serde::Serialize;

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
