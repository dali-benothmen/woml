use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};
use woml_engine::event::{RunEventPayload, StepAttemptFailedData};
use woml_engine::model::{BackoffPolicy, EdgeCondition, ModelIssueCode, RetryPolicy};
use woml_engine::{
  fold_events, AttemptFailureKind, CompiledWorkflowDefinition, InMemoryDagEngine,
  InMemoryEventStore, RunEvent, RunStatus,
};

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/hello.events.v1.json");
const HOST_CRASHED_EVENT: &str =
  include_str!("../../../woml/tests/fixtures/run-events/host-crashed.event.v1.json");
const INTERRUPTED_EVENT: &str =
  include_str!("../../../woml/tests/fixtures/run-events/interrupted.event.v1.json");
const HELLO_HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";

fn hello_model() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(HELLO_MODEL).expect("hello model must deserialize")
}

fn hello_events() -> Vec<RunEvent> {
  serde_json::from_str(HELLO_EVENTS).expect("hello events must deserialize")
}

#[test]
fn accepts_the_existing_compiled_model_fixture_unchanged() {
  let original: Value = serde_json::from_str(HELLO_MODEL).unwrap();
  let model = hello_model();

  model.validate_for_execution().unwrap();
  assert_eq!(model.schema_version, 1);
  assert_eq!(model.workflow_id, "hello");
  assert_eq!(
    model
      .metadata
      .as_ref()
      .and_then(|metadata| metadata.version.as_deref()),
    Some("0.1")
  );
  assert_eq!(model.graph.entry_node_ids, ["a"]);
  assert_eq!(model.terminal_node_id(), Some("b"));
  assert_eq!(serde_json::to_value(model).unwrap(), original);
}

#[test]
fn independently_rejects_bad_versions_missing_nodes_and_cycles() {
  let mut bad_version = hello_model();
  bad_version.schema_version = 2;
  let codes: Vec<_> = bad_version
    .validate_for_execution()
    .unwrap_err()
    .issues
    .into_iter()
    .map(|issue| issue.code)
    .collect();
  assert!(codes.contains(&ModelIssueCode::UnsupportedSchemaVersion));

  let mut missing_endpoint = hello_model();
  missing_endpoint.graph.edges[0].to = "missing".to_string();
  let codes: Vec<_> = missing_endpoint
    .validate_for_execution()
    .unwrap_err()
    .issues
    .into_iter()
    .map(|issue| issue.code)
    .collect();
  assert!(codes.contains(&ModelIssueCode::UnknownEdgeEndpoint));

  let mut cyclic = hello_model();
  cyclic
    .graph
    .edges
    .push(woml_engine::model::CompiledWorkflowEdge {
      id: "b-to-a".to_string(),
      from: "b".to_string(),
      to: "a".to_string(),
      condition: EdgeCondition::Always,
      branch_id: None,
    });
  let codes: Vec<_> = cyclic
    .validate_for_execution()
    .unwrap_err()
    .issues
    .into_iter()
    .map(|issue| issue.code)
    .collect();
  assert!(codes.contains(&ModelIssueCode::CyclicGraph));
}

#[test]
fn rejects_every_staged_execution_construct_in_r2() {
  let mut unknown_handler = hello_model();
  unknown_handler.graph.nodes[0].handler = "services.http".to_string();
  assert!(unknown_handler
    .validate_for_execution()
    .unwrap_err()
    .issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnknownHandler));

  let mut unsupported_inputs = hello_model();
  unsupported_inputs.graph.nodes[0].inputs =
    woml_engine::model::ValueExpression::ContextReference {
      path: vec!["trigger".to_string()],
    };
  assert!(unsupported_inputs
    .validate_for_execution()
    .unwrap_err()
    .issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedNodeInputs));

  let mut retry = hello_model();
  retry.graph.nodes[0].retry_policy = Some(RetryPolicy {
    max_attempts: 2,
    backoff: BackoffPolicy::None,
  });
  assert!(retry
    .validate_for_execution()
    .unwrap_err()
    .issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedRetry));

  let mut branch = hello_model();
  branch.graph.edges[0].condition = EdgeCondition::Truthy {
    value: woml_engine::model::ValueExpression::Literal { value: json!(true) },
  };
  branch.graph.edges[0].branch_id = Some("decision".to_string());
  let branch_issues = branch.validate_for_execution().unwrap_err().issues;
  assert!(branch_issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedEdgeCondition));
  assert!(branch_issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedBranch));

  let mut parallel = hello_model();
  let mut third = parallel.graph.nodes[1].clone();
  third.id = "c".to_string();
  parallel.graph.nodes.push(third);
  parallel
    .graph
    .edges
    .push(woml_engine::model::CompiledWorkflowEdge {
      id: "a-to-c".to_string(),
      from: "a".to_string(),
      to: "c".to_string(),
      condition: EdgeCondition::Always,
      branch_id: None,
    });
  assert!(parallel
    .validate_for_execution()
    .unwrap_err()
    .issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedNonSequentialDag));
}

#[test]
fn folding_is_pure_deterministic_and_publishes_only_successes() {
  let events = hello_events();
  let before_success = fold_events(&events[..2]).unwrap();
  assert!(before_success.context.steps.is_empty());

  let after_a = fold_events(&events[..3]).unwrap();
  assert_eq!(
    after_a.context.steps.get("a"),
    Some(&json!({ "x": "World" }))
  );
  assert!(!after_a.context.steps.contains_key("b"));

  let first = fold_events(&events).unwrap();
  let second = fold_events(&events).unwrap();
  assert_eq!(first, second);
  assert_eq!(first.status, RunStatus::Succeeded);
  assert_eq!(first.result, Some(json!({ "message": "Hello World" })));
  assert_eq!(first.context.trigger, Map::new());
  assert_eq!(first.context.steps.len(), 2);
}

#[test]
fn a_failed_attempt_publishes_no_output_and_is_not_replayed() {
  let events = hello_events();
  let mut failure: RunEvent = serde_json::from_str(HOST_CRASHED_EVENT).unwrap();
  failure.run_id = events[0].run_id.clone();
  failure.event_id = "evt_hello_failed_003".to_string();
  let RunEventPayload::StepAttemptFailed(data) = &mut failure.payload else {
    panic!("expected failed attempt");
  };
  data.invocation_id = "inv_hello_a_01".to_string();

  let history = vec![events[0].clone(), events[1].clone(), failure.clone()];
  let projection = fold_events(&history).unwrap();
  assert!(projection.context.steps.is_empty());

  let mut engine = InMemoryDagEngine::new(hello_model(), HELLO_HASH).unwrap();
  for event in history {
    engine.append_event(event).unwrap();
  }
  assert!(engine.ready_node_ids("run_hello_01").unwrap().is_empty());

  let mut retry = events[1].clone();
  retry.event_id = "evt_hello_retry_004".to_string();
  retry.sequence = 4;
  let RunEventPayload::StepAttemptStarted(data) = &mut retry.payload else {
    panic!("expected started attempt");
  };
  data.attempt = 2;
  data.invocation_id = "inv_hello_a_02".to_string();
  assert!(engine.append_event(retry).is_err());
}

#[test]
fn canonical_host_crash_and_interrupted_failures_remain_distinct() {
  let host: RunEvent = serde_json::from_str(HOST_CRASHED_EVENT).unwrap();
  let interrupted: RunEvent = serde_json::from_str(INTERRUPTED_EVENT).unwrap();
  host.validate().unwrap();
  interrupted.validate().unwrap();

  let RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
    failure: host_failure,
    ..
  }) = host.payload
  else {
    panic!("expected failed attempt");
  };
  let RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
    failure: interrupted_failure,
    ..
  }) = interrupted.payload
  else {
    panic!("expected failed attempt");
  };
  assert_eq!(host_failure.kind, AttemptFailureKind::HostCrashed);
  assert_eq!(interrupted_failure.kind, AttemptFailureKind::Interrupted);
  assert_ne!(host_failure.code, interrupted_failure.code);
}

#[test]
fn event_store_is_append_only_and_rejects_an_invalid_append_atomically() {
  let events = hello_events();
  let mut store = InMemoryEventStore::default();
  store.append(events[0].clone()).unwrap();

  let invalid = events[2].clone();
  assert!(store.append(invalid).is_err());
  assert_eq!(store.events("run_hello_01").len(), 1);

  store.append(events[1].clone()).unwrap();
  assert!(store.append(events[1].clone()).is_err());
  assert_eq!(store.events("run_hello_01").len(), 2);
}

#[test]
fn engine_selects_ready_nodes_in_model_order_and_blocks_in_flight_work() {
  let events = hello_events();
  let mut engine = InMemoryDagEngine::new(hello_model(), HELLO_HASH).unwrap();

  engine.append_event(events[0].clone()).unwrap();
  assert_eq!(engine.ready_node_ids("run_hello_01").unwrap(), ["a"]);

  engine.append_event(events[1].clone()).unwrap();
  assert!(engine.ready_node_ids("run_hello_01").unwrap().is_empty());

  engine.append_event(events[2].clone()).unwrap();
  assert_eq!(engine.ready_node_ids("run_hello_01").unwrap(), ["b"]);

  engine.append_event(events[3].clone()).unwrap();
  assert!(engine.ready_node_ids("run_hello_01").unwrap().is_empty());
  engine.append_event(events[4].clone()).unwrap();
  assert!(engine.ready_node_ids("run_hello_01").unwrap().is_empty());
  engine.append_event(events[5].clone()).unwrap();
  assert_eq!(
    engine.projection("run_hello_01").unwrap().status,
    RunStatus::Succeeded
  );
}

#[test]
fn engine_can_start_a_run_without_creating_mutable_context_state() {
  let mut engine = InMemoryDagEngine::new(hello_model(), HELLO_HASH).unwrap();
  let mut trigger = Map::new();
  trigger.insert("name".to_string(), json!("Dali"));
  let projection = engine
    .start_run(
      "evt_new_001",
      "run_new_01",
      Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap(),
      trigger.clone(),
    )
    .unwrap();

  assert_eq!(projection.context.trigger, trigger);
  assert!(projection.context.steps.is_empty());
  assert_eq!(engine.ready_node_ids("run_new_01").unwrap(), ["a"]);
  assert_eq!(engine.events("run_new_01").len(), 1);
}

#[test]
fn engine_rejects_starting_a_node_before_its_dependencies_succeed() {
  let events = hello_events();
  let mut engine = InMemoryDagEngine::new(hello_model(), HELLO_HASH).unwrap();
  engine.append_event(events[0].clone()).unwrap();

  let error = engine.append_event(events[3].clone()).unwrap_err();
  assert!(error.to_string().contains("not ready"));
  assert_eq!(engine.events("run_hello_01").len(), 1);
}
