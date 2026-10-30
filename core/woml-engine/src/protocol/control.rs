//! protocol control.

use super::{SCRIPT_HOST_PROTOCOL, SCRIPT_HOST_PROTOCOL_VERSION};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadyMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub host_instance_id: String,
}

impl ReadyMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "ready"
      || self.host_instance_id.is_empty()
      || self.host_instance_id.chars().count() > 256
    {
      return Err("The child did not send a valid Script Host v12 ready message.".to_string());
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub reason: &'static str,
}

impl<'a> CancelMessage<'a> {
  pub fn parallel_fail_fast(invocation_id: &'a str) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "cancel",
      invocation_id,
      reason: "parallel_fail_fast",
    }
  }

  pub fn run_cancelled(invocation_id: &'a str) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "cancel",
      invocation_id,
      reason: "run_cancelled",
    }
  }
}
