//! projection operations.

use crate::event::OperationExecutionMode;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationIdentity {
  pub invocation_id: String,
  pub call_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationStatus {
  Started,
  Succeeded {
    duration_ms: f64,
    result_bytes: u64,
    result_digest: String,
  },
  Failed {
    duration_ms: f64,
    failure: crate::CapabilityFailure,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationProjection {
  pub identity: OperationIdentity,
  pub node_id: String,
  pub attempt_number: u32,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
  pub status: OperationStatus,
}
