//! event remote.

use serde::Deserialize;
use serde::Serialize;

/// A workflow-visible fact emitted only after the coordinator commits the
/// corresponding authoritative Store v20 transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteDispatchTransitionedData {
  pub dispatch_id: String,
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub target: String,
  pub state: crate::RemoteDispatchStateV1,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub worker_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub terminal_digest: Option<String>,
}
