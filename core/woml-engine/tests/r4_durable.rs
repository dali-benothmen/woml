use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::event::{RunStartedData, StepAttemptFailedData, StepAttemptStartedData};
use woml_engine::projection::AttemptStatus;
use woml_engine::{
  fold_events, AttemptFailure, AttemptFailureKind, CompiledWorkflowDefinition, DurableDagEngine,
  DurableEventStore, DurableStoreError, RunEvent, RunEventPayload, RunStatus,
};

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/hello.events.v1.json");
const HELLO_HASH: &str = "sha256:74d4a6799119042d1cdcf2ed3e1e8e30228b3fbb80ad6750c1256ebd335b03ae";

fn hello_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(HELLO_MODEL).unwrap()
}

fn hello_events() -> Vec<RunEvent> {
  serde_json::from_str(HELLO_EVENTS).unwrap()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-r4-{label}-{}.sqlite",
        Uuid::new_v4().simple()
      )),
    }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

#[test]
fn completed_run_reopens_to_the_exact_projection_from_events_alone() {
  let database = TemporaryDatabase::new("completed");
  let fixture_events = hello_events();
  let expected = fold_events(&fixture_events).unwrap();

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(hello_model(), HELLO_HASH, store).unwrap();
    for event in &fixture_events {
      match &event.payload {
        RunEventPayload::RunStarted(RunStartedData { trigger, .. }) => {
          engine
            .start_run(
              event.event_id.clone(),
              event.run_id.clone(),
              event.occurred_at,
              trigger.clone(),
            )
            .unwrap();
        }
        payload => {
          engine
            .append_payload(
              event.event_id.clone(),
              &event.run_id,
              event.occurred_at,
              payload.clone(),
            )
            .unwrap();
        }
      }
    }
    assert_eq!(engine.projection("run_hello_01").unwrap(), expected);
  }

  let mut reopened = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(reopened.projection("run_hello_01").unwrap(), expected);
  assert_eq!(reopened.events("run_hello_01").unwrap(), fixture_events);
  assert_eq!(
    reopened
      .run_binding("run_hello_01")
      .unwrap()
      .definition_hash,
    HELLO_HASH
  );
  assert_eq!(reopened.definition(HELLO_HASH).unwrap(), hello_model());

  let report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 0);
  assert_eq!(reopened.projection("run_hello_01").unwrap(), expected);
}

#[test]
fn recovery_atomically_fails_an_uncertain_attempt_without_replaying_it() {
  let database = TemporaryDatabase::new("interrupted");
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(hello_model(), HELLO_HASH, store).unwrap();
    engine
      .start_run(
        "evt_interrupted_start",
        "run_interrupted",
        Utc::now(),
        Map::new(),
      )
      .unwrap();
    engine
      .append_payload(
        "evt_interrupted_attempt",
        "run_interrupted",
        Utc::now(),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: "a".to_string(),
          attempt: 1,
          invocation_id: "inv_uncertain_a".to_string(),
          handler: "runtime.script".to_string(),
        }),
      )
      .unwrap();
  }

  let mut reopened = DurableEventStore::open(database.path()).unwrap();
  let report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 1);

  let events = reopened.events("run_interrupted").unwrap();
  assert_eq!(events.len(), 4);
  for event in &events {
    event.validate().unwrap();
  }
  assert!(matches!(
    events[2].payload,
    RunEventPayload::StepAttemptFailed(_)
  ));
  assert!(matches!(events[3].payload, RunEventPayload::RunFailed(_)));

  let projection = reopened.projection("run_interrupted").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(projection.context.steps.is_empty());
  assert_eq!(
    projection.failure.as_ref().map(|failure| failure.kind),
    Some(AttemptFailureKind::Interrupted)
  );
  assert!(matches!(
    projection.attempts[0].status,
    AttemptStatus::Failed { .. }
  ));

  let second_report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(second_report.recovered_runs, 0);
  assert_eq!(reopened.events("run_interrupted").unwrap().len(), 4);

  let resumed = DurableDagEngine::resume(reopened, "run_interrupted").unwrap();
  assert!(resumed
    .ready_node_ids("run_interrupted")
    .unwrap()
    .is_empty());
}

#[test]
fn invalid_append_rolls_back_without_consuming_a_sequence_number() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&hello_model(), HELLO_HASH)
    .unwrap();
  store
    .start_run(
      "evt_atomic_1",
      "run_atomic",
      Utc::now(),
      "hello",
      HELLO_HASH,
      Map::new(),
    )
    .unwrap();

  let success_without_start = hello_events()[2].payload.clone();
  assert!(store
    .append_payload(
      "run_atomic",
      "evt_atomic_invalid",
      Utc::now(),
      success_without_start,
    )
    .is_err());
  assert_eq!(store.events("run_atomic").unwrap().len(), 1);

  let (_, projection) = store
    .append_payload(
      "run_atomic",
      "evt_atomic_2",
      Utc::now(),
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "a".to_string(),
        attempt: 1,
        invocation_id: "inv_atomic_a".to_string(),
        handler: "runtime.script".to_string(),
      }),
    )
    .unwrap();
  assert_eq!(projection.last_sequence, 2);
  assert_eq!(store.events("run_atomic").unwrap()[1].sequence, 2);
}

#[test]
fn run_and_definition_bindings_are_immutable() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let workflow = hello_model();
  store.register_definition(&workflow, HELLO_HASH).unwrap();
  store
    .start_run(
      "evt_binding_1",
      "run_binding",
      Utc::now(),
      "hello",
      HELLO_HASH,
      Map::new(),
    )
    .unwrap();

  let duplicate = store.start_run(
    "evt_binding_2",
    "run_binding",
    Utc::now(),
    "hello",
    HELLO_HASH,
    Map::new(),
  );
  assert!(matches!(
    duplicate,
    Err(DurableStoreError::RunAlreadyExists(_))
  ));

  let mut changed = workflow;
  changed.metadata.as_mut().unwrap().name = Some("Different model".to_string());
  let conflict = store.register_definition(&changed, HELLO_HASH);
  assert!(matches!(
    conflict,
    Err(DurableStoreError::DefinitionConflict(_))
  ));
  assert_eq!(
    store.run_binding("run_binding").unwrap().definition_hash,
    HELLO_HASH
  );
}

#[test]
fn a_run_without_an_in_flight_attempt_remains_safely_resumable() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&hello_model(), HELLO_HASH)
    .unwrap();
  store
    .start_run(
      "evt_resumable_1",
      "run_resumable",
      Utc::now(),
      "hello",
      HELLO_HASH,
      Map::from_iter([("name".to_string(), json!("Dali"))]),
    )
    .unwrap();

  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.resumable_runs, 1);
  assert_eq!(report.interrupted_attempts, 0);
  assert_eq!(store.events("run_resumable").unwrap().len(), 1);
  assert_eq!(
    store.projection("run_resumable").unwrap().status,
    RunStatus::Running
  );
}

#[test]
fn recovery_finishes_a_run_when_only_the_safe_terminal_event_was_missing() {
  let database = TemporaryDatabase::new("missing-run-success");
  let fixture_events = hello_events();
  let expected = fold_events(&fixture_events).unwrap();
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(hello_model(), HELLO_HASH, store).unwrap();
    for event in fixture_events.iter().take(5) {
      match &event.payload {
        RunEventPayload::RunStarted(RunStartedData { trigger, .. }) => {
          engine
            .start_run(
              event.event_id.clone(),
              event.run_id.clone(),
              event.occurred_at,
              trigger.clone(),
            )
            .unwrap();
        }
        payload => {
          engine
            .append_payload(
              event.event_id.clone(),
              &event.run_id,
              event.occurred_at,
              payload.clone(),
            )
            .unwrap();
        }
      }
    }
  }

  let mut reopened = DurableEventStore::open(database.path()).unwrap();
  let report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 0);
  assert_eq!(reopened.projection("run_hello_01").unwrap(), expected);
  assert_eq!(reopened.events("run_hello_01").unwrap().len(), 6);
  assert_eq!(
    reopened.recover_interrupted_runs().unwrap().recovered_runs,
    0
  );
}

#[test]
fn recovery_finishes_a_known_failure_without_reclassifying_it_as_interrupted() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&hello_model(), HELLO_HASH)
    .unwrap();
  store
    .start_run(
      "evt_known_failure_1",
      "run_known_failure",
      Utc::now(),
      "hello",
      HELLO_HASH,
      Map::new(),
    )
    .unwrap();
  store
    .append_payload(
      "run_known_failure",
      "evt_known_failure_2",
      Utc::now(),
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "a".to_string(),
        attempt: 1,
        invocation_id: "inv_known_failure".to_string(),
        handler: "runtime.script".to_string(),
      }),
    )
    .unwrap();
  let failure = AttemptFailure {
    kind: AttemptFailureKind::ScriptThrew,
    code: AttemptFailureKind::ScriptThrew.code().to_string(),
    message: "boom".to_string(),
    details: None,
  };
  store
    .append_payload(
      "run_known_failure",
      "evt_known_failure_3",
      Utc::now(),
      RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
        node_id: "a".to_string(),
        attempt: 1,
        invocation_id: "inv_known_failure".to_string(),
        failure: failure.clone(),
      }),
    )
    .unwrap();

  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 0);
  assert_eq!(store.events("run_known_failure").unwrap().len(), 4);
  let projection = store.projection("run_known_failure").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert_eq!(projection.failure, Some(failure));
}

#[test]
fn sqlite_itself_rejects_mutation_of_durable_records() {
  let database = TemporaryDatabase::new("append-only");
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&hello_model(), HELLO_HASH)
      .unwrap();
    store
      .start_run(
        "evt_append_only_1",
        "run_append_only",
        Utc::now(),
        "hello",
        HELLO_HASH,
        Map::new(),
      )
      .unwrap();
  }

  let connection = Connection::open(database.path()).unwrap();
  assert!(connection
    .execute(
      "UPDATE woml_run_events SET sequence = 2 WHERE run_id = 'run_append_only'",
      [],
    )
    .is_err());
  assert!(connection
    .execute(
      "UPDATE woml_runs SET workflow_id = 'different' WHERE run_id = 'run_append_only'",
      [],
    )
    .is_err());
  assert!(connection
    .execute(
      "DELETE FROM woml_definitions WHERE definition_hash = ?1",
      [HELLO_HASH],
    )
    .is_err());
}
