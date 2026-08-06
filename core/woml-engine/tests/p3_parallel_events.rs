use std::path::PathBuf;

use chrono::Utc;
use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::{
  fold_events, AttemptFailureKind, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  InMemoryDagEngine, ParallelGroupOutcome, ParallelGroupStatus, RunEvent, RunEventPayload,
  RunFailure, RunStatus,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const SUCCEEDED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-succeeded.events.v3.json");
const WAIT_ALL_FAILED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-wait-all-failed.events.v3.json");
const FAIL_FAST_FAILED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-fail-fast.events.v3.json");
const HASH: &str = "sha256:d58dfcefdcd6c40db659042c41e17ca6c8d652033f90f120734d5cd95819b45c";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
}

fn events(json: &str) -> Vec<RunEvent> {
  serde_json::from_str(json).unwrap()
}

fn success_events() -> Vec<RunEvent> {
  events(SUCCEEDED)
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new() -> Self {
    Self {
      path: std::env::temp_dir().join(format!("woml-p3-{}.sqlite", Uuid::new_v4())),
    }
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

fn append_fixture_prefix(store: &mut DurableEventStore, fixture: &[RunEvent], length: usize) {
  let first = &fixture[0];
  let RunEventPayload::RunStarted(start) = &first.payload else {
    panic!("fixture must begin with run_started");
  };
  store.register_definition(&model(), HASH).unwrap();
  store
    .start_run(
      first.event_id.clone(),
      first.run_id.clone(),
      first.occurred_at,
      start.workflow_id.clone(),
      start.definition_hash.clone(),
      start.trigger.clone(),
    )
    .unwrap();
  for event in &fixture[1..length] {
    store
      .append_payload(
        event.run_id.clone(),
        event.event_id.clone(),
        event.occurred_at,
        event.payload.clone(),
      )
      .unwrap();
  }
}

#[test]
fn frozen_v3_histories_fold_group_state_and_parallel_failures() {
  let succeeded = success_events();
  for event in &succeeded {
    event.validate().unwrap();
  }
  let projection = fold_events(&succeeded).unwrap();
  assert_eq!(projection.event_schema_version, Some(3));
  assert_eq!(projection.status, RunStatus::Succeeded);
  let group = projection.parallel_groups.get("fieldData").unwrap();
  assert_eq!(
    group.fork_context.steps.keys().cloned().collect::<Vec<_>>(),
    vec!["loadField"]
  );
  assert!(matches!(
    group.status,
    ParallelGroupStatus::Completed {
      outcome: ParallelGroupOutcome::Succeeded,
      ..
    }
  ));
  assert!(projection.context.steps.contains_key("loadWeather"));
  assert!(projection.context.steps.contains_key("loadSoil"));
  assert!(!projection.context.steps.contains_key("fieldData"));

  for fixture in [WAIT_ALL_FAILED, FAIL_FAST_FAILED] {
    let projection = fold_events(&events(fixture)).unwrap();
    assert_eq!(projection.status, RunStatus::Failed);
    assert!(matches!(
      projection.failure,
      Some(RunFailure::Parallel { .. })
    ));
  }
}

#[test]
fn model_v3_starts_event_v3_and_rejects_mixed_versions() {
  let mut engine = InMemoryDagEngine::new_for_event_history(model(), HASH).unwrap();
  let projection = engine
    .start_run("evt_start", "run_v3", Utc::now(), Map::new())
    .unwrap();
  assert_eq!(projection.event_schema_version, Some(3));

  let mut mixed = success_events();
  mixed[4].event_schema_version = 2;
  assert!(fold_events(&mixed[..5])
    .unwrap_err()
    .to_string()
    .contains("mixes event schema versions"));
}

#[test]
fn group_events_reject_duplicate_and_impossible_transitions() {
  let fixture = success_events();
  let mut duplicate_start = fixture[..4].to_vec();
  let mut event = fixture[3].clone();
  event.event_id = "evt_duplicate_start".to_string();
  event.sequence = 5;
  duplicate_start.push(event);
  assert!(fold_events(&duplicate_start)
    .unwrap_err()
    .to_string()
    .contains("started more than once"));

  let mut completion_without_start = vec![fixture[0].clone(), fixture[8].clone()];
  completion_without_start[1].sequence = 2;
  assert!(fold_events(&completion_without_start)
    .unwrap_err()
    .to_string()
    .contains("completed before it started"));

  let mut invalid = fixture[8].clone();
  let RunEventPayload::ParallelGroupCompleted(data) = &mut invalid.payload else {
    panic!("expected completion");
  };
  data.outcome = ParallelGroupOutcome::Failed;
  data.failed_node_ids = vec!["buildReport".to_string()];
  let mut engine = InMemoryDagEngine::new_for_event_history(model(), HASH).unwrap();
  for event in &fixture[..8] {
    engine.append_event(event.clone()).unwrap();
  }
  assert!(engine
    .append_event(invalid)
    .unwrap_err()
    .to_string()
    .contains("outside that group"));
}

#[test]
fn active_child_count_cannot_exceed_compiled_concurrency() {
  let mut workflow = model();
  let start = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "__woml_parallel__fieldData__start")
    .unwrap();
  let woml_engine::model::ValueExpression::Object { fields } = &mut start.inputs else {
    panic!("parallel start inputs must be an object");
  };
  *fields.get_mut("concurrency").unwrap() = woml_engine::model::ValueExpression::Literal {
    value: Value::from(1),
  };
  workflow.validate_structure().unwrap();

  let fixture = success_events();
  let mut engine = InMemoryDagEngine::new_for_event_history(workflow, HASH).unwrap();
  for event in &fixture[..5] {
    engine.append_event(event.clone()).unwrap();
  }
  assert!(engine
    .append_event(fixture[5].clone())
    .unwrap_err()
    .to_string()
    .contains("concurrency cap of 1"));
}

#[test]
fn sqlite_reopen_preserves_safe_parallel_boundaries_exactly() {
  let fixture = success_events();
  for length in [4_usize, 8, 9] {
    let database = TemporaryDatabase::new();
    let mut store = DurableEventStore::open(&database.path).unwrap();
    append_fixture_prefix(&mut store, &fixture, length);
    let before = store.projection(&fixture[0].run_id).unwrap();
    drop(store);

    let mut reopened = DurableEventStore::open(&database.path).unwrap();
    let after = reopened.projection(&fixture[0].run_id).unwrap();
    assert_eq!(after, before, "prefix length {length}");
    let report = reopened.recover_interrupted_runs().unwrap();
    assert_eq!(report.recovered_runs, 0, "prefix length {length}");
    assert_eq!(report.resumable_runs, 1, "prefix length {length}");
    assert_eq!(
      reopened.events(&fixture[0].run_id).unwrap().len(),
      length,
      "safe recovery must not synthesize events"
    );
  }
}

#[test]
fn recovery_fails_in_flight_parallel_attempt_without_replaying_successes() {
  let fixture = success_events();
  let mut partial = fixture[..5].to_vec();
  partial.push(fixture[7].clone());
  partial[5].sequence = 6;
  partial[5].event_id = "evt_weather_success".to_string();
  let RunEventPayload::StepAttemptSucceeded(success) = &mut partial[5].payload else {
    panic!("expected success event");
  };
  success.node_id = "loadWeather".to_string();
  success.invocation_id = "inv_weather_01".to_string();

  let database = TemporaryDatabase::new();
  let mut store = DurableEventStore::open(&database.path).unwrap();
  append_fixture_prefix(&mut store, &partial, partial.len());
  store
    .append_payload(
      "run_parallel_01",
      "evt_soil_started",
      Utc::now(),
      fixture[5].payload.clone(),
    )
    .unwrap();
  drop(store);

  let mut reopened = DurableEventStore::open(&database.path).unwrap();
  let report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 1);
  let projection = reopened.projection("run_parallel_01").unwrap();
  assert_eq!(projection.status, RunStatus::Running);
  assert!(projection.context.steps.contains_key("loadWeather"));
  assert!(!projection.context.steps.contains_key("loadSoil"));
  assert!(projection.attempts.iter().any(|attempt| {
    attempt.identity.node_id == "loadSoil"
      && matches!(
        &attempt.status,
        woml_engine::projection::AttemptStatus::Failed { failure }
          if failure.kind == AttemptFailureKind::Interrupted
      )
  }));

  let engine = DurableDagEngine::resume(reopened, "run_parallel_01").unwrap();
  let ready = engine.ready_node_ids("run_parallel_01").unwrap();
  assert!(!ready.iter().any(|node_id| node_id == "loadWeather"));
  assert!(!ready.iter().any(|node_id| node_id == "loadSoil"));
}
