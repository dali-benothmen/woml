//! Private webhook admission.

use crate::webhook::authorized;
use crate::webhook::dispatch_run;
use crate::webhook::handle_event_publication;
use crate::webhook::handle_external_signal;
use crate::webhook::handle_whatsapp_callback;
use crate::webhook::has_json_content_type;
use crate::webhook::inbound_trace_context;
use crate::webhook::rejected_response;
use crate::webhook::schema_issue;
use crate::webhook::ExternalTriggerAdmissionReceiver;
use crate::webhook::RunProgressIdentity;
use crate::webhook::StartupManualTrigger;
use crate::webhook::TriggerProgress;
use crate::webhook::WebhookAcceptedResponse;
use crate::webhook::WebhookErrorBody;
use crate::webhook::WebhookErrorResponse;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT_VERSION;
use crate::webhook::WEBHOOK_MAX_BODY_BYTES;
use crate::webhook::WHATSAPP_CALLBACK_PATH;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::EventServiceAcceptedRun;
use crate::TriggerAdmissionRequest;
use actix_web::http::header;
use actix_web::http::Method;
use actix_web::http::StatusCode;
use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::Value;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc as tokio_mpsc;
use uuid::Uuid;

pub(in crate::webhook) async fn handle_webhook(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
) -> HttpResponse {
  if !state.admission_open.load(Ordering::Acquire) {
    let mut response = HttpResponse::ServiceUnavailable();
    response.insert_header((header::CACHE_CONTROL, "no-store"));
    response.insert_header((header::RETRY_AFTER, "1"));
    return response.json(WebhookErrorResponse {
      error: WebhookErrorBody {
        code: "WOML_RUNTIME_NOT_READY",
        message: "The WOML deployment is still activating; retry this request.",
        issues: None,
      },
    });
  }
  if request.path() == WHATSAPP_CALLBACK_PATH {
    return handle_whatsapp_callback(request, body, state).await;
  }
  let inbound_trace_context = inbound_trace_context(&request);
  let signal = request
    .path()
    .strip_prefix("/v1/signals/")
    .map(str::to_string);
  if let Some(signal) = signal {
    return handle_external_signal(request, body, state, signal).await;
  }
  let event_name = request
    .path()
    .strip_prefix("/_woml/events/")
    .map(str::to_string);
  if let Some(event_name) = event_name {
    return handle_event_publication(request, body, state, event_name).await;
  }
  let Some(route) = state.routes.get(request.path()).cloned() else {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::NOT_FOUND,
      "WOML_TRIGGER_NOT_FOUND",
      "No WOML webhook is registered for this route.",
      None,
    );
  };
  if request.method() != Method::POST {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::METHOD_NOT_ALLOWED,
      "WOML_TRIGGER_METHOD_NOT_ALLOWED",
      "This WOML webhook accepts POST requests only.",
      None,
    );
  }
  if !authorized(&request, &route.authentication) {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::UNAUTHORIZED,
      "WOML_TRIGGER_UNAUTHORIZED",
      "Webhook authentication failed.",
      None,
    );
  }
  if !has_json_content_type(&request) {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::BAD_REQUEST,
      "WOML_TRIGGER_PAYLOAD_INVALID",
      "Webhook payload must use application/json.",
      None,
    );
  }
  if request
    .headers()
    .get(header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
    .is_some_and(|length| length > WEBHOOK_MAX_BODY_BYTES as u64)
  {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::PAYLOAD_TOO_LARGE,
      "WOML_TRIGGER_PAYLOAD_TOO_LARGE",
      "Webhook payload exceeds the 1 MiB limit.",
      None,
    );
  }

  let mut bytes = Vec::new();
  while let Some(chunk) = body.next().await {
    let Ok(chunk) = chunk else {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::BAD_REQUEST,
        "WOML_TRIGGER_PAYLOAD_INVALID",
        "Webhook payload must be a valid JSON object.",
        None,
      );
    };
    if bytes.len().saturating_add(chunk.len()) > WEBHOOK_MAX_BODY_BYTES {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_TRIGGER_PAYLOAD_TOO_LARGE",
        "Webhook payload exceeds the 1 MiB limit.",
        None,
      );
    }
    bytes.extend_from_slice(&chunk);
  }
  let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::BAD_REQUEST,
      "WOML_TRIGGER_PAYLOAD_INVALID",
      "Webhook payload must be a valid JSON object.",
      None,
    );
  };
  let Value::Object(payload) = value else {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::BAD_REQUEST,
      "WOML_TRIGGER_PAYLOAD_INVALID",
      "Webhook payload must be a valid JSON object.",
      None,
    );
  };

  if let Some(validator) = &route.schema {
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
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::BAD_REQUEST,
        "WOML_TRIGGER_SCHEMA_INVALID",
        "Webhook payload does not match the declared schema.",
        Some(issues),
      );
    }
  }

  let source_identity = match request.headers().get("Idempotency-Key") {
    Some(value) => match value.to_str() {
      Ok(value) if !value.is_empty() && value.len() <= 512 => value.to_string(),
      _ => {
        return rejected_response(
          state.get_ref(),
          Some(route.as_ref()),
          StatusCode::BAD_REQUEST,
          "WOML_TRIGGER_PAYLOAD_INVALID",
          "Webhook payload must be a valid JSON object.",
          None,
        )
      }
    },
    None => format!("webhook_{}", Uuid::new_v4().simple()),
  };
  let admission = TriggerAdmissionRequest {
    workflow_id: route.workflow_id.clone(),
    definition_hash: route.definition_hash.clone(),
    trigger_id: route.trigger_id.clone(),
    trigger_handler: "trigger.webhook".to_string(),
    source_identity,
    payload,
    received_at: Utc::now(),
  };
  let database_path = state.database_path.clone();
  let admitted = web::block(move || {
    let mut store = DurableEventStore::open_ready(database_path)?;
    store.admit_trigger_occurrence(admission)
  })
  .await;

  let outcome = match admitted {
    Ok(Ok(outcome)) => outcome,
    Ok(Err(DurableStoreError::TriggerIdempotencyConflict)) => {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::CONFLICT,
        "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
        "This idempotency key is already bound to a different payload.",
        None,
      )
    }
    Ok(Err(DurableStoreError::RuntimePolicyQueueFull)) => {
      state.report_rejected(
        Some(route.as_ref()),
        "WOML_POLICY_QUEUE_FULL",
        "The durable WOML policy queue is full; retry this request.",
      );
      let mut response = HttpResponse::ServiceUnavailable();
      response.insert_header((header::CACHE_CONTROL, "no-store"));
      response.insert_header((header::RETRY_AFTER, "1"));
      return response.json(WebhookErrorResponse {
        error: WebhookErrorBody {
          code: "WOML_POLICY_QUEUE_FULL",
          message: "The durable WOML policy queue is full; retry this request.",
          issues: None,
        },
      });
    }
    Ok(Err(_)) | Err(_) => {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::SERVICE_UNAVAILABLE,
        "WOML_TRIGGER_UNAVAILABLE",
        "The durable WOML trigger authority is unavailable.",
        None,
      )
    }
  };

  let identity = RunProgressIdentity {
    workflow_id: route.workflow_id.clone(),
    trigger_id: route.trigger_id.clone(),
    trigger_handler: "trigger.webhook".to_string(),
    occurrence_id: outcome.occurrence_id,
    run_id: outcome.run_id.clone(),
  };
  state.execution.report_telemetry_admission_context(
    &outcome.run_id,
    inbound_trace_context,
    Utc::now(),
  );
  state.report_accepted(&identity, outcome.duplicate);
  if !outcome.duplicate {
    state.report_run_started(&identity);
    dispatch_run(state.get_ref(), identity);
  }
  HttpResponse::Accepted().json(WebhookAcceptedResponse {
    run_id: outcome.run_id,
    status: if route.runtime_policy {
      "queued"
    } else {
      "accepted"
    },
    duplicate: outcome.duplicate,
  })
}

pub(in crate::webhook) fn admit_startup_manual(
  state: &WebhookRuntimeState,
  startup: StartupManualTrigger,
) {
  let request = TriggerAdmissionRequest {
    workflow_id: startup.workflow_id.clone(),
    definition_hash: startup.definition_hash,
    trigger_id: startup.trigger_id.clone(),
    trigger_handler: "trigger.manual".to_string(),
    source_identity: format!("manual_{}", Uuid::new_v4().simple()),
    payload: serde_json::Map::new(),
    received_at: Utc::now(),
  };
  let admitted = DurableEventStore::open_ready(&state.database_path)
    .and_then(|mut store| store.admit_trigger_occurrence(request));
  match admitted {
    Ok(outcome) => {
      let identity = RunProgressIdentity {
        workflow_id: startup.workflow_id,
        trigger_id: startup.trigger_id,
        trigger_handler: "trigger.manual".to_string(),
        occurrence_id: outcome.occurrence_id,
        run_id: outcome.run_id,
      };
      state.report_accepted(&identity, false);
      state.report_run_started(&identity);
      dispatch_run(state, identity);
    }
    Err(error) => state.report(TriggerProgress::OccurrenceRejected {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: Some(startup.workflow_id),
      trigger_id: Some(startup.trigger_id),
      trigger_handler: "trigger.manual".to_string(),
      code: if matches!(error, DurableStoreError::RuntimePolicyQueueFull) {
        "WOML_POLICY_QUEUE_FULL"
      } else {
        "WOML_TRIGGER_UNAVAILABLE"
      }
      .to_string(),
      message: if matches!(error, DurableStoreError::RuntimePolicyQueueFull) {
        "The durable WOML policy queue is full; retry the manual run."
      } else {
        "The durable WOML trigger authority is unavailable."
      }
      .to_string(),
      occurred_at: Utc::now(),
    }),
  }
}

pub(in crate::webhook) async fn run_external_ingress(
  state: web::Data<WebhookRuntimeState>,
  mut receiver: ExternalTriggerAdmissionReceiver,
) {
  while let Some(command) = receiver.recv().await {
    let request = command.request;
    let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.admit_trigger");
    let workflow_id = request.workflow_id.clone();
    let trigger_id = request.trigger_id.clone();
    let trigger_handler = request.trigger_handler.clone();
    let database_path = state.database_path.clone();
    let admitted = web::block(move || {
      let mut store = DurableEventStore::open_ready(database_path)?;
      store.admit_trigger_occurrence(request)
    })
    .await;

    let outcome = match admitted {
      Ok(Ok(outcome)) => {
        span.run_id(outcome.run_id.clone());
        span.count("duplicate", usize::from(outcome.duplicate));
        span.succeed();
        outcome
      }
      Ok(Err(error)) => {
        let manual = trigger_handler == "trigger.manual";
        let (code, message) = match &error {
          DurableStoreError::TriggerIdempotencyConflict => (
            "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
            if manual {
              "This manual request identity is already bound to another run."
            } else {
              "This provider event is already bound to a different payload."
            },
          ),
          DurableStoreError::RuntimePolicyQueueFull => (
            "WOML_POLICY_QUEUE_FULL",
            if manual {
              "The durable WOML policy queue is full; try the manual trigger again later."
            } else {
              "The durable WOML policy queue is full; the provider may retry this event."
            },
          ),
          DurableStoreError::DefinitionNotFound(_)
          | DurableStoreError::TriggerDefinitionMismatch => (
            "WOML_TRIGGER_DEFINITION_MISMATCH",
            "The provider event does not match the active workflow definition; restart the WOML runtime.",
          ),
          DurableStoreError::TriggerNotFound { .. } => (
            "WOML_TRIGGER_NOT_FOUND",
            "The provider event references a trigger that is not registered in the active workflow.",
          ),
          DurableStoreError::TriggerHandlerMismatch => (
            "WOML_TRIGGER_HANDLER_MISMATCH",
            "The provider event type does not match the compiled workflow trigger.",
          ),
          DurableStoreError::Contract(_) | DurableStoreError::InvalidModel(_) => (
            "WOML_TRIGGER_CONTRACT_INVALID",
            "The compiled workflow and durable trigger contracts are incompatible; update or rebuild WOML.",
          ),
          _ => (
            "WOML_TRIGGER_UNAVAILABLE",
            if manual {
              "The durable WOML trigger authority is temporarily unavailable; retry the manual request."
            } else {
              "The durable WOML trigger authority is temporarily unavailable; the provider may retry this event."
            },
          ),
        };
        state.report(TriggerProgress::OccurrenceRejected {
          contract: TRIGGER_PROGRESS_CONTRACT,
          contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
          workflow_id: Some(workflow_id),
          trigger_id: Some(trigger_id),
          trigger_handler,
          code: code.to_string(),
          message: message.to_string(),
          occurred_at: Utc::now(),
        });
        let _ = command.response.send(Err(error));
        continue;
      }
      Err(_) => {
        state.report(TriggerProgress::OccurrenceRejected {
          contract: TRIGGER_PROGRESS_CONTRACT,
          contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
          workflow_id: Some(workflow_id),
          trigger_id: Some(trigger_id),
          trigger_handler,
          code: "WOML_TRIGGER_UNAVAILABLE".to_string(),
          message: "The durable WOML trigger authority is unavailable.".to_string(),
          occurred_at: Utc::now(),
        });
        let _ = command.response.send(Err(DurableStoreError::Contract(
          "The external trigger admission task failed.".to_string(),
        )));
        continue;
      }
    };

    let identity = RunProgressIdentity {
      workflow_id,
      trigger_id,
      trigger_handler,
      occurrence_id: outcome.occurrence_id.clone(),
      run_id: outcome.run_id.clone(),
    };
    state.report_accepted(&identity, outcome.duplicate);
    if !outcome.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    let _ = command.response.send(Ok(outcome));
  }
}

pub(in crate::webhook) async fn run_internal_event_dispatch(
  state: web::Data<WebhookRuntimeState>,
  mut receiver: tokio_mpsc::UnboundedReceiver<EventServiceAcceptedRun>,
) {
  while let Some(accepted) = receiver.recv().await {
    let identity = RunProgressIdentity {
      workflow_id: accepted.workflow_id,
      trigger_id: accepted.trigger_id,
      trigger_handler: "trigger.event".to_string(),
      occurrence_id: accepted.occurrence_id,
      run_id: accepted.run_id,
    };
    state.report_accepted(&identity, accepted.duplicate);
    if !accepted.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
  }
}
