use chrono::{Duration, Utc};
use rusqlite::Connection;
use serde_json::{json, Map};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use woml_engine::{
  ingest_remote_output_batch, CompiledWorkflowDefinition, DurableEventStore, DurableStoreError,
  OperationalBodyV1, OperationalExecutionPhaseV1, OperationalIdentityV1, OperationalLogLevelV1,
  OperationalOutputLimitsV1, OperationalOutputStore, OperationalRecordDraftV1, OperationalSourceV1,
  RemoteDispatchError, RemoteDispatchRequestV1, RemoteDispatchResolutionV1, RemoteDispatchStateV1,
  RemoteDispatchTransitionedData, RemoteOperationalAppendOutcomeV1, RemoteOperationalRecordV1,
  RunEvent, RunEventPayload, TriggerAdmissionRequest, WorkerOutputBatchV1,
  DURABLE_STORE_SCHEMA_VERSION, RUN_EVENT_SCHEMA_VERSION_V20,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json");
const HASH: &str = "sha256:2121212121212121212121212121212121212121212121212121212121212121";
const TARGET_DIGEST: &str =
  "sha256:d7f8a876583ef92e40aabaca6ed0a78f4189bbeaa374caeca1b5ed4ae7039f93";
const PACKAGE_DIGEST: &str =
  "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-remote-dispatch-{label}-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn store_and_run() -> (DurableEventStore, String) {
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store.register_definition(&workflow, HASH).unwrap();
  let run_id = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id,
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "remote-dispatch-test".to_string(),
      payload: Map::new(),
      received_at: Utc::now(),
    })
    .unwrap()
    .run_id;
  (store, run_id)
}

fn request(run_id: &str, now: chrono::DateTime<Utc>) -> RemoteDispatchRequestV1 {
  RemoteDispatchRequestV1 {
    run_id: run_id.to_string(),
    node_id: "applyRelease".to_string(),
    iteration_identity: String::new(),
    attempt: 1,
    invocation_id: "inv_remote_01".to_string(),
    target: "production-private".to_string(),
    target_digest: TARGET_DIGEST.to_string(),
    package_digest: PACKAGE_DIGEST.to_string(),
    requested_at: now,
  }
}

#[test]
fn request_offer_accept_start_heartbeat_and_terminal_are_durable_and_idempotent() {
  assert_eq!(DURABLE_STORE_SCHEMA_VERSION, 20);
  let now = Utc::now();
  let (mut store, run_id) = store_and_run();
  let requested = store
    .request_remote_dispatch(&request(&run_id, now))
    .unwrap();
  assert_eq!(requested.state, RemoteDispatchStateV1::Requested);
  assert_eq!(
    store
      .request_remote_dispatch(&request(&run_id, now))
      .unwrap(),
    requested
  );
  let offered = store
    .offer_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(30),
      now,
    )
    .unwrap();
  assert_eq!(offered.state, RemoteDispatchStateV1::Offered);
  assert_eq!(
    store
      .offer_remote_dispatch(
        &requested.dispatch_id,
        "production-runner-01",
        "session_01",
        "offer_01",
        now + Duration::seconds(30),
        now,
      )
      .unwrap(),
    offered
  );
  assert!(matches!(
    store.offer_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(31),
      now,
    ),
    Err(RemoteDispatchError::Conflict)
  ));
  store
    .accept_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(1),
    )
    .unwrap();
  store
    .start_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      now + Duration::seconds(2),
    )
    .unwrap();
  store
    .heartbeat_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      1,
      &json!({"progress": 0.5}),
      now + Duration::seconds(3),
    )
    .unwrap();
  let terminal = store
    .terminal_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      RemoteDispatchStateV1::Succeeded,
      "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      now + Duration::seconds(4),
    )
    .unwrap();
  assert_eq!(terminal.state, RemoteDispatchStateV1::Succeeded);
  let inspection = store
    .inspect_remote_dispatch(&requested.dispatch_id)
    .unwrap();
  assert_eq!(inspection.profile, "woml.remote-dispatch-inspection/v1");
  assert_eq!(inspection.dispatch_id, requested.dispatch_id);
  assert_eq!(inspection.state, "succeeded");
  assert_eq!(inspection.output_sequence, 1);
  assert!(store
    .terminal_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      RemoteDispatchStateV1::Failed,
      "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
      now + Duration::seconds(5),
    )
    .is_err());
}

#[test]
fn dispatch_requests_are_bound_to_the_exact_model_v21_placement_and_attempt() {
  let now = Utc::now();
  let (mut store, run_id) = store_and_run();
  let mut forged = request(&run_id, now);
  forged.node_id = "summary".to_string();
  assert!(matches!(
    store.request_remote_dispatch(&forged),
    Err(RemoteDispatchError::Conflict)
  ));
  let mut wrong_target = request(&run_id, now);
  wrong_target.target = "maintenance-runner".to_string();
  assert!(matches!(
    store.request_remote_dispatch(&wrong_target),
    Err(RemoteDispatchError::Conflict)
  ));
  let requested = store
    .request_remote_dispatch(&request(&run_id, now))
    .unwrap();
  let mut changed_package = request(&run_id, now);
  changed_package.package_digest =
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string();
  assert!(matches!(
    store.request_remote_dispatch(&changed_package),
    Err(RemoteDispatchError::Conflict)
  ));
  assert_eq!(
    store
      .inspect_remote_dispatch(&requested.dispatch_id)
      .unwrap()
      .state,
    "requested"
  );
}

#[test]
fn event_v20_carries_only_a_bounded_committed_dispatch_fact() {
  let event = RunEvent {
    event_schema_version: RUN_EVENT_SCHEMA_VERSION_V20,
    event_id: "event_remote_requested_01".to_string(),
    run_id: "run_remote_01".to_string(),
    sequence: 3,
    occurred_at: Utc::now(),
    iteration: None,
    payload: RunEventPayload::RemoteDispatchTransitioned(RemoteDispatchTransitionedData {
      dispatch_id: format!("dispatch_{}", "a".repeat(64)),
      node_id: "deployApplication".to_string(),
      attempt: 1,
      invocation_id: "inv_remote_01".to_string(),
      target: "production-private".to_string(),
      state: RemoteDispatchStateV1::Requested,
      worker_id: None,
      terminal_digest: None,
    }),
  };
  event.validate().unwrap();
  let mut forged = event;
  if let RunEventPayload::RemoteDispatchTransitioned(data) = &mut forged.payload {
    data.worker_id = Some("unbound-worker".to_string());
  }
  assert!(forged.validate().is_err());
}

#[test]
fn loss_after_acceptance_is_ambiguous_until_explicit_reconciliation() {
  let now = Utc::now();
  let (mut store, run_id) = store_and_run();
  let requested = store
    .request_remote_dispatch(&request(&run_id, now))
    .unwrap();
  store
    .offer_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(5),
      now,
    )
    .unwrap();
  store
    .accept_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(1),
    )
    .unwrap();
  let lost = store
    .mark_lost_remote_dispatches(now + Duration::seconds(2), now + Duration::seconds(10), 100)
    .unwrap();
  assert_eq!(lost[0].state, RemoteDispatchStateV1::Ambiguous);
  assert_eq!(store.recover_remote_dispatches(100).unwrap().len(), 1);
  let late = store
    .terminal_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      RemoteDispatchStateV1::Succeeded,
      "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      now + Duration::seconds(11),
    )
    .unwrap();
  assert_eq!(late.state, RemoteDispatchStateV1::Succeeded);
  assert!(matches!(
    store.reconcile_remote_dispatch(
      &requested.dispatch_id,
      RemoteDispatchResolutionV1::AuthorizePossibleDuplicateRetry,
      "operator-one",
      "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
      now + Duration::seconds(12),
    ),
    Err(RemoteDispatchError::Conflict)
  ));
  assert!(matches!(
    store.reconcile_remote_dispatch(
      &requested.dispatch_id,
      RemoteDispatchResolutionV1::AuthorizePossibleDuplicateRetry,
      "operator-one",
      "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
      now + Duration::seconds(13),
    ),
    Err(RemoteDispatchError::Conflict)
  ));
  assert!(matches!(
    store.offer_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_02",
      "offer_02",
      now + Duration::seconds(30),
      now + Duration::seconds(14),
    ),
    Err(RemoteDispatchError::Conflict)
  ));
}

#[test]
fn remote_output_is_ordered_deduplicated_and_cursor_compatible() {
  let now = Utc::now();
  let (mut store, run_id) = store_and_run();
  let requested = store
    .request_remote_dispatch(&request(&run_id, now))
    .unwrap();
  store
    .offer_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(30),
      now,
    )
    .unwrap();
  store
    .accept_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now,
    )
    .unwrap();
  store
    .start_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      now,
    )
    .unwrap();
  let output_database = TemporaryDatabase::new("output");
  let output = OperationalOutputStore::open(output_database.path(), "deployment-remote").unwrap();
  let mut record = RemoteOperationalRecordV1 {
    sequence: 1,
    body: OperationalBodyV1::Output {
      source: OperationalSourceV1::Stdout,
      level: Some(OperationalLogLevelV1::Info),
      text: "remote line\n".to_string(),
      byte_count: 12,
    },
    record_digest: String::new(),
  };
  record.record_digest = record.computed_digest().unwrap();
  let batch = WorkerOutputBatchV1 {
    profile: "woml.worker-protocol/v1".to_string(),
    protocol_version: 1,
    message_id: "output_01".to_string(),
    message_type: "output".to_string(),
    worker_id: "production-runner-01".to_string(),
    session_id: "session_01".to_string(),
    sent_at: now,
    dispatch_id: requested.dispatch_id.clone(),
    records: vec![record.clone()],
  };
  let first = ingest_remote_output_batch(&mut store, &output, &batch, now).unwrap();
  assert_eq!((first.accepted, first.duplicates), (1, 0));
  let replay = ingest_remote_output_batch(&mut store, &output, &batch, now).unwrap();
  assert_eq!((replay.accepted, replay.duplicates), (0, 1));
  let page = output.read_run_page(&run_id, 0, 10).unwrap();
  assert_eq!(page.records.len(), 1);
  assert_eq!(page.next_after, first.last_local_sequence);

  let mut reordered = record;
  reordered.sequence = 3;
  reordered.record_digest = reordered.computed_digest().unwrap();
  let mut invalid_batch = batch;
  invalid_batch.message_id = "output_03".to_string();
  invalid_batch.records = vec![reordered];
  assert!(ingest_remote_output_batch(&mut store, &output, &invalid_batch, now).is_err());
}

#[test]
fn dropped_remote_output_still_advances_the_durable_remote_cursor() {
  let database = TemporaryDatabase::new("bounded-output");
  let output = OperationalOutputStore::open_with_limits(
    database.path(),
    "deployment-remote",
    OperationalOutputLimitsV1 {
      max_attempt_bytes: 10,
      max_run_bytes: 15,
      max_store_bytes: 20,
      retention_ms: 60_000,
    },
  )
  .unwrap();
  let dispatch_id = format!("dispatch_{}", "a".repeat(64));
  let draft = |text: &str| OperationalRecordDraftV1 {
    identity: OperationalIdentityV1 {
      workflow_id: "remote-output-budget".to_string(),
      run_id: "run_remote_output_budget".to_string(),
      node_id: Some("remoteStep".to_string()),
      attempt: Some(1),
      invocation_id: Some("inv_remote_output_budget".to_string()),
      phase: OperationalExecutionPhaseV1::Forward,
    },
    body: OperationalBodyV1::Output {
      source: OperationalSourceV1::Stdout,
      level: Some(OperationalLogLevelV1::Info),
      text: text.to_string(),
      byte_count: text.len() as u64,
    },
  };
  let digest = |sequence: u64| format!("sha256:{sequence:064x}");

  output
    .append_remote_bounded(&dispatch_id, 1, &digest(1), draft("12345678"))
    .unwrap();
  output
    .append_remote_bounded(&dispatch_id, 2, &digest(2), draft("abcd"))
    .unwrap();
  output
    .append_remote_bounded(&dispatch_id, 3, &digest(3), draft("dropped"))
    .unwrap();
  let replay = output
    .append_remote_bounded(&dispatch_id, 3, &digest(3), draft("dropped"))
    .unwrap();
  assert!(matches!(
    replay,
    RemoteOperationalAppendOutcomeV1::Duplicate { .. }
  ));
  output
    .append_remote_bounded(&dispatch_id, 4, &digest(4), draft("also dropped"))
    .expect("the sequence after a dropped record remains admissible");
}

#[test]
fn preaccept_offer_loss_is_safe_but_conflicting_acceptance_fails_closed() {
  let now = Utc::now();
  let (mut store, run_id) = store_and_run();
  let requested = store
    .request_remote_dispatch(&request(&run_id, now))
    .unwrap();
  store
    .offer_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(1),
      now,
    )
    .unwrap();
  let expired = store
    .mark_lost_remote_dispatches(now + Duration::seconds(2), now + Duration::seconds(2), 100)
    .unwrap();
  assert_eq!(expired[0].state, RemoteDispatchStateV1::Requested);
  assert!(store
    .accept_remote_dispatch(
      &requested.dispatch_id,
      "production-runner-01",
      "session_01",
      "offer_01",
      now + Duration::seconds(3),
    )
    .is_err());
  assert!(store.recover_remote_dispatches(10_001).is_err());
}

#[test]
fn store_v19_migrates_additively_to_v20_and_future_versions_fail_closed() {
  let database = TemporaryDatabase::new("migration");
  drop(DurableEventStore::open(database.path()).unwrap());
  {
    let connection = Connection::open(database.path()).unwrap();
    connection
      .execute_batch(
        "DROP TABLE woml_remote_dispatch_audit;
         DROP TABLE woml_remote_dispatches;
         UPDATE woml_store_metadata SET value='19' WHERE key='schema_version';",
      )
      .unwrap();
  }
  drop(DurableEventStore::open(database.path()).unwrap());
  {
    let connection = Connection::open(database.path()).unwrap();
    let version: String = connection
      .query_row(
        "SELECT value FROM woml_store_metadata WHERE key='schema_version'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(version, DURABLE_STORE_SCHEMA_VERSION.to_string());
    let tables: i64 = connection
      .query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
         AND name IN ('woml_remote_dispatches','woml_remote_dispatch_audit')",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(tables, 2);
    connection
      .execute(
        "UPDATE woml_store_metadata SET value='999' WHERE key='schema_version'",
        [],
      )
      .unwrap();
  }
  assert!(matches!(
    DurableEventStore::open(database.path()),
    Err(DurableStoreError::UnsupportedStoreVersion(version)) if version == "999"
  ));
}
