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
