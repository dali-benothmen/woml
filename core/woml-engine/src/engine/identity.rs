//! engine identity.

use sha2::Digest;
use sha2::Sha256;

pub fn step_effect_idempotency_key(run_id: &str, definition_hash: &str, node_id: &str) -> String {
  let run_id = serde_json::to_string(run_id).expect("serializing a string cannot fail");
  let definition_hash =
    serde_json::to_string(definition_hash).expect("serializing a string cannot fail");
  let node_id = serde_json::to_string(node_id).expect("serializing a string cannot fail");
  let canonical = format!(
    "{{\"contract\":\"woml.step-effect\",\"definitionHash\":{definition_hash},\"nodeId\":{node_id},\"runId\":{run_id},\"version\":1}}"
  );
  format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}
