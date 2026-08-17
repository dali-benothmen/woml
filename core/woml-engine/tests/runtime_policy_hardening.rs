use std::sync::{Arc, Barrier};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde_json::Map;
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, PolicyExecutionClaimResult, PublicRunStatus,
  RunCancellationStatus, RunEventPayload, TriggerAdmissionRequest,
};

const DEFINITION_HASH: &str =
  "sha256:7777777777777777777777777777777777777777777777777777777777777777";

struct TestDatabase(std::path::PathBuf);

impl TestDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-rp7-{label}-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn policy_model(workflow_id: &str, concurrency: u32) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  workflow.workflow_id = workflow_id.to_string();
  let policy = workflow.runtime_policy.as_mut().unwrap();
  policy.concurrency = Some(concurrency);
  policy.rate_limit = None;
  policy.timeout_ms = None;
  policy.queue.as_mut().unwrap().name = workflow_id.to_string();
  workflow
}

fn admit(
  store: &mut DurableEventStore,
  workflow_id: &str,
  source_identity: &str,
  received_at: chrono::DateTime<Utc>,
) -> String {
  store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow_id.to_string(),
      definition_hash: DEFINITION_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: source_identity.to_string(),
      payload: Map::new(),
      received_at,
    })
    .unwrap()
    .run_id
}

#[test]
fn large_bounded_queue_rebuilds_entirely_from_durable_history() {
  const RUNS: usize = 1_000;
  let database = TestDatabase::new("large-queue");
  let workflow = policy_model("rp7-large-queue", 4);
  let started_at = Utc.with_ymd_and_hms(2026, 8, 12, 17, 0, 0).unwrap();
  let run_ids = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store
      .register_definition(&workflow, DEFINITION_HASH)
      .unwrap();
    (0..RUNS)
      .map(|index| {
        admit(
          &mut store,
          &workflow.workflow_id,
          &format!("rp7-large-{index}"),
          started_at + chrono::Duration::microseconds(index as i64),
        )
      })
      .collect::<Vec<_>>()
  };

  let mut store = DurableEventStore::open(&database.0).unwrap();
  let cancelled = store
    .request_run_cancellation(&run_ids[500], "cancel-rp7-large", Utc::now())
    .unwrap();
  assert_eq!(cancelled.status, RunCancellationStatus::Accepted);
  drop(store);

  let connection = Connection::open(&database.0).unwrap();
  connection
    .execute(
      "DELETE FROM woml_runtime_policy_queue WHERE occurrence_sequence % 3 = 0",
      [],
    )
    .unwrap();
  connection
    .execute(
      "UPDATE woml_run_summaries
       SET status = 'running', waiting_for = 'rate_limit'
       WHERE run_id IN (SELECT run_id FROM woml_runtime_policy_queue LIMIT 25)",
      [],
    )
    .unwrap();
  drop(connection);

  let mut recovered = DurableEventStore::open(&database.0).unwrap();
  recovered.rebuild_runtime_policy_indexes().unwrap();
  let queued_rows: i64 = Connection::open(&database.0)
    .unwrap()
    .query_row(
      "SELECT COUNT(*) FROM woml_runtime_policy_queue",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(queued_rows, (RUNS - 1) as i64);
  assert_eq!(
    recovered.projection(&run_ids[0]).unwrap().status,
    woml_engine::RunStatus::Queued
  );
  assert_eq!(
    recovered.inspect_run_v3(&run_ids[500]).unwrap().status,
    PublicRunStatus::Cancelling
  );
  assert_eq!(recovered.list_runs_v2(200).unwrap().runs.len(), 200);
  assert!(recovered
    .events(&run_ids[500])
    .unwrap()
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::RunCancellationRequested(_))));
}

#[test]
fn many_processes_contending_for_one_policy_never_oversubscribe() {
  const CONTENDERS: usize = 24;
  const CONCURRENCY: usize = 4;
  let database = Arc::new(TestDatabase::new("contention"));
  let workflow = policy_model("rp7-contention", CONCURRENCY as u32);
  // Store startup intentionally expires abandoned scheduler claims using wall
  // time. Keep this contention lease live regardless of the calendar date on
  // which the release suite runs.
  let now = Utc::now();
  let run_ids = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store
      .register_definition(&workflow, DEFINITION_HASH)
      .unwrap();
    (0..CONTENDERS)
      .map(|index| {
        admit(
          &mut store,
          &workflow.workflow_id,
          &format!("rp7-contender-{index}"),
          now + chrono::Duration::microseconds(index as i64),
        )
      })
      .collect::<Vec<_>>()
  };
  let barrier = Arc::new(Barrier::new(CONTENDERS));
  let handles = run_ids
    .into_iter()
    .enumerate()
    .map(|(index, run_id)| {
      let database = Arc::clone(&database);
      let barrier = Arc::clone(&barrier);
      std::thread::spawn(move || {
        let mut store = DurableEventStore::open(&database.0).unwrap();
        barrier.wait();
        store
          .try_claim_policy_run(
            &run_id,
            &format!("rp7-process-{index}"),
            now,
            Duration::from_secs(30),
          )
          .unwrap()
      })
    })
    .collect::<Vec<_>>();
  let outcomes = handles
    .into_iter()
    .map(|handle| handle.join().unwrap())
    .collect::<Vec<_>>();
  let claimed = outcomes
    .iter()
    .filter(|result| matches!(result, PolicyExecutionClaimResult::Claimed { .. }))
    .count();
  assert!((1..=CONCURRENCY).contains(&claimed));
  assert_eq!(
    DurableEventStore::open(&database.0)
      .unwrap()
      .active_policy_claim_count(&workflow.workflow_id, now)
      .unwrap(),
    claimed as u32
  );
}

#[test]
fn repeated_recovery_is_idempotent_and_does_not_rewrite_event_truth() {
  let database = TestDatabase::new("recovery");
  let workflow = policy_model("rp7-recovery", 1);
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 19, 0, 0).unwrap();
  let run_id = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store
      .register_definition(&workflow, DEFINITION_HASH)
      .unwrap();
    admit(&mut store, &workflow.workflow_id, "rp7-recovery", now)
  };
  let before = DurableEventStore::open(&database.0)
    .unwrap()
    .events(&run_id)
    .unwrap();

  for _ in 0..5 {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store.rebuild_runtime_policy_indexes().unwrap();
    assert_eq!(store.events(&run_id).unwrap(), before);
    assert_eq!(
      store.inspect_run_v3(&run_id).unwrap().status,
      PublicRunStatus::Queued
    );
  }
}
