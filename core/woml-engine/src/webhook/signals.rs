//! Private webhook signals.

use crate::webhook::has_json_content_type;
use crate::webhook::rejected_response;
use crate::webhook::runtime_failure_code;
use crate::webhook::ActiveRunGuard;
use crate::webhook::TriggerProgress;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT_VERSION;
use crate::DurableEventStore;
use actix_web::http::header;
use actix_web::http::Method;
use actix_web::http::StatusCode;
use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use subtle::ConstantTimeEq;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::webhook) struct SignalHttpRequestV1 {
  pub(in crate::webhook) run_id: String,
  pub(in crate::webhook) wait_id: String,
  pub(in crate::webhook) correlation: String,
  pub(in crate::webhook) payload: Value,
}

fn signal_http_response(
  status: StatusCode,
  outcome: &crate::ExternalSignalAdmissionOutcome,
) -> HttpResponse {
  let mut response = HttpResponse::build(status);
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  response.json(outcome)
}

pub(in crate::webhook) async fn handle_external_signal(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
  signal: String,
) -> HttpResponse {
  let Some(expected_digest) = state.signal_token_digest else {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::NOT_FOUND,
      "WOML_SIGNAL_TARGET_NOT_FOUND",
      "External signal ingress is disabled.",
      None,
    );
  };
  if request.method() != Method::POST {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::METHOD_NOT_ALLOWED,
      "WOML_SIGNAL_METHOD_NOT_ALLOWED",
      "External signal ingress accepts POST requests only.",
      None,
    );
  }
  let authorization = request
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "));
  let authenticated = authorization.is_some_and(|token| {
    let provided: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    bool::from(provided.ct_eq(&expected_digest))
  });
  if !authenticated {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::FORBIDDEN,
      "WOML_SIGNAL_ACCESS_DENIED",
      "External signal authentication failed.",
      None,
    );
  }
  {
    let Ok(mut rate) = state.signal_rate_window.lock() else {
      return rejected_response(
        state.get_ref(),
        None,
        StatusCode::SERVICE_UNAVAILABLE,
        "WOML_SIGNAL_UNAVAILABLE",
        "External signal admission is temporarily unavailable.",
        None,
      );
    };
    let now = Utc::now();
    if now - rate.0 >= ChronoDuration::minutes(1) {
      *rate = (now, 0);
    }
    if rate.1 >= 120 {
      return rejected_response(
        state.get_ref(),
        None,
        StatusCode::TOO_MANY_REQUESTS,
        "WOML_SIGNAL_RATE_LIMITED",
        "External signal ingress exceeded 120 requests per minute.",
        None,
      );
    }
    rate.1 += 1;
  }
  if !has_json_content_type(&request) {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::BAD_REQUEST,
      "WOML_SIGNAL_PAYLOAD_INVALID",
      "External signal payload must use application/json.",
      None,
    );
  }
  let Some(idempotency_key) = request
    .headers()
    .get("Idempotency-Key")
    .and_then(|value| value.to_str().ok())
    .map(str::to_string)
  else {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::BAD_REQUEST,
      "WOML_SIGNAL_IDEMPOTENCY_REQUIRED",
      "External signal ingress requires an Idempotency-Key header.",
      None,
    );
  };
  let mut bytes = Vec::new();
  while let Some(chunk) = body.next().await {
    let Ok(chunk) = chunk else {
      return rejected_response(
        state.get_ref(),
        None,
        StatusCode::BAD_REQUEST,
        "WOML_SIGNAL_PAYLOAD_INVALID",
        "External signal request body is invalid.",
        None,
      );
    };
    if bytes.len() + chunk.len() > crate::EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES + 4_096 {
      return rejected_response(
        state.get_ref(),
        None,
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_SIGNAL_PAYLOAD_TOO_LARGE",
        "External signal payload exceeds the 1 MiB limit.",
        None,
      );
    }
    bytes.extend_from_slice(&chunk);
  }
  let decoded: SignalHttpRequestV1 = match serde_json::from_slice(&bytes) {
    Ok(decoded) => decoded,
    Err(_) => {
      return rejected_response(
        state.get_ref(),
        None,
        StatusCode::BAD_REQUEST,
        "WOML_SIGNAL_PAYLOAD_INVALID",
        "External signal request body does not match Signal HTTP v1.",
        None,
      )
    }
  };
  let _active = ActiveRunGuard::acquire(&state);
  let admitted = crate::admit_external_signal_durable(
    state.database_path.clone(),
    crate::ExternalSignalAdmissionRequest {
      profile: crate::EXTERNAL_SIGNAL_ADMISSION_PROFILE.to_string(),
      signal,
      run_id: decoded.run_id,
      wait_id: decoded.wait_id,
      correlation: decoded.correlation,
      idempotency_key,
      payload: decoded.payload,
      received_at: Utc::now(),
      authentication_class: "http_bearer".to_string(),
    },
    state.execution.clone(),
  )
  .await;
  let Ok(admitted) = admitted else {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::SERVICE_UNAVAILABLE,
      "WOML_SIGNAL_STORE_UNAVAILABLE",
      "The durable signal authority is temporarily unavailable.",
      None,
    );
  };
  if let Some(continuation) = admitted.continuation {
    let workflow_id = DurableEventStore::open_ready(&state.database_path)
      .and_then(|store| store.run_binding(&admitted.admission.run_id))
      .map(|binding| binding.workflow_id);
    if let Ok(workflow_id) = workflow_id {
      let terminal = match continuation {
        Ok(crate::WorkflowRuntimeOutcome::Succeeded { .. }) => Some(("succeeded", None)),
        Ok(crate::WorkflowRuntimeOutcome::Waiting { .. })
        | Ok(crate::WorkflowRuntimeOutcome::WaitingTime { .. })
        | Ok(crate::WorkflowRuntimeOutcome::WaitingSignal { .. }) => None,
        Err(error) => Some(("failed", Some(runtime_failure_code(&error)))),
      };
      if let Some((status, failure_code)) = terminal {
        state.report(TriggerProgress::RunTerminal {
          contract: TRIGGER_PROGRESS_CONTRACT,
          contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
          workflow_id,
          run_id: admitted.admission.run_id.clone(),
          status,
          failure_code,
          occurred_at: Utc::now(),
        });
      }
    }
  }
  let status = match admitted.admission.status {
    crate::ExternalSignalAdmissionStatus::Accepted => StatusCode::ACCEPTED,
    crate::ExternalSignalAdmissionStatus::Duplicate => StatusCode::OK,
    crate::ExternalSignalAdmissionStatus::IdentityConflict
    | crate::ExternalSignalAdmissionStatus::AlreadyResolved => StatusCode::CONFLICT,
    crate::ExternalSignalAdmissionStatus::TargetNotFound => StatusCode::NOT_FOUND,
    crate::ExternalSignalAdmissionStatus::Rejected => StatusCode::UNPROCESSABLE_ENTITY,
  };
  signal_http_response(status, &admitted.admission)
}
