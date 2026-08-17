use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::Map;
use uuid::Uuid;
use woml_engine::{
  execute_workflow_durable, execute_workflow_durable_outcome, resume_workflow_durable_outcome,
  settle_approval_timeout_durable, ApprovalTimeoutSettlementStatus, BusinessOutcome,
  CompiledWorkflowDefinition, DurableEventStore, LifecycleFailureKind, RunEventPayload,
  RunOutcomeDecidedData, RunStatus, RunTimeoutSettlement, RuntimeExecutionError,
  RuntimeExecutionOptions, ScriptHostProcessOptions, SystemEngineClock, TriggerAdmissionRequest,
  WorkflowRuntimeOutcome,
};

struct TestDatabase(std::path::PathBuf);

impl TestDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-rp5-{}.sqlite", Uuid::new_v4().simple())))
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn timeout_model(workflow_id: &str, timeout_ms: u64) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  workflow.workflow_id = workflow_id.to_string();
  let policy = workflow.runtime_policy.as_mut().unwrap();
  policy.concurrency = Some(1);
  policy.timeout_ms = Some(timeout_ms);
  policy.rate_limit = None;
  policy.queue.as_mut().unwrap().name = workflow_id.to_string();
  workflow
}

fn admit(
  store: &mut DurableEventStore,
  workflow: &CompiledWorkflowDefinition,
  hash: &str,
  now: chrono::DateTime<Utc>,
) -> String {
  store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: hash.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: format!("timeout:{}", workflow.workflow_id),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap()
    .run_id
}

#[test]
fn timeout_begins_at_first_execution_not_queue_admission() {
  let hash = "sha256:5555555555555555555555555555555555555555555555555555555555555555";
  let admitted_at = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
  let started_at = admitted_at + chrono::Duration::minutes(5);
  let workflow = timeout_model("timeout-boundary", 10_000);
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store.register_definition(&workflow, hash).unwrap();
  let run_id = admit(&mut store, &workflow, hash, admitted_at);

  assert!(matches!(
    store
      .settle_run_timeout(&run_id, admitted_at + chrono::Duration::hours(1))
      .unwrap(),
    RunTimeoutSettlement::NotConfigured
  ));
  store
    .claim_policy_run(&run_id, "owner", started_at, Duration::from_secs(30))
    .unwrap();
  let deadline = started_at + chrono::Duration::seconds(10);
  assert_eq!(
    store.projection(&run_id).unwrap().timeout_at,
    Some(deadline)
  );
  assert!(matches!(
    store
      .settle_run_timeout(&run_id, deadline - chrono::Duration::nanoseconds(1))
      .unwrap(),
    RunTimeoutSettlement::NotDue { deadline_at } if deadline_at == deadline
  ));
  let RunTimeoutSettlement::TimedOut { projection } =
    store.settle_run_timeout(&run_id, deadline).unwrap()
  else {
    panic!("the deadline must win at its exact boundary");
  };
  assert_eq!(projection.status, RunStatus::Finalizing);
  assert_eq!(projection.business_outcome, Some(BusinessOutcome::Failed));
  assert_eq!(
    projection
      .lifecycle_failure
      .as_ref()
      .map(|failure| failure.kind),
    Some(LifecycleFailureKind::TimedOut)
  );
  assert_eq!(
    projection
      .lifecycle_failure
      .as_ref()
      .map(|failure| failure.code.as_str()),
    Some("WOML_WORKFLOW_TIMED_OUT")
  );
  let events = store.events(&run_id).unwrap();
  let timeout_index = events
    .iter()
    .position(|event| matches!(event.payload, RunEventPayload::RunTimeoutReached(_)))
    .unwrap();
  let outcome_index = events
    .iter()
    .position(|event| matches!(event.payload, RunEventPayload::RunOutcomeDecided(_)))
    .unwrap();
  assert!(timeout_index < outcome_index);
}

#[test]
fn committed_business_outcome_and_cancellation_each_beat_timeout() {
  let hash = "sha256:5656565656565656565656565656565656565656565656565656565656565656";
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 11, 0, 0).unwrap();
  let workflow = timeout_model("timeout-races", 1_000);
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store.register_definition(&workflow, hash).unwrap();

  let succeeded = admit(&mut store, &workflow, hash, now);
  let success_claim = store
    .claim_policy_run(&succeeded, "success", now, Duration::from_secs(30))
    .unwrap();
  store
    .decide_run_outcome(
      &succeeded,
      RunOutcomeDecidedData::Succeeded {
        result: serde_json::json!({"ok": true}),
      },
      now + chrono::Duration::milliseconds(999),
    )
    .unwrap();
  assert!(matches!(
    store
      .settle_run_timeout(&succeeded, now + chrono::Duration::seconds(1))
      .unwrap(),
    RunTimeoutSettlement::LostRace { .. }
  ));
  assert!(!store
    .events(&succeeded)
    .unwrap()
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::RunTimeoutReached(_))));

  let cancelled = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: hash.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "timeout:cancelled".to_string(),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap()
    .run_id;
  store
    .release_policy_claim(&succeeded, "success", &success_claim.claim_id)
    .unwrap();
  store
    .claim_policy_run(&cancelled, "cancel", now, Duration::from_secs(30))
    .unwrap();
  store
    .request_run_cancellation(&cancelled, "cancel-command", now)
    .unwrap();
  assert!(matches!(
    store
      .settle_run_timeout(&cancelled, now + chrono::Duration::seconds(1))
      .unwrap(),
    RunTimeoutSettlement::LostRace { .. }
  ));
}

#[tokio::test]
async fn active_bun_script_is_stopped_by_the_total_workflow_timeout() {
  if !std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success())
  {
    return;
  }
  let database = TestDatabase::new();
  let hash = "sha256:5757575757575757575757575757575757575757575757575757575757575757";
  let mut workflow = timeout_model("timeout-runtime", 100);
  let source = workflow.graph.nodes[0].inputs.clone();
  let woml_engine::model::ValueExpression::Object { mut fields } = source else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: serde_json::Value::String(
        "await new Promise(resolve => setTimeout(resolve, 5_000)); return { late: true };"
          .to_string(),
      ),
    },
  );
  workflow.graph.nodes[0].inputs = woml_engine::model::ValueExpression::Object { fields };
  let host =
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  let options = RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host),
    10_000,
  );
  let started = std::time::Instant::now();
  let error = execute_workflow_durable(
    workflow,
    hash.to_string(),
    Map::new(),
    options,
    database.0.clone(),
  )
  .await
  .unwrap_err();
  assert!(started.elapsed() < Duration::from_secs(4));
  assert!(
    matches!(
      &error,
      RuntimeExecutionError::RunFailed(details)
        if details.code == "WOML_WORKFLOW_TIMED_OUT"
    ),
    "unexpected timeout error: {error:?}"
  );
  let store = DurableEventStore::open(&database.0).unwrap();
  let run = store
    .list_runs_v2(1)
    .unwrap()
    .runs
    .into_iter()
    .next()
    .unwrap();
  let events = store.events(&run.run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::RunTimeoutReached(_)))
      .count(),
    1
  );
  assert!(!events.iter().any(|event| matches!(
    event.payload,
    RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Succeeded { .. })
  )));
}

#[tokio::test]
async fn approval_wait_uses_the_workflow_deadline_and_resumes_as_timeout_failure() {
  if !std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success())
  {
    return;
  }
  let database = TestDatabase::new();
  let hash = "sha256:5858585858585858585858585858585858585858585858585858585858585858";
  let mut value: serde_json::Value = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/approval.compiled.v4.json"
  ))
  .unwrap();
  value["schemaVersion"] = serde_json::Value::from(12);
  value["runtimePolicy"] = serde_json::json!({
    "profileVersion": 1,
    "timeoutMs": 1_500,
    "queue": {
      "name": "approval-timeout",
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
  let runtime_options = || {
    RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new(std::path::PathBuf::from("bun"), host.clone()),
      2_000,
    )
  };
  let outcome = execute_workflow_durable_outcome(
    workflow,
    hash.to_string(),
    Map::new(),
    runtime_options(),
    database.0.clone(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Waiting {
    run_id, approval, ..
  } = outcome
  else {
    panic!("approval workflow did not pause");
  };
  let deadline = approval
    .expires_at
    .expect("workflow deadline is exposed to the waiter");
  assert!(deadline <= Utc::now() + chrono::Duration::seconds(2));
  let wait = (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO) + Duration::from_millis(50);
  tokio::time::sleep(wait).await;
  let settlement = settle_approval_timeout_durable(
    database.0.clone(),
    &run_id,
    &approval.approval_id,
    &SystemEngineClock,
  )
  .unwrap();
  assert_eq!(settlement.status, ApprovalTimeoutSettlementStatus::Settled);
  let error = resume_workflow_durable_outcome(database.0.clone(), &run_id, runtime_options())
    .await
    .unwrap_err();
  assert!(matches!(
    error,
    RuntimeExecutionError::RunFailed(details)
      if details.code == "WOML_WORKFLOW_TIMED_OUT"
  ));
}
