//! Managed Discord messaging capability.

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler,
};

const DISCORD_API: &str = "https://discord.com/api/v10";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiscordSendRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  bot_token: String,
  conversation_id: String,
  text: String,
  #[serde(default)]
  reply_to_message_id: Option<String>,
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

fn snowflake(value: &str) -> bool {
  (17..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse(input: &Value) -> Result<DiscordSendRequest, CapabilityFailure> {
  let request: DiscordSendRequest = serde_json::from_value(input.clone()).map_err(|_| {
    failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_DISCORD_REQUEST_INVALID",
      "Discord send input does not match Discord Messaging v1.",
      false,
      false,
    )
  })?;
  if request.contract != "woml.discord-message"
    || request.contract_version != 1
    || request.kind != "send"
    || request.bot_token.is_empty()
    || request.bot_token.len() > 512
    || !snowflake(&request.conversation_id)
    || request.text.is_empty()
    || request.text.chars().count() > 2_000
    || request
      .reply_to_message_id
      .as_deref()
      .is_some_and(|value| !snowflake(value))
  {
    return Err(failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_DISCORD_REQUEST_INVALID",
      "Discord send input contains an invalid token, channel, text, or reply identity.",
      false,
      false,
    ));
  }
  Ok(request)
}

fn confirmed_message_identity(
  status: reqwest::StatusCode,
  value: Value,
) -> Result<(String, String), CapabilityFailure> {
  if !status.is_success() {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
      let retry_after_ms = value
        .get("retry_after")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(|seconds| (seconds * 1_000.0).ceil() as u64);
      let mut error = failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_DISCORD_RATE_LIMITED",
        "Discord rate-limited the message request.",
        true,
        false,
      );
      error.details = retry_after_ms.map(|milliseconds| {
        Map::from_iter([("retryAfterMs".to_string(), Value::from(milliseconds))])
      });
      return Err(error);
    }
    return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
      failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_DISCORD_AUTH_FAILED",
        "Discord rejected the configured bot token.",
        false,
        false,
      )
    } else if matches!(
      status,
      reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    ) {
      failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_DISCORD_DESTINATION_INVALID",
        "Discord could not access the configured channel.",
        false,
        false,
      )
    } else {
      failure(
        CapabilityFailureKind::Ambiguous,
        "WOML_DISCORD_DELIVERY_AMBIGUOUS",
        "Discord did not confirm whether the message was created.",
        false,
        true,
      )
    });
  }
  let message_id = value.get("id").and_then(Value::as_str);
  let conversation_id = value.get("channel_id").and_then(Value::as_str);
  match (message_id, conversation_id) {
    (Some(message_id), Some(conversation_id))
      if snowflake(message_id) && snowflake(conversation_id) =>
    {
      Ok((message_id.to_string(), conversation_id.to_string()))
    }
    _ => Err(failure(
      CapabilityFailureKind::Ambiguous,
      "WOML_DISCORD_RESPONSE_INVALID",
      "Discord accepted the request but returned no valid message identity.",
      false,
      true,
    )),
  }
}

#[derive(Debug, Clone)]
pub struct ManagedDiscordHandler {
  client: reqwest::Client,
  api_base: String,
}

impl Default for ManagedDiscordHandler {
  fn default() -> Self {
    Self {
      client: reqwest::Client::new(),
      api_base: DISCORD_API.to_string(),
    }
  }
}

impl ManagedDiscordHandler {
  pub fn new(client: reqwest::Client) -> Self {
    Self {
      client,
      api_base: DISCORD_API.to_string(),
    }
  }
}

impl CapabilityHandler for ManagedDiscordHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "discord".to_string(),
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
      ("provider".to_string(), Value::String("discord".to_string())),
      (
        "conversationId".to_string(),
        Value::String(request.conversation_id),
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
      let url = format!("{api_base}/channels/{}/messages", request.conversation_id);
      let mut body = json!({ "content": request.text });
      if let Some(reply) = request.reply_to_message_id {
        body["message_reference"] = json!({ "message_id": reply });
      }
      let response = tokio::select! {
        _ = cancellation.cancelled() => {
          return Err(failure(
            CapabilityFailureKind::Cancelled,
            "WOML_DISCORD_CANCELLED",
            "Discord messaging was cancelled before confirmation.",
            false,
            true,
          ));
        }
        response = client
          .post(url)
          .header(reqwest::header::AUTHORIZATION, format!("Bot {}", request.bot_token))
          .json(&body)
          .send() => response.map_err(|_| failure(
            CapabilityFailureKind::Ambiguous,
            "WOML_DISCORD_DELIVERY_AMBIGUOUS",
            "The Discord connection ended without confirming whether the message was created.",
            false,
            true,
          ))?,
      };
      let status = response.status();
      let value: Value = tokio::select! {
        _ = cancellation.cancelled() => {
          return Err(failure(
            CapabilityFailureKind::Cancelled,
            "WOML_DISCORD_CANCELLED",
            "Discord messaging was cancelled before confirmation.",
            false,
            true,
          ));
        }
        value = response.json() => value.map_err(|_| failure(
          CapabilityFailureKind::Ambiguous,
          "WOML_DISCORD_RESPONSE_INVALID",
          "Discord returned an invalid response after the send request.",
          false,
          true,
        ))?,
      };
      let (message_id, conversation_id) = confirmed_message_identity(status, value)?;
      Ok(json!({
        "provider": "discord",
        "conversationId": conversation_id,
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
  fn validates_requests_and_accepts_only_confirmed_snowflake_identities() {
    parse(&json!({
      "contract": "woml.discord-message",
      "contractVersion": 1,
      "kind": "send",
      "botToken": "discord-token",
      "conversationId": "200000000000000001",
      "text": "Hello from WOML"
    }))
    .expect("valid Discord request");
    let identity = confirmed_message_identity(
      reqwest::StatusCode::OK,
      json!({
        "id": "200000000000000002",
        "channel_id": "200000000000000001"
      }),
    )
    .expect("confirmed Discord identity");
    assert_eq!(
      identity,
      (
        "200000000000000002".to_string(),
        "200000000000000001".to_string()
      )
    );
  }

  #[test]
  fn rate_limits_are_retryable_and_uncertain_failures_are_ambiguous() {
    let limited = confirmed_message_identity(
      reqwest::StatusCode::TOO_MANY_REQUESTS,
      json!({ "retry_after": 1.25 }),
    )
    .expect_err("rate limit");
    assert!(limited.retryable);
    assert!(!limited.ambiguous);
    assert_eq!(
      limited
        .details
        .and_then(|details| details.get("retryAfterMs").cloned()),
      Some(json!(1250))
    );

    let uncertain = confirmed_message_identity(
      reqwest::StatusCode::BAD_GATEWAY,
      json!({ "message": "upstream" }),
    )
    .expect_err("ambiguous send");
    assert!(uncertain.ambiguous);
    assert!(!uncertain.retryable);
  }
}
