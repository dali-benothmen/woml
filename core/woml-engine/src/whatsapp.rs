//! Managed WhatsApp Cloud API messaging capability.

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler,
};

const WHATSAPP_GRAPH_API: &str = "https://graph.facebook.com/v23.0";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WhatsAppTemplate {
  name: String,
  language: String,
  parameters: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WhatsAppSendRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  access_token: String,
  phone_number_id: String,
  conversation_id: String,
  template: WhatsAppTemplate,
}

fn failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable,
    ambiguous,
    details: None,
  }
}

fn digits(value: &str, minimum: usize, maximum: usize) -> bool {
  (minimum..=maximum).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_template_name(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 512
    && value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_lowercase())
    && value
      .bytes()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_language(value: &str) -> bool {
  let bytes = value.as_bytes();
  (bytes.len() == 2 || bytes.len() == 3) && bytes.iter().all(u8::is_ascii_lowercase)
    || ((bytes.len() == 5 || bytes.len() == 6)
      && bytes[0..bytes.len() - 3].iter().all(u8::is_ascii_lowercase)
      && bytes[bytes.len() - 3] == b'_'
      && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_uppercase))
}

fn parse(input: &Value) -> Result<WhatsAppSendRequest, CapabilityFailure> {
  let request: WhatsAppSendRequest = serde_json::from_value(input.clone()).map_err(|_| {
    failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_WHATSAPP_REQUEST_INVALID",
      "WhatsApp send input does not match WhatsApp Messaging v1.",
      false,
      false,
    )
  })?;
  if request.contract != "woml.whatsapp-message"
    || request.contract_version != 1
    || request.kind != "send"
    || request.access_token.is_empty()
    || request.access_token.len() > 2048
    || !digits(&request.phone_number_id, 6, 32)
    || !digits(&request.conversation_id, 8, 16)
    || !valid_template_name(&request.template.name)
    || !valid_language(&request.template.language)
    || request.template.parameters.len() > 32
    || request
      .template
      .parameters
      .iter()
      .any(|value| value.chars().count() > 1024)
  {
    return Err(failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_WHATSAPP_REQUEST_INVALID",
      "WhatsApp send input contains an invalid credential, destination, or template.",
      false,
      false,
    ));
  }
  Ok(request)
}

fn confirmed_message_identity(
  status: reqwest::StatusCode,
  value: Value,
) -> Result<String, CapabilityFailure> {
  if !status.is_success() {
    let meta_code = value
      .get("error")
      .and_then(|error| error.get("code"))
      .and_then(Value::as_i64);
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || meta_code == Some(4) {
      return Err(failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_WHATSAPP_RATE_LIMITED",
        "WhatsApp rate-limited the message request.",
        true,
        false,
      ));
    }
    if matches!(status.as_u16(), 401 | 403) || meta_code == Some(190) {
      return Err(failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_WHATSAPP_AUTH_FAILED",
        "Meta rejected the configured WhatsApp access token or permission.",
        false,
        false,
      ));
    }
    if status == reqwest::StatusCode::BAD_REQUEST {
      return Err(failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_WHATSAPP_REQUEST_REJECTED",
        "WhatsApp rejected the recipient or approved-template request.",
        false,
        false,
      ));
    }
    return Err(failure(
      CapabilityFailureKind::Ambiguous,
      "WOML_WHATSAPP_DELIVERY_AMBIGUOUS",
      "Meta did not confirm whether the WhatsApp message was accepted.",
      false,
      true,
    ));
  }
  value
    .get("messages")
    .and_then(Value::as_array)
    .and_then(|messages| messages.first())
    .and_then(|message| message.get("id"))
    .and_then(Value::as_str)
    .filter(|message_id| !message_id.is_empty() && message_id.len() <= 512)
    .map(str::to_string)
    .ok_or_else(|| {
      failure(
        CapabilityFailureKind::Ambiguous,
        "WOML_WHATSAPP_RESPONSE_INVALID",
        "Meta accepted the request but returned no valid message identity.",
        false,
        true,
      )
    })
}

#[derive(Debug, Clone)]
pub struct ManagedWhatsAppHandler {
  client: reqwest::Client,
  api_base: String,
}

impl Default for ManagedWhatsAppHandler {
  fn default() -> Self {
    Self {
      client: reqwest::Client::new(),
      api_base: WHATSAPP_GRAPH_API.to_string(),
    }
  }
}

impl ManagedWhatsAppHandler {
  pub fn new(client: reqwest::Client) -> Self {
    Self {
      client,
      api_base: WHATSAPP_GRAPH_API.to_string(),
    }
  }
}

impl CapabilityHandler for ManagedWhatsAppHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "whatsapp".to_string(),
      operation: "send".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse(&request.input).map(|_| ())
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    let Ok(request) = parse(input) else {
      return Map::new();
    };
    Map::from_iter([
      (
        "provider".to_string(),
        Value::String("whatsapp".to_string()),
      ),
      (
        "conversationId".to_string(),
        Value::String(request.conversation_id),
      ),
      (
        "templateName".to_string(),
        Value::String(request.template.name),
      ),
    ])
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    result
      .get("messageId")
      .and_then(Value::as_str)
      .map(|message_id| {
        Map::from_iter([(
          "providerMessageId".to_string(),
          Value::String(message_id.to_string()),
        )])
      })
      .unwrap_or_default()
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = match parse(&input) {
      Ok(request) => request,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let client = self.client.clone();
    let api_base = self.api_base.clone();
    Box::pin(async move {
      let url = format!("{api_base}/{}/messages", request.phone_number_id);
      let parameters = request
        .template
        .parameters
        .iter()
        .map(|text| json!({ "type": "text", "text": text }))
        .collect::<Vec<_>>();
      let body = json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": request.conversation_id,
        "type": "template",
        "template": {
          "name": request.template.name,
          "language": { "code": request.template.language },
          "components": [{ "type": "body", "parameters": parameters }]
        }
      });
      let response = tokio::select! {
        _ = cancellation.cancelled() => {
          return Err(failure(
            CapabilityFailureKind::Cancelled,
            "WOML_WHATSAPP_CANCELLED",
            "WhatsApp messaging was cancelled before confirmation.",
            false,
            true,
          ));
        }
        response = client
          .post(url)
          .bearer_auth(request.access_token)
          .json(&body)
          .send() => response.map_err(|_| failure(
            CapabilityFailureKind::Ambiguous,
            "WOML_WHATSAPP_DELIVERY_AMBIGUOUS",
            "The WhatsApp request ended before Meta confirmed delivery.",
            false,
            true,
          ))?,
      };
      let status = response.status();
      let value = response.json::<Value>().await.map_err(|_| {
        failure(
          CapabilityFailureKind::Ambiguous,
          "WOML_WHATSAPP_RESPONSE_INVALID",
          "Meta returned an invalid WhatsApp response.",
          false,
          true,
        )
      })?;
      let message_id = confirmed_message_identity(status, value)?;
      Ok(json!({
        "provider": "whatsapp",
        "conversationId": request.conversation_id,
        "messageId": message_id,
        "acceptedAt": chrono::Utc::now().to_rfc3339(),
      }))
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn validates_the_frozen_template_shape_without_exposing_the_token() {
    let input = json!({
      "contract": "woml.whatsapp-message",
      "contractVersion": 1,
      "kind": "send",
      "accessToken": "secret-access-token",
      "phoneNumberId": "123456789012345",
      "conversationId": "15551234567",
      "template": {
        "name": "build_completed",
        "language": "en_US",
        "parameters": ["build-42"]
      }
    });
    let request = parse(&input).expect("valid WhatsApp request");
    assert_eq!(request.template.name, "build_completed");
    let metadata = ManagedWhatsAppHandler::default().safe_metadata(&input);
    assert_eq!(metadata.get("provider"), Some(&json!("whatsapp")));
    assert!(!metadata
      .values()
      .any(|value| value == "secret-access-token"));
  }
}
