use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Map};
use tokio::sync::Mutex;
use woml_engine::event::StepAttemptStartedData;
use woml_engine::{
  derive_operation_key, fold_events, step_effect_idempotency_key, CapabilityCallIdentity,
  CapabilityCallLimits, CapabilityCallRequest, CapabilityCallResult, CapabilityCancellationToken,
  CapabilityFailureKind, CapabilityRegistry, CompiledWorkflowDefinition,
  DurableCapabilityAuthority, DurableEventStore, OperationExecutionMode, OperationFailedData,
  OperationStartedData, OperationStatus, RunEvent, RunEventPayload, RunStartedData, RunStatus,
  TestCapabilityHandler,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const SUCCESS_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/services-http.events.v8.json");
const FAILED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/services-http-failed.events.v8.json");
const DEFINITION_HASH: &str =
  "sha256:1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn frozen_capability_call_messages_validate_in_rust() {
  let request: CapabilityCallRequest = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/services-contracts/capability-request.v1.json"
  ))
  .unwrap();
  request.validate().unwrap();
  for fixture in [
    include_str!("../../../woml/tests/fixtures/services-contracts/capability-succeeded.v1.json"),
    include_str!("../../../woml/tests/fixtures/services-contracts/capability-failed.v1.json"),
    include_str!("../../../woml/tests/fixtures/services-contracts/capability-host-crashed.v1.json"),
  ] {
    let result: CapabilityCallResult = serde_json::from_str(fixture).unwrap();
    result.validate().unwrap();
  }
}

fn single_node_model() -> CompiledWorkflowDefinition {
  let mut model: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  model.graph.nodes.retain(|node| node.id == "loadCustomer");
  model.graph.edges.clear();
  model.validate_structure().unwrap();
  model
}

#[test]
fn rust_enforces_the_frozen_model_v8_script_profile() {
  let model: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  model.validate_structure().unwrap();

  let mut missing_runtime = model.clone();
  missing_runtime.graph.nodes[0].script_runtime = None;
  assert!(missing_runtime.validate_structure().is_err());

  let mut malformed_secret = model.clone();
  malformed_secret.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets = vec!["_PRIVATE".to_string()];
  assert!(malformed_secret.validate_structure().is_err());

  let mut older_model = model;
  older_model.schema_version = 7;
  assert!(older_model.validate_structure().is_err());
}

#[test]
fn operation_identity_derivation_is_frozen() {
  assert_eq!(
    derive_operation_key(
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "test.control.echo",
    ),
    "sha256:a4f2312c82a5effefb558f1abe704c2b6f71978e7bdb3d6f7c245434dbdb53fb"
  );
}

fn request(
  run_id: &str,
  invocation_id: &str,
  call_id: &str,
  input: serde_json::Value,
) -> CapabilityCallRequest {
  let step_key = step_effect_idempotency_key(run_id, DEFINITION_HASH, "loadCustomer");
  let operation_name = format!("test.control.{call_id}");
  CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: invocation_id.to_string(),
    call_id: call_id.to_string(),
    run_id: run_id.to_string(),
    node_id: "loadCustomer".to_string(),
    attempt_number: 1,
    capability: "test".to_string(),
    operation: "control".to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: woml_engine::capability::CapabilityIdentityMode::Named,
      operation_key: derive_operation_key(&step_key, &operation_name),
      step_idempotency_key: step_key,
      operation_name,
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits::default(),
    input,
  }
}

fn active_store(run_id: &str, invocation_id: &str) -> DurableEventStore {
  let model = single_node_model();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store.register_definition(&model, DEFINITION_HASH).unwrap();
  store
    .append_payload(
      run_id,
      format!("event-{run_id}-start"),
      Utc::now(),
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: model.workflow_id,
        definition_hash: DEFINITION_HASH.to_string(),
        trigger_id: Some("start".to_string()),
        trigger_handler: Some("trigger.manual".to_string()),
        trigger_occurrence_id: Some(format!("occurrence-{run_id}")),
        trigger: Map::new(),
      }),
    )
    .unwrap();
  store
    .append_payload(
      run_id,
      format!("event-{run_id}-attempt"),
      Utc::now(),
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "loadCustomer".to_string(),
        attempt: 1,
        invocation_id: invocation_id.to_string(),
        handler: "runtime.script".to_string(),
        idempotency_key: Some(step_effect_idempotency_key(
          run_id,
          DEFINITION_HASH,
          "loadCustomer",
        )),
      }),
    )
    .unwrap();
  store
}

#[test]
fn frozen_v8_histories_validate_and_fold() {
  let succeeded: Vec<RunEvent> = serde_json::from_str(SUCCESS_EVENTS).unwrap();
  let failed: Vec<RunEvent> = serde_json::from_str(FAILED_EVENTS).unwrap();
  let success_projection = fold_events(&succeeded).unwrap();
  let failed_projection = fold_events(&failed).unwrap();

  assert_eq!(success_projection.status, RunStatus::Succeeded);
  assert!(matches!(
    success_projection
      .operations
      .values()
      .next()
      .unwrap()
      .status,
    OperationStatus::Succeeded { .. }
  ));
  assert_eq!(failed_projection.status, RunStatus::Failed);
  assert!(matches!(
    failed_projection.operations.values().next().unwrap().status,
    OperationStatus::Failed { .. }
  ));
}

#[test]
fn sqlite_and_in_memory_folds_match_every_frozen_v8_history() {
  for fixture in [SUCCESS_EVENTS, FAILED_EVENTS] {
    let events: Vec<RunEvent> = serde_json::from_str(fixture).unwrap();
    let run_id = events[0].run_id.clone();
    let expected = fold_events(&events).unwrap();
    let model = single_node_model();
    let mut store = DurableEventStore::open_in_memory().unwrap();
    store.register_definition(&model, DEFINITION_HASH).unwrap();
    for event in events {
      store
        .append_payload(
          event.run_id,
          event.event_id,
          event.occurred_at,
          event.payload,
        )
        .unwrap();
    }
    assert_eq!(store.projection(&run_id).unwrap(), expected);
  }
}

#[tokio::test(flavor = "current_thread")]
async fn registry_enforces_results_sizes_cancellation_crashes_and_concurrency() {
  let registry = Arc::new(CapabilityRegistry::new(1, 2));
  registry.register(Arc::new(TestCapabilityHandler)).unwrap();

  let echo = registry
    .execute(
      request(
        "run-a",
        "inv-a",
        "echo",
        json!({"mode": "echo", "value": {"ok": true}}),
      ),
      CapabilityCancellationToken::default(),
    )
    .await;
  assert!(matches!(echo, CapabilityCallResult::Succeeded(_)));

  let mut oversized = request(
    "run-a",
    "inv-b",
    "oversized",
    json!({"mode": "echo", "value": "too large"}),
  );
  oversized.limits.input_bytes = 1;
  assert!(matches!(
    registry.execute(oversized, CapabilityCancellationToken::default()).await,
    CapabilityCallResult::Failed(result)
      if result.error.kind == CapabilityFailureKind::InputTooLarge
  ));

  let cancellation = CapabilityCancellationToken::default();
  cancellation.cancel();
  assert!(matches!(
    registry
      .execute(
        request(
          "run-a",
          "inv-c",
          "cancel",
          json!({"mode": "delay", "delayMs": 50})
        ),
        cancellation,
      )
      .await,
    CapabilityCallResult::Cancelled(_)
  ));

  assert!(matches!(
    registry.execute(
      request("run-a", "inv-d", "panic", json!({"mode": "panic"})),
      CapabilityCancellationToken::default(),
    ).await,
    CapabilityCallResult::Failed(result)
      if result.error.kind == CapabilityFailureKind::HandlerCrashed && !result.error.ambiguous
  ));

  let first_registry = registry.clone();
  let first = tokio::spawn(async move {
    first_registry
      .execute(
        request(
          "run-a",
          "inv-shared",
          "slow",
          json!({"mode": "delay", "delayMs": 30}),
        ),
        CapabilityCancellationToken::default(),
      )
      .await
  });
  tokio::task::yield_now().await;
  let second = registry
    .execute(
      request("run-a", "inv-shared", "second", json!({"mode": "echo"})),
      CapabilityCancellationToken::default(),
    )
    .await;
  assert!(matches!(
    second,
    CapabilityCallResult::Failed(result) if result.error.code == "WOML_CAPABILITY_BACKPRESSURE"
  ));
  assert!(matches!(
    first.await.unwrap(),
    CapabilityCallResult::Succeeded(_)
  ));
}

#[tokio::test(flavor = "current_thread")]
async fn durable_authority_commits_terminal_event_before_returning_result() {
  let store = Arc::new(Mutex::new(active_store("run-authority", "inv-authority")));
  let registry = Arc::new(CapabilityRegistry::default());
  registry.register(Arc::new(TestCapabilityHandler)).unwrap();
  let authority = DurableCapabilityAuthority::new(registry, store.clone());
  let call = request(
    "run-authority",
    "inv-authority",
    "call-echo",
    json!({"mode": "echo", "value": {"message": "hello"}}),
  );

  let result = authority
    .execute(call.clone(), CapabilityCancellationToken::default())
    .await
    .unwrap();
  assert!(matches!(result, CapabilityCallResult::Succeeded(_)));
  let projection = store.lock().await.projection("run-authority").unwrap();
  assert!(matches!(
    projection.operations.values().next().unwrap().status,
    OperationStatus::Succeeded { .. }
  ));

  let duplicate = authority
    .execute(call, CapabilityCancellationToken::default())
    .await;
  assert!(
    duplicate.is_err(),
    "duplicate call correlation must fail closed"
  );
}

#[test]
fn recovery_closes_interrupted_managed_operation_as_ambiguous_before_the_attempt() {
  let mut store = active_store("run-recovery", "inv-recovery");
  let call = request(
    "run-recovery",
    "inv-recovery",
    "call-active",
    json!({"mode": "delay"}),
  );
  store
    .append_payload(
      "run-recovery",
      "event-operation-start",
      Utc::now(),
      RunEventPayload::OperationStarted(OperationStartedData {
        node_id: call.node_id,
        attempt_number: call.attempt_number,
        invocation_id: call.invocation_id,
        call_id: call.call_id,
        operation_key: call.identity.operation_key,
        capability: call.capability,
        operation: call.operation,
        execution_mode: OperationExecutionMode::Managed,
        metadata: Map::new(),
      }),
    )
    .unwrap();

  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  let projection = store.projection("run-recovery").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(matches!(
    projection.operations.values().next().unwrap().status,
    OperationStatus::Failed { ref failure, .. }
      if failure.kind == CapabilityFailureKind::Interrupted && failure.ambiguous
  ));
  let events = store.events("run-recovery").unwrap();
  assert!(matches!(
    events[3].payload,
    RunEventPayload::OperationFailed(OperationFailedData { .. })
  ));
}

#[test]
fn recovery_closes_interrupted_observed_fetch_as_ambiguous_before_the_attempt() {
  let mut store = active_store("run-fetch-recovery", "inv-fetch-recovery");
  let call = request(
    "run-fetch-recovery",
    "inv-fetch-recovery",
    "fetch-active",
    json!(null),
  );
  store
    .append_payload(
      "run-fetch-recovery",
      "event-fetch-operation-start",
      Utc::now(),
      RunEventPayload::OperationStarted(OperationStartedData {
        node_id: call.node_id,
        attempt_number: call.attempt_number,
        invocation_id: call.invocation_id,
        call_id: call.call_id,
        operation_key: call.identity.operation_key,
        capability: "http".to_string(),
        operation: "fetch".to_string(),
        execution_mode: OperationExecutionMode::Observed,
        metadata: Map::new(),
      }),
    )
    .unwrap();

  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  let projection = store.projection("run-fetch-recovery").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(matches!(
    projection.operations.values().next().unwrap().status,
    OperationStatus::Failed { ref failure, .. }
      if failure.kind == CapabilityFailureKind::Interrupted
        && failure.ambiguous
        && failure.code == "WOML_NATIVE_FETCH_INTERRUPTED"
  ));
}
