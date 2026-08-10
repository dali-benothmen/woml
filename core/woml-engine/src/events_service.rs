//! Workflow-originated named-event publication through the existing durable
//! trigger-ingress authority.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use chrono::Utc;
use futures_util::future::BoxFuture;
use jsonschema::Validator;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler, DurableEventStore,
  DurableStoreError, InternalEventAdmissionRequest, TriggerAdmissionOutcome,
  TriggerAdmissionRequest,
};

pub const EVENTS_SERVICE_CONTRACT: &str = "woml.events";
pub const EVENTS_SERVICE_CONTRACT_VERSION: u32 = 1;
pub const MAX_INTERNAL_EVENT_DEPTH: u32 = 32;

#[derive(Clone)]
pub struct EventServiceSubscriber {
  pub workflow_id: String,
  pub definition_hash: String,
  pub trigger_id: String,
  pub schema: Option<Arc<Validator>>,
}

#[derive(Debug, Clone)]
pub struct EventServiceAcceptedRun {
  pub workflow_id: String,
  pub trigger_id: String,
  pub occurrence_id: String,
  pub run_id: String,
  pub duplicate: bool,
}

pub type EventServiceRunDispatcher = Arc<dyn Fn(EventServiceAcceptedRun) + Send + Sync>;

#[derive(Clone)]
pub struct ManagedEventsHandler {
  database_path: PathBuf,
  subscribers: Arc<BTreeMap<String, Vec<EventServiceSubscriber>>>,
  dispatcher: EventServiceRunDispatcher,
}

impl std::fmt::Debug for ManagedEventsHandler {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ManagedEventsHandler")
      .field("database_path", &self.database_path)
      .field("event_name_count", &self.subscribers.len())
      .finish()
  }
}

impl ManagedEventsHandler {
  pub fn new(
    database_path: PathBuf,
    subscribers: BTreeMap<String, Vec<EventServiceSubscriber>>,
    dispatcher: EventServiceRunDispatcher,
  ) -> Self {
    Self {
      database_path,
      subscribers: Arc::new(subscribers),
      dispatcher,
    }
  }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventsRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  operation: String,
  input: EventsInput,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsInput {
  name: String,
  payload: Map<String, Value>,
}

impl CapabilityHandler for ManagedEventsHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "events".to_string(),
      operation: "emit".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::IdempotentWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse_request(&request.input).map(|_| ())
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    let Some(name) = input
      .get("input")
      .and_then(|input| input.get("name"))
      .and_then(Value::as_str)
    else {
      return Map::new();
    };
    Map::from_iter([
      ("eventName".to_string(), Value::String(name.to_string())),
      (
        "subscriberCount".to_string(),
        Value::from(self.subscribers.get(name).map_or(0, Vec::len)),
      ),
    ])
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let Some(data) = result.get("data") else {
      return Map::new();
    };
    let mut metadata = Map::new();
    for field in ["acceptedCount", "duplicateCount", "rejectedCount"] {
      if let Some(value) = data.get(field).and_then(Value::as_u64) {
        metadata.insert(field.to_string(), Value::from(value));
      }
    }
    if let Some(status) = data.get("status").and_then(Value::as_str) {
      metadata.insert("status".to_string(), Value::String(status.to_string()));
    }
    metadata
  }

  fn execute(
    &self,
    _input: Value,
    _cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    Box::pin(async { Err(events_unavailable()) })
  }

  fn execute_request_scoped(
    &self,
    call: &CapabilityCallRequest,
    _workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = match parse_request(&call.input) {
      Ok(request) => request,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let parent_run_id = call.run_id.clone();
    let publication_id = format!("internal:v1:{}", call.identity.operation_key);
    let subscribers = self
      .subscribers
      .get(&request.input.name)
      .cloned()
      .unwrap_or_default();
    let database_path = self.database_path.clone();
    let dispatcher = Arc::clone(&self.dispatcher);
    Box::pin(async move {
      tokio::task::spawn_blocking(move || {
        publish(
          database_path,
          publication_id,
          parent_run_id,
          request.input,
          subscribers,
          dispatcher,
          cancellation,
        )
      })
      .await
      .map_err(|_| events_unavailable())?
    })
  }
}

fn publish(
  database_path: PathBuf,
  publication_id: String,
  parent_run_id: String,
  input: EventsInput,
  subscribers: Vec<EventServiceSubscriber>,
  dispatcher: EventServiceRunDispatcher,
  cancellation: CapabilityCancellationToken,
) -> Result<Value, CapabilityFailure> {
  let emitted_at = Utc::now();
  let mut deliveries = Vec::with_capacity(subscribers.len());
  let mut accepted_count = 0_u64;
  let mut duplicate_count = 0_u64;
  let mut rejected_count = 0_u64;
  let mut maximum_depth = 0_u32;
  for subscriber in subscribers {
    if cancellation.is_cancelled() {
      return Err(events_cancelled());
    }
    if let Some(validator) = &subscriber.schema {
      let payload = Value::Object(input.payload.clone());
      let mut issues = validator
        .iter_errors(&payload)
        .take(100)
        .map(|error| {
          json!({
            "path": error.instance_path().as_str(),
            "message": "Event payload does not satisfy this subscriber schema."
          })
        })
        .collect::<Vec<_>>();
      if !issues.is_empty() {
        issues.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
        rejected_count += 1;
        deliveries.push(json!({
          "workflowId": subscriber.workflow_id,
          "triggerId": subscriber.trigger_id,
          "status": "rejected",
          "code": "WOML_TRIGGER_SCHEMA_INVALID",
          "message": "Event payload does not match this subscriber schema.",
          "retryable": false,
          "issues": issues,
        }));
        continue;
      }
    }

    let trigger = TriggerAdmissionRequest {
      workflow_id: subscriber.workflow_id.clone(),
      definition_hash: subscriber.definition_hash.clone(),
      trigger_id: subscriber.trigger_id.clone(),
      trigger_handler: "trigger.event".to_string(),
      source_identity: event_source_identity(
        &publication_id,
        &subscriber.workflow_id,
        &subscriber.trigger_id,
      ),
      payload: input.payload.clone(),
      received_at: emitted_at,
    };
    let admitted = DurableEventStore::open(&database_path).and_then(|mut store| {
      store.admit_internal_event_occurrence(InternalEventAdmissionRequest {
        publication_id: publication_id.clone(),
        parent_run_id: parent_run_id.clone(),
        event_name: input.name.clone(),
        trigger,
        emitted_at,
      })
    });
    match admitted {
      Ok(outcome) => {
        maximum_depth = maximum_depth.max(outcome.depth);
        accepted_count += 1;
        if outcome.occurrence.duplicate {
          duplicate_count += 1;
        }
        deliveries.push(accepted_delivery(&subscriber, &outcome.occurrence));
        (dispatcher)(EventServiceAcceptedRun {
          workflow_id: subscriber.workflow_id,
          trigger_id: subscriber.trigger_id,
          occurrence_id: outcome.occurrence.occurrence_id,
          run_id: outcome.occurrence.run_id,
          duplicate: outcome.occurrence.duplicate,
        });
      }
      Err(error) => {
        rejected_count += 1;
        deliveries.push(rejected_delivery(&subscriber, &error));
      }
    }
  }
  let status = if rejected_count == 0 {
    "accepted"
  } else if accepted_count == 0 {
    "rejected"
  } else {
    "partial"
  };
  Ok(json!({
    "contract": EVENTS_SERVICE_CONTRACT,
    "contractVersion": EVENTS_SERVICE_CONTRACT_VERSION,
    "kind": "result",
    "operation": "emit",
    "data": {
      "publicationId": publication_id,
      "eventName": input.name,
      "status": status,
      "depth": maximum_depth,
      "acceptedCount": accepted_count,
      "duplicateCount": duplicate_count,
      "rejectedCount": rejected_count,
      "deliveries": deliveries,
    }
  }))
}

fn accepted_delivery(
  subscriber: &EventServiceSubscriber,
  outcome: &TriggerAdmissionOutcome,
) -> Value {
  json!({
    "workflowId": subscriber.workflow_id,
    "triggerId": subscriber.trigger_id,
    "status": "accepted",
    "runId": outcome.run_id,
    "duplicate": outcome.duplicate,
  })
}

fn rejected_delivery(subscriber: &EventServiceSubscriber, error: &DurableStoreError) -> Value {
  let (code, message, retryable) = match error {
    DurableStoreError::InternalEventCycle => (
      "WOML_EVENT_CYCLE",
      "This event would repeat a subscriber already present in its lineage.",
      false,
    ),
    DurableStoreError::InternalEventDepthExceeded => (
      "WOML_EVENT_DEPTH_EXCEEDED",
      "This event exceeds the maximum internal event depth.",
      false,
    ),
    DurableStoreError::InternalEventIdempotencyConflict
    | DurableStoreError::TriggerIdempotencyConflict => (
      "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
      "This publication identity is already bound to different event data.",
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
  };
  json!({
    "workflowId": subscriber.workflow_id,
    "triggerId": subscriber.trigger_id,
    "status": "rejected",
    "code": code,
    "message": message,
    "retryable": retryable,
  })
}

fn parse_request(input: &Value) -> Result<EventsRequest, CapabilityFailure> {
  let request: EventsRequest = serde_json::from_value(input.clone())
    .map_err(|_| events_input("Events Service request is invalid."))?;
  if request.contract != EVENTS_SERVICE_CONTRACT
    || request.contract_version != EVENTS_SERVICE_CONTRACT_VERSION
    || request.kind != "request"
    || request.operation != "emit"
    || !valid_event_name(&request.input.name)
  {
    return Err(events_input(
      "Events Service request does not match Events Service v1.",
    ));
  }
  let payload_bytes = serde_json::to_vec(&request.input.payload)
    .map_err(|_| events_input("Event payload is invalid JSON."))?
    .len();
  if payload_bytes > 1_048_576 {
    return Err(events_failure(
      CapabilityFailureKind::InputTooLarge,
      "WOML_EVENT_PAYLOAD_TOO_LARGE",
      "Event payload exceeds the 1 MiB limit.",
      false,
      false,
    ));
  }
  Ok(request)
}

fn valid_event_name(value: &str) -> bool {
  if value.len() > 256
    || !value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_lowercase())
  {
    return false;
  }
  let mut has_separator = false;
  let mut previous_separator = false;
  for byte in value.bytes() {
    let separator = matches!(byte, b'.' | b'_' | b'-');
    if separator {
      if previous_separator {
        return false;
      }
      has_separator = true;
    } else if !byte.is_ascii_lowercase() && !byte.is_ascii_digit() {
      return false;
    }
    previous_separator = separator;
  }
  has_separator && !previous_separator
}

fn event_source_identity(publication_id: &str, workflow_id: &str, trigger_id: &str) -> String {
  use sha2::{Digest, Sha256};
  let mut hasher = Sha256::new();
  hasher.update(publication_id.as_bytes());
  hasher.update([0]);
  hasher.update(workflow_id.as_bytes());
  hasher.update([0]);
  hasher.update(trigger_id.as_bytes());
  format!("event:v1:sha256:{}", hex::encode(hasher.finalize()))
}

fn events_input(message: impl Into<String>) -> CapabilityFailure {
  events_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_EVENTS_INPUT_INVALID",
    message,
    false,
    false,
  )
}

fn events_unavailable() -> CapabilityFailure {
  events_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_EVENTS_UNAVAILABLE",
    "The internal event publication authority is unavailable.",
    true,
    false,
  )
}

fn events_cancelled() -> CapabilityFailure {
  events_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_EVENTS_CANCELLED",
    "The internal event publication was cancelled.",
    false,
    false,
  )
}

fn events_failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: impl Into<String>,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.into(),
    retryable,
    ambiguous,
    details: None,
  }
}
