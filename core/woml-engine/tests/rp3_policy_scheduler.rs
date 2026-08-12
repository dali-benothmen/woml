use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::event::StepAttemptStartedData;
use woml_engine::model::{BackoffPolicy, RetryPolicy, ValueExpression};
use woml_engine::{
  execute_admitted_trigger_run_durable, execute_workflow_durable, execute_workflow_durable_outcome,
  step_effect_idempotency_key, CompiledWorkflowDefinition, DurableEventStore, DurableStoreError,
  ManagedWorkflowCallsHandler, PolicyExecutionClaimResult, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, RuntimePolicyProgressPhase,
  ScriptHostProcessOptions, TriggerAdmissionRequest, WorkflowRuntimeOutcome,
  WorkflowTargetRegistry,
};

struct TestDatabase(std::path::PathBuf);

impl TestDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-rp3-{}.sqlite", Uuid::new_v4().simple())))
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn policy_model(workflow_id: &str, concurrency: u32, queue: &str) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  workflow.workflow_id = workflow_id.to_string();
  let policy = workflow.runtime_policy.as_mut().unwrap();
  policy.concurrency = Some(concurrency);
  policy.timeout_ms = None;
  policy.rate_limit = None;
  policy.queue.as_mut().unwrap().name = queue.to_string();
  workflow
}

fn admit(
  store: &mut DurableEventStore,
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  source: &str,
  received_at: chrono::DateTime<Utc>,
) -> String {
  store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: definition_hash.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: source.to_string(),
      payload: Map::from_iter([("source".to_string(), Value::String(source.to_string()))]),
      received_at,
    })
    .unwrap()
    .run_id
}

#[test]
fn concurrency_is_global_fifo_and_first_start_is_exactly_once() {
  let database = TestDatabase::new();
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let hash = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
  let workflow = policy_model("orders", 2, "orders");
  let (first, second, third) = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store.register_definition(&workflow, hash).unwrap();
    (
      admit(&mut store, &workflow, hash, "one", now),
      admit(
        &mut store,
        &workflow,
        hash,
        "two",
        now + chrono::Duration::milliseconds(1),
      ),
      admit(
        &mut store,
        &workflow,
        hash,
        "three",
        now + chrono::Duration::milliseconds(2),
      ),
    )
  };

  let mut process_a = DurableEventStore::open(&database.0).unwrap();
  let mut process_b = DurableEventStore::open(&database.0).unwrap();
  let first_claim = process_a
    .claim_policy_run(&first, "process-a", now, Duration::from_secs(30))
    .unwrap();
  let second_claim = process_b
    .claim_policy_run(&second, "process-b", now, Duration::from_secs(30))
    .unwrap();
  assert!(matches!(
    process_a
      .try_claim_policy_run(&third, "process-c", now, Duration::from_secs(30))
      .unwrap(),
    PolicyExecutionClaimResult::Waiting { .. }
  ));
  assert_eq!(
    process_b.active_policy_claim_count("orders", now).unwrap(),
    2
  );

  process_a
    .release_policy_claim(&first, "process-a", &first_claim.claim_id)
    .unwrap();
  let third_claim = process_b
    .claim_policy_run(&third, "process-c", now, Duration::from_secs(30))
    .unwrap();
  assert_eq!(
    process_a.active_policy_claim_count("orders", now).unwrap(),
    2
  );

  process_b
    .release_policy_claim(&third, "process-c", &third_claim.claim_id)
    .unwrap();
  process_a
    .claim_policy_run(&first, "process-d", now, Duration::from_secs(30))
    .unwrap();
  let starts = process_a
    .events(&first)
    .unwrap()
    .into_iter()
    .filter(|event| matches!(event.payload, RunEventPayload::RunExecutionStarted(_)))
    .count();
  assert_eq!(starts, 1);
  assert_eq!(
    process_a.projection(&first).unwrap().status,
    RunStatus::Running
  );

  process_b
    .release_policy_claim(&second, "process-b", &second_claim.claim_id)
    .unwrap();
}

#[test]
fn shared_queue_skips_an_older_run_blocked_by_its_workflow_limit() {
  let database = TestDatabase::new();
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let hash_a = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1";
  let hash_b = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb2";
  let workflow_a = policy_model("workflow-a", 1, "shared");
  let workflow_b = policy_model("workflow-b", 1, "shared");
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(&workflow_a, hash_a).unwrap();
  store.register_definition(&workflow_b, hash_b).unwrap();
  let a1 = admit(&mut store, &workflow_a, hash_a, "a1", now);
  let a2 = admit(
    &mut store,
    &workflow_a,
    hash_a,
    "a2",
    now + chrono::Duration::milliseconds(1),
  );
  let b1 = admit(
    &mut store,
    &workflow_b,
    hash_b,
    "b1",
    now + chrono::Duration::milliseconds(2),
  );

  store
    .claim_policy_run(&a1, "a-owner", now, Duration::from_secs(30))
    .unwrap();
  assert!(matches!(
    store
      .try_claim_policy_run(&a2, "a-waiter", now, Duration::from_secs(30))
      .unwrap(),
    PolicyExecutionClaimResult::Waiting { .. }
  ));
  assert!(matches!(
    store
      .try_claim_policy_run(&b1, "b-owner", now, Duration::from_secs(30))
      .unwrap(),
    PolicyExecutionClaimResult::Claimed { .. }
  ));
}

#[test]
fn separate_processes_cannot_oversubscribe_one_workflow() {
  let database = Arc::new(TestDatabase::new());
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let hash = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
  let workflow = policy_model("single-slot", 1, "single-slot");
  let (first, second) = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store.register_definition(&workflow, hash).unwrap();
    (
      admit(&mut store, &workflow, hash, "first", now),
      admit(
        &mut store,
        &workflow,
        hash,
        "second",
        now + chrono::Duration::milliseconds(1),
      ),
    )
  };
  let barrier = Arc::new(Barrier::new(2));
  let handles = [first, second]
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
            &format!("process-{index}"),
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
  assert_eq!(
    outcomes
      .iter()
      .filter(|outcome| matches!(outcome, PolicyExecutionClaimResult::Claimed { .. }))
      .count(),
    1
  );
  assert_eq!(
    DurableEventStore::open(&database.0)
      .unwrap()
      .active_policy_claim_count("single-slot", now)
      .unwrap(),
    1
  );
}

#[test]
fn expired_owner_with_an_ambiguous_attempt_fails_closed() {
  let database = TestDatabase::new();
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let hash = "sha256:4444444444444444444444444444444444444444444444444444444444444444";
  let workflow = policy_model("crash-safe", 1, "crash-safe");
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(&workflow, hash).unwrap();
  let run_id = admit(&mut store, &workflow, hash, "crash", now);
  store
    .claim_policy_run(&run_id, "dead-owner", now, Duration::from_secs(1))
    .unwrap();
  store
    .append_payload(
      &run_id,
      "attempt-started",
      now,
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "processOrder".to_string(),
        attempt: 1,
        invocation_id: "invocation-1".to_string(),
        handler: "runtime.script".to_string(),
        idempotency_key: Some(step_effect_idempotency_key(&run_id, hash, "processOrder")),
      }),
    )
    .unwrap();
  let after_expiry = now + chrono::Duration::seconds(2);
  assert!(matches!(
    store.try_claim_policy_run(
      &run_id,
      "replacement",
      after_expiry,
      Duration::from_secs(30)
    ),
    Err(DurableStoreError::SchedulerRecoveryRequired(candidate)) if candidate == run_id
  ));
  assert!(store
    .recover_policy_run_after_lease_loss(&run_id, after_expiry)
    .unwrap());
  assert_eq!(store.projection(&run_id).unwrap().status, RunStatus::Failed);
}

#[test]
fn queued_and_ownerless_runs_are_rediscovered_after_restart() {
  let database = TestDatabase::new();
  let now = Utc::now() - chrono::Duration::seconds(5);
  let hash = "sha256:4545454545454545454545454545454545454545454545454545454545454545";
  let workflow = policy_model("restart-safe", 1, "restart-safe");
  let (queued, ownerless) = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store.register_definition(&workflow, hash).unwrap();
    let queued = admit(&mut store, &workflow, hash, "queued-restart", now);
    let ownerless = admit(
      &mut store,
      &workflow,
      hash,
      "ownerless-restart",
      now + chrono::Duration::milliseconds(1),
    );
    let claim = store
      .claim_policy_run(&queued, "temporary-owner", now, Duration::from_secs(1))
      .unwrap();
    store
      .release_policy_claim(&queued, "temporary-owner", &claim.claim_id)
      .unwrap();
    (queued, ownerless)
  };

  let recovered = DurableEventStore::open(&database.0)
    .unwrap()
    .recover_undispatched_trigger_runs()
    .unwrap();
  let recovered_ids = recovered
    .iter()
    .map(|work| work.occurrence.run_id.as_str())
    .collect::<Vec<_>>();
  assert!(recovered_ids.contains(&queued.as_str()));
  assert!(recovered_ids.contains(&ownerless.as_str()));
}

#[tokio::test]
async fn model_v12_runs_through_the_existing_dag_executor_after_admission() {
  let bun_available = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success());
  if !bun_available {
    return;
  }
  let database = TestDatabase::new();
  let hash = "sha256:5555555555555555555555555555555555555555555555555555555555555555";
  let workflow = policy_model("runtime-policy-e2e", 1, "runtime-policy-e2e");
  let host =
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  let progress = Arc::new(Mutex::new(Vec::new()));
  let captured = Arc::clone(&progress);
  let options = RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host),
    2_000,
  )
  .with_runtime_policy_progress_reporter(Arc::new(move |message| {
    captured.lock().unwrap().push(message);
  }));

  let execution = execute_workflow_durable(
    workflow,
    hash.to_string(),
    Map::from_iter([("orderId".to_string(), Value::String("order-42".to_string()))]),
    options,
    database.0.clone(),
  )
  .await
  .unwrap();

  assert_eq!(execution.workflow_id, "runtime-policy-e2e");
  assert_eq!(execution.result, serde_json::json!({ "ok": true }));
  assert!(matches!(
    execution.events.first().map(|event| &event.payload),
    Some(RunEventPayload::RunAdmitted(_))
  ));
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::RunExecutionStarted(_)))
      .count(),
    1
  );
  assert_eq!(
    DurableEventStore::open(&database.0)
      .unwrap()
      .active_policy_claim_count("runtime-policy-e2e", Utc::now())
      .unwrap(),
    0
  );
  assert_eq!(
    progress
      .lock()
      .unwrap()
      .iter()
      .map(|message| message.phase)
      .collect::<Vec<_>>(),
    [
      RuntimePolicyProgressPhase::Eligible,
      RuntimePolicyProgressPhase::Started,
    ]
  );
}

#[tokio::test]
async fn retry_delay_releases_then_reacquires_the_workflow_slot() {
  let bun_available = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success());
  if !bun_available {
    return;
  }
  let database = TestDatabase::new();
  let hash = "sha256:6666666666666666666666666666666666666666666666666666666666666666";
  let mut workflow = policy_model("runtime-policy-retry", 1, "runtime-policy-retry");
  let node = workflow.graph.nodes.first_mut().unwrap();
  node.retry_policy = Some(RetryPolicy {
    max_attempts: 2,
    backoff: BackoffPolicy::Fixed { delay_ms: 500 },
  });
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(
        "if (attempt.number === 1) throw new Error('retry me'); return { ok: true };".to_string(),
      ),
    },
  );
  let host =
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  let options = RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host),
    2_000,
  );
  let state_path = database.0.clone();
  let execution = tokio::spawn(execute_workflow_durable(
    workflow,
    hash.to_string(),
    Map::new(),
    options,
    state_path.clone(),
  ));

  let mut observed_retry_wait = false;
  for _ in 0..100 {
    if let Ok(store) = DurableEventStore::open(&state_path) {
      if let Some(run) = store.list_runs_v2(1).unwrap().runs.first() {
        let retry_scheduled = store
          .events(&run.run_id)
          .unwrap()
          .iter()
          .any(|event| matches!(event.payload, RunEventPayload::StepRetryScheduled(_)));
        if retry_scheduled {
          assert_eq!(
            store
              .active_policy_claim_count("runtime-policy-retry", Utc::now())
              .unwrap(),
            0
          );
          observed_retry_wait = true;
          break;
        }
      }
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  assert!(
    observed_retry_wait,
    "the durable retry wait was not observed"
  );
  let result = execution.await.unwrap().unwrap();
  assert_eq!(result.result, serde_json::json!({ "ok": true }));
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::RunExecutionStarted(_)))
      .count(),
    1
  );
}

#[tokio::test]
async fn synchronous_workflow_call_releases_then_reacquires_the_parent_slot() {
  let bun_available = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success());
  if !bun_available {
    return;
  }
  let database = TestDatabase::new();
  let parent_hash = "sha256:7777777777777777777777777777777777777777777777777777777777777777";
  let child_hash = "sha256:8888888888888888888888888888888888888888888888888888888888888888";
  let mut parent_value: Value = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/workflow-calls/request-risk.compiled.v8.json"
  ))
  .unwrap();
  parent_value["schemaVersion"] = Value::from(12);
  parent_value["runtimePolicy"] = serde_json::json!({
    "profileVersion": 1,
    "concurrency": 1,
    "queue": {
      "name": "workflow-call-parent",
      "discipline": "work_conserving_fifo"
    }
  });
  parent_value["graph"]["nodes"][0]["inputs"]["fields"]["source"]["value"] =
    Value::String(
      "const risk = await services.workflows.call('calculate-risk', { customerId: 'customer-42' }); return { score: risk.score };"
        .to_string(),
    );
  let parent: CompiledWorkflowDefinition = serde_json::from_value(parent_value).unwrap();
  let mut child_value: Value = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/workflow-calls/calculate-risk.compiled.v10.json"
  ))
  .unwrap();
  child_value["schemaVersion"] = Value::from(12);
  child_value["runtimePolicy"] = serde_json::json!({
    "profileVersion": 1,
    "concurrency": 1,
    "queue": {
      "name": "workflow-call-child",
      "discipline": "work_conserving_fifo"
    }
  });
  child_value["graph"]["nodes"][0]["inputs"]["fields"]["source"]["value"] = Value::String(
    "await new Promise(resolve => setTimeout(resolve, 600)); return { score: 90 };".to_string(),
  );
  let child: CompiledWorkflowDefinition = serde_json::from_value(child_value).unwrap();
  let host =
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  let targets = Arc::new(WorkflowTargetRegistry::new("rp3-runtime").unwrap());
  targets.register(&parent, parent_hash).unwrap();
  targets.register(&child, child_hash).unwrap();
  targets.seal();
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(&child, child_hash).unwrap();
  drop(store);
  let options = RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host),
    2_000,
  );
  options
    .capability_registry
    .register(Arc::new(
      ManagedWorkflowCallsHandler::new(database.0.clone(), targets).with_execution(&options),
    ))
    .unwrap();
  let state_path = database.0.clone();
  let execution = tokio::spawn(execute_workflow_durable(
    parent,
    parent_hash.to_string(),
    Map::new(),
    options,
    state_path.clone(),
  ));
  let deadline = Instant::now() + Duration::from_secs(10);
  let run_id = loop {
    if let Ok(store) = DurableEventStore::open(&state_path) {
      if let Some(run) = store
        .list_runs_v2(10)
        .unwrap()
        .runs
        .into_iter()
        .find(|run| run.workflow_id == "request-risk")
      {
        break run.run_id;
      }
    }
    assert!(Instant::now() < deadline, "parent was not admitted");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  };
  let mut observed_suspended_slot = false;
  loop {
    let store = DurableEventStore::open(&database.0).unwrap();
    let projection = store.projection(&run_id).unwrap();
    if projection.status == RunStatus::Succeeded {
      break;
    }
    assert_ne!(
      projection.status,
      RunStatus::Failed,
      "parent failed: {:?}",
      store.events(&run_id).unwrap()
    );
    let child_admitted: bool = rusqlite::Connection::open(&database.0)
      .unwrap()
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM woml_workflow_calls WHERE parent_run_id = ?1)",
        [&run_id],
        |row| row.get(0),
      )
      .unwrap();
    if child_admitted
      && store
        .active_policy_claim_count("request-risk", Utc::now())
        .unwrap()
        == 0
    {
      observed_suspended_slot = true;
    }
    assert!(
      Instant::now() < deadline,
      "parent did not finish; runs={:?}; task_finished={}",
      store.list_runs_v2(10).unwrap().runs,
      execution.is_finished()
    );
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
  assert!(observed_suspended_slot);
  let result = execution.await.unwrap().unwrap();
  assert_eq!(result.result, serde_json::json!({ "score": 90 }));
}

#[tokio::test]
async fn queued_cancellation_finalizes_without_executing_business_steps() {
  let database = TestDatabase::new();
  let now = Utc::now();
  let hash = "sha256:9999999999999999999999999999999999999999999999999999999999999999";
  let workflow = policy_model("queued-cancellation", 1, "queued-cancellation");
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(&workflow, hash).unwrap();
  let active = admit(&mut store, &workflow, hash, "active", now);
  let queued = admit(
    &mut store,
    &workflow,
    hash,
    "queued",
    now + chrono::Duration::milliseconds(1),
  );
  let claim = store
    .claim_policy_run(&active, "active-owner", now, Duration::from_secs(30))
    .unwrap();
  store
    .request_run_cancellation(&queued, "cancel-queued", now)
    .unwrap();
  assert_eq!(
    store.projection(&queued).unwrap().status,
    RunStatus::Cancelling
  );
  store
    .release_policy_claim(&active, "active-owner", &claim.claim_id)
    .unwrap();
  drop(store);

  let result = execute_admitted_trigger_run_durable(
    database.0.clone(),
    &queued,
    RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new("bun", "unused-script-host.ts"),
      2_000,
    ),
  )
  .await;
  assert!(
    matches!(result, Err(RuntimeExecutionError::RunCancelled(_))),
    "unexpected queued cancellation result: {result:?}"
  );
  let store = DurableEventStore::open(&database.0).unwrap();
  assert_eq!(
    store.projection(&queued).unwrap().status,
    RunStatus::Cancelled
  );
  assert!(!store
    .events(&queued)
    .unwrap()
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_))));
  assert_eq!(
    store
      .active_policy_claim_count("queued-cancellation", Utc::now())
      .unwrap(),
    0
  );
}

#[tokio::test]
async fn human_approval_wait_releases_the_policy_slot() {
  let bun_available = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success());
  if !bun_available {
    return;
  }
  let database = TestDatabase::new();
  let hash = "sha256:abababababababababababababababababababababababababababababababab";
  let mut value: Value = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/approval.compiled.v4.json"
  ))
  .unwrap();
  value["schemaVersion"] = Value::from(12);
  value["runtimePolicy"] = serde_json::json!({
    "profileVersion": 1,
    "concurrency": 1,
    "queue": {
      "name": "approval-policy",
      "discipline": "work_conserving_fifo"
    }
  });
  for node in value["graph"]["nodes"].as_array_mut().unwrap() {
    if node["handler"] == "runtime.script" {
      node["scriptRuntime"] = serde_json::json!({
        "bindingVersion": 1,
        "bindings": ["context", "attempt", "services", "secrets"],
        "requiredSecrets": []
      });
    }
  }
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  let host =
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  let outcome = execute_workflow_durable_outcome(
    workflow,
    hash.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host),
      2_000,
    ),
    database.0.clone(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Waiting { run_id, .. } = outcome else {
    panic!("approval workflow did not pause");
  };
  let store = DurableEventStore::open(&database.0).unwrap();
  assert_eq!(
    store.projection(&run_id).unwrap().status,
    RunStatus::Waiting
  );
  assert_eq!(
    store
      .active_policy_claim_count("publish-article", Utc::now())
      .unwrap(),
    0
  );
}

#[tokio::test]
async fn model_v12_keeps_existing_parallel_execution_semantics() {
  let bun_available = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success());
  if !bun_available {
    return;
  }
  let database = TestDatabase::new();
  let hash = "sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
  let mut value: Value = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/parallel.compiled.v3.json"
  ))
  .unwrap();
  value["schemaVersion"] = Value::from(12);
  value["runtimePolicy"] = serde_json::json!({
    "profileVersion": 1,
    "concurrency": 1,
    "queue": {
      "name": "parallel-policy",
      "discipline": "work_conserving_fifo"
    }
  });
  for node in value["graph"]["nodes"].as_array_mut().unwrap() {
    if node["handler"] == "runtime.script" {
      node["scriptRuntime"] = serde_json::json!({
        "bindingVersion": 1,
        "bindings": ["context", "attempt", "services", "secrets"],
        "requiredSecrets": []
      });
    }
  }
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  let host =
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  let result = execute_workflow_durable(
    workflow,
    hash.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host),
      2_000,
    ),
    database.0.clone(),
  )
  .await
  .unwrap();
  assert_eq!(
    result.result,
    serde_json::json!({ "summary": "Weather 22°C, soil 41%" })
  );
  assert_eq!(
    DurableEventStore::open(&database.0)
      .unwrap()
      .active_policy_claim_count("field-report", Utc::now())
      .unwrap(),
    0
  );
}
