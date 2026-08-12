use std::{
  fs,
  path::PathBuf,
  process::Command,
  sync::{Arc, Barrier},
  thread,
  time::{Duration, Instant},
};

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use woml_engine::capability::CapabilityIdentityMode;
use woml_engine::event::StepAttemptStartedData;
use woml_engine::{
  derive_operation_key, execute_workflow_durable, step_effect_idempotency_key,
  CapabilityCallIdentity, CapabilityCallLimits, CapabilityCallRequest, CompiledWorkflowDefinition,
  DurableEventStore, DurableStateError, DurableStateStore, OperationExecutionMode,
  OperationStartedData, OperationStatus, RunEventPayload, RunStartedData, RunStatus,
  RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const DEFINITION_HASH: &str =
  "sha256:4444444444444444444444444444444444444444444444444444444444444444";

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new(name: &str) -> Self {
    let path =
      std::env::temp_dir().join(format!("woml-ds4-{name}-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&path).unwrap();
    Self(path)
  }

  fn state(&self) -> PathBuf {
    self.0.join("state.sqlite")
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

fn call(
  operation: &str,
  input: Value,
  name: Option<&str>,
  seed: &str,
  step_key: Option<&str>,
) -> CapabilityCallRequest {
  let step_key = step_key
    .map(str::to_string)
    .unwrap_or_else(|| format!("sha256:{}", "a".repeat(64)));
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
    run_id: "run_ds4".to_string(),
    node_id: "loadCustomer".to_string(),
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

fn single_node_model() -> CompiledWorkflowDefinition {
  let mut model: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  model.workflow_id = "ds4-hardening".to_string();
  model.graph.nodes.retain(|node| node.id == "loadCustomer");
  model.graph.edges.clear();
  model.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  model.validate_structure().unwrap();
  model
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(())?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(PathBuf::from("bun"), host))
}

#[test]
#[ignore]
fn ds4_process_worker() {
  let Ok(path) = std::env::var("WOML_DS4_WORKER_STATE") else {
    return;
  };
  let worker = std::env::var("WOML_DS4_WORKER_ID").unwrap();
  let operations: usize = std::env::var("WOML_DS4_WORKER_OPERATIONS")
    .unwrap()
    .parse()
    .unwrap();
  let store = DurableStateStore::open(path).unwrap();
  for index in 0..operations {
    store
      .execute(
        "process-contention",
        &call(
          "increment",
          json!({ "key": "shared", "amount": 1 }),
          Some(&format!("worker-{worker}-{index}")),
          &format!("{worker}-{index}"),
          None,
        ),
      )
      .unwrap();
  }
}

#[test]
fn independent_processes_serialize_atomic_increments_without_lost_updates() {
  let directory = TestDirectory::new("process-contention");
  drop(DurableStateStore::open(directory.state()).unwrap());
  let executable = std::env::current_exe().unwrap();
  let workers = 4;
  let operations = 16;
  let mut children = Vec::new();
  for worker in 0..workers {
    children.push(
      Command::new(&executable)
        .args(["--exact", "ds4_process_worker", "--ignored", "--nocapture"])
        .env("WOML_DS4_WORKER_STATE", directory.state())
        .env("WOML_DS4_WORKER_ID", worker.to_string())
        .env("WOML_DS4_WORKER_OPERATIONS", operations.to_string())
        .spawn()
        .unwrap(),
    );
  }
  for child in &mut children {
    assert!(child.wait().unwrap().success());
  }
  let result = DurableStateStore::open(directory.state())
    .unwrap()
    .execute(
      "process-contention",
      &call("get", json!({ "key": "shared" }), None, "read", None),
    )
    .unwrap();
  assert_eq!(
    data(&result)["value"].as_u64(),
    Some((workers * operations) as u64)
  );
  assert_eq!(
    data(&result)["version"].as_u64(),
    Some((workers * operations) as u64)
  );
}

#[test]
fn concurrent_duplicate_identity_applies_once_while_distinct_names_both_apply() {
  let directory = TestDirectory::new("identity-contention");
  let store = Arc::new(DurableStateStore::open(directory.state()).unwrap());
  let barrier = Arc::new(Barrier::new(3));
  let mut duplicates = Vec::new();
  for worker in 0..2 {
    let store = Arc::clone(&store);
    let barrier = Arc::clone(&barrier);
    duplicates.push(thread::spawn(move || {
      barrier.wait();
      store.execute(
        "identity-contention",
        &call(
          "increment",
          json!({ "key": "same", "amount": 1 }),
          Some("same-operation"),
          &format!("duplicate-{worker}"),
          None,
        ),
      )
    }));
  }
  barrier.wait();
  let results = duplicates
    .into_iter()
    .map(|thread| thread.join().unwrap().unwrap())
    .collect::<Vec<_>>();
  assert_eq!(results.iter().filter(|result| result.duplicate).count(), 1);
  assert!(results.iter().all(|result| data(result)["value"] == 1));

  for (index, name) in ["independent-a", "independent-b"].into_iter().enumerate() {
    store
      .execute(
        "identity-contention",
        &call(
          "increment",
          json!({ "key": "same", "amount": 1 }),
          Some(name),
          &format!("independent-{index}"),
          None,
        ),
      )
      .unwrap();
  }
  let final_value = store
    .execute(
      "identity-contention",
      &call("get", json!({ "key": "same" }), None, "final", None),
    )
    .unwrap();
  assert_eq!(data(&final_value)["value"], 3);
  assert_eq!(data(&final_value)["version"], 3);
}

#[test]
fn database_lock_wait_is_bounded_and_short_contention_recovers() {
  let directory = TestDirectory::new("busy-backoff");
  let store = Arc::new(DurableStateStore::open(directory.state()).unwrap());
  let locker = Connection::open(directory.state()).unwrap();
  locker.execute_batch("BEGIN EXCLUSIVE").unwrap();
  let started = Instant::now();
  let waiting = {
    let store = Arc::clone(&store);
    thread::spawn(move || {
      store.execute(
        "busy-backoff",
        &call(
          "set",
          json!({ "key": "waited", "value": true }),
          Some("wait-for-lock"),
          "wait-for-lock",
          None,
        ),
      )
    })
  };
  thread::sleep(Duration::from_millis(150));
  locker.execute_batch("COMMIT").unwrap();
  waiting.join().unwrap().unwrap();
  assert!(started.elapsed() >= Duration::from_millis(125));
  assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn startup_integrity_audit_rejects_tampered_digests_quotas_and_results() {
  for corruption in ["key-digest", "quota", "result"] {
    let directory = TestDirectory::new(corruption);
    let store = DurableStateStore::open(directory.state()).unwrap();
    store
      .execute(
        "integrity-audit",
        &call(
          "set",
          json!({ "key": "private", "value": { "secret": "hidden" } }),
          Some("write-private"),
          corruption,
          None,
        ),
      )
      .unwrap();
    drop(store);
    let connection = Connection::open(directory.state()).unwrap();
    match corruption {
      "key-digest" => {
        connection
          .execute(
            "UPDATE woml_state_entries SET key_digest = ?1",
            [format!("sha256:{}", "0".repeat(64))],
          )
          .unwrap();
      }
      "quota" => {
        connection
          .execute(
            "UPDATE woml_state_quotas SET value_bytes = value_bytes + 1",
            [],
          )
          .unwrap();
      }
      "result" => {
        let trigger_sql: String = connection
          .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = 'woml_state_mutations_no_update'",
            [],
            |row| row.get(0),
          )
          .unwrap();
        connection
          .execute("DROP TRIGGER woml_state_mutations_no_update", [])
          .unwrap();
        let tampered = json!({
          "contract": "woml.state",
          "contractVersion": 1,
          "kind": "result",
          "operation": "set",
          "data": {
            "stored": true,
            "version": 999,
            "updatedAt": "2026-08-12T20:01:00Z"
          }
        });
        let encoded = canonical_json(&tampered).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(b"woml.state-result\0v1\0");
        hasher.update(&encoded);
        connection
          .execute(
            "UPDATE woml_state_mutations SET result_json = ?1, result_digest = ?2",
            (
              String::from_utf8(encoded).unwrap(),
              format!("sha256:{}", hex::encode(hasher.finalize())),
            ),
          )
          .unwrap();
        connection.execute_batch(&trigger_sql).unwrap();
      }
      _ => unreachable!(),
    }
    drop(connection);
    assert!(matches!(
      DurableStateStore::open(directory.state()),
      Err(DurableStateError::StoreCorrupt)
    ));
  }
}

#[test]
fn state_operation_latency_and_size_stay_within_ds4_budgets() {
  const OPERATIONS: usize = 256;
  const VALUE_BYTES: usize = 1_024;
  const P95_BUDGET_MS: f64 = 100.0;
  const DATABASE_BUDGET_BYTES: u64 = 16 * 1_024 * 1_024;

  let directory = TestDirectory::new("performance-budgets");
  let store = DurableStateStore::open(directory.state()).unwrap();
  let value = "x".repeat(VALUE_BYTES);
  let mut latencies = Vec::with_capacity(OPERATIONS);
  for index in 0..OPERATIONS {
    let execution = store
      .execute(
        "performance-budgets",
        &call(
          "set",
          json!({ "key": format!("entry-{index}"), "value": value }),
          Some(&format!("store-entry-{index}")),
          &format!("performance-{index}"),
          None,
        ),
      )
      .unwrap();
    latencies.push(execution.duration_ms);
  }
  latencies.sort_by(f64::total_cmp);
  let p95 = latencies[(OPERATIONS * 95).div_ceil(100) - 1];
  drop(store);
  let database_bytes = fs::metadata(directory.state()).unwrap().len();
  println!(
    "DS4 state benchmark: operations={OPERATIONS}, valueBytes={VALUE_BYTES}, p95Ms={p95:.3}, databaseBytes={database_bytes}"
  );
  assert!(
    p95 < P95_BUDGET_MS,
    "p95 {p95:.3} ms exceeded {P95_BUDGET_MS} ms"
  );
  assert!(
    database_bytes < DATABASE_BUDGET_BYTES,
    "database size {database_bytes} exceeded {DATABASE_BUDGET_BYTES} bytes"
  );
}

#[cfg(unix)]
#[test]
fn local_state_database_permissions_are_owner_only() {
  use std::os::unix::fs::PermissionsExt;
  let directory = TestDirectory::new("permissions");
  drop(DurableStateStore::open(directory.state()).unwrap());
  assert_eq!(
    fs::metadata(directory.state())
      .unwrap()
      .permissions()
      .mode()
      & 0o777,
    0o600
  );
}

#[test]
fn recovery_uses_a_committed_state_mutation_as_settlement_proof() {
  let directory = TestDirectory::new("settlement-recovery");
  let model = single_node_model();
  let run_id = "run_ds4_recovery";
  let invocation_id = "inv_ds4_recovery";
  let step_key = step_effect_idempotency_key(run_id, DEFINITION_HASH, "loadCustomer");
  let request = call(
    "increment",
    json!({ "key": "recovered-secret-key", "amount": 1 }),
    Some("recover-once"),
    "recovery",
    Some(&step_key),
  );
  let mut request = CapabilityCallRequest {
    run_id: run_id.to_string(),
    invocation_id: invocation_id.to_string(),
    ..request
  };
  request.call_id = "call_ds4_recovery".to_string();

  let mut event_store = DurableEventStore::open(directory.state()).unwrap();
  event_store
    .register_definition(&model, DEFINITION_HASH)
    .unwrap();
  event_store
    .append_payload(
      run_id,
      "evt-run-started",
      Utc::now(),
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: model.workflow_id.clone(),
        definition_hash: DEFINITION_HASH.to_string(),
        trigger_id: Some("start".to_string()),
        trigger_handler: Some("trigger.manual".to_string()),
        trigger_occurrence_id: Some("occ_ds4_recovery".to_string()),
        ingress: None,
        trigger: Map::new(),
      }),
    )
    .unwrap();
  event_store
    .append_payload(
      run_id,
      "evt-step-started",
      Utc::now(),
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "loadCustomer".to_string(),
        attempt: 1,
        invocation_id: invocation_id.to_string(),
        handler: "runtime.script".to_string(),
        idempotency_key: Some(step_key),
      }),
    )
    .unwrap();
  let state = DurableStateStore::open(directory.state()).unwrap();
  let mut metadata = state
    .execute("ds4-hardening", &request)
    .unwrap()
    .safe_metadata();
  for terminal_field in [
    "outcome",
    "resultDigest",
    "durationMs",
    "version",
    "valueBytes",
  ] {
    metadata.remove(terminal_field);
  }
  event_store
    .append_payload(
      run_id,
      "evt-operation-started",
      Utc::now(),
      RunEventPayload::OperationStarted(OperationStartedData {
        node_id: request.node_id.clone(),
        attempt_number: 1,
        invocation_id: invocation_id.to_string(),
        call_id: request.call_id.clone(),
        operation_key: request.identity.operation_key,
        capability: "state".to_string(),
        operation: "increment".to_string(),
        execution_mode: OperationExecutionMode::Managed,
        metadata,
      }),
    )
    .unwrap();

  let report = event_store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  let projection = event_store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(matches!(
    projection.operations.values().next().unwrap().status,
    OperationStatus::Succeeded { .. }
  ));
  let encoded = serde_json::to_string(&event_store.events(run_id).unwrap()).unwrap();
  assert!(!encoded.contains("recovered-secret-key"));
}

#[tokio::test]
async fn run_inspection_and_run_ownership_do_not_expose_or_own_state() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("inspection-retention");
  let mut model = single_node_model();
  let woml_engine::model::ValueExpression::Object { fields } = &mut model.graph.nodes[0].inputs
  else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(
        r#"
        await services.state.set("inspection-secret-key", {
          secret: "inspection-secret-value"
        }, { name: "remember-inspection-secret" });
        return { ok: true };
      "#
      ),
    },
  );
  let execution = execute_workflow_durable(
    model,
    DEFINITION_HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.state(),
  )
  .await
  .unwrap();
  let store = DurableEventStore::open(directory.state()).unwrap();
  let inspection =
    serde_json::to_string(&store.inspect_run_v2(&execution.run_id).unwrap()).unwrap();
  assert!(!inspection.contains("inspection-secret-key"));
  assert!(!inspection.contains("inspection-secret-value"));
  drop(store);

  let state = DurableStateStore::open(directory.state()).unwrap();
  let remembered = state
    .execute(
      "ds4-hardening",
      &call(
        "get",
        json!({ "key": "inspection-secret-key" }),
        None,
        "after-retention",
        None,
      ),
    )
    .unwrap();
  assert_eq!(
    data(&remembered)["value"]["secret"],
    "inspection-secret-value"
  );
  let connection = Connection::open(directory.state()).unwrap();
  for table in [
    "woml_state_entries",
    "woml_state_mutations",
    "woml_state_quotas",
  ] {
    let foreign_keys: u64 = connection
      .query_row(
        &format!("SELECT COUNT(*) FROM pragma_foreign_key_list('{table}')"),
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(foreign_keys, 0, "{table} must not be owned by a run");
  }
}
