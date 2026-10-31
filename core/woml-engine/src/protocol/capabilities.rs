//! protocol capabilities.

use super::{SCRIPT_HOST_PROTOCOL, SCRIPT_HOST_PROTOCOL_VERSION};
use crate::CapabilityCallRequest;
use crate::CapabilityCallResult;
use crate::CapabilityFailure;
use crate::NativeFetchObservation;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub call: CapabilityCallRequest,
}

impl CapabilityCallMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "capability_call"
      || self.invocation_id != self.call.invocation_id
      || self.call_id != self.call.call_id
    {
      return Err("The child sent an invalid script-host v4 capability call envelope.".to_string());
    }
    self.call.validate().map_err(|error| error.message)
  }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityResultMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub call_id: &'a str,
  pub result: &'a CapabilityCallResult,
}

impl<'a> CapabilityResultMessage<'a> {
  pub fn new(result: &'a CapabilityCallResult) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "capability_result",
      invocation_id: result.invocation_id(),
      call_id: result.call_id(),
      result,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchObservationMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub request_id: String,
  pub observation: NativeFetchObservation,
}

impl FetchObservationMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != SCRIPT_HOST_PROTOCOL
      || self.protocol_version != SCRIPT_HOST_PROTOCOL_VERSION
      || self.message_type != "fetch_observation"
      || self.invocation_id != self.observation.invocation_id()
      || self.request_id != self.observation.request_id()
    {
      return Err("The child sent an invalid native-Fetch observation envelope.".to_string());
    }
    self.observation.validate()
  }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchObservationAckMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub request_id: &'a str,
  pub accepted: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<&'a CapabilityFailure>,
}

impl<'a> FetchObservationAckMessage<'a> {
  pub fn accepted(invocation_id: &'a str, request_id: &'a str) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "fetch_observation_ack",
      invocation_id,
      request_id,
      accepted: true,
      error: None,
    }
  }

  pub fn rejected(
    invocation_id: &'a str,
    request_id: &'a str,
    error: &'a CapabilityFailure,
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "fetch_observation_ack",
      invocation_id,
      request_id,
      accepted: false,
      error: Some(error),
    }
  }
}
