use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::{
  execute_retention, last_retention_result, plan_retention, CompiledWorkflowDefinition,
  DurableEventStore, RetentionError, RetentionPolicyV1, RunEventPayload, RunStartedData,
  RunSucceededData, ScriptRuntimeBindings,
};

const DEFINITION_HASH: &str =
  "sha256:1111111111111111111111111111111111111111111111111111111111111111";

struct TestDatabase(PathBuf);

impl TestDatabase {
  fn new(label: &str) -> Self {
    let root = std::env::temp_dir().join(format!("woml-pro8-{label}-{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&root).unwrap();
    Self(root.join("state.sqlite"))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    if let Some(parent) = self.0.parent() {
      let _ = fs::remove_dir_all(parent);
    }
  }
}

fn workflow() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/hello.compiled.v1.json"
  ))
  .unwrap();
  workflow.schema_version = 11;
  for node in &mut workflow.graph.nodes {
    if node.handler == "runtime.script" {
      node.script_runtime = Some(ScriptRuntimeBindings {
        binding_version: 1,
        bindings: vec![
          "context".to_string(),
          "attempt".to_string(),
          "services".to_string(),
          "secrets".to_string(),
        ],
        required_secrets: Vec::new(),
      });
    }
  }
  workflow
}

fn store(path: &Path) -> DurableEventStore {
  let mut store = DurableEventStore::open(path).unwrap();
  store
    .register_definition(&workflow(), DEFINITION_HASH)
    .unwrap();
  store
}

fn start_run(store: &mut DurableEventStore, run_id: &str, at: chrono::DateTime<Utc>) {
  store
    .append_payload(
      run_id,
      &format!("evt_{run_id}_start"),
      at,
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: "hello".to_string(),
        definition_hash: DEFINITION_HASH.to_string(),
        trigger_id: Some("start".to_string()),
        trigger_handler: Some("trigger.manual".to_string()),
        trigger_occurrence_id: Some(format!("occ_{run_id}")),
        ingress: None,
        trigger: Map::new(),
      }),
    )
    .unwrap();
}

fn succeed_run(store: &mut DurableEventStore, run_id: &str, at: chrono::DateTime<Utc>) {
  start_run(store, run_id, at);
  store
    .append_payload(
      run_id,
      &format!("evt_{run_id}_success"),
      at,
      RunEventPayload::RunSucceeded(RunSucceededData {
        terminal_node_id: "buildMessage".to_string(),
        result: json!({"run": run_id}),
      }),
    )
    .unwrap();
}

fn policy(now: chrono::DateTime<Utc>) -> RetentionPolicyV1 {
  let cutoff = now - Duration::days(30);
  RetentionPolicyV1 {
    policy_id: "retention_pro8_tests".to_string(),
    succeeded_before: cutoff,
    failed_before: cutoff,
    cancelled_before: cutoff,
  }
}

fn count(connection: &Connection, table: &str) -> i64 {
  connection
    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
      row.get(0)
    })
    .unwrap()
}

#[test]
fn dry_run_matches_effect_and_batches_without_touching_state_or_definitions() {
  let database = TestDatabase::new("journey");
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let old = now - Duration::days(90);
  let mut store = store(database.path());
  for index in 0..305 {
    succeed_run(&mut store, &format!("run_old_{index:04}"), old);
  }
  succeed_run(&mut store, "run_recent", now - Duration::days(2));
  start_run(&mut store, "run_active", old);
  for run_id in [
    "run_queued",
    "run_waiting",
    "run_retrying",
    "run_finalizing",
  ] {
    start_run(&mut store, run_id, old);
  }
  drop(store);
  let connection = Connection::open(database.path()).unwrap();
  for (run_id, status) in [
    ("run_queued", "queued"),
    ("run_waiting", "waiting"),
    ("run_retrying", "retrying"),
    ("run_finalizing", "finalizing"),
  ] {
    connection
      .execute(
        "UPDATE woml_run_summaries SET status = ?1, updated_at = ?2 WHERE run_id = ?3",
        params![status, old.to_rfc3339(), run_id],
      )
      .unwrap();
  }
  connection
    .execute(
      "INSERT INTO woml_state_entries(
         scope_digest, key_digest, key_text, value_json, value_bytes, version, updated_at
       ) VALUES (?1, ?2, 'counter', '42', 2, 1, ?3)",
      params![
        format!("sha256:{}", "a".repeat(64)),
        format!("sha256:{}", "b".repeat(64)),
        old.to_rfc3339(),
      ],
    )
    .unwrap();
  connection
    .execute(
      "INSERT INTO woml_state_quotas(scope_digest, live_keys, value_bytes)
       VALUES (?1, 1, 2)",
      [format!("sha256:{}", "a".repeat(64))],
    )
    .unwrap();
  drop(connection);

  let plan = plan_retention(database.path(), &policy(now), now).unwrap();
  assert_eq!(plan.eligible_runs, 305);
  assert!(plan.estimated_bytes > 0);
  assert_eq!(
    count(&Connection::open(database.path()).unwrap(), "woml_runs"),
    311
  );

  let execution = execute_retention(
    database.path(),
    &policy(now),
    "lease_pro8_journey",
    "test_pro8",
    false,
    now,
  )
  .unwrap();
  assert_eq!(execution.result.deleted_runs, plan.eligible_runs);
  assert_eq!(execution.result.deleted_bytes, plan.estimated_bytes);
  assert_eq!(execution.result.state_entries_deleted, 0);
  assert_eq!(execution.batches, 2);
  let connection = Connection::open(database.path()).unwrap();
  assert_eq!(count(&connection, "woml_runs"), 6);
  assert_eq!(count(&connection, "woml_state_entries"), 1);
  assert_eq!(count(&connection, "woml_state_mutations"), 0);
  assert_eq!(count(&connection, "woml_definitions"), 1);
  assert_eq!(count(&connection, "woml_definition_module_artifacts"), 0);
  assert_eq!(count(&connection, "woml_maintenance_lease"), 0);
  let guards: i64 = connection
    .query_row(
      "SELECT COUNT(*) FROM sqlite_master
       WHERE type = 'trigger' AND name IN (
         'woml_runs_no_delete', 'woml_run_events_no_delete',
         'woml_approval_tokens_no_delete', 'woml_notification_capabilities_no_delete',
         'woml_trigger_occurrences_no_delete',
         'woml_internal_event_publications_no_delete',
         'woml_internal_event_deliveries_no_delete', 'woml_workflow_calls_no_delete'
       )",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(guards, 8);
}

#[test]
fn dependencies_and_deduplication_protect_live_history_but_closed_groups_prune() {
  let database = TestDatabase::new("dependencies");
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let old = now - Duration::days(90);
  let mut store = store(database.path());
  succeed_run(&mut store, "run_trigger_protected", old);
  succeed_run(&mut store, "run_active_parent", old);
  start_run(&mut store, "run_active_child", old);
  succeed_run(&mut store, "run_closed_parent", old);
  succeed_run(&mut store, "run_closed_child", old);
  drop(store);
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute(
      "INSERT INTO woml_trigger_occurrences(
         occurrence_id, workflow_id, trigger_id, trigger_handler, definition_hash,
         source_identity_hash, payload_hash, received_at, run_id
       ) VALUES ('occ_protected', 'hello', 'start', 'trigger.manual', ?1, ?2, ?3, ?4, 'run_trigger_protected')",
      params![
        DEFINITION_HASH,
        format!("sha256:{}", "c".repeat(64)),
        format!("sha256:{}", "d".repeat(64)),
        (now - Duration::days(2)).to_rfc3339(),
      ],
    )
    .unwrap();
  for (key, parent, child, state) in [
    (
      "call_active",
      "run_active_parent",
      "run_active_child",
      "running",
    ),
    (
      "call_closed",
      "run_closed_parent",
      "run_closed_child",
      "succeeded",
    ),
  ] {
    connection
      .execute(
        "INSERT INTO woml_workflow_calls(
           call_key, parent_run_id, parent_node_id, parent_attempt, target_workflow_id,
           target_definition_hash, child_run_id, payload_digest, depth, state, admitted_at
         ) VALUES (?1, ?2, 'buildMessage', 1, 'hello', ?3, ?4, ?5, 1, ?6, ?7)",
        params![
          key,
          parent,
          DEFINITION_HASH,
          child,
          format!(
            "sha256:{}",
            if key == "call_active" { "e" } else { "f" }.repeat(64)
          ),
          state,
          old.to_rfc3339(),
        ],
      )
      .unwrap();
  }
  drop(connection);

  let plan = plan_retention(database.path(), &policy(now), now).unwrap();
  assert_eq!(plan.eligible_runs, 2);
  let execution = execute_retention(
    database.path(),
    &policy(now),
    "lease_pro8_dependencies",
    "test_pro8",
    false,
    now,
  )
  .unwrap();
  assert_eq!(execution.result.deleted_runs, 2);
  let connection = Connection::open(database.path()).unwrap();
  for retained in [
    "run_trigger_protected",
    "run_active_parent",
    "run_active_child",
  ] {
    let exists: bool = connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM woml_runs WHERE run_id = ?1)",
        [retained],
        |row| row.get(0),
      )
      .unwrap();
    assert!(exists, "{retained} must remain protected");
  }
  assert_eq!(count(&connection, "woml_workflow_calls"), 1);
}

#[test]
fn a_live_maintenance_lease_rejects_retention_without_partial_deletion() {
  let database = TestDatabase::new("lease");
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let mut store = store(database.path());
  succeed_run(&mut store, "run_old", now - Duration::days(90));
  drop(store);
  Connection::open(database.path())
    .unwrap()
    .execute(
      "INSERT INTO woml_maintenance_lease(
         singleton, lease_id, operation, owner_id, expires_at
       ) VALUES (1, 'lease_backup', 'backup', 'other', ?1)",
      [(now + Duration::minutes(5)).to_rfc3339()],
    )
    .unwrap();
  assert!(matches!(
    execute_retention(
      database.path(),
      &policy(now),
      "lease_pro8_conflict",
      "test_pro8",
      false,
      now,
    ),
    Err(RetentionError::MaintenanceBusy)
  ));
  assert_eq!(
    count(&Connection::open(database.path()).unwrap(), "woml_runs"),
    1
  );
}

#[test]
fn an_empty_successful_pass_still_records_a_durable_audit_result() {
  let database = TestDatabase::new("empty-audit");
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let mut store = store(database.path());
  succeed_run(&mut store, "run_recent", now - Duration::days(1));
  drop(store);

  let execution = execute_retention(
    database.path(),
    &policy(now),
    "lease_pro8_empty",
    "test_pro8",
    false,
    now,
  )
  .unwrap();
  assert_eq!(execution.result.deleted_runs, 0);
  assert_eq!(
    last_retention_result(database.path()).unwrap(),
    Some(execution.result)
  );
}
