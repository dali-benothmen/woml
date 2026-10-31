//! protocol modules.

use super::{SCRIPT_HOST_PROTOCOL, SCRIPT_HOST_PROTOCOL_VERSION};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeModuleBinding {
  pub name: String,
  pub bundle_digest: String,
  pub exports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterModuleMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub bundle_digest: &'a str,
  pub bundle: &'a str,
  pub source_map_digest: &'a str,
  pub source_map: &'a str,
}

impl<'a> RegisterModuleMessage<'a> {
  pub fn new(
    bundle_digest: &'a str,
    bundle: &'a str,
    source_map_digest: &'a str,
    source_map: &'a str,
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "register_module",
      bundle_digest,
      bundle,
      source_map_digest,
      source_map,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleRegisteredMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub bundle_digest: String,
  pub source_map_digest: String,
  pub accepted: bool,
  #[serde(default)]
  pub code: Option<String>,
  #[serde(default)]
  pub message: Option<String>,
}

impl ModuleRegisteredMessage {
  pub fn validate(
    &self,
    expected_digest: &str,
    expected_source_map_digest: &str,
  ) -> Result<(), String> {
    let envelope = self.protocol == SCRIPT_HOST_PROTOCOL
      && self.protocol_version == SCRIPT_HOST_PROTOCOL_VERSION
      && self.message_type == "module_registered"
      && self.bundle_digest == expected_digest
      && self.source_map_digest == expected_source_map_digest;
    let outcome = if self.accepted {
      self.code.is_none() && self.message.is_none()
    } else {
      matches!(
        self.code.as_deref(),
        Some("WOML_MODULE_DIGEST_MISMATCH" | "WOML_MODULE_CACHE_LIMIT_EXCEEDED")
      ) && self
        .message
        .as_ref()
        .is_some_and(|message| !message.is_empty())
    };
    if envelope && outcome {
      Ok(())
    } else {
      Err("The child sent an invalid Script Host v12 module registration response.".to_string())
    }
  }
}
