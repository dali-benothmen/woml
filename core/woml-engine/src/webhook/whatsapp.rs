//! Private webhook whatsapp.

use crate::webhook::dispatch_run;
use crate::webhook::has_json_content_type;
use crate::webhook::RunProgressIdentity;
use crate::webhook::WebhookErrorBody;
use crate::webhook::WebhookErrorResponse;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS;
use crate::webhook::COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES;
use crate::webhook::WEBHOOK_MAX_BODY_BYTES;
use crate::DurableEventStore;
use crate::TriggerAdmissionRequest;
use actix_web::http::header;
use actix_web::http::Method;
use actix_web::http::StatusCode;
use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::SecondsFormat;
use chrono::Utc;
use futures_util::StreamExt;
use hmac::Hmac;
use hmac::Mac;
use serde::Deserialize;
use serde_json::Map;
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub(in crate::webhook) struct WhatsAppSubscriber {
  pub(in crate::webhook) workflow_id: String,
  pub(in crate::webhook) definition_hash: String,
  pub(in crate::webhook) trigger_id: String,
}

#[derive(Debug, Deserialize)]
pub(in crate::webhook) struct WhatsAppHandshakeQuery {
  #[serde(rename = "hub.mode")]
  pub(in crate::webhook) mode: Option<String>,
  #[serde(rename = "hub.verify_token")]
  pub(in crate::webhook) verify_token: Option<String>,
  #[serde(rename = "hub.challenge")]
  pub(in crate::webhook) challenge: Option<String>,
}

fn whatsapp_callback_error(
  status: StatusCode,
  code: &'static str,
  message: &'static str,
) -> HttpResponse {
  let mut response = HttpResponse::build(status);
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  response.json(WebhookErrorResponse {
    error: WebhookErrorBody {
      code,
      message,
      issues: None,
    },
  })
}

pub(in crate::webhook) fn whatsapp_signature_valid(
  request: &HttpRequest,
  body: &[u8],
  secret: &str,
) -> bool {
  let Some(signature) = request
    .headers()
    .get("X-Hub-Signature-256")
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("sha256="))
  else {
    return false;
  };
  let Ok(signature) = hex::decode(signature) else {
    return false;
  };
  let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
    return false;
  };
  mac.update(body);
  mac.verify_slice(&signature).is_ok()
}

pub(in crate::webhook) fn whatsapp_decision_payload(
  value: &str,
) -> Option<(crate::ApprovalDecision, &str)> {
  let (decision, raw_capability) = if let Some(capability) = value.strip_prefix("woml_approve:") {
    (crate::ApprovalDecision::Approved, capability)
  } else {
    (
      crate::ApprovalDecision::Rejected,
      value.strip_prefix("woml_reject:")?,
    )
  };
  let capability = raw_capability.strip_prefix("ncap_")?;
  let (id, secret) = capability.split_once('.')?;
  let compact = id.len() == 16 && secret.len() == 32;
  let legacy = id.len() == 32 && secret.len() == 64;
  if (!compact && !legacy)
    || !id
      .bytes()
      .chain(secret.bytes())
      .all(|byte| byte.is_ascii_hexdigit())
  {
    return None;
  }
  Some((decision, raw_capability))
}

pub(in crate::webhook) fn bounded_whatsapp_message_id(value: &str) -> bool {
  !value.is_empty() && value.len() <= 512 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

pub(in crate::webhook) async fn handle_whatsapp_callback(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
) -> HttpResponse {
  if state.whatsapp_subscribers.is_empty() {
    return whatsapp_callback_error(
      StatusCode::NOT_FOUND,
      "WOML_WHATSAPP_CALLBACK_NOT_FOUND",
      "No loaded WOML workflow subscribes to WhatsApp.",
    );
  }
  if request.method() == Method::GET {
    let Ok(query) = serde_urlencoded::from_str::<WhatsAppHandshakeQuery>(request.query_string())
    else {
      return whatsapp_callback_error(
        StatusCode::BAD_REQUEST,
        "WOML_WHATSAPP_HANDSHAKE_INVALID",
        "The WhatsApp callback verification query is invalid.",
      );
    };
    let Some(expected) = state.whatsapp_verify_token.as_deref() else {
      return whatsapp_callback_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "WOML_WHATSAPP_UNAVAILABLE",
        "WhatsApp callback verification is unavailable.",
      );
    };
    let supplied = query.verify_token.as_deref().unwrap_or_default();
    let authorized = supplied.len() == expected.len()
      && bool::from(supplied.as_bytes().ct_eq(expected.as_bytes()));
    if query.mode.as_deref() != Some("subscribe") || !authorized {
      return whatsapp_callback_error(
        StatusCode::FORBIDDEN,
        "WOML_WHATSAPP_HANDSHAKE_REJECTED",
        "WhatsApp callback verification failed.",
      );
    }
    let Some(challenge) = query
      .challenge
      .filter(|value| !value.is_empty() && value.len() <= 512)
    else {
      return whatsapp_callback_error(
        StatusCode::BAD_REQUEST,
        "WOML_WHATSAPP_HANDSHAKE_INVALID",
        "The WhatsApp callback challenge is missing.",
      );
    };
    let mut response = HttpResponse::Ok();
    response.insert_header((header::CACHE_CONTROL, "no-store"));
    return response
      .content_type("text/plain; charset=utf-8")
      .body(challenge);
  }
  if request.method() != Method::POST {
    return whatsapp_callback_error(
      StatusCode::METHOD_NOT_ALLOWED,
      "WOML_WHATSAPP_METHOD_NOT_ALLOWED",
      "The WhatsApp callback accepts GET verification and POST delivery only.",
    );
  }
  if !has_json_content_type(&request) {
    return whatsapp_callback_error(
      StatusCode::BAD_REQUEST,
      "WOML_WHATSAPP_PAYLOAD_INVALID",
      "WhatsApp callback payloads must use application/json.",
    );
  }
  let mut bytes = Vec::new();
  while let Some(chunk) = body.next().await {
    let Ok(chunk) = chunk else {
      return whatsapp_callback_error(
        StatusCode::BAD_REQUEST,
        "WOML_WHATSAPP_PAYLOAD_INVALID",
        "The WhatsApp callback body could not be read.",
      );
    };
    if bytes.len().saturating_add(chunk.len()) > WEBHOOK_MAX_BODY_BYTES {
      return whatsapp_callback_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_WHATSAPP_PAYLOAD_TOO_LARGE",
        "The WhatsApp callback exceeds the 1 MiB limit.",
      );
    }
    bytes.extend_from_slice(&chunk);
  }
  let Some(secret) = state.whatsapp_app_secret.as_deref() else {
    return whatsapp_callback_error(
      StatusCode::SERVICE_UNAVAILABLE,
      "WOML_WHATSAPP_UNAVAILABLE",
      "WhatsApp signature verification is unavailable.",
    );
  };
  if !whatsapp_signature_valid(&request, &bytes, secret) {
    return whatsapp_callback_error(
      StatusCode::UNAUTHORIZED,
      "WOML_WHATSAPP_SIGNATURE_INVALID",
      "WhatsApp callback signature verification failed.",
    );
  }
  let Ok(envelope) = serde_json::from_slice::<Value>(&bytes) else {
    return whatsapp_callback_error(
      StatusCode::BAD_REQUEST,
      "WOML_WHATSAPP_PAYLOAD_INVALID",
      "The WhatsApp callback is not valid JSON.",
    );
  };
  if envelope.get("object").and_then(Value::as_str) != Some("whatsapp_business_account") {
    return whatsapp_callback_error(
      StatusCode::BAD_REQUEST,
      "WOML_WHATSAPP_PAYLOAD_INVALID",
      "The callback is not a WhatsApp Business Account event.",
    );
  }

  let mut accepted = Vec::<(RunProgressIdentity, bool)>::new();
  let Some(entries) = envelope.get("entry").and_then(Value::as_array).cloned() else {
    return whatsapp_callback_error(
      StatusCode::BAD_REQUEST,
      "WOML_WHATSAPP_PAYLOAD_INVALID",
      "The WhatsApp callback entry list is invalid.",
    );
  };
  if entries.len() > COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS {
    return whatsapp_callback_error(
      StatusCode::PAYLOAD_TOO_LARGE,
      "WOML_WHATSAPP_BATCH_TOO_LARGE",
      "The WhatsApp callback contains too many entries.",
    );
  }
  for entry in entries {
    let Some(changes) = entry.get("changes").and_then(Value::as_array).cloned() else {
      return whatsapp_callback_error(
        StatusCode::BAD_REQUEST,
        "WOML_WHATSAPP_PAYLOAD_INVALID",
        "The WhatsApp callback change list is invalid.",
      );
    };
    if changes.len() > COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS {
      return whatsapp_callback_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_WHATSAPP_BATCH_TOO_LARGE",
        "The WhatsApp callback contains too many changes.",
      );
    }
    for change in changes {
      if change.get("field").and_then(Value::as_str) != Some("messages") {
        continue;
      }
      let Some(value) = change.get("value") else {
        continue;
      };
      let Some(phone_number_id) = value
        .get("metadata")
        .and_then(|metadata| metadata.get("phone_number_id"))
        .and_then(Value::as_str)
      else {
        continue;
      };
      let Some(subscribers) = state.whatsapp_subscribers.get(phone_number_id).cloned() else {
        return whatsapp_callback_error(
          StatusCode::NOT_FOUND,
          "WOML_WHATSAPP_PHONE_NOT_FOUND",
          "No loaded WOML trigger is registered for this WhatsApp Phone Number ID.",
        );
      };
      let messages = match value.get("messages") {
        None => Vec::new(),
        Some(Value::Array(messages)) => messages.clone(),
        Some(_) => {
          return whatsapp_callback_error(
            StatusCode::BAD_REQUEST,
            "WOML_WHATSAPP_PAYLOAD_INVALID",
            "The WhatsApp callback message list is invalid.",
          )
        }
      };
      if messages.len() > COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS {
        return whatsapp_callback_error(
          StatusCode::PAYLOAD_TOO_LARGE,
          "WOML_WHATSAPP_BATCH_TOO_LARGE",
          "The WhatsApp callback contains too many messages.",
        );
      }
      for message in messages {
        let Some(message_id) = message.get("id").and_then(Value::as_str) else {
          continue;
        };
        let Some(sender) = message.get("from").and_then(Value::as_str) else {
          continue;
        };
        if !bounded_whatsapp_message_id(message_id)
          || !(8..=16).contains(&sender.len())
          || !sender.bytes().all(|byte| byte.is_ascii_digit())
        {
          continue;
        }
        if let Some(button_payload) = message
          .get("interactive")
          .and_then(|interactive| interactive.get("button_reply"))
          .and_then(|button| button.get("id"))
          .and_then(Value::as_str)
          .or_else(|| {
            message
              .get("button")
              .and_then(|button| button.get("payload"))
              .and_then(Value::as_str)
          })
        {
          if let Some((decision, capability)) = whatsapp_decision_payload(button_payload) {
            let database_path = state.database_path.clone();
            let capability = capability.to_string();
            let actor = format!("whatsapp:{sender}");
            let resolved = web::block(move || {
              let mut store = DurableEventStore::open_ready(database_path)?;
              store.resolve_notification_approval(&capability, &actor, decision, Utc::now())
            })
            .await;
            if !matches!(resolved, Ok(Ok(_))) {
              return whatsapp_callback_error(
                StatusCode::CONFLICT,
                "WOML_WHATSAPP_DECISION_REJECTED",
                "The WhatsApp approval decision is invalid, expired, or already conflicts with the accepted decision.",
              );
            }
            continue;
          }
        }
        let mut payload = Map::new();
        payload.insert("type".to_string(), Value::String("message".to_string()));
        payload.insert(
          "provider".to_string(),
          Value::String("whatsapp".to_string()),
        );
        payload.insert("event".to_string(), Value::String("message".to_string()));
        payload.insert("senderId".to_string(), Value::String(sender.to_string()));
        payload.insert(
          "conversationId".to_string(),
          Value::String(sender.to_string()),
        );
        payload.insert(
          "conversationType".to_string(),
          Value::String("direct".to_string()),
        );
        payload.insert(
          "messageId".to_string(),
          Value::String(message_id.to_string()),
        );
        payload.insert(
          "providerData".to_string(),
          Value::Object(Map::from_iter([(
            "phoneNumberId".to_string(),
            Value::String(phone_number_id.to_string()),
          )])),
        );
        if let Some(timestamp) = message
          .get("timestamp")
          .and_then(Value::as_str)
          .filter(|value| {
            !value.is_empty()
              && value.len() <= 20
              && value.bytes().all(|byte| byte.is_ascii_digit())
          })
        {
          if let Some(occurred_at) = timestamp
            .parse::<i64>()
            .ok()
            .and_then(|seconds| chrono::DateTime::<Utc>::from_timestamp(seconds, 0))
          {
            payload.insert(
              "occurredAt".to_string(),
              Value::String(occurred_at.to_rfc3339_opts(SecondsFormat::Millis, true)),
            );
          }
        }
        let text = message
          .get("text")
          .and_then(|text| text.get("body"))
          .and_then(Value::as_str)
          .or_else(|| {
            message
              .get("interactive")
              .and_then(|interactive| interactive.get("button_reply"))
              .and_then(|button| button.get("title"))
              .and_then(Value::as_str)
          })
          .or_else(|| {
            message
              .get("button")
              .and_then(|button| button.get("text"))
              .and_then(Value::as_str)
          });
        let Some(text) = text.filter(|value| {
          !value.is_empty() && value.len() <= COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES
        }) else {
          continue;
        };
        payload.insert("text".to_string(), Value::String(text.to_string()));
        if let Some(reply_to) = message
          .get("context")
          .and_then(|context| context.get("id"))
          .and_then(Value::as_str)
          .filter(|value| bounded_whatsapp_message_id(value))
        {
          payload.insert(
            "replyToMessageId".to_string(),
            Value::String(reply_to.to_string()),
          );
        }
        for subscriber in subscribers.clone() {
          let admission = TriggerAdmissionRequest {
            workflow_id: subscriber.workflow_id.clone(),
            definition_hash: subscriber.definition_hash.clone(),
            trigger_id: subscriber.trigger_id.clone(),
            trigger_handler: "trigger.whatsapp".to_string(),
            source_identity: format!(
              "whatsapp:{phone_number_id}:{message_id}:{}:{}",
              subscriber.workflow_id, subscriber.trigger_id
            ),
            payload: payload.clone(),
            received_at: Utc::now(),
          };
          let database_path = state.database_path.clone();
          let admitted = web::block(move || {
            let mut store = DurableEventStore::open_ready(database_path)?;
            store.admit_trigger_occurrence(admission)
          })
          .await;
          let Ok(Ok(outcome)) = admitted else {
            return whatsapp_callback_error(
              StatusCode::SERVICE_UNAVAILABLE,
              "WOML_TRIGGER_UNAVAILABLE",
              "The durable WOML trigger authority rejected the WhatsApp event.",
            );
          };
          accepted.push((
            RunProgressIdentity {
              workflow_id: subscriber.workflow_id,
              trigger_id: subscriber.trigger_id,
              trigger_handler: "trigger.whatsapp".to_string(),
              occurrence_id: outcome.occurrence_id,
              run_id: outcome.run_id,
            },
            outcome.duplicate,
          ));
        }
      }
      // Delivery-status callbacks are authenticated and acknowledged. Message
      // acceptance remains authoritative in the operation event that carries
      // the provider message ID; statuses never create workflow runs.
    }
  }
  for (identity, duplicate) in accepted {
    state.report_accepted(&identity, duplicate);
    if !duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
  }
  let mut response = HttpResponse::Ok();
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  response.body("EVENT_RECEIVED")
}
