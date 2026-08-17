use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use woml_engine::event::StepAttemptFailedData;
use woml_engine::{
  derive_operation_key, derive_workflow_call_key, execute_admitted_trigger_run_durable,
  step_effect_idempotency_key, AttemptFailure, AttemptFailureKind, CapabilityCallIdentity,
  CapabilityCallLimits, CapabilityCallRequest, CapabilityCancellationToken, CapabilityHandler,
  CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, DurableStoreError,
  ManagedWorkflowCallsHandler, OperationExecutionMode, OperationStartedData,
  OperationSucceededData, RunEventPayload, RunStatus, RuntimeExecutionOptions,
  ScriptHostProcessOptions, TriggerAdmissionRequest, WorkflowCallAdmissionRequest,
  WorkflowCallIndexState, WorkflowCallProgress, WorkflowRuntimeOutcome, WorkflowTargetRegistry,
};

const PARENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/request-risk.compiled.v8.json");
const CHILD_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/calculate-risk.compiled.v10.json");
const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const PARENT_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CHILD_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const APPROVAL_HASH: &str =
  "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-wc4-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
  }
}

fn parent_model(retry: bool) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(PARENT_MODEL).unwrap();
  if retry {
    value["graph"]["nodes"][0]["retryPolicy"] = json!({
      "maxAttempts": 2,
      "backoff": {
        "kind": "exponential",
        "initialDelayMs": 1,
        "multiplier": 2,
        "maximumDelayMs": 1
      }
    });
  }
  serde_json::from_value(value).unwrap()
}

fn child_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(CHILD_MODEL).unwrap()
}

fn approval_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(APPROVAL_MODEL).unwrap()
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(bun, host))
}

fn seed_parent(
  database: &TemporaryDatabase,
  retry: bool,
) -> (String, DateTime<Utc>, CompiledWorkflowDefinition) {
  let parent = parent_model(retry);
  let now = Utc::now();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&parent, PARENT_HASH).unwrap();
  store
    .register_definition(&child_model(), CHILD_HASH)
    .unwrap();
  let accepted = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: "request-risk".to_string(),
      definition_hash: PARENT_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: format!("wc4:{}", Uuid::new_v4().simple()),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &accepted.run_id).unwrap();
  engine
    .start_step_attempt(&accepted.run_id, "requestRisk", 1, "inv_wc4_1", now)
    .unwrap();
  drop(engine.into_store());
  (accepted.run_id, now, parent)
}

fn admission(
  parent_run_id: &str,
  parent_attempt: u32,
  target_workflow_id: &str,
  target_definition_hash: &str,
  payload: Map<String, Value>,
  at: DateTime<Utc>,
) -> WorkflowCallAdmissionRequest {
  let operation_name = "workflows.call";
  let step_key = step_effect_idempotency_key(parent_run_id, PARENT_HASH, "requestRisk");
  let call_key = derive_workflow_call_key(&step_key, target_workflow_id, operation_name);
  WorkflowCallAdmissionRequest {
    child_run_id: format!("run_call_{}", call_key.strip_prefix("sha256:").unwrap()),
    call_key,
    parent_run_id: parent_run_id.to_string(),
    parent_node_id: "requestRisk".to_string(),
    parent_attempt,
    target_workflow_id: target_workflow_id.to_string(),
    target_definition_hash: target_definition_hash.to_string(),
    payload,
    admitted_at: at,
  }
}

fn capability_request(parent_run_id: &str) -> CapabilityCallRequest {
  let step_key = step_effect_idempotency_key(parent_run_id, PARENT_HASH, "requestRisk");
  let operation_name = "workflows.call".to_string();
  CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: "inv_wc4_1".to_string(),
    call_id: "call_wc4".to_string(),
    run_id: parent_run_id.to_string(),
    node_id: "requestRisk".to_string(),
    attempt_number: 1,
    capability: "workflows".to_string(),
    operation: "call".to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: woml_engine::capability::CapabilityIdentityMode::Automatic,
      operation_key: derive_operation_key(&step_key, &operation_name),
      step_idempotency_key: step_key,
      operation_name,
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits::default(),
    input: json!({
      "contract": "woml.workflow-call",
      "contractVersion": 1,
      "kind": "request",
      "workflowId": "calculate-risk",
      "payload": { "customerId": "customer-42" },
      "options": { "timeoutMs": 2_000 }
    }),
  }
}

#[tokio::test]
async fn human_approval_target_is_rejected_before_child_admission() {
  let database = TemporaryDatabase::new("approval-preflight");
  let (parent_run_id, _, _) = seed_parent(&database, false);
  DurableEventStore::open(database.path())
    .unwrap()
    .register_definition(&approval_model(), APPROVAL_HASH)
    .unwrap();
  let targets = Arc::new(WorkflowTargetRegistry::new("runtime_wc6_approval").unwrap());
  targets.register(&approval_model(), APPROVAL_HASH).unwrap();
  targets.seal();
  let progress = Arc::new(Mutex::new(Vec::new()));
  let captured = Arc::clone(&progress);
  let execution = RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new("bun", "unused-script-host"),
    2_000,
  )
  .with_workflow_call_progress_reporter(Arc::new(move |message| {
    captured.lock().unwrap().push(message);
  }));
  let handler = ManagedWorkflowCallsHandler::new(database.path().to_path_buf(), targets)
    .with_execution(&execution);
  let mut request = capability_request(&parent_run_id);
  request.input["workflowId"] = json!("publish-article");

  let failure = handler.safe_request_metadata(&request).unwrap_err();
  assert_eq!(failure.code, "WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED");
  assert!(failure.message.contains("contains Human Approval"));
  assert!(failure.message.contains("Run it independently"));
  assert!(progress.lock().unwrap().iter().any(|message| matches!(
    message,
    WorkflowCallProgress::CallRejected {
      code,
      target_workflow_id,
      ..
    } if code == "WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED" && target_workflow_id == "publish-article"
  )));

  let connection = rusqlite::Connection::open(database.path()).unwrap();
  let child_count: i64 = connection
    .query_row("SELECT COUNT(*) FROM woml_workflow_calls", [], |row| {
      row.get(0)
    })
    .unwrap();
  assert_eq!(child_count, 0);
}

#[test]
fn a_later_step_attempt_reattaches_to_the_original_child_identity() {
  let database = TemporaryDatabase::new("retry-reattach");
  let (parent_run_id, now, _) = seed_parent(&database, true);
  let request = admission(
    &parent_run_id,
    1,
    "calculate-risk",
    CHILD_HASH,
    Map::from_iter([("customerId".to_string(), json!("customer-42"))]),
    now,
  );
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let first = store.admit_workflow_call(request.clone()).unwrap();
  let mut engine = DurableDagEngine::resume(store, &parent_run_id).unwrap();
  let failure = engine
    .record_step_attempt_failure(
      &parent_run_id,
      now,
      StepAttemptFailedData {
        node_id: "requestRisk".to_string(),
        attempt: 1,
        invocation_id: "inv_wc4_1".to_string(),
        failure: AttemptFailure {
          kind: AttemptFailureKind::ScriptThrew,
          code: "WOML_SCRIPT_THROWN".to_string(),
          message: "retry after the call".to_string(),
          ..AttemptFailure::legacy_defaults()
        },
      },
    )
    .unwrap();
  let scheduled_at = match failure.disposition {
    woml_engine::StepFailureDisposition::RetryScheduled { scheduled_at, .. } => scheduled_at,
    other => panic!("expected retry, got {other:?}"),
  };
  engine
    .start_step_attempt(
      &parent_run_id,
      "requestRisk",
      2,
      "inv_wc4_2",
      scheduled_at + ChronoDuration::milliseconds(1),
    )
    .unwrap();
  let mut store = engine.into_store();
  let mut retry_request = request;
  retry_request.parent_attempt = 2;
  retry_request.admitted_at = scheduled_at + ChronoDuration::milliseconds(1);
  let second = store.admit_workflow_call(retry_request).unwrap();

  assert!(second.duplicate);
  assert_eq!(second.admission.child_run_id, first.admission.child_run_id);
  assert_eq!(second.admission.parent_attempt, 1);
}

#[test]
fn recursive_self_and_indirect_calls_are_rejected_before_another_child_exists() {
  let database = TemporaryDatabase::new("cycles");
  let (parent_run_id, now, _) = seed_parent(&database, false);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let self_call = admission(
    &parent_run_id,
    1,
    "request-risk",
    PARENT_HASH,
    Map::new(),
    now,
  );
  assert!(matches!(
    store.admit_workflow_call(self_call),
    Err(DurableStoreError::WorkflowCallCycle)
  ));

  let first = store
    .admit_workflow_call(admission(
      &parent_run_id,
      1,
      "calculate-risk",
      CHILD_HASH,
      Map::new(),
      now,
    ))
    .unwrap();
  let mut child_engine = DurableDagEngine::resume(store, &first.admission.child_run_id).unwrap();
  child_engine
    .start_step_attempt(
      &first.admission.child_run_id,
      "calculate",
      1,
      "inv_wc4_child",
      now,
    )
    .unwrap();
  let mut store = child_engine.into_store();
  let child_step_key =
    step_effect_idempotency_key(&first.admission.child_run_id, CHILD_HASH, "calculate");
  let cycle_key = derive_workflow_call_key(&child_step_key, "request-risk", "workflows.call");
  let indirect = WorkflowCallAdmissionRequest {
    child_run_id: format!("run_call_{}", cycle_key.strip_prefix("sha256:").unwrap()),
    call_key: cycle_key,
    parent_run_id: first.admission.child_run_id.clone(),
    parent_node_id: "calculate".to_string(),
    parent_attempt: 1,
    target_workflow_id: "request-risk".to_string(),
    target_definition_hash: PARENT_HASH.to_string(),
    payload: Map::new(),
    admitted_at: now,
  };
  assert!(matches!(
    store.admit_workflow_call(indirect),
    Err(DurableStoreError::WorkflowCallCycle)
  ));
}

#[test]
fn one_concurrent_caller_claims_the_child_and_every_other_caller_observes_it() {
  let database = TemporaryDatabase::new("claim");
  let (parent_run_id, now, _) = seed_parent(&database, false);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let admitted = store
    .admit_workflow_call(admission(
      &parent_run_id,
      1,
      "calculate-risk",
      CHILD_HASH,
      Map::new(),
      now,
    ))
    .unwrap();
  drop(store);
  let barrier = Arc::new(Barrier::new(8));
  let claims = std::thread::scope(|scope| {
    (0..8)
      .map(|_| {
        let barrier = Arc::clone(&barrier);
        let path = database.path().to_path_buf();
        let call_key = admitted.admission.call_key.clone();
        scope.spawn(move || {
          barrier.wait();
          DurableEventStore::open(path)
            .unwrap()
            .claim_workflow_call_execution(&call_key)
            .unwrap()
        })
      })
      .collect::<Vec<_>>()
      .into_iter()
      .map(|thread| thread.join().unwrap())
      .collect::<Vec<_>>()
  });
  assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);
}

#[tokio::test]
async fn duplicate_transports_wait_for_and_return_the_same_executed_child() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("duplicate-transport");
  let (parent_run_id, _, _) = seed_parent(&database, false);
  let targets = Arc::new(WorkflowTargetRegistry::new("runtime_wc4").unwrap());
  targets.register(&child_model(), CHILD_HASH).unwrap();
  targets.seal();
  let execution = RuntimeExecutionOptions::new(host, 2_000);
  let handler = Arc::new(
    ManagedWorkflowCallsHandler::new(database.path().to_path_buf(), targets)
      .with_execution(&execution),
  );
  let request = capability_request(&parent_run_id);
  let first =
    handler.execute_request_scoped(&request, None, CapabilityCancellationToken::default());
  let second =
    handler.execute_request_scoped(&request, None, CapabilityCancellationToken::default());
  let (first, second) = tokio::join!(first, second);
  let first = first.unwrap();
  let second = second.unwrap();
  assert_eq!(first["childRunId"], second["childRunId"]);
  assert_eq!(first["result"], json!({ "score": 90 }));
  assert_eq!(second["result"], json!({ "score": 90 }));
  let connection = rusqlite::Connection::open(database.path()).unwrap();
  let child_count: i64 = connection
    .query_row(
      "SELECT COUNT(*) FROM woml_runs WHERE run_id LIKE 'run_call_%'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(child_count, 1);
}

#[tokio::test]
async fn child_success_and_operation_commit_still_fail_closed_before_parent_commit() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("post-result-crash");
  let (parent_run_id, now, _) = seed_parent(&database, false);
  let targets = Arc::new(WorkflowTargetRegistry::new("runtime_wc4_terminal").unwrap());
  targets.register(&child_model(), CHILD_HASH).unwrap();
  targets.seal();
  let execution = RuntimeExecutionOptions::new(host, 2_000);
  let handler = ManagedWorkflowCallsHandler::new(database.path().to_path_buf(), targets)
    .with_execution(&execution);
  let request = capability_request(&parent_run_id);
  let started_metadata = handler.safe_request_metadata(&request).unwrap();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .append_payload(
      parent_run_id.clone(),
      format!("evt_{}", Uuid::new_v4().simple()),
      now,
      RunEventPayload::OperationStarted(OperationStartedData {
        node_id: request.node_id.clone(),
        attempt_number: request.attempt_number,
        invocation_id: request.invocation_id.clone(),
        call_id: request.call_id.clone(),
        operation_key: request.identity.operation_key.clone(),
        capability: request.capability.clone(),
        operation: request.operation.clone(),
        execution_mode: OperationExecutionMode::Managed,
        metadata: started_metadata,
      }),
    )
    .unwrap();
  drop(store);

  let result = handler
    .execute_request_scoped(&request, None, CapabilityCancellationToken::default())
    .await
    .unwrap();
  let result_metadata = handler.safe_result_metadata(&result);
  assert_eq!(result_metadata["targetWorkflowId"], "calculate-risk");
  assert_eq!(result_metadata["lineageDepth"], 1);
  assert!(result_metadata.contains_key("payloadDigest"));
  assert!(!format!("{result_metadata:?}").contains("customer-42"));
  let encoded = serde_json::to_vec(&result).unwrap();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .append_payload(
      parent_run_id.clone(),
      format!("evt_{}", Uuid::new_v4().simple()),
      Utc::now(),
      RunEventPayload::OperationSucceeded(OperationSucceededData {
        node_id: request.node_id,
        attempt_number: request.attempt_number,
        invocation_id: request.invocation_id,
        call_id: request.call_id,
        operation_key: request.identity.operation_key,
        capability: request.capability,
        operation: request.operation,
        execution_mode: OperationExecutionMode::Managed,
        metadata: result_metadata,
        duration_ms: 1.0,
        result_bytes: encoded.len() as u64,
        result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&encoded))),
      }),
    )
    .unwrap();

  store.recover_interrupted_runs().unwrap();
  let parent = store.projection(&parent_run_id).unwrap();
  assert_eq!(parent.status, RunStatus::Failed);
  assert!(format!("{:?}", parent.failure).contains("Interrupted"));
  let child_run_id = result["childRunId"].as_str().unwrap();
  assert_eq!(
    store.projection(child_run_id).unwrap().status,
    RunStatus::Succeeded
  );
  let call_key = format!("sha256:{}", child_run_id.strip_prefix("run_call_").unwrap());
  assert_eq!(
    store.workflow_call(&call_key).unwrap().unwrap().state,
    WorkflowCallIndexState::Succeeded
  );
}

#[tokio::test]
async fn recovery_repairs_a_running_index_after_the_child_terminal_commit() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("terminal-index-repair");
  let (parent_run_id, now, _) = seed_parent(&database, false);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let admitted = store
    .admit_workflow_call(admission(
      &parent_run_id,
      1,
      "calculate-risk",
      CHILD_HASH,
      Map::from_iter([("customerId".to_string(), json!("customer-42"))]),
      now,
    ))
    .unwrap();
  assert!(store
    .claim_workflow_call_execution(&admitted.admission.call_key)
    .unwrap());
  drop(store);

  let outcome = execute_admitted_trigger_run_durable(
    database.path().to_path_buf(),
    &admitted.admission.child_run_id,
    RuntimeExecutionOptions::new(host, 2_000),
  )
  .await
  .unwrap();
  assert!(matches!(outcome, WorkflowRuntimeOutcome::Succeeded { .. }));
  let mut store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store
      .workflow_call(&admitted.admission.call_key)
      .unwrap()
      .unwrap()
      .state,
    WorkflowCallIndexState::Running
  );
  store.recover_interrupted_runs().unwrap();
  assert_eq!(
    store
      .workflow_call(&admitted.admission.call_key)
      .unwrap()
      .unwrap()
      .state,
    WorkflowCallIndexState::Succeeded
  );
  assert_eq!(
    store
      .projection(&admitted.admission.child_run_id)
      .unwrap()
      .result,
    Some(json!({ "score": 90 }))
  );
  assert_eq!(
    store.projection(&parent_run_id).unwrap().status,
    RunStatus::Failed
  );
}

#[test]
fn startup_recovery_fails_the_ambiguous_parent_and_reconstructs_child_index_state() {
  let database = TemporaryDatabase::new("recovery");
  let (parent_run_id, now, _) = seed_parent(&database, false);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let request = admission(
    &parent_run_id,
    1,
    "calculate-risk",
    CHILD_HASH,
    Map::new(),
    now,
  );
  let admitted = store.admit_workflow_call(request).unwrap();
  assert!(store
    .claim_workflow_call_execution(&admitted.admission.call_key)
    .unwrap());
  store
    .append_payload(
      parent_run_id.clone(),
      format!("evt_{}", Uuid::new_v4().simple()),
      now,
      RunEventPayload::OperationStarted(OperationStartedData {
        node_id: "requestRisk".to_string(),
        attempt_number: 1,
        invocation_id: "inv_wc4_1".to_string(),
        call_id: "call_wc4".to_string(),
        operation_key: derive_operation_key(
          &step_effect_idempotency_key(&parent_run_id, PARENT_HASH, "requestRisk"),
          "workflows.call",
        ),
        capability: "workflows".to_string(),
        operation: "call".to_string(),
        execution_mode: OperationExecutionMode::Managed,
        metadata: Map::from_iter([(
          "childRunId".to_string(),
          json!(admitted.admission.child_run_id),
        )]),
      }),
    )
    .unwrap();
  let report = store.recover_interrupted_runs().unwrap();
  assert!(report.recovered_runs >= 1);
  let parent = store.projection(&parent_run_id).unwrap();
  assert_eq!(parent.status, RunStatus::Failed);
  assert!(format!("{:?}", parent.failure).contains("Interrupted"));
  let call = store
    .workflow_call(&admitted.admission.call_key)
    .unwrap()
    .unwrap();
  assert_eq!(call.state, WorkflowCallIndexState::Admitted);
  let connection = rusqlite::Connection::open(database.path()).unwrap();
  let child_count: i64 = connection
    .query_row(
      "SELECT COUNT(*) FROM woml_runs WHERE run_id LIKE 'run_call_%'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(child_count, 1);
}
