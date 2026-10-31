//! Private webhook routing.

use crate::dispatch_admitted_workflow_call;
use crate::webhook::WebhookRuntimeState;
use crate::DurableEventStore;
use crate::WorkflowRoutingAcknowledgement;
use crate::WorkflowRoutingWakeup;
use crate::WORKFLOW_ROUTING_CONTRACT;
use crate::WORKFLOW_ROUTING_CONTRACT_VERSION;
use actix_web::http::header;
use actix_web::http::StatusCode;
use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use std::sync::atomic::Ordering;
use subtle::ConstantTimeEq;

pub(in crate::webhook) const WORKFLOW_RUNTIME_LEASE_SECONDS: i64 = 10;

const WORKFLOW_RUNTIME_RENEW_SECONDS: u64 = 2;

const WORKFLOW_PENDING_SCAN_MILLISECONDS: u64 = 250;

pub(in crate::webhook) async fn handle_workflow_call_wakeup(
  state: web::Data<WebhookRuntimeState>,
  request: HttpRequest,
  wakeup: web::Json<WorkflowRoutingWakeup>,
) -> HttpResponse {
  if !state.admission_open.load(Ordering::Acquire) {
    return workflow_routing_acknowledgement(
      StatusCode::SERVICE_UNAVAILABLE,
      &wakeup.child_run_id,
      false,
      Some("WOML_RUNTIME_NOT_READY"),
    );
  }
  let supplied = request
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
    .unwrap_or_default();
  let expected = state.workflow_routing_credential.as_bytes();
  let authorized =
    supplied.len() == expected.len() && bool::from(supplied.as_bytes().ct_eq(expected));
  if !authorized {
    return workflow_routing_acknowledgement(
      StatusCode::UNAUTHORIZED,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  }
  if wakeup.contract != WORKFLOW_ROUTING_CONTRACT
    || wakeup.contract_version != WORKFLOW_ROUTING_CONTRACT_VERSION
    || wakeup.kind != "wakeup"
    || wakeup.runtime_id != state.workflow_targets.runtime_id()
  {
    return workflow_routing_acknowledgement(
      StatusCode::BAD_REQUEST,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  }
  let admission = DurableEventStore::open_ready(&state.database_path)
    .and_then(|store| store.workflow_call_for_child(&wakeup.child_run_id));
  let Ok(Some(admission)) = admission else {
    return workflow_routing_acknowledgement(
      StatusCode::NOT_FOUND,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  };
  if admission.call_key != wakeup.call_key
    || !state.workflow_targets.owns(
      &admission.target_workflow_id,
      &admission.target_definition_hash,
    )
  {
    return workflow_routing_acknowledgement(
      StatusCode::CONFLICT,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_DEFINITION_MISMATCH"),
    );
  }
  if dispatch_admitted_workflow_call(
    state.database_path.clone(),
    admission,
    state.execution.clone(),
  )
  .await
  .is_err()
  {
    return workflow_routing_acknowledgement(
      StatusCode::SERVICE_UNAVAILABLE,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  }
  workflow_routing_acknowledgement(StatusCode::OK, &wakeup.child_run_id, true, None)
}

fn workflow_routing_acknowledgement(
  status: StatusCode,
  child_run_id: &str,
  accepted: bool,
  code: Option<&str>,
) -> HttpResponse {
  HttpResponse::build(status).json(WorkflowRoutingAcknowledgement {
    contract: WORKFLOW_ROUTING_CONTRACT.to_string(),
    contract_version: WORKFLOW_ROUTING_CONTRACT_VERSION,
    kind: "acknowledgement".to_string(),
    child_run_id: child_run_id.to_string(),
    accepted,
    code: code.map(str::to_string),
  })
}

pub(in crate::webhook) async fn run_workflow_routing_maintenance(
  state: web::Data<WebhookRuntimeState>,
) {
  let mut next_renewal = tokio::time::Instant::now();
  loop {
    let now = Utc::now();
    let targets = state.workflow_targets.targets();
    if tokio::time::Instant::now() >= next_renewal {
      let renewed = DurableEventStore::open_ready(&state.database_path).and_then(|mut store| {
        store.renew_workflow_runtime_routes(
          state.workflow_targets.runtime_id(),
          now,
          now + ChronoDuration::seconds(WORKFLOW_RUNTIME_LEASE_SECONDS),
        )
      });
      if renewed.is_ok_and(|count| count != targets.len()) {
        return;
      }
      next_renewal = tokio::time::Instant::now()
        + std::time::Duration::from_secs(WORKFLOW_RUNTIME_RENEW_SECONDS);
    }
    for target in targets {
      let pending = DurableEventStore::open_ready(&state.database_path)
        .and_then(|store| store.admitted_workflow_calls_for_target(&target));
      if let Ok(pending) = pending {
        for admission in pending {
          let _ = dispatch_admitted_workflow_call(
            state.database_path.clone(),
            admission,
            state.execution.clone(),
          )
          .await;
        }
      }
    }
    tokio::time::sleep(std::time::Duration::from_millis(
      WORKFLOW_PENDING_SCAN_MILLISECONDS,
    ))
    .await;
  }
}
