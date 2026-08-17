use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::{
  fold_events, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, DurableStoreError,
  RunEvent, RunEventPayload, RunStatus, TriggerAdmissionRequest, COMPILED_MODEL_SCHEMA_VERSION_V7,
  RUN_EVENT_SCHEMA_VERSION_V7,
};

const WEBHOOK_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-webhook.compiled.v7.json");
const SLACK_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-slack.compiled.v7.json");
const SCHEDULE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-schedule.compiled.v7.json");
const INTERVAL_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-interval.compiled.v7.json");
const EVENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-event.compiled.v7.json");
const EVENT_HISTORY: &str =
  include_str!("../../../woml/tests/fixtures/run-events/webhook-trigger.events.v7.json");
const WEBHOOK_HASH: &str =
  "sha256:4b4899d13cefc7ed88033d24c898549a4eb8862bebf4a73ed1c26f0af99bd082";

fn model(source: &str) -> CompiledWorkflowDefinition {
  serde_json::from_str(source).unwrap()
}

fn payload(order_id: &str) -> Map<String, Value> {
  Map::from_iter([("orderId".to_string(), json!(order_id))])
}

fn request(source_identity: &str, order_id: &str) -> TriggerAdmissionRequest {
  TriggerAdmissionRequest {
    workflow_id: "webhook-trigger-contract".to_string(),
    definition_hash: WEBHOOK_HASH.to_string(),
    trigger_id: "newOrder".to_string(),
    trigger_handler: "trigger.webhook".to_string(),
    source_identity: source_identity.to_string(),
    payload: payload(order_id),
    received_at: Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap(),
  }
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-t2-{label}-{}.sqlite",
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
fn rust_validates_every_frozen_model_v7_trigger_shape() {
  for source in [
    WEBHOOK_MODEL,
    SLACK_MODEL,
    SCHEDULE_MODEL,
    INTERVAL_MODEL,
    EVENT_MODEL,
  ] {
    let workflow = model(source);
    assert_eq!(workflow.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V7);
    workflow.validate_structure().unwrap();
    workflow.validate_for_durable_execution().unwrap();
  }

  let mut malformed = model(WEBHOOK_MODEL);
  malformed.triggers[1].handler = "trigger.unknown".to_string();
  assert!(malformed.validate_structure().is_err());

  let mut resolved_secret = model(WEBHOOK_MODEL);
  let woml_engine::model::ValueExpression::Object { fields } =
    &mut resolved_secret.triggers[1].config
  else {
    panic!("webhook config must be an object");
  };
  let woml_engine::model::ValueExpression::Object {
    fields: authentication,
  } = fields.get_mut("authentication").unwrap()
  else {
    panic!("webhook authentication must be an object");
  };
  authentication.insert(
    "secret".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!("resolved-secret"),
    },
  );
  assert!(resolved_secret.validate_structure().is_err());
}

#[test]
fn event_v7_fixture_folds_the_direct_trigger_and_occurrence_identity() {
  let history: Vec<RunEvent> = serde_json::from_str(EVENT_HISTORY).unwrap();
  assert!(history
    .iter()
    .all(|event| event.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V7));
  let projection = fold_events(&history).unwrap();
  assert_eq!(projection.status, RunStatus::Succeeded);
  assert_eq!(projection.trigger_id.as_deref(), Some("newOrder"));
  assert_eq!(
    projection.trigger_handler.as_deref(),
    Some("trigger.webhook")
  );
  assert_eq!(
    projection.trigger_occurrence_id.as_deref(),
    Some("occ_webhook_001")
  );
  assert_eq!(projection.context.trigger, payload("order-42"));
}

#[test]
fn one_admission_atomically_creates_the_occurrence_run_and_start_event() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
    .unwrap();

  let outcome = store
    .admit_trigger_occurrence(request("order-delivery-123", "order-42"))
    .unwrap();
  assert!(!outcome.duplicate);

  let occurrence = store.trigger_occurrence(&outcome.occurrence_id).unwrap();
  assert_eq!(occurrence.run_id, outcome.run_id);
  assert_eq!(occurrence.definition_hash, WEBHOOK_HASH);
  assert_eq!(
    occurrence.source_identity_hash,
    "sha256:f58c4282a8670ec0528685b21d21f8e426a15629d7b91ba850a9c60782c30b09"
  );
  assert_eq!(
    occurrence.payload_hash,
    "sha256:c5fbc3b263e2a89956ed3f7fe3cc81d3e7d0925adc8fc14af3ef374850bdd976"
  );

  let events = store.events(&outcome.run_id).unwrap();
  assert_eq!(events.len(), 1);
  assert_eq!(events[0].event_schema_version, RUN_EVENT_SCHEMA_VERSION_V7);
  let RunEventPayload::RunStarted(start) = &events[0].payload else {
    panic!("the atomic event must be run_started");
  };
  assert_eq!(start.trigger_id.as_deref(), Some("newOrder"));
  assert_eq!(
    start.trigger_occurrence_id.as_deref(),
    Some(outcome.occurrence_id.as_str())
  );
  assert_eq!(start.trigger, payload("order-42"));
  assert_eq!(
    store.projection(&outcome.run_id).unwrap().context.trigger,
    payload("order-42")
  );
  assert!(!serde_json::to_string(&occurrence)
    .unwrap()
    .contains("order-delivery-123"));
  assert!(!serde_json::to_string(&events)
    .unwrap()
    .contains("order-delivery-123"));
}

#[test]
fn rejection_before_the_transaction_creates_nothing() {
  let database = TemporaryDatabase::new("pre-transaction");
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
    .unwrap();
  let mut invalid = request("", "order-42");
  invalid.source_identity = String::new();
  assert!(matches!(
    store.admit_trigger_occurrence(invalid),
    Err(DurableStoreError::Contract(_))
  ));
  drop(store);

  let connection = Connection::open(database.path()).unwrap();
  for table in ["woml_runs", "woml_run_events", "woml_trigger_occurrences"] {
    let count: i64 = connection
      .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
      })
      .unwrap();
    assert_eq!(
      count, 0,
      "{table} must remain empty after preflight rejection"
    );
  }
}

#[test]
fn payload_hash_is_canonical_across_object_key_order() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
    .unwrap();

  let mut first = request("canonical-key", "order-42");
  first.payload.insert("quantity".to_string(), json!(2));
  let accepted = store.admit_trigger_occurrence(first).unwrap();

  let mut replay = request("canonical-key", "order-42");
  replay.payload = Map::from_iter([
    ("quantity".to_string(), json!(2)),
    ("orderId".to_string(), json!("order-42")),
  ]);
  let duplicate = store.admit_trigger_occurrence(replay).unwrap();
  assert!(duplicate.duplicate);
  assert_eq!(duplicate.run_id, accepted.run_id);
}

#[test]
fn duplicate_and_conflict_are_distinct_and_definition_updates_do_not_replay() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
    .unwrap();
  let first = store
    .admit_trigger_occurrence(request("stable-key", "order-42"))
    .unwrap();
  let duplicate = store
    .admit_trigger_occurrence(request("stable-key", "order-42"))
    .unwrap();
  assert!(duplicate.duplicate);
  assert_eq!(duplicate.run_id, first.run_id);
  assert_eq!(duplicate.occurrence_id, first.occurrence_id);
  assert_eq!(store.events(&first.run_id).unwrap().len(), 1);

  assert!(matches!(
    store.admit_trigger_occurrence(request("stable-key", "changed")),
    Err(DurableStoreError::TriggerIdempotencyConflict)
  ));
  assert_eq!(store.events(&first.run_id).unwrap().len(), 1);

  let updated_hash = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  let mut updated = model(WEBHOOK_MODEL);
  let woml_engine::model::ValueExpression::Object { fields } = &mut updated.graph.nodes[0].inputs
  else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!("return { updated: true };"),
    },
  );
  store.register_definition(&updated, updated_hash).unwrap();
  let mut after_update = request("stable-key", "order-42");
  after_update.definition_hash = updated_hash.to_string();
  let duplicate = store.admit_trigger_occurrence(after_update).unwrap();
  assert!(duplicate.duplicate);
  assert_eq!(duplicate.run_id, first.run_id);
  assert_eq!(
    store
      .trigger_occurrence(&duplicate.occurrence_id)
      .unwrap()
      .definition_hash,
    WEBHOOK_HASH
  );
}

#[test]
fn concurrent_identical_submissions_create_one_run_and_one_start_event() {
  let database = TemporaryDatabase::new("concurrent");
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
      .unwrap();
  }
  let barrier = Arc::new(Barrier::new(3));
  let handles = (0..2)
    .map(|_| {
      let path = database.path().to_path_buf();
      let barrier = Arc::clone(&barrier);
      thread::spawn(move || {
        let mut store = DurableEventStore::open(path).unwrap();
        barrier.wait();
        store
          .admit_trigger_occurrence(request("concurrent-key", "order-42"))
          .unwrap()
      })
    })
    .collect::<Vec<_>>();
  barrier.wait();
  let outcomes = handles
    .into_iter()
    .map(|handle| handle.join().unwrap())
    .collect::<Vec<_>>();
  assert_eq!(
    outcomes.iter().filter(|outcome| !outcome.duplicate).count(),
    1
  );
  assert_eq!(
    outcomes.iter().filter(|outcome| outcome.duplicate).count(),
    1
  );
  assert_eq!(outcomes[0].run_id, outcomes[1].run_id);
  assert_eq!(outcomes[0].occurrence_id, outcomes[1].occurrence_id);

  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(store.events(&outcomes[0].run_id).unwrap().len(), 1);
}

#[test]
fn failure_during_the_transaction_rolls_back_run_event_and_occurrence() {
  let database = TemporaryDatabase::new("rollback");
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
      .unwrap();
  }
  {
    let connection = Connection::open(database.path()).unwrap();
    connection
      .execute_batch(
        "CREATE TRIGGER woml_test_abort_occurrence
         BEFORE INSERT ON woml_trigger_occurrences
         BEGIN SELECT RAISE(ABORT, 'simulated crash before occurrence commit'); END;",
      )
      .unwrap();
  }
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    assert!(matches!(
      store.admit_trigger_occurrence(request("rollback-key", "order-42")),
      Err(DurableStoreError::Sqlite(_))
    ));
  }
  let connection = Connection::open(database.path()).unwrap();
  for table in ["woml_runs", "woml_run_events", "woml_trigger_occurrences"] {
    let count: i64 = connection
      .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
      })
      .unwrap();
    assert_eq!(count, 0, "{table} must roll back atomically");
  }
}

#[test]
fn restart_recovers_a_committed_run_that_was_never_dispatched() {
  let database = TemporaryDatabase::new("recovery");
  let accepted = {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
      .unwrap();
    store
      .admit_trigger_occurrence(request("recovery-key", "order-42"))
      .unwrap()
  };

  let recovery = {
    let store = DurableEventStore::open(database.path()).unwrap();
    store.recover_undispatched_trigger_runs().unwrap()
  };
  assert_eq!(recovery.len(), 1);
  assert_eq!(recovery[0].occurrence.run_id, accepted.run_id);
  assert_eq!(recovery[0].trigger, payload("order-42"));

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::resume(store, &accepted.run_id).unwrap();
    engine
      .start_step_attempt(
        &accepted.run_id,
        "capture",
        1,
        "invocation_recovered_001",
        Utc::now(),
      )
      .unwrap();
  }
  let store = DurableEventStore::open(database.path()).unwrap();
  assert!(store
    .recover_undispatched_trigger_runs()
    .unwrap()
    .is_empty());
}

#[test]
fn corrupt_occurrence_history_fails_closed() {
  let database = TemporaryDatabase::new("corrupt");
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&model(WEBHOOK_MODEL), WEBHOOK_HASH)
      .unwrap();
    store
      .admit_trigger_occurrence(request("corrupt-key", "order-42"))
      .unwrap();
  }
  {
    let connection = Connection::open(database.path()).unwrap();
    connection
      .execute_batch(
        "DROP TRIGGER woml_trigger_occurrences_no_update;
         UPDATE woml_trigger_occurrences
         SET occurrence_id = 'occ_corrupt000000000000000000000000';
         CREATE TRIGGER woml_trigger_occurrences_no_update
         BEFORE UPDATE ON woml_trigger_occurrences
         BEGIN
           SELECT RAISE(ABORT, 'WOML trigger occurrences are immutable');
         END;",
      )
      .unwrap();
  }
  let mut store = DurableEventStore::open(database.path()).unwrap();
  assert!(matches!(
    store.admit_trigger_occurrence(request("corrupt-key", "order-42")),
    Err(DurableStoreError::TriggerHistoryInvalid(_))
  ));
}
