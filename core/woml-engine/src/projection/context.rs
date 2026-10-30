//! projection context.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkflowContext {
  pub trigger: Map<String, Value>,
  pub steps: Map<String, Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub lock: Option<LockContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockContext {
  pub id: String,
  pub key: String,
  pub fencing_token: String,
  pub acquired_at: String,
}
