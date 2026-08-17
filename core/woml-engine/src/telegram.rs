//! Managed Telegram messaging capability.

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler,
};

const TELEGRAM_API: &str = "https://api.telegram.org";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TelegramSendRequest {
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

fn positive_integer(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 20
    && !value.starts_with('0')
    && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse(input: &Value) -> Result<TelegramSendRequest, CapabilityFailure> {
  let request: TelegramSendRequest = serde_json::from_value(input.clone()).map_err(|_| {
    failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_TELEGRAM_REQUEST_INVALID",
      "Telegram send input does not match Telegram Messaging v1.",
      false,
      false,
    )
  })?;
  let chat = request
    .conversation_id
    .strip_prefix('-')
    .unwrap_or(&request.conversation_id);
  if request.contract != "woml.telegram-message"
    || request.contract_version != 1
    || request.kind != "send"
    || request.bot_token.is_empty()
    || request.bot_token.len() > 512
    || !positive_integer(chat)
    || request.text.is_empty()
    || request.text.chars().count() > 4096
    || request
      .reply_to_message_id
      .as_deref()
      .is_some_and(|value| !positive_integer(value))
  {
    return Err(failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_TELEGRAM_REQUEST_INVALID",
      "Telegram send input contains an invalid token, chat, text, or reply identity.",
      false,
      false,
    ));
  }
  Ok(request)
}

#[derive(Debug, Deserialize)]
struct TelegramEnvelope {
  ok: bool,
  #[serde(default)]
  result: Option<Value>,
  #[serde(default)]
  error_code: Option<u16>,
  #[serde(default)]
  parameters: Option<TelegramParameters>,
}

#[derive(Debug, Deserialize)]
struct TelegramParameters {
  #[serde(default)]
  retry_after: Option<u64>,
}

fn confirmed_message_identity(
  status: reqwest::StatusCode,
  envelope: TelegramEnvelope,
) -> Result<(String, String), CapabilityFailure> {
  if !status.is_success() || !envelope.ok {
    let code = envelope.error_code.unwrap_or(status.as_u16());
    if code == 429 {
      let retry_after_ms = envelope
        .parameters
        .and_then(|parameters| parameters.retry_after)
        .and_then(|seconds| seconds.checked_mul(1_000));
      let mut error = failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_TELEGRAM_RATE_LIMITED",
        "Telegram rate-limited the message request.",
        true,
        false,
      );
      error.details = retry_after_ms.map(|milliseconds| {
        Map::from_iter([("retryAfterMs".to_string(), Value::from(milliseconds))])
      });
      return Err(error);
    }
    return Err(if code == 401 {
      failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_TELEGRAM_AUTH_FAILED",
        "Telegram rejected the configured bot token.",
        false,
        false,
      )
    } else if matches!(code, 400 | 403) {
      failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_TELEGRAM_DESTINATION_INVALID",
        "Telegram could not access the configured chat.",
        false,
        false,
      )
    } else {
      failure(
        CapabilityFailureKind::Ambiguous,
        "WOML_TELEGRAM_DELIVERY_AMBIGUOUS",
        "Telegram did not confirm whether the message was created.",
        false,
        true,
      )
    });
  }
  envelope
    .result
    .and_then(|value| {
      let message_id = value.get("message_id")?.as_i64()?.to_string();
      let conversation_id = value.get("chat")?.get("id")?.as_i64()?.to_string();
      Some((message_id, conversation_id))
    })
    .ok_or_else(|| {
      failure(
        CapabilityFailureKind::Ambiguous,
        "WOML_TELEGRAM_RESPONSE_INVALID",
        "Telegram accepted the request but returned no valid message identity.",
        false,
        true,
      )
    })
}

#[derive(Debug, Clone)]
pub struct ManagedTelegramHandler {
  client: reqwest::Client,
  api_base: String,
}

impl Default for ManagedTelegramHandler {
  fn default() -> Self {
    Self {
      client: reqwest::Client::new(),
      api_base: TELEGRAM_API.to_string(),
    }
  }
}

impl ManagedTelegramHandler {
  pub fn new(client: reqwest::Client) -> Self {
    Self {
      client,
      api_base: TELEGRAM_API.to_string(),
    }
  }
}

impl CapabilityHandler for ManagedTelegramHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "telegram".to_string(),
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
        Value::String("telegram".to_string()),
      ),
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
      let url = format!("{api_base}/bot{}/sendMessage", request.bot_token);
      let mut body = json!({
        "chat_id": request.conversation_id,
        "text": request.text,
      });
      if let Some(reply) = request.reply_to_message_id {
        body["reply_parameters"] = json!({
          "message_id": reply.parse::<u64>().expect("validated message ID")
        });
      }
      let response = tokio::select! {
        _ = cancellation.cancelled() => {
          return Err(failure(
            CapabilityFailureKind::Cancelled,
            "WOML_TELEGRAM_CANCELLED",
            "Telegram messaging was cancelled before confirmation.",
            false,
            true,
          ));
        }
        response = client.post(url).json(&body).send() => response.map_err(|_| failure(
          CapabilityFailureKind::Ambiguous,
          "WOML_TELEGRAM_DELIVERY_AMBIGUOUS",
          "The Telegram connection ended without confirming whether the message was created.",
          false,
          true,
        ))?,
      };
      let status = response.status();
      let envelope: TelegramEnvelope = tokio::select! {
        _ = cancellation.cancelled() => {
          return Err(failure(
            CapabilityFailureKind::Cancelled,
            "WOML_TELEGRAM_CANCELLED",
            "Telegram messaging was cancelled before confirmation.",
            false,
            true,
          ));
        }
        envelope = response.json() => envelope.map_err(|_| {
          failure(
            CapabilityFailureKind::Ambiguous,
            "WOML_TELEGRAM_RESPONSE_INVALID",
            "Telegram returned an invalid response after the send request.",
            false,
            true,
          )
        })?,
      };
      let (message_id, conversation_id) = confirmed_message_identity(status, envelope)?;
      Ok(json!({
        "provider": "telegram",
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
  use crate::capability::CapabilityIdentityMode;
  use crate::{CapabilityCallIdentity, CapabilityCallLimits};

  fn request(input: Value) -> CapabilityCallRequest {
    CapabilityCallRequest {
      contract: "woml.capability-call".to_string(),
      contract_version: 1,
      message_type: "request".to_string(),
      invocation_id: "capinv_telegram_test".to_string(),
      call_id: "call_telegram_test".to_string(),
      run_id: "run_telegram_test".to_string(),
      node_id: "sendMessage".to_string(),
      attempt_number: 1,
      capability: "telegram".to_string(),
      operation: "send".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      identity: CapabilityCallIdentity {
        mode: CapabilityIdentityMode::Named,
        step_idempotency_key:
          "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        operation_name: "telegramMessage".to_string(),
        operation_key: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
          .to_string(),
        provider_idempotency_key: None,
      },
      limits: CapabilityCallLimits::default(),
      input,
    }
  }

  fn input() -> Value {
    json!({
      "contract": "woml.telegram-message",
      "contractVersion": 1,
      "kind": "send",
      "botToken": "123456789:test-token",
      "conversationId": "-1001234567890",
      "text": "Hello from WOML"
    })
  }

  #[test]
  fn accepts_only_a_confirmed_public_message_identity() {
    let identity = confirmed_message_identity(
      reqwest::StatusCode::OK,
      TelegramEnvelope {
        ok: true,
        result: Some(json!({
          "message_id": 42,
          "chat": { "id": -1001234567890_i64 }
        })),
        error_code: None,
        parameters: None,
      },
    )
    .expect("confirmed identity");
    assert_eq!(identity, ("42".to_string(), "-1001234567890".to_string()));

    let error = confirmed_message_identity(
      reqwest::StatusCode::TOO_MANY_REQUESTS,
      TelegramEnvelope {
        ok: false,
        result: None,
        error_code: Some(429),
        parameters: Some(TelegramParameters {
          retry_after: Some(3),
        }),
      },
    )
    .expect_err("rate limit is not a confirmed delivery");
    assert_eq!(error.code, "WOML_TELEGRAM_RATE_LIMITED");
    assert_eq!(
      error.details,
      Some(Map::from_iter([(
        "retryAfterMs".to_string(),
        Value::from(3_000_u64),
      )]))
    );
  }

  #[test]
  fn validation_and_safe_metadata_never_expose_the_bot_token() {
    let handler = ManagedTelegramHandler::default();
    let valid_request = request(input());
    handler
      .validate_request(&valid_request)
      .expect("valid request");
    let metadata = handler.safe_metadata(&valid_request.input);
    assert_eq!(metadata.get("provider"), Some(&json!("telegram")));
    assert_eq!(
      metadata.get("conversationId"),
      Some(&json!("-1001234567890"))
    );
    assert!(!serde_json::to_string(&metadata)
      .expect("metadata JSON")
      .contains("test-token"));

    let invalid = request(json!({
      "contract": "woml.telegram-message",
      "contractVersion": 1,
      "kind": "send",
      "botToken": "secret",
      "conversationId": "chat-name",
      "text": "Hello"
    }));
    let failure = handler
      .validate_request(&invalid)
      .expect_err("named chat is rejected");
    assert_eq!(failure.code, "WOML_TELEGRAM_REQUEST_INVALID");
  }
}
