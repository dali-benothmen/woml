use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::{Duration, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde_json::Map;
use uuid::Uuid;
use woml_engine::{
  create_online_backup, inspect_backup_store, prepare_restored_store, record_verified_backup,
  run_presentation_from_store_v1, ApprovalRequestedData, ApprovalTimeoutPolicy, BackupError,
  CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, RunStatus,
  DURABLE_STORE_SCHEMA_VERSION, RUN_PRESENTATION_PROFILE,
};

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";
const MODULE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/modules/customer-import.compiled.v9.json");
const MODULE_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const APPROVAL_HASH: &str =
  "sha256:c85377270773c4abb178ba2811109843be53df66c91fedea04bb37d586901aa9";

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new(label: &str) -> Self {
    let path = std::env::temp_dir().join(format!("woml-pro7-{label}-{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&path).unwrap();
    Self(path)
  }

  fn join(&self, name: &str) -> PathBuf {
    self.0.join(name)
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

fn seed(path: &Path) {
  let model: CompiledWorkflowDefinition = serde_json::from_str(HELLO_MODEL).unwrap();
  let store = DurableEventStore::open(path).unwrap();
  let mut engine = DurableDagEngine::new(model, HELLO_HASH, store).unwrap();
  engine
    .start_run(
      "event_pro7_start",
      "run_pro7",
      Utc.with_ymd_and_hms(2026, 8, 12, 9, 0, 0).unwrap(),
      Map::new(),
    )
    .unwrap();
  drop(engine);
}

fn approval_first_model() -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(APPROVAL_MODEL).unwrap();
  workflow.graph.entry_node_ids = vec!["editorApproval".to_string()];
  workflow
    .graph
    .nodes
    .retain(|node| node.id != "prepareArticle");
  workflow
    .graph
    .edges
    .retain(|edge| edge.id != "prepareArticle-to-editorApproval");
  workflow
}

#[test]
fn online_backup_is_coherent_during_writes_and_restores_exact_history() {
  let directory = TestDirectory::new("online");
  let source = directory.join("source.sqlite");
  let backup = directory.join("backup.sqlite");
  let restored = directory.join("restored.sqlite");
  seed(&source);
  {
    let connection = Connection::open(&source).unwrap();
    connection
      .pragma_update(None, "journal_mode", "WAL")
      .unwrap();
    connection
      .execute_batch(
        "CREATE TABLE pro7_write_probe(sequence INTEGER PRIMARY KEY, body BLOB NOT NULL);",
      )
      .unwrap();
    let body = vec![7_u8; 1024];
    for sequence in 1..=2_000 {
      connection
        .execute(
          "INSERT INTO pro7_write_probe(sequence, body) VALUES (?1, ?2)",
          params![sequence, body],
        )
        .unwrap();
    }
  }
  let mut owner_store = DurableEventStore::open(&source).unwrap();
  let now = Utc::now();
  owner_store
    .acquire_runtime_owner(
      "deployment_pro7",
      &format!("sha256:{}", "a".repeat(64)),
      "runtime_pro7",
      now,
      now + Duration::seconds(30),
    )
    .unwrap();
  drop(owner_store);

  let barrier = Arc::new(Barrier::new(2));
  let writer_path = source.clone();
  let writer_barrier = Arc::clone(&barrier);
  let writer = thread::spawn(move || {
    let connection = Connection::open(writer_path).unwrap();
    connection
      .busy_timeout(std::time::Duration::from_secs(5))
      .unwrap();
    writer_barrier.wait();
    let body = vec![9_u8; 1024];
    for sequence in 2_001..=2_300 {
      connection
        .execute(
          "INSERT INTO pro7_write_probe(sequence, body) VALUES (?1, ?2)",
          params![sequence, body],
        )
        .unwrap();
    }
  });
  barrier.wait();
  let inventory = create_online_backup(
    &source,
    &backup,
    "lease_pro7_online",
    "test_pro7",
    "deployment_fallback",
  )
  .unwrap();
  writer.join().unwrap();
  assert_eq!(inventory.deployment_id.as_deref(), Some("deployment_pro7"));
  assert_eq!(inventory.definition_hashes, [HELLO_HASH]);

  let copied = Connection::open(&backup).unwrap();
  let (count, maximum): (u64, u64) = copied
    .query_row(
      "SELECT COUNT(*), MAX(sequence) FROM pro7_write_probe",
      [],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap();
  assert_eq!(count, maximum);
  fs::copy(&backup, &restored).unwrap();
  let prepared = prepare_restored_store(
    &restored,
    &[HELLO_HASH.to_string()],
    "backup_pro7_online",
    Utc::now(),
  )
  .unwrap();
  assert_eq!(prepared.store_version, DURABLE_STORE_SCHEMA_VERSION);
  assert!(prepared.runtime_instance_id.is_none());
  let restored_store = DurableEventStore::open(&restored).unwrap();
  assert_eq!(restored_store.events("run_pro7").unwrap().len(), 1);
  let presentation = run_presentation_from_store_v1(&restored_store, "run_pro7").unwrap();
  assert_eq!(presentation.profile, RUN_PRESENTATION_PROFILE);
  assert_eq!(presentation.workflow.id, "hello");
  assert_eq!(
    presentation.status,
    woml_engine::PresentationRunStatus::Running
  );

  record_verified_backup(&source, "backup_pro7_online", Utc::now()).unwrap();
  let connection = Connection::open(&source).unwrap();
  let recorded: String = connection
    .query_row(
      "SELECT backup_id FROM woml_last_verified_backup WHERE singleton = 1",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(recorded, "backup_pro7_online");
}

#[test]
fn restore_upgrades_v13_on_the_temporary_copy_and_rejects_future_stores() {
  let directory = TestDirectory::new("upgrade");
  let old = directory.join("v13.sqlite");
  seed(&old);
  let connection = Connection::open(&old).unwrap();
  connection
    .execute_batch(
      "DROP TABLE woml_runtime_owner;
       DROP TABLE woml_maintenance_lease;
       DROP TABLE woml_last_verified_backup;
       UPDATE woml_store_metadata SET value = '13' WHERE key = 'schema_version';",
    )
    .unwrap();
  drop(connection);
  assert_eq!(inspect_backup_store(&old).unwrap().store_version, 13);
  assert_eq!(
    prepare_restored_store(
      &old,
      &[HELLO_HASH.to_string()],
      "backup_pro7_v13",
      Utc::now(),
    )
    .unwrap()
    .store_version,
    14
  );

  let future = directory.join("future.sqlite");
  fs::copy(&old, &future).unwrap();
  Connection::open(&future)
    .unwrap()
    .execute(
      "UPDATE woml_store_metadata SET value = '999' WHERE key = 'schema_version'",
      [],
    )
    .unwrap();
  assert!(matches!(
    inspect_backup_store(&future),
    Err(BackupError::UnsupportedStoreVersion(999))
  ));
}

#[test]
fn maintenance_conflict_fails_without_leaving_a_partial_database() {
  let directory = TestDirectory::new("lease");
  let source = directory.join("source.sqlite");
  let backup = directory.join("partial.sqlite");
  seed(&source);
  Connection::open(&source)
    .unwrap()
    .execute(
      "INSERT INTO woml_maintenance_lease(
         singleton, lease_id, operation, owner_id, expires_at
       ) VALUES (1, 'lease_other', 'retention', 'other', ?1)",
      [(Utc::now() + Duration::minutes(5)).to_rfc3339()],
    )
    .unwrap();
  assert!(matches!(
    create_online_backup(
      &source,
      &backup,
      "lease_pro7_conflict",
      "test_pro7",
      "deployment_pro7"
    ),
    Err(BackupError::MaintenanceBusy)
  ));
  assert!(!backup.exists());
}

#[test]
fn backup_rejects_a_definition_with_a_missing_required_module_artifact() {
  let directory = TestDirectory::new("artifact");
  let source = directory.join("source.sqlite");
  let backup = directory.join("backup.sqlite");
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(MODULE_MODEL).unwrap();
  let mut store = DurableEventStore::open(&source).unwrap();
  store.register_definition(&workflow, MODULE_HASH).unwrap();
  drop(store);

  let error = create_online_backup(
    &source,
    &backup,
    "lease_pro7_artifact",
    "test_pro7",
    "deployment_pro7",
  )
  .expect_err("a required module artifact cannot be omitted from a verified backup");
  assert!(matches!(error, BackupError::Durable(_)));
  assert!(!backup.exists());
}

#[test]
fn restore_preserves_an_unresolved_human_approval_wait() {
  let directory = TestDirectory::new("approval");
  let source = directory.join("source.sqlite");
  let backup = directory.join("backup.sqlite");
  let restored = directory.join("restored.sqlite");
  let now = Utc::now();
  let store = DurableEventStore::open(&source).unwrap();
  let mut engine = DurableDagEngine::new(approval_first_model(), APPROVAL_HASH, store).unwrap();
  engine
    .start_run("event_pro7_approval", "run_pro7_approval", now, Map::new())
    .unwrap();
  engine
    .request_approval(
      "run_pro7_approval",
      now,
      ApprovalRequestedData {
        approval_id: "editorApproval".to_string(),
        request_id: "aprreq_pro7_restore".to_string(),
        expires_at: Some(now + Duration::hours(24)),
        on_timeout: ApprovalTimeoutPolicy::Reject,
      },
    )
    .unwrap();
  drop(engine);

  create_online_backup(
    &source,
    &backup,
    "lease_pro7_approval",
    "test_pro7",
    "deployment_pro7",
  )
  .unwrap();
  fs::copy(&backup, &restored).unwrap();
  prepare_restored_store(
    &restored,
    &[APPROVAL_HASH.to_string()],
    "backup_pro7_approval",
    now,
  )
  .unwrap();

  let restored_store = DurableEventStore::open(&restored).unwrap();
  let projection = restored_store.projection("run_pro7_approval").unwrap();
  assert_eq!(projection.status, RunStatus::Waiting);
  let approval = projection
    .approval_requests
    .get("editorApproval")
    .expect("the unresolved approval must survive restore");
  assert_eq!(approval.request_id, "aprreq_pro7_restore");
}
