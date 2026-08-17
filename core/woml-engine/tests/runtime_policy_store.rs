use std::time::Duration;

use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::{
  fold_events, CompiledWorkflowDefinition, DurableEventStore, DurableStoreError, PublicRunStatus,
  RunEvent, RunEventPayload, RunExecutionStartedData, RunStatus, RunTimeoutReachedData,
  TriggerAdmissionRequest, DURABLE_STORE_SCHEMA_VERSION, RUNTIME_POLICY_QUEUE_CEILING,
};

const DEFINITION_A: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DEFINITION_B: &str =
  "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const LEGACY_DEFINITION: &str =
  "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

struct TestDatabase(std::path::PathBuf);

impl TestDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-rp2-{}.sqlite", Uuid::new_v4().simple())))
  }

  fn path(&self) -> &std::path::Path {
    &self.0
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn policy_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap()
}

#[test]
fn rust_deserializes_validates_and_folds_the_reviewed_event_v11_fixture() {
  let events: Vec<RunEvent> = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/events.v11.json"
  ))
  .unwrap();
  for event in &events {
    event.validate().unwrap();
  }
  let projection = fold_events(&events).unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert_eq!(projection.queue.as_deref(), Some("orders"));
  assert_eq!(projection.occurrence_sequence, Some(42));
  assert_eq!(
    projection.timeout_reached_at,
    Some(Utc.with_ymd_and_hms(2026, 8, 11, 12, 10, 1).unwrap())
  );
}

fn legacy_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap()
}

fn admission(
  definition_hash: &str,
  source_identity: &str,
  received_at: chrono::DateTime<Utc>,
) -> TriggerAdmissionRequest {
  TriggerAdmissionRequest {
    workflow_id: "policy-demo".to_string(),
    definition_hash: definition_hash.to_string(),
    trigger_id: "start".to_string(),
    trigger_handler: "trigger.manual".to_string(),
    source_identity: source_identity.to_string(),
    payload: Map::from_iter([("orderId".to_string(), Value::String("order-1".to_string()))]),
    received_at,
  }
}

#[test]
fn model_v12_admission_is_atomic_queued_and_idempotent() {
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let workflow = policy_model();
  store.register_definition(&workflow, DEFINITION_A).unwrap();

  let first = store
    .admit_trigger_occurrence(admission(DEFINITION_A, "manual:one", now))
    .unwrap();
  let duplicate = store
    .admit_trigger_occurrence(admission(DEFINITION_A, "manual:one", now))
    .unwrap();

  assert!(!first.duplicate);
  assert!(duplicate.duplicate);
  assert_eq!(duplicate.run_id, first.run_id);
  let mut conflicting = admission(DEFINITION_A, "manual:one", now);
  conflicting
    .payload
    .insert("orderId".to_string(), Value::String("order-2".to_string()));
  assert!(matches!(
    store.admit_trigger_occurrence(conflicting),
    Err(DurableStoreError::TriggerIdempotencyConflict)
  ));
  let events = store.events(&first.run_id).unwrap();
  assert_eq!(events.len(), 1);
  assert!(matches!(events[0].payload, RunEventPayload::RunAdmitted(_)));
  let projection = store.projection(&first.run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Queued);
  assert_eq!(projection.workflow_id.as_deref(), Some("policy-demo"));

  let list = store.list_runs_v2(10).unwrap();
  assert_eq!(list.profile, "woml.run-list/v2");
  assert_eq!(list.runs.len(), 1);
  assert_eq!(list.runs[0].status, PublicRunStatus::Queued);
  assert_eq!(list.runs[0].admitted_at, now);
  assert_eq!(list.runs[0].started_at, None);
  assert_eq!(list.runs[0].queue.as_deref(), Some("orders"));
  assert!(store.list_runs(10).unwrap().runs.is_empty());

  let inspection = store.inspect_run_v3(&first.run_id).unwrap();
  assert_eq!(inspection.status, PublicRunStatus::Queued);
  assert_eq!(inspection.workflow_id, "policy-demo");
  assert_eq!(inspection.policy.queue, "orders");
  assert_eq!(inspection.policy.timeout_at, None);
}

#[test]
fn startup_recovers_a_missing_rebuildable_queue_index() {
  let database = TestDatabase::new();
  let now = Utc::now();
  let run_id = {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&policy_model(), DEFINITION_A)
      .unwrap();
    store
      .admit_trigger_occurrence(admission(DEFINITION_A, "manual:recovery", now))
      .unwrap()
      .run_id
  };
  Connection::open(database.path())
    .unwrap()
    .execute(
      "DELETE FROM woml_runtime_policy_queue WHERE run_id = ?1",
      [&run_id],
    )
    .unwrap();

  let recovered = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    recovered.projection(&run_id).unwrap().status,
    RunStatus::Queued
  );
  assert_eq!(
    recovered.list_runs_v2(10).unwrap().runs[0].status,
    PublicRunStatus::Queued
  );
}

#[test]
fn queue_indexes_rebuild_and_expired_claims_are_reclaimable() {
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&policy_model(), DEFINITION_A)
    .unwrap();
  let admitted = store
    .admit_trigger_occurrence(admission(DEFINITION_A, "manual:queue", now))
    .unwrap();

  let first = store
    .claim_policy_run(&admitted.run_id, "owner-a", now, Duration::from_secs(1))
    .unwrap();
  assert_eq!(first.profile, "woml.scheduler-claim/v1");
  assert!(matches!(
    store.claim_policy_run(&admitted.run_id, "owner-b", now, Duration::from_secs(1)),
    Err(DurableStoreError::SchedulerClaimConflict(_))
  ));
  let reclaimed = store
    .claim_policy_run(
      &admitted.run_id,
      "owner-b",
      now + chrono::Duration::seconds(2),
      Duration::from_secs(5),
    )
    .unwrap();
  assert_ne!(reclaimed.claim_id, first.claim_id);

  store.rebuild_runtime_policy_indexes().unwrap();
  assert_eq!(
    store.list_runs_v2(10).unwrap().runs[0].status,
    PublicRunStatus::Running
  );
}

#[test]
fn execution_start_is_distinct_and_uses_the_compiled_timeout() {
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store
    .register_definition(&policy_model(), DEFINITION_A)
    .unwrap();
  let admitted = store
    .admit_trigger_occurrence(admission(DEFINITION_A, "manual:start", now))
    .unwrap();
  let started_at = now + chrono::Duration::seconds(3);
  let timeout_at = started_at + chrono::Duration::minutes(10);

  store
    .append_payload(
      &admitted.run_id,
      "event-started",
      started_at,
      RunEventPayload::RunExecutionStarted(RunExecutionStartedData {
        started_at,
        timeout_at: Some(timeout_at),
      }),
    )
    .unwrap();

  let projection = store.projection(&admitted.run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Running);
  assert_eq!(projection.started_at, Some(started_at));
  assert_eq!(projection.timeout_at, Some(timeout_at));
  let summary = &store.list_runs_v2(10).unwrap().runs[0];
  assert_eq!(summary.started_at, Some(started_at));
  assert_eq!(summary.queue.as_deref(), Some("orders"));

  store
    .append_payload(
      &admitted.run_id,
      "event-timeout",
      timeout_at,
      RunEventPayload::RunTimeoutReached(RunTimeoutReachedData {
        deadline_at: timeout_at,
        code: "WOML_WORKFLOW_TIMED_OUT".to_string(),
      }),
    )
    .unwrap();
  assert_eq!(
    store
      .projection(&admitted.run_id)
      .unwrap()
      .timeout_reached_at,
    Some(timeout_at)
  );
}

#[test]
fn active_policy_definition_conflicts_are_rejected() {
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let first = policy_model();
  let mut second = first.clone();
  second.runtime_policy.as_mut().unwrap().concurrency = Some(5);
  store.register_definition(&first, DEFINITION_A).unwrap();
  store.register_definition(&second, DEFINITION_B).unwrap();
  store
    .admit_trigger_occurrence(admission(DEFINITION_A, "manual:a", now))
    .unwrap();

  assert!(matches!(
    store.validate_runtime_policy_activation(&second, DEFINITION_B),
    Err(DurableStoreError::RuntimePolicyDefinitionConflict(workflow_id))
      if workflow_id == "policy-demo"
  ));

  assert!(matches!(
    store.admit_trigger_occurrence(admission(
      DEFINITION_B,
      "manual:b",
      now + chrono::Duration::seconds(1)
    )),
    Err(DurableStoreError::RuntimePolicyDefinitionConflict(workflow_id))
      if workflow_id == "policy-demo"
  ));
}

#[test]
fn queue_ceiling_rejects_without_creating_a_run_and_keeps_retry_identity_free() {
  let file = TestDatabase::new();
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let mut store = DurableEventStore::open(file.path()).unwrap();
  store
    .register_definition(&policy_model(), DEFINITION_A)
    .unwrap();
  let connection = Connection::open(file.path()).unwrap();
  connection
    .execute_batch(&format!(
      "WITH RECURSIVE sequence(value) AS (
         SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < {RUNTIME_POLICY_QUEUE_CEILING}
       )
       INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
       SELECT printf('run_ceiling_%05d', value), 'policy-demo', '{DEFINITION_A}', '{now}'
       FROM sequence;
       WITH RECURSIVE sequence(value) AS (
         SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < {RUNTIME_POLICY_QUEUE_CEILING}
       )
       INSERT INTO woml_runtime_policy_queue(
         run_id, workflow_id, queue_name, admitted_at, occurrence_sequence
       )
       SELECT printf('run_ceiling_%05d', value), 'policy-demo', 'orders', '{now}', value
       FROM sequence;"
    ))
    .unwrap();
  drop(connection);

  assert!(matches!(
    store.admit_trigger_occurrence(admission(DEFINITION_A, "manual:overflow", now)),
    Err(DurableStoreError::RuntimePolicyQueueFull)
  ));
  let connection = Connection::open(file.path()).unwrap();
  let created: i64 = connection
    .query_row(
      "SELECT COUNT(*) FROM woml_trigger_occurrences WHERE source_identity_hash IS NOT NULL",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(created, 0);
}

#[test]
fn store_v11_migrates_transactionally_without_rewriting_legacy_history() {
  let file = TestDatabase::new();
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let before = {
    let mut store = DurableEventStore::open(file.path()).unwrap();
    let workflow = legacy_model();
    store
      .register_definition(&workflow, LEGACY_DEFINITION)
      .unwrap();
    let admitted = store
      .admit_trigger_occurrence(TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id,
        definition_hash: LEGACY_DEFINITION.to_string(),
        trigger_id: "start".to_string(),
        trigger_handler: "trigger.manual".to_string(),
        source_identity: "legacy:migration".to_string(),
        payload: Map::new(),
        received_at: now,
      })
      .unwrap();
    (
      admitted.run_id.clone(),
      store.events(&admitted.run_id).unwrap(),
    )
  };
  let connection = Connection::open(file.path()).unwrap();
  connection
    .execute_batch(
      "DROP TABLE woml_scheduler_claims;
       DROP TABLE woml_runtime_policy_starts;
       DROP TABLE woml_runtime_policy_queue;
       DROP TABLE woml_runtime_policy_bindings;
       ALTER TABLE woml_run_summaries RENAME TO woml_run_summaries_v12;
       DROP INDEX woml_run_summaries_updated;
       CREATE TABLE woml_run_summaries (
         run_id TEXT PRIMARY KEY,
         workflow_id TEXT NOT NULL,
         status TEXT NOT NULL,
         started_at TEXT NOT NULL,
         updated_at TEXT NOT NULL,
         FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
       );
       CREATE INDEX woml_run_summaries_updated
         ON woml_run_summaries(updated_at DESC, run_id DESC);
       INSERT INTO woml_run_summaries(run_id, workflow_id, status, started_at, updated_at)
         SELECT run_id, workflow_id, status, started_at, updated_at
         FROM woml_run_summaries_v12 WHERE started_at IS NOT NULL;
       DROP TABLE woml_run_summaries_v12;
       UPDATE woml_store_metadata SET value = '11' WHERE key = 'schema_version';",
    )
    .unwrap();
  drop(connection);

  let migrated = DurableEventStore::open(file.path()).unwrap();
  assert_eq!(migrated.events(&before.0).unwrap(), before.1);
  assert_eq!(migrated.list_runs(10).unwrap().runs.len(), 1);
  assert_eq!(migrated.list_runs_v2(10).unwrap().runs.len(), 1);
  let version: String = Connection::open(file.path())
    .unwrap()
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, DURABLE_STORE_SCHEMA_VERSION.to_string());
}

#[test]
fn startup_rejects_unknown_versions_and_corrupt_v12_shape() {
  let future = TestDatabase::new();
  drop(DurableEventStore::open(future.path()).unwrap());
  Connection::open(future.path())
    .unwrap()
    .execute(
      "UPDATE woml_store_metadata SET value = '99' WHERE key = 'schema_version'",
      [],
    )
    .unwrap();
  assert!(matches!(
    DurableEventStore::open(future.path()),
    Err(DurableStoreError::UnsupportedStoreVersion(version)) if version == "99"
  ));

  let corrupt = TestDatabase::new();
  drop(DurableEventStore::open(corrupt.path()).unwrap());
  Connection::open(corrupt.path())
    .unwrap()
    .execute("DROP TABLE woml_runtime_policy_queue", [])
    .unwrap();
  assert!(matches!(
    DurableEventStore::open(corrupt.path()),
    Err(DurableStoreError::Contract(message)) if message.contains("woml_runtime_policy_queue")
  ));
}
