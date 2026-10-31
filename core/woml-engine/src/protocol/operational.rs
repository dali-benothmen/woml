//! protocol operational.

use super::{SCRIPT_HOST_PROTOCOL, SCRIPT_HOST_PROTOCOL_VERSION};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptLogLevelV1 {
  Debug,
  Info,
  Warn,
  Error,
}

impl From<ScriptLogLevelV1> for crate::OperationalLogLevelV1 {
  fn from(value: ScriptLogLevelV1) -> Self {
    match value {
      ScriptLogLevelV1::Debug => Self::Debug,
      ScriptLogLevelV1::Info => Self::Info,
      ScriptLogLevelV1::Warn => Self::Warn,
      ScriptLogLevelV1::Error => Self::Error,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptLogMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub request_id: String,
  pub frame_sequence: u64,
  pub level: ScriptLogLevelV1,
  pub text: String,
}

impl ScriptLogMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "script_log"
      || !valid_operational_request_id(&self.request_id)
      || self.frame_sequence == 0
      || self.text.len() > 4096
      || self.text.contains('\0')
    {
      return Err("The child sent an invalid Script Host v12 script-log frame.".to_string());
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgressReportMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub request_id: String,
  pub frame_sequence: u64,
  pub progress_id: String,
  pub current: Option<f64>,
  pub total: Option<f64>,
  pub unit: Option<String>,
  pub message: Option<String>,
}

impl ProgressReportMessage {
  pub fn validate(&self) -> Result<(), String> {
    let valid = self.protocol == SCRIPT_HOST_PROTOCOL
      && self.protocol_version == SCRIPT_HOST_PROTOCOL_VERSION
      && self.message_type == "progress_report"
      && valid_operational_request_id(&self.request_id)
      && self.frame_sequence > 0
      && !self.progress_id.is_empty()
      && self.progress_id.len() <= 128
      && !self.progress_id.chars().any(char::is_whitespace)
      && (self.current.is_some() || self.message.is_some())
      && self
        .current
        .is_none_or(|value| value.is_finite() && (0.0..=9_007_199_254_740_991.0).contains(&value))
      && self.total.is_none_or(|value| {
        value.is_finite()
          && value > 0.0
          && value <= 9_007_199_254_740_991.0
          && self.current.is_none_or(|current| current <= value)
      })
      && self.unit.as_ref().is_none_or(|unit| {
        self.current.is_some() && !unit.is_empty() && unit.len() <= 32 && !unit.contains('\0')
      })
      && self.message.as_ref().is_none_or(|message| {
        !message.is_empty() && message.len() <= 512 && !message.contains('\0')
      });
    if valid {
      Ok(())
    } else {
      Err("The child sent an invalid Script Host v12 progress frame.".to_string())
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressAckMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub request_id: &'a str,
  pub frame_sequence: u64,
  pub accepted: bool,
}

impl<'a> ProgressAckMessage<'a> {
  pub fn accepted(request_id: &'a str, frame_sequence: u64) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "progress_ack",
      request_id,
      frame_sequence,
      accepted: true,
    }
  }
}

fn valid_operational_request_id(value: &str) -> bool {
  !value.is_empty() && value.len() <= 320 && !value.chars().any(char::is_whitespace)
}
