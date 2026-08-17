use std::{fs, path::PathBuf, sync::Arc, thread};

use rusqlite::Connection;
use serde_json::{json, Value};
use woml_engine::capability::CapabilityIdentityMode;
use woml_engine::{
  derive_operation_key, CapabilityCallIdentity, CapabilityCallLimits, CapabilityCallRequest,
  DurableEventStore, DurableStateError, DurableStateLimits, DurableStateStore, FixedStateClock,
  DURABLE_STORE_SCHEMA_VERSION,
};

struct TestDatabase(PathBuf);

impl TestDatabase {
  fn new(label: &str) -> Self {
    let root = std::env::temp_dir().join(format!(
      "woml-ds2-{label}-{}",
      uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&root).unwrap();
    Self(root.join("state.sqlite"))
  }

  fn path(&self) -> &PathBuf {
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

fn call(operation: &str, input: Value, name: Option<&str>, seed: &str) -> CapabilityCallRequest {
  let step_key = format!("sha256:{}", "a".repeat(64));
  let operation_name = name.map_or_else(
    || format!("state.{operation}"),
    |name| format!("state.{operation}.{name}"),
  );
  CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: format!("inv_{seed}"),
    call_id: format!("call_{seed}"),
    run_id: "run_ds2".to_string(),
    node_id: "remember".to_string(),
    attempt_number: 1,
    capability: "state".to_string(),
    operation: operation.to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: if name.is_some() {
        CapabilityIdentityMode::Named
      } else {
        CapabilityIdentityMode::Automatic
      },
      operation_key: derive_operation_key(&step_key, &operation_name),
      operation_name,
      step_idempotency_key: step_key,
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits::default(),
    input: json!({
      "contract": "woml.state",
      "contractVersion": 1,
      "kind": "request",
      "operation": operation,
      "input": input,
    }),
  }
}

fn data(execution: &woml_engine::DurableStateExecution) -> &Value {
  &execution.result["data"]
}

#[test]
fn store_v12_migrates_transactionally_through_state_v1_to_store_v14() {
  let database = TestDatabase::new("migration");
  drop(DurableEventStore::open(database.path()).unwrap());
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute_batch(
      "DROP TABLE woml_state_entries;
       DROP TABLE woml_state_mutations;
       DROP TABLE woml_state_quotas;
       DROP TABLE woml_runtime_owner;
       DROP TABLE woml_maintenance_lease;
       DROP TABLE woml_last_verified_backup;
       UPDATE woml_store_metadata SET value = '12' WHERE key = 'schema_version';",
    )
    .unwrap();
  drop(connection);

  DurableStateStore::open(database.path()).unwrap();
  let connection = Connection::open(database.path()).unwrap();
  let version: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, DURABLE_STORE_SCHEMA_VERSION.to_string());
  assert_eq!(DURABLE_STORE_SCHEMA_VERSION, 14);
  for object in [
    "woml_state_entries",
    "woml_state_mutations",
    "woml_state_quotas",
  ] {
    let exists: bool = connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [object],
        |row| row.get(0),
      )
      .unwrap();
    assert!(exists, "missing {object}");
  }
}

#[test]
fn failed_v12_migration_rolls_back_without_claiming_a_newer_store() {
  let database = TestDatabase::new("migration-rollback");
  drop(DurableEventStore::open(database.path()).unwrap());
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute_batch(
      "DROP TABLE woml_state_entries;
       DROP TABLE woml_state_mutations;
       DROP TABLE woml_state_quotas;
       CREATE TABLE woml_state_entries (wrong_column TEXT);
       UPDATE woml_store_metadata SET value = '12' WHERE key = 'schema_version';",
    )
    .unwrap();
  drop(connection);

  assert!(DurableEventStore::open(database.path()).is_err());
  let connection = Connection::open(database.path()).unwrap();
  let version: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, "12");
  let columns: Vec<String> = connection
    .prepare("PRAGMA table_info(woml_state_entries)")
    .unwrap()
    .query_map([], |row| row.get(1))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap();
  assert_eq!(columns, vec!["wrong_column"]);
}

#[test]
fn all_state_operations_have_exact_results_and_monotonic_versions() {
  let database = TestDatabase::new("operations");
  let clock = Arc::new(FixedStateClock::new(1_786_525_200_000));
  let store = DurableStateStore::open_with(
    database.path(),
    clock.clone(),
    DurableStateLimits::default(),
  )
  .unwrap();

  let missing = store
    .execute(
      "agent",
      &call("get", json!({ "key": "count" }), None, "get0"),
    )
    .unwrap();
  assert_eq!(data(&missing), &json!({ "found": false }));

  let first = store
    .execute(
      "agent",
      &call(
        "set",
        json!({ "key": "count", "value": 10, "ifVersion": 0 }),
        Some("create-count"),
        "set1",
      ),
    )
    .unwrap();
  assert_eq!(
    data(&first),
    &json!({ "stored": true, "version": 1, "updatedAt": "2026-08-12T09:00:00.000Z" })
  );
  clock.advance(1_000);

  let second = store
    .execute(
      "agent",
      &call(
        "set",
        json!({ "key": "count", "value": 20, "ifVersion": 1 }),
        Some("replace-count"),
        "set2",
      ),
    )
    .unwrap();
  assert_eq!(data(&second)["version"], 2);

  let present = store
    .execute(
      "agent",
      &call("has", json!({ "key": "count" }), None, "has"),
    )
    .unwrap();
  assert_eq!(data(&present), &json!({ "present": true, "version": 2 }));

  let loser = store
    .execute(
      "agent",
      &call(
        "set_if_absent",
        json!({ "key": "count", "value": 99 }),
        Some("initialize-existing"),
        "absent1",
      ),
    )
    .unwrap();
  assert_eq!(data(&loser)["stored"], false);
  assert_eq!(data(&loser)["value"], 20);
  assert_eq!(data(&loser)["version"], 2);

  let incremented = store
    .execute(
      "agent",
      &call(
        "increment",
        json!({ "key": "count", "amount": 2, "ifVersion": 2 }),
        Some("add-two"),
        "increment",
      ),
    )
    .unwrap();
  assert_eq!(data(&incremented)["value"], 22);
  assert_eq!(data(&incremented)["version"], 3);

  let deleted = store
    .execute(
      "agent",
      &call(
        "delete",
        json!({ "key": "count", "ifVersion": 3 }),
        Some("remove-count"),
        "delete",
      ),
    )
    .unwrap();
  assert_eq!(data(&deleted), &json!({ "deleted": true }));
  assert_eq!(deleted.version, Some(4));

  let recreated = store
    .execute(
      "agent",
      &call(
        "set_if_absent",
        json!({ "key": "count", "value": 99 }),
        Some("recreate-count"),
        "absent2",
      ),
    )
    .unwrap();
  assert_eq!(data(&recreated)["stored"], true);
  assert_eq!(data(&recreated)["version"], 5);

  let found = store
    .execute(
      "agent",
      &call("get", json!({ "key": "count" }), None, "get1"),
    )
    .unwrap();
  assert_eq!(data(&found)["value"], 99);
  assert_eq!(data(&found)["version"], 5);

  let other_workflow = store
    .execute(
      "other",
      &call("get", json!({ "key": "count" }), None, "other"),
    )
    .unwrap();
  assert_eq!(data(&other_workflow), &json!({ "found": false }));
}

#[test]
fn duplicate_increment_reattaches_once_and_changed_input_fails_closed() {
  let database = TestDatabase::new("idempotency");
  let store = DurableStateStore::open(database.path()).unwrap();
  let increment = call(
    "increment",
    json!({ "key": "runs", "amount": 1 }),
    Some("count-run"),
    "increment",
  );
  let first = store.execute("workflow", &increment).unwrap();
  let duplicate = store.execute("workflow", &increment).unwrap();
  assert!(!first.duplicate);
  assert!(duplicate.duplicate);
  assert_eq!(duplicate.result, first.result);

  let changed = call(
    "increment",
    json!({ "key": "runs", "amount": 2 }),
    Some("count-run"),
    "changed-call-id-does-not-change-operation-key",
  );
  assert_eq!(
    store.execute("workflow", &changed),
    Err(DurableStateError::OperationIdentityConflict)
  );
  let current = store
    .execute(
      "workflow",
      &call("get", json!({ "key": "runs" }), None, "read"),
    )
    .unwrap();
  assert_eq!(data(&current)["value"], 1);
}

#[test]
fn compare_and_set_is_atomic_across_independent_connections() {
  let database = TestDatabase::new("contention");
  let store = DurableStateStore::open(database.path()).unwrap();
  store
    .execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "owner", "value": "initial", "ifVersion": 0 }),
        Some("initialize"),
        "initialize",
      ),
    )
    .unwrap();
  drop(store);

  let path_a = database.path().clone();
  let path_b = database.path().clone();
  let first = thread::spawn(move || {
    DurableStateStore::open(path_a).unwrap().execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "owner", "value": "a", "ifVersion": 1 }),
        Some("claim-a"),
        "claim-a",
      ),
    )
  });
  let second = thread::spawn(move || {
    DurableStateStore::open(path_b).unwrap().execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "owner", "value": "b", "ifVersion": 1 }),
        Some("claim-b"),
        "claim-b",
      ),
    )
  });
  let outcomes = [first.join().unwrap(), second.join().unwrap()];
  assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
  assert_eq!(
    outcomes
      .iter()
      .filter(|result| **result == Err(DurableStateError::Conflict))
      .count(),
    1
  );

  let store = DurableStateStore::open(database.path()).unwrap();
  let current = store
    .execute(
      "workflow",
      &call("get", json!({ "key": "owner" }), None, "read"),
    )
    .unwrap();
  assert_eq!(data(&current)["version"], 2);
}

#[test]
fn quota_and_value_failures_roll_back_without_evicting_existing_state() {
  let database = TestDatabase::new("quota");
  let store = DurableStateStore::open_with(
    database.path(),
    Arc::new(FixedStateClock::new(1_786_525_200_000)),
    DurableStateLimits {
      max_keys: 1,
      max_bytes: 8,
      max_value_bytes: 8,
    },
  )
  .unwrap();
  store
    .execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "kept", "value": "a" }),
        Some("keep"),
        "keep",
      ),
    )
    .unwrap();
  assert_eq!(
    store.execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "rejected", "value": "b" }),
        Some("reject"),
        "reject",
      ),
    ),
    Err(DurableStateError::QuotaExceeded)
  );
  assert_eq!(
    store.execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "kept", "value": "too-large" }),
        Some("oversized"),
        "oversized",
      ),
    ),
    Err(DurableStateError::ValueTooLarge)
  );
  let kept = store
    .execute(
      "workflow",
      &call("get", json!({ "key": "kept" }), None, "read"),
    )
    .unwrap();
  assert_eq!(data(&kept)["value"], "a");
  assert_eq!(data(&kept)["version"], 1);

  let connection = Connection::open(database.path()).unwrap();
  let quota: (u64, u64) = connection
    .query_row(
      "SELECT live_keys, value_bytes FROM woml_state_quotas",
      [],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap();
  assert_eq!(quota, (1, 3));
  let mutations: u64 = connection
    .query_row("SELECT COUNT(*) FROM woml_state_mutations", [], |row| {
      row.get(0)
    })
    .unwrap();
  assert_eq!(mutations, 1);
}

#[test]
fn metadata_is_redacted_and_corruption_fails_closed() {
  let database = TestDatabase::new("redaction-corruption");
  let store = DurableStateStore::open(database.path()).unwrap();
  let result = store
    .execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "private/customer:42", "value": { "token": "very-secret" } }),
        Some("remember-private-record"),
        "private",
      ),
    )
    .unwrap();
  let metadata = serde_json::to_string(&result.safe_metadata()).unwrap();
  for forbidden in [
    "private/customer:42",
    "very-secret",
    "run_ds2",
    "remember-private-record",
  ] {
    assert!(!metadata.contains(forbidden));
  }
  assert!(metadata.contains("keyDigest"));
  assert!(metadata.contains("inputDigest"));
  assert!(metadata.contains("resultDigest"));

  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute("UPDATE woml_state_entries SET value_json = 'not-json'", [])
    .unwrap();
  drop(connection);
  assert_eq!(
    store.execute(
      "workflow",
      &call(
        "get",
        json!({ "key": "private/customer:42" }),
        None,
        "corrupt"
      )
    ),
    Err(DurableStateError::StoreCorrupt)
  );
  drop(store);

  Connection::open(database.path())
    .unwrap()
    .execute("DROP TABLE woml_state_quotas", [])
    .unwrap();
  assert!(matches!(
    DurableStateStore::open(database.path()),
    Err(DurableStateError::StoreCorrupt)
  ));
}

#[test]
fn invalid_conditions_names_keys_and_integer_values_use_stable_failures() {
  let database = TestDatabase::new("failures");
  let store = DurableStateStore::open(database.path()).unwrap();
  let invalid_name = call(
    "set",
    json!({ "key": "key", "value": 1 }),
    None,
    "automatic-write",
  );
  assert_eq!(
    store.execute("workflow", &invalid_name),
    Err(DurableStateError::OperationNameInvalid)
  );
  assert_eq!(
    store.execute(
      "workflow",
      &call("get", json!({ "key": "" }), None, "bad-key")
    ),
    Err(DurableStateError::KeyInvalid)
  );

  store
    .execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "text", "value": "not-an-integer" }),
        Some("write-text"),
        "text",
      ),
    )
    .unwrap();
  assert_eq!(
    store.execute(
      "workflow",
      &call(
        "increment",
        json!({ "key": "text", "amount": 1 }),
        Some("increment-text"),
        "increment-text",
      ),
    ),
    Err(DurableStateError::IntegerRequired)
  );
  assert_eq!(
    store.execute(
      "workflow",
      &call(
        "set",
        json!({ "key": "text", "value": "changed", "ifVersion": 99 }),
        Some("stale-write"),
        "stale",
      ),
    ),
    Err(DurableStateError::Conflict)
  );
  assert_eq!(DurableStateError::Conflict.code(), "WOML_STATE_CONFLICT");
  assert!(!DurableStateError::Conflict.retryable());
  assert!(!DurableStateError::Conflict.ambiguous());
}
