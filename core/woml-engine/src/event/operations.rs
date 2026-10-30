//! event operations.

use crate::capability::CapabilityFailure;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationExecutionMode {
  Observed,
  Managed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationStartedData {
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationSucceededData {
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
  pub duration_ms: f64,
  pub result_bytes: u64,
  pub result_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFailedData {
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
  pub duration_ms: f64,
  pub failure: CapabilityFailure,
}
