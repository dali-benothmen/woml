//! event iterations.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachIterationScope {
  pub for_each_id: String,
  pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachOpenedData {
  pub for_each_id: String,
  pub total: u32,
  pub items_digest: String,
  pub concurrency: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachIterationIdentityData {
  pub for_each_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachIterationSucceededData {
  pub for_each_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForEachIterationFailureKind {
  ScriptThrew,
  TimedOut,
  NonJson,
  WorkerCrashed,
  HostCrashed,
  ResultTooLarge,
  ContextTooLarge,
  Interrupted,
  CapabilityFailed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachIterationFailedData {
  pub for_each_id: String,
  pub failed_node_id: String,
  pub failure_kind: ForEachIterationFailureKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForEachSkipReason {
  Failure,
  Cancellation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachIterationSkippedData {
  pub for_each_id: String,
  pub reason: ForEachSkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForEachSettledData {
  pub for_each_id: String,
  pub total: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub skipped: u32,
  pub aggregate_digest: String,
}
