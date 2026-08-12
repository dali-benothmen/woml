use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::{
  derive_workflow_call_key, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  DurableStoreError, TriggerAdmissionRequest, WorkflowCallAdmissionRequest,
  DURABLE_STORE_SCHEMA_VERSION,
};

const PARENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/request-risk.compiled.v8.json");
const CHILD_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/calculate-risk.compiled.v10.json");
const PARENT_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CHILD_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-wc7-{label}-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    for suffix in ["", "-wal", "-shm"] {
      let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
    }
  }
}

fn model(source: &str) -> CompiledWorkflowDefinition {
  serde_json::from_str(source).unwrap()
}

fn seed_call(path: &Path) -> (String, WorkflowCallAdmissionRequest) {
  let now = Utc::now();
  let mut store = DurableEventStore::open(path).unwrap();
  store
    .register_definition(&model(PARENT_MODEL), PARENT_HASH)
    .unwrap();
  store
    .register_definition(&model(CHILD_MODEL), CHILD_HASH)
    .unwrap();
  let parent = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: "request-risk".to_string(),
      definition_hash: PARENT_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: format!("wc7:{}", Uuid::new_v4().simple()),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &parent.run_id).unwrap();
  engine
    .start_step_attempt(&parent.run_id, "requestRisk", 1, "inv_wc7_parent", now)
    .unwrap();
  let mut store = engine.into_store();
  let call_key = derive_workflow_call_key(
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "calculate-risk",
    "call1",
  );
  let request = WorkflowCallAdmissionRequest {
    child_run_id: format!("run_call_{}", call_key.strip_prefix("sha256:").unwrap()),
    call_key,
    parent_run_id: parent.run_id.clone(),
    parent_node_id: "requestRisk".to_string(),
    parent_attempt: 1,
    target_workflow_id: "calculate-risk".to_string(),
    target_definition_hash: CHILD_HASH.to_string(),
    payload: Map::from_iter([("customerId".to_string(), json!("customer-42"))]),
    admitted_at: now,
  };
  store.admit_workflow_call(request.clone()).unwrap();
  (parent.run_id, request)
}

fn schema_version(path: &Path) -> String {
  Connection::open(path)
    .unwrap()
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn workflow_call_identity_is_immutable_and_durable() {
  let database = TemporaryDatabase::new("immutable");
  let (_, request) = seed_call(database.path());
  let connection = Connection::open(database.path()).unwrap();

  let update = connection
    .execute(
      "UPDATE woml_workflow_calls SET target_workflow_id = 'tampered' WHERE call_key = ?1",
      [&request.call_key],
    )
    .unwrap_err();
  assert!(update.to_string().contains("identity is immutable"));
  let delete = connection
    .execute(
      "DELETE FROM woml_workflow_calls WHERE call_key = ?1",
      [&request.call_key],
    )
    .unwrap_err();
  assert!(delete.to_string().contains("workflow calls are durable"));

  assert_eq!(
    DurableEventStore::open(database.path())
      .unwrap()
      .workflow_call(&request.call_key)
      .unwrap()
      .unwrap()
      .target_workflow_id,
    "calculate-risk"
  );
}

#[test]
fn tampered_call_index_is_rejected_instead_of_becoming_runtime_truth() {
  let database = TemporaryDatabase::new("corrupt-index");
  let (_, request) = seed_call(database.path());
  let store = DurableEventStore::open(database.path()).unwrap();
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute_batch("DROP TRIGGER woml_workflow_calls_identity_no_update;")
    .unwrap();
  connection
    .execute(
      "UPDATE woml_workflow_calls SET target_workflow_id = 'tampered' WHERE call_key = ?1",
      [&request.call_key],
    )
    .unwrap();

  assert!(matches!(
    store.workflow_call(&request.call_key),
    Err(DurableStoreError::WorkflowCallHistoryInvalid(message))
      if message == "stored child run binding does not match its workflow call"
  ));
  assert!(matches!(
    DurableEventStore::open(database.path()),
    Err(DurableStoreError::Contract(message))
      if message.contains("woml_workflow_calls_identity_no_update")
  ));
}

#[test]
fn v9_to_v10_migration_preserves_calls_definitions_and_event_histories() {
  let database = TemporaryDatabase::new("migration");
  let (parent_run_id, request) = seed_call(database.path());
  let before = DurableEventStore::open(database.path()).unwrap();
  let before_parent_events = before.events(&parent_run_id).unwrap();
  let before_child_events = before.events(&request.child_run_id).unwrap();
  let before_call = before.workflow_call(&request.call_key).unwrap().unwrap();
  drop(before);

  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute_batch(
      "DROP INDEX woml_workflow_runtime_routes_runtime;
       DROP TABLE woml_workflow_runtime_routes;
       UPDATE woml_store_metadata SET value = '9' WHERE key = 'schema_version';",
    )
    .unwrap();
  drop(connection);
  assert_eq!(schema_version(database.path()), "9");

  let migrated = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    schema_version(database.path()),
    DURABLE_STORE_SCHEMA_VERSION.to_string()
  );
  assert_eq!(
    migrated.workflow_call(&request.call_key).unwrap(),
    Some(before_call)
  );
  assert_eq!(
    migrated.events(&parent_run_id).unwrap(),
    before_parent_events
  );
  assert_eq!(
    migrated.events(&request.child_run_id).unwrap(),
    before_child_events
  );
  assert_eq!(migrated.definition(CHILD_HASH).unwrap(), model(CHILD_MODEL));
  let route_table_exists: bool = Connection::open(database.path())
    .unwrap()
    .query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'woml_workflow_runtime_routes')",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert!(route_table_exists);
}

#[test]
fn unknown_future_store_version_fails_without_mutating_the_artifact() {
  let database = TemporaryDatabase::new("future-version");
  let store = DurableEventStore::open(database.path()).unwrap();
  drop(store);
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute(
      "UPDATE woml_store_metadata SET value = '999' WHERE key = 'schema_version'",
      [],
    )
    .unwrap();
  drop(connection);

  assert!(matches!(
    DurableEventStore::open(database.path()),
    Err(DurableStoreError::UnsupportedStoreVersion(version)) if version == "999"
  ));
  assert_eq!(schema_version(database.path()), "999");
}
