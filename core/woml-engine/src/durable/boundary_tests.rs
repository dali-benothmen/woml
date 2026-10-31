//! Characterization tests for opening, transaction rollback, and cache authority.

use super::{DurableEventStore, DurableStoreError, ProjectionFoldState};
use crate::event::{StepAttemptStartedData, StepAttemptSucceededData};
use crate::{CompiledWorkflowDefinition, RunEventPayload};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Map};
use std::path::PathBuf;
use uuid::Uuid;

const MODEL: &str = include_str!("../../../../woml/tests/fixtures/hello.compiled.v1.json");
const HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";
const RUN: &str = "run_store_boundary";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-store-boundary-{}.sqlite", Uuid::new_v4())))
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    for suffix in ["", "-wal", "-shm"] {
      let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
    }
  }
}

fn running_store() -> DurableEventStore {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  store.register_definition(&workflow, HASH).unwrap();
  store
    .start_run(
      "evt_store_boundary_start",
      RUN,
      Utc::now(),
      "hello",
      HASH,
      Map::new(),
    )
    .unwrap();
  store.projection(RUN).unwrap();
  store
}

fn attempt_started() -> RunEventPayload {
  RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
    node_id: "a".to_string(),
    attempt: 1,
    invocation_id: "inv_store_boundary_a".to_string(),
    handler: "runtime.script".to_string(),
    idempotency_key: None,
  })
}

#[test]
fn fresh_bootstrap_matches_the_entire_historically_migrated_schema() {
  let fresh = DurableEventStore::open_in_memory().unwrap();
  let database = TemporaryDatabase::new();
  let legacy = Connection::open(&database.0).unwrap();
  legacy.execute_batch(super::CREATE_SCHEMA).unwrap();
  legacy
    .execute(
      "INSERT INTO woml_store_metadata(key, value) VALUES ('schema_version', '1')",
      [],
    )
    .unwrap();
  drop(legacy);
  // A populated sqlite_master forces the original v1 -> v20 upgrade path.
  let migrated = DurableEventStore::open(&database.0).unwrap();
  let schema = |connection: &Connection| {
    connection
      .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
      .unwrap()
      .query_map([], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, Option<String>>(3)?,
        ))
      })
      .unwrap()
      .collect::<Result<Vec<_>, _>>()
      .unwrap()
  };
  assert_eq!(schema(&fresh.connection), schema(&migrated.connection));
  for connection in [&fresh.connection, &migrated.connection] {
    let version: String = connection
      .query_row(
        "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(version, super::STORE_SCHEMA_VERSION_V20);
    let integrity: String = connection
      .query_row("PRAGMA integrity_check", [], |row| row.get(0))
      .unwrap();
    assert_eq!(integrity, "ok");
    assert!(!super::runtime_policy_indexes_need_rebuild(connection).unwrap());
    let synchronous: i64 = connection
      .query_row("PRAGMA synchronous", [], |row| row.get(0))
      .unwrap();
    assert_eq!(
      synchronous, 2,
      "bootstrap must preserve FULL synchronization"
    );
    let foreign_keys: i64 = connection
      .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
      .unwrap();
    assert_eq!(foreign_keys, 1);
  }
}

#[test]
fn fresh_bootstrap_rolls_back_all_objects_on_disk_full_and_can_be_retried() {
  let database = TemporaryDatabase::new();
  let mut connection = Connection::open(&database.0).unwrap();
  connection
    .execute_batch("PRAGMA max_page_count = 4;")
    .unwrap();
  let error = super::bootstrap::initialize_empty_store(&mut connection).unwrap_err();
  assert!(
    matches!(error, DurableStoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _))
    if error.code == rusqlite::ErrorCode::DiskFull)
  );
  assert!(connection.is_autocommit());
  let objects: i64 = connection
    .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))
    .unwrap();
  assert_eq!(
    objects, 0,
    "failed bootstrap must not publish a partial schema or marker"
  );
  connection
    .execute_batch("PRAGMA max_page_count = 100000;")
    .unwrap();
  assert!(super::bootstrap::initialize_empty_store(&mut connection).unwrap());
  assert!(!super::bootstrap::initialize_empty_store(&mut connection).unwrap());
}

#[test]
fn fresh_bootstrap_does_not_treat_partial_or_foreign_schemas_as_empty() {
  let mut connection = Connection::open_in_memory().unwrap();
  connection.execute_batch(super::CREATE_SCHEMA).unwrap();
  assert!(!super::bootstrap::initialize_empty_store(&mut connection).unwrap());
  let marker_count: i64 = connection
    .query_row("SELECT COUNT(*) FROM woml_store_metadata", [], |row| {
      row.get(0)
    })
    .unwrap();
  assert_eq!(marker_count, 0);
  let mut foreign = Connection::open_in_memory().unwrap();
  foreign
    .execute_batch("CREATE TABLE foreign_data(value TEXT);")
    .unwrap();
  assert!(!super::bootstrap::initialize_empty_store(&mut foreign).unwrap());
}

#[test]
fn simultaneous_first_openers_publish_one_complete_store() {
  let database = TemporaryDatabase::new();
  let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
  let threads: Vec<_> = (0..4)
    .map(|_| {
      let path = database.0.clone();
      let barrier = barrier.clone();
      std::thread::spawn(move || {
        barrier.wait();
        let store = DurableEventStore::open(&path).unwrap();
        assert!(store.connection.is_autocommit());
      })
    })
    .collect();
  for thread in threads {
    thread.join().unwrap();
  }
  drop(DurableEventStore::open_ready(&database.0).unwrap());
}

#[test]
fn ready_open_does_not_run_schema_repair_or_require_a_writer_lock() {
  let database = TemporaryDatabase::new();
  drop(DurableEventStore::open(&database.0).unwrap());
  let mut writer = Connection::open(&database.0).unwrap();
  writer
    .execute_batch("DROP INDEX woml_run_summaries_updated;")
    .unwrap();
  let transaction = writer
    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
    .unwrap();

  // This intentionally damaged database proves that ready opening does not
  // validate/repair schema. Production callers must establish startup readiness.
  let ready = DurableEventStore::open_ready(&database.0).unwrap();
  let foreign_keys: i64 = ready
    .connection
    .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
    .unwrap();
  assert_eq!(foreign_keys, 1);
  let repaired: bool = ready
    .connection
    .query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'woml_run_summaries_updated')",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert!(!repaired);
  drop(ready);
  transaction.rollback().unwrap();
  drop(writer);
  assert!(matches!(
    DurableEventStore::open(&database.0),
    Err(DurableStoreError::Contract(_))
  ));
}

#[test]
fn ready_open_rejects_old_and_future_markers_without_migrating_them() {
  let database = TemporaryDatabase::new();
  drop(DurableEventStore::open(&database.0).unwrap());
  let connection = Connection::open(&database.0).unwrap();
  for version in ["19", "21"] {
    connection
      .execute(
        "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version'",
        [version],
      )
      .unwrap();
    assert!(
      matches!(DurableEventStore::open_ready(&database.0), Err(DurableStoreError::UnsupportedStoreVersion(value)) if value == version)
    );
    let actual: String = connection
      .query_row(
        "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(actual, version);
  }
}

#[test]
fn invalid_second_payload_rolls_back_the_whole_batch_and_candidate_cache() {
  let mut store = running_store();
  let before = store.events(RUN).unwrap();
  let projection = store.projection(RUN).unwrap();
  let result = store.append_payloads_atomically(
    RUN,
    vec![
      (
        "evt_batch_attempt".to_string(),
        Utc::now(),
        attempt_started(),
      ),
      (
        "evt_batch_invalid".to_string(),
        Utc::now(),
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: "missing".to_string(),
          attempt: 1,
          invocation_id: "inv_store_boundary_missing".to_string(),
          output: json!({}),
        }),
      ),
    ],
  );
  assert!(result.is_err());
  assert!(!store.projection_cache.borrow().contains_key(RUN));
  assert_eq!(store.events(RUN).unwrap(), before);
  assert_eq!(store.projection(RUN).unwrap(), projection);

  let accepted = store
    .append_payloads_atomically(
      RUN,
      vec![(
        "evt_batch_attempt".to_string(),
        Utc::now(),
        attempt_started(),
      )],
    )
    .unwrap();
  assert_eq!(accepted.last_sequence, 2);
  assert_eq!(store.events(RUN).unwrap()[1].event_id, "evt_batch_attempt");
}

#[test]
fn late_summary_failure_rolls_back_history_and_prevents_cache_publication() {
  let mut store = running_store();
  let before = store.events(RUN).unwrap();
  let projection = store.projection(RUN).unwrap();
  let summary = || -> (String, String) {
    store
      .connection
      .query_row(
        "SELECT status, updated_at FROM woml_run_summaries WHERE run_id = ?1",
        [RUN],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap()
  };
  let previous_summary = summary();
  store
    .connection
    .execute_batch(
      "CREATE TRIGGER rcm_reject_summary BEFORE UPDATE ON woml_run_summaries
     BEGIN SELECT RAISE(ABORT, 'injected summary failure'); END;",
    )
    .unwrap();
  let result = store.append_payloads_atomically(
    RUN,
    vec![(
      "evt_late_attempt".to_string(),
      Utc::now(),
      attempt_started(),
    )],
  );
  assert!(matches!(result, Err(DurableStoreError::Sqlite(_))));
  assert!(!store.projection_cache.borrow().contains_key(RUN));
  assert_eq!(store.events(RUN).unwrap(), before);
  assert_eq!(store.projection(RUN).unwrap(), projection);
  let current_summary: (String, String) = store
    .connection
    .query_row(
      "SELECT status, updated_at FROM woml_run_summaries WHERE run_id = ?1",
      [RUN],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap();
  assert_eq!(current_summary, previous_summary);

  store
    .connection
    .execute_batch("DROP TRIGGER rcm_reject_summary;")
    .unwrap();
  let accepted = store
    .append_payloads_atomically(
      RUN,
      vec![(
        "evt_late_attempt".to_string(),
        Utc::now(),
        attempt_started(),
      )],
    )
    .unwrap();
  assert_eq!(accepted.last_sequence, 2);
  assert_eq!(store.events(RUN).unwrap()[1].event_id, "evt_late_attempt");
}

#[test]
fn equal_event_count_does_not_make_a_different_history_cache_valid() {
  let store = running_store();
  let expected = store.projection(RUN).unwrap();
  let mut stale_history = store.events(RUN).unwrap();
  stale_history[0].event_id = "evt_not_committed".to_string();
  let RunEventPayload::RunStarted(start) = &mut stale_history[0].payload else {
    panic!("expected start");
  };
  start.trigger.insert("forged".to_string(), json!(true));
  let stale = ProjectionFoldState::from_events(&stale_history).unwrap();
  assert_eq!(stale.projection().last_sequence, expected.last_sequence);
  assert_ne!(stale.projection().context, expected.context);
  store
    .projection_cache
    .borrow_mut()
    .insert(RUN.to_string(), stale);
  assert_eq!(store.projection(RUN).unwrap(), expected);
  assert_eq!(
    store
      .projection_cache
      .borrow()
      .get(RUN)
      .unwrap()
      .last_event_id(),
    Some("evt_store_boundary_start")
  );
}

#[test]
fn failed_recovery_write_rolls_back_interruption_and_retries_idempotently() {
  let mut store = running_store();
  store
    .append_payload(RUN, "evt_recovery_attempt", Utc::now(), attempt_started())
    .unwrap();
  let before = store.events(RUN).unwrap();
  let projection = store.projection(RUN).unwrap();
  store
    .connection
    .execute_batch(
      "CREATE TRIGGER rcm_reject_recovery BEFORE INSERT ON woml_run_events
     WHEN NEW.sequence = 4
     BEGIN SELECT RAISE(ABORT, 'injected recovery failure'); END;",
    )
    .unwrap();

  // Recovery can write the interrupted attempt before deciding the run. A
  // late failure must roll both back, rather than leave half a recovered run.
  assert!(matches!(
    store.recover_interrupted_runs(),
    Err(DurableStoreError::Sqlite(_))
  ));
  assert_eq!(store.events(RUN).unwrap(), before);
  assert_eq!(store.projection(RUN).unwrap(), projection);

  store
    .connection
    .execute_batch("DROP TRIGGER rcm_reject_recovery;")
    .unwrap();
  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 1);
  let recovered = store.events(RUN).unwrap();
  assert_eq!(recovered.len(), 4);
  assert!(matches!(
    recovered[2].payload,
    RunEventPayload::StepAttemptFailed(_)
  ));
  assert!(matches!(
    recovered[3].payload,
    RunEventPayload::RunFailed(_)
  ));
  assert_eq!(store.recover_interrupted_runs().unwrap().recovered_runs, 0);
  assert_eq!(store.events(RUN).unwrap(), recovered);
}
