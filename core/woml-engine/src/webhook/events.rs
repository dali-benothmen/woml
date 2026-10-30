//! Private webhook events.

use crate::webhook::authorized_bearer;
use crate::webhook::dispatch_run;
use crate::webhook::error_response;
use crate::webhook::has_json_content_type;
use crate::webhook::inbound_trace_context;
use crate::webhook::schema_issue;
use crate::webhook::valid_event_name;
use crate::webhook::RunProgressIdentity;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::WebhookSchemaIssue;
use crate::webhook::WEBHOOK_MAX_BODY_BYTES;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::TriggerAdmissionRequest;
use actix_web::http::header;
use actix_web::http::Method;
use actix_web::http::StatusCode;
use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::Utc;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all_fields = "camelCase")]
pub(in crate::webhook) enum EventDeliveryResponse {
  #[serde(rename = "accepted")]
  Accepted {
    workflow_id: String,
    trigger_id: String,
    run_id: String,
    duplicate: bool,
  },
  #[serde(rename = "rejected")]
  Rejected {
    workflow_id: String,
    trigger_id: String,
    code: &'static str,
    message: &'static str,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    issues: Option<Vec<WebhookSchemaIssue>>,
  },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::webhook) struct EventPublishedResponse {
  pub(in crate::webhook) event_id: String,
  pub(in crate::webhook) event_name: String,
  pub(in crate::webhook) status: &'static str,
  pub(in crate::webhook) deliveries: Vec<EventDeliveryResponse>,
}

pub(in crate::webhook) async fn handle_event_publication(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
  event_name: String,
) -> HttpResponse {
  let inbound_trace_context = inbound_trace_context(&request);
  if request.method() != Method::POST {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::METHOD_NOT_ALLOWED,
      "WOML_EVENT_METHOD_NOT_ALLOWED",
      "The WOML event publisher accepts POST requests only.",
    );
  }
  if !valid_event_name(&event_name) {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::BAD_REQUEST,
      "WOML_EVENT_NAME_INVALID",
      "Event name is invalid.",
    );
  }
  let Some(subscribers) = state.event_subscribers.get(&event_name).cloned() else {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::NOT_FOUND,
      "WOML_EVENT_NOT_FOUND",
      "No loaded WOML workflow subscribes to this event name.",
    );
  };
  let Some(token_digest) = state.event_token_digests.get(&event_name) else {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::SERVICE_UNAVAILABLE,
      "WOML_EVENT_UNAVAILABLE",
      "The WOML event publisher is unavailable.",
    );
  };
  if !authorized_bearer(&request, token_digest) {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::UNAUTHORIZED,
      "WOML_EVENT_UNAUTHORIZED",
      "Event publisher authentication failed.",
    );
  }
  let event_id = match request
    .headers()
    .get("Event-ID")
    .and_then(|value| value.to_str().ok())
  {
    Some(value) if valid_event_id(value) => value.to_string(),
    _ => {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::BAD_REQUEST,
        "WOML_EVENT_ID_INVALID",
        "Event-ID must contain 1 to 256 URL-safe identifier characters.",
      )
    }
  };
  if !has_json_content_type(&request) {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::BAD_REQUEST,
      "WOML_EVENT_PAYLOAD_INVALID",
      "Event payload must use application/json.",
    );
  }
  if request
    .headers()
    .get(header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
    .is_some_and(|length| length > WEBHOOK_MAX_BODY_BYTES as u64)
  {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::PAYLOAD_TOO_LARGE,
      "WOML_EVENT_PAYLOAD_TOO_LARGE",
      "Event payload exceeds the 1 MiB limit.",
    );
  }

  let mut bytes = Vec::new();
  while let Some(chunk) = body.next().await {
    let Ok(chunk) = chunk else {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::BAD_REQUEST,
        "WOML_EVENT_PAYLOAD_INVALID",
        "Event payload must be a valid JSON object.",
      );
    };
    if bytes.len().saturating_add(chunk.len()) > WEBHOOK_MAX_BODY_BYTES {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_EVENT_PAYLOAD_TOO_LARGE",
        "Event payload exceeds the 1 MiB limit.",
      );
    }
    bytes.extend_from_slice(&chunk);
  }
  let payload = match serde_json::from_slice::<Value>(&bytes) {
    Ok(Value::Object(payload)) => payload,
    _ => {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::BAD_REQUEST,
        "WOML_EVENT_PAYLOAD_INVALID",
        "Event payload must be a valid JSON object.",
      )
    }
  };

  let received_at = Utc::now();
  let mut deliveries = Vec::with_capacity(subscribers.len());
  for subscriber in subscribers {
    if let Some(validator) = &subscriber.schema {
      let value = Value::Object(payload.clone());
      let mut issues = validator
        .iter_errors(&value)
        .take(100)
        .map(schema_issue)
        .collect::<Vec<_>>();
      if !issues.is_empty() {
        issues.sort_by(|left, right| {
          left
            .path
            .cmp(&right.path)
            .then(left.message.cmp(right.message))
        });
        let code = "WOML_TRIGGER_SCHEMA_INVALID";
        let message = "Event payload does not match this subscriber schema.";
        state.report_event_rejected(Some(&subscriber), code, message);
        deliveries.push(EventDeliveryResponse::Rejected {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          code,
          message,
          retryable: false,
          issues: Some(issues),
        });
        continue;
      }
    }

    let admission = TriggerAdmissionRequest {
      workflow_id: subscriber.workflow_id.clone(),
      definition_hash: subscriber.definition_hash.clone(),
      trigger_id: subscriber.trigger_id.clone(),
      trigger_handler: "trigger.event".to_string(),
      source_identity: event_source_identity(
        &event_id,
        &subscriber.workflow_id,
        &subscriber.trigger_id,
      ),
      payload: payload.clone(),
      received_at,
    };
    let database_path = state.database_path.clone();
    let admitted = web::block(move || {
      let mut store = DurableEventStore::open_ready(database_path)?;
      store.admit_trigger_occurrence(admission)
    })
    .await;
    match admitted {
      Ok(Ok(outcome)) => {
        let identity = RunProgressIdentity {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          trigger_handler: "trigger.event".to_string(),
          occurrence_id: outcome.occurrence_id,
          run_id: outcome.run_id.clone(),
        };
        state.execution.report_telemetry_admission_context(
          &outcome.run_id,
          inbound_trace_context.clone(),
          received_at,
        );
        state.report_accepted(&identity, outcome.duplicate);
        if !outcome.duplicate {
          state.report_run_started(&identity);
          dispatch_run(state.get_ref(), identity);
        }
        deliveries.push(EventDeliveryResponse::Accepted {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          run_id: outcome.run_id,
          duplicate: outcome.duplicate,
        });
      }
      Ok(Err(error)) => {
        let (code, message, retryable) = event_admission_failure(&error);
        state.report_event_rejected(Some(&subscriber), code, message);
        deliveries.push(EventDeliveryResponse::Rejected {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          code,
          message,
          retryable,
          issues: None,
        });
      }
      Err(_) => {
        let code = "WOML_TRIGGER_UNAVAILABLE";
        let message = "The durable WOML trigger authority is unavailable.";
        state.report_event_rejected(Some(&subscriber), code, message);
        deliveries.push(EventDeliveryResponse::Rejected {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          code,
          message,
          retryable: true,
          issues: None,
        });
      }
    }
  }

  let accepted = deliveries
    .iter()
    .filter(|delivery| matches!(delivery, EventDeliveryResponse::Accepted { .. }))
    .count();
  let status = if accepted == deliveries.len() {
    "accepted"
  } else if accepted == 0 {
    "rejected"
  } else {
    "partial"
  };
  let mut response = HttpResponse::Ok();
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  response.json(EventPublishedResponse {
    event_id,
    event_name,
    status,
    deliveries,
  })
}

fn valid_event_id(value: &str) -> bool {
  let bytes = value.as_bytes();
  !bytes.is_empty()
    && bytes.len() <= 256
    && bytes[0].is_ascii_alphanumeric()
    && bytes[1..]
      .iter()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn event_source_identity(event_id: &str, workflow_id: &str, trigger_id: &str) -> String {
  let mut hasher = Sha256::new();
  hasher.update(event_id.as_bytes());
  hasher.update([0]);
  hasher.update(workflow_id.as_bytes());
  hasher.update([0]);
  hasher.update(trigger_id.as_bytes());
  format!("event:v1:sha256:{}", hex::encode(hasher.finalize()))
}

fn event_admission_failure(error: &DurableStoreError) -> (&'static str, &'static str, bool) {
  match error {
    DurableStoreError::RuntimePolicyQueueFull => (
      "WOML_POLICY_QUEUE_FULL",
      "The durable WOML policy queue is full; retry this subscriber delivery.",
      true,
    ),
    DurableStoreError::TriggerIdempotencyConflict => (
      "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
      "This event ID is already bound to a different payload.",
      false,
    ),
    DurableStoreError::TriggerDefinitionMismatch
    | DurableStoreError::TriggerHandlerMismatch
    | DurableStoreError::DefinitionConflict(_) => (
      "WOML_TRIGGER_DEFINITION_MISMATCH",
      "The event subscriber no longer matches its registered workflow definition.",
      false,
    ),
    DurableStoreError::TriggerHistoryInvalid(_) | DurableStoreError::InvalidStoredEvent(_) => (
      "WOML_TRIGGER_HISTORY_INVALID",
      "The durable event subscriber history is contradictory.",
      false,
    ),
    _ => (
      "WOML_TRIGGER_UNAVAILABLE",
      "The durable WOML trigger authority is unavailable.",
      true,
    ),
  }
}

fn event_rejected_response(
  state: &WebhookRuntimeState,
  status: StatusCode,
  code: &'static str,
  message: &'static str,
) -> HttpResponse {
  state.report_event_rejected(None, code, message);
  error_response(status, code, message, None)
}
