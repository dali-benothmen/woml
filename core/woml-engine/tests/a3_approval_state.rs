use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use serde_json::Value;
use uuid::Uuid;
use woml_engine::{
  fold_events, ApprovalDecision, ApprovalDecisionSource, ApprovalRequestStatus, ApprovalResolution,
  CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, DurableStoreError, RunEvent,
  RunEventPayload, RunFailure, RunStatus, DURABLE_STORE_SCHEMA_VERSION,
};

const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const APPROVAL_TIMEOUT_FAIL_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/approval-timeout-fail.compiled.v4.json");
const APPROVED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-approved.events.v4.json");
const REJECTED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-rejected.events.v4.json");
const TIMEOUT_REJECTED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-timeout-rejected.events.v4.json");
const TIMEOUT_FAILED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-timeout-failed.events.v4.json");
const APPROVAL_CONTEXT: &str =
  include_str!("../../../woml/tests/fixtures/approval.context.v0.1.json");
const APPROVAL_RESULT: &str =
  include_str!("../../../woml/tests/fixtures/approval.result.v0.1.json");
const APPROVAL_HASH: &str =
  "sha256:c85377270773c4abb178ba2811109843be53df66c91fedea04bb37d586901aa9";
const APPROVAL_TIMEOUT_FAIL_HASH: &str =
  "sha256:56c90146b60cddfc6df253d0276e4306936ed1a63ac2c5e355286b96500a07b0";

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/hello.events.v1.json");
const HELLO_HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";

fn approval_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(APPROVAL_MODEL).unwrap()
}

fn events(source: &str) -> Vec<RunEvent> {
  serde_json::from_str(source).unwrap()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-a3-{label}-{}.sqlite",
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

fn append_history(engine: &mut DurableDagEngine, history: &[RunEvent], limit: usize) {
  for event in history.iter().take(limit) {
    match &event.payload {
      RunEventPayload::RunStarted(data) => {
        engine
          .start_run(
            event.event_id.clone(),
            event.run_id.clone(),
            event.occurred_at,
            data.trigger.clone(),
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

#[test]
fn event_v4_fixtures_fold_waiting_decisions_and_timeout_failure() {
  for source in [
    APPROVED_EVENTS,
    REJECTED_EVENTS,
    TIMEOUT_REJECTED_EVENTS,
    TIMEOUT_FAILED_EVENTS,
  ] {
    let history = events(source);
    assert!(history.iter().all(|event| event.event_schema_version == 4));
    let projection = fold_events(&history).unwrap();
    assert!(matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed
    ));
  }

  let approved = fold_events(&events(APPROVED_EVENTS)).unwrap();
  let expected_context: Value = serde_json::from_str(APPROVAL_CONTEXT).unwrap();
  assert_eq!(
    serde_json::to_value(&approved.context).unwrap(),
    expected_context
  );
  assert_eq!(
    approved.result,
    Some(serde_json::from_str(APPROVAL_RESULT).unwrap())
  );
  assert!(matches!(
    approved.approval_requests["editorApproval"].status,
    ApprovalRequestStatus::Resolved {
      resolution: ApprovalResolution::Decision {
        decision: ApprovalDecision::Approved,
        source: ApprovalDecisionSource::Human,
      },
      ..
    }
  ));

  let timeout_failed = fold_events(&events(TIMEOUT_FAILED_EVENTS)).unwrap();
  assert_eq!(timeout_failed.status, RunStatus::Failed);
  assert!(!timeout_failed.context.steps.contains_key("editorApproval"));
  assert!(matches!(
    timeout_failed.failure,
    Some(RunFailure::Approval { .. })
  ));
}

#[test]
fn every_v4_history_matches_its_immutable_compiled_definition() {
  for (model, hash, source) in [
    (APPROVAL_MODEL, APPROVAL_HASH, APPROVED_EVENTS),
    (APPROVAL_MODEL, APPROVAL_HASH, REJECTED_EVENTS),
    (APPROVAL_MODEL, APPROVAL_HASH, TIMEOUT_REJECTED_EVENTS),
    (
      APPROVAL_TIMEOUT_FAIL_MODEL,
      APPROVAL_TIMEOUT_FAIL_HASH,
      TIMEOUT_FAILED_EVENTS,
    ),
  ] {
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(model).unwrap();
    let history = events(source);
    let store = DurableEventStore::open_in_memory().unwrap();
    let mut engine = DurableDagEngine::new_for_event_history(workflow, hash, store).unwrap();
    append_history(&mut engine, &history, history.len());
    assert_eq!(engine.events(&history[0].run_id).unwrap(), history);
  }
}

#[test]
fn waiting_is_reconstructed_and_invalid_resolution_histories_fail_closed() {
  let history = events(APPROVED_EVENTS);
  let waiting = fold_events(&history[..4]).unwrap();
  assert_eq!(waiting.status, RunStatus::Waiting);
  assert!(waiting.context.steps.get("editorApproval").is_none());
  assert!(matches!(
    waiting.approval_requests["editorApproval"].status,
    ApprovalRequestStatus::Waiting
  ));

  let mut work_while_waiting = history[..4].to_vec();
  let mut attempt = history[5].clone();
  attempt.sequence = 5;
  work_while_waiting.push(attempt);
  assert!(fold_events(&work_while_waiting).is_err());

  let mut wrong_request = history[..5].to_vec();
  let RunEventPayload::ApprovalResolved(data) = &mut wrong_request[4].payload else {
    panic!("fixture must resolve approval");
  };
  data.request_id = "aprreq_wrong_request".to_string();
  assert!(fold_events(&wrong_request).is_err());

  let mut late_human = history[..5].to_vec();
  late_human[4].occurred_at = late_human[3]
    .occurred_at
    .checked_add_signed(Duration::hours(24))
    .unwrap();
  assert!(fold_events(&late_human).is_err());
}

#[test]
fn approved_history_round_trips_through_sqlite_and_reopens_identically() {
  let database = TemporaryDatabase::new("approved-history");
  let history = events(APPROVED_EVENTS);
  let expected = fold_events(&history).unwrap();

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine =
      DurableDagEngine::new_for_event_history(approval_model(), APPROVAL_HASH, store).unwrap();
    append_history(&mut engine, &history, history.len());
    assert_eq!(engine.projection(&history[0].run_id).unwrap(), expected);
  }

  let reopened = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(reopened.projection(&history[0].run_id).unwrap(), expected);
  assert_eq!(reopened.events(&history[0].run_id).unwrap(), history);
}

#[test]
fn secure_tokens_reissue_after_restart_without_entering_events_or_context() {
  let database = TemporaryDatabase::new("tokens");
  let history = events(APPROVED_EVENTS);
  let run_id = history[0].run_id.clone();
  let issued_at = history[3].occurred_at + Duration::seconds(1);

  let first = {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine =
      DurableDagEngine::new_for_event_history(approval_model(), APPROVAL_HASH, store).unwrap();
    append_history(&mut engine, &history, 4);
    let mut store = engine.into_store();
    let token = store
      .issue_approval_token(&run_id, "editorApproval", "aprreq_editor_ok", issued_at)
      .unwrap();
    assert_eq!(token.token.len(), 4 + 32 + 1 + 64);
    assert_eq!(
      token.credential_expires_at,
      history[3].occurred_at + Duration::hours(24)
    );
    token
  };

  let second = {
    let mut reopened = DurableEventStore::open(database.path()).unwrap();
    assert_eq!(
      reopened.projection(&run_id).unwrap().status,
      RunStatus::Waiting
    );
    assert_eq!(reopened.events(&run_id).unwrap(), history[..4]);
    let binding = reopened
      .verify_approval_token(&first.token, issued_at + Duration::minutes(1))
      .unwrap();
    assert_eq!(binding.request_id, "aprreq_editor_ok");
    assert_eq!(binding.approval_id, "editorApproval");

    let token = reopened
      .reissue_approval_token(
        &run_id,
        "editorApproval",
        "aprreq_editor_ok",
        issued_at + Duration::hours(1),
      )
      .unwrap();
    assert_ne!(token.token, first.token);
    assert_eq!(
      reopened
        .approval_token_count_for_request(&run_id, "editorApproval", "aprreq_editor_ok")
        .unwrap(),
      2
    );
    reopened
      .verify_approval_token(&first.token, issued_at + Duration::hours(2))
      .unwrap();
    assert_eq!(reopened.events(&run_id).unwrap(), history[..4]);
    token
  };

  let connection = Connection::open(database.path()).unwrap();
  let hash_lengths: Vec<i64> = connection
    .prepare("SELECT length(secret_hash) FROM woml_approval_tokens ORDER BY token_id")
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap();
  assert_eq!(hash_lengths, [32, 32]);
  assert!(connection
    .execute(
      "UPDATE woml_approval_tokens SET request_id = 'changed' WHERE token_id = ?1",
      [&first.token_id],
    )
    .is_err());
  assert!(connection
    .execute(
      "DELETE FROM woml_approval_tokens WHERE token_id = ?1",
      [&second.token_id],
    )
    .is_err());
  drop(connection);

  let database_bytes = std::fs::read(database.path()).unwrap();
  assert!(!database_bytes
    .windows(first.token.len())
    .any(|window| window == first.token.as_bytes()));
  let first_secret = first.token.split_once('.').unwrap().1.as_bytes();
  assert!(!database_bytes
    .windows(first_secret.len())
    .any(|window| window == first_secret));

  let store = DurableEventStore::open(database.path()).unwrap();
  let mut wrong = first.token.clone().into_bytes();
  *wrong.last_mut().unwrap() = if *wrong.last().unwrap() == b'0' {
    b'1'
  } else {
    b'0'
  };
  let wrong = String::from_utf8(wrong).unwrap();
  assert!(matches!(
    store.verify_approval_token(&wrong, issued_at + Duration::minutes(1)),
    Err(DurableStoreError::InvalidApprovalToken)
  ));
  assert!(matches!(
    store.verify_approval_token(&first.token, first.credential_expires_at),
    Err(DurableStoreError::ExpiredApprovalToken)
  ));
}

#[test]
fn migrates_v1_to_v2_without_rewriting_existing_definitions_runs_or_events() {
  let database = TemporaryDatabase::new("migration");
  let hello_events: Vec<RunEvent> = events(HELLO_EVENTS);
  let original_event_json = hello_events
    .iter()
    .map(serde_json::to_string)
    .collect::<Result<Vec<_>, _>>()
    .unwrap();

  {
    let connection = Connection::open(database.path()).unwrap();
    connection
      .execute_batch(
        "CREATE TABLE woml_store_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE woml_definitions (
           definition_hash TEXT PRIMARY KEY, workflow_id TEXT NOT NULL,
           schema_version INTEGER NOT NULL, model_json TEXT NOT NULL, created_at TEXT NOT NULL
         );
         CREATE TABLE woml_runs (
           run_id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL,
           definition_hash TEXT NOT NULL, created_at TEXT NOT NULL
         );
         CREATE TABLE woml_run_events (
           run_id TEXT NOT NULL, sequence INTEGER NOT NULL,
           event_id TEXT NOT NULL UNIQUE, event_schema_version INTEGER NOT NULL,
           event_json TEXT NOT NULL, PRIMARY KEY (run_id, sequence)
         );
         INSERT INTO woml_store_metadata(key, value) VALUES ('schema_version', '1');",
      )
      .unwrap();
    connection
      .execute(
        "INSERT INTO woml_definitions VALUES (?1, 'hello', 1, ?2, ?3)",
        params![HELLO_HASH, HELLO_MODEL, Utc::now().to_rfc3339()],
      )
      .unwrap();
    connection
      .execute(
        "INSERT INTO woml_runs VALUES (?1, 'hello', ?2, ?3)",
        params![
          hello_events[0].run_id,
          HELLO_HASH,
          hello_events[0].occurred_at.to_rfc3339()
        ],
      )
      .unwrap();
    for (event, event_json) in hello_events.iter().zip(&original_event_json) {
      connection
        .execute(
          "INSERT INTO woml_run_events VALUES (?1, ?2, ?3, ?4, ?5)",
          params![
            event.run_id,
            event.sequence,
            event.event_id,
            event.event_schema_version,
            event_json,
          ],
        )
        .unwrap();
    }
  }

  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(store.events(&hello_events[0].run_id).unwrap(), hello_events);
  drop(store);

  let connection = Connection::open(database.path()).unwrap();
  let version: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, DURABLE_STORE_SCHEMA_VERSION.to_string());
  let migrated_event_json: Vec<String> = connection
    .prepare("SELECT event_json FROM woml_run_events ORDER BY sequence")
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap();
  assert_eq!(migrated_event_json, original_event_json);
  let stored_model: String = connection
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [HELLO_HASH],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(stored_model, HELLO_MODEL);
}

#[test]
fn rejects_corrupt_v2_shape_and_never_leaks_tokens_in_errors() {
  let database = TemporaryDatabase::new("corrupt-v2");
  {
    let connection = Connection::open(database.path()).unwrap();
    connection
      .execute_batch(
        "CREATE TABLE woml_store_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO woml_store_metadata(key, value) VALUES ('schema_version', '2');",
      )
      .unwrap();
  }
  let error = DurableEventStore::open(database.path()).unwrap_err();
  assert!(matches!(error, DurableStoreError::Contract(_)));

  let token = "apr_00000000000000000000000000000000.ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
  assert!(!error.to_string().contains(token));
}
