use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::event::{StepAttemptStartedData, StepAttemptSucceededData};
use woml_engine::{
  resolve_human_approval_durable, resume_workflow_durable_outcome, settle_approval_timeout_durable,
  ApprovalDecision, ApprovalDecisionOutcomeStatus, ApprovalDecisionSource, ApprovalResolution,
  ApprovalTimeoutSettlementStatus, CompiledWorkflowDefinition, DurableEventStore,
  DurableStoreError, FixedEngineClock, RunEventPayload, RunStatus, RuntimeExecutionError,
  RuntimeExecutionOptions, ScriptHostProcessOptions, WorkflowRuntimeOutcome,
};

const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const APPROVAL_TIMEOUT_FAIL_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/approval-timeout-fail.compiled.v4.json");
const APPROVAL_HASH: &str =
  "sha256:c85377270773c4abb178ba2811109843be53df66c91fedea04bb37d586901aa9";
const TEST_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn base_time() -> DateTime<Utc> {
  Utc.with_ymd_and_hms(2026, 8, 7, 10, 0, 0).unwrap()
}

fn model(source: &str) -> CompiledWorkflowDefinition {
  serde_json::from_str(source).unwrap()
}

fn approval_first(mut workflow: CompiledWorkflowDefinition) -> CompiledWorkflowDefinition {
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

fn nested_approval_model() -> CompiledWorkflowDefinition {
  let mut workflow = approval_first(model(APPROVAL_MODEL));
  workflow.graph.nodes.retain(|node| node.id != "publish");
  workflow.graph.nodes.push(
    serde_json::from_value(json!({
      "id": "legalApproval",
      "handler": "engine.approval-wait",
      "inputs": {
        "kind": "object",
        "fields": {
          "timeoutMs": { "kind": "literal", "value": 86400000 },
          "onTimeout": { "kind": "literal", "value": "reject" }
        }
      },
      "metadata": { "name": "Legal approval" }
    }))
    .unwrap(),
  );
  workflow.graph.nodes.push(
    serde_json::from_value(json!({
      "id": "__woml_approval__legalApproval__join",
      "handler": "engine.approval-join",
      "inputs": { "kind": "object", "fields": {} }
    }))
    .unwrap(),
  );

  workflow
    .graph
    .edges
    .iter_mut()
    .find(|edge| edge.id == "editorApproval:approved")
    .unwrap()
    .to = "legalApproval".to_string();
  workflow
    .graph
    .edges
    .iter_mut()
    .find(|edge| edge.id == "editorApproval:approved:join")
    .unwrap()
    .from = "__woml_approval__legalApproval__join".to_string();
  workflow.graph.edges.extend([
    serde_json::from_value(json!({
      "id": "legalApproval:approved",
      "from": "legalApproval",
      "to": "__woml_approval__legalApproval__join",
      "condition": {
        "kind": "equals",
        "left": { "kind": "contextReference", "path": ["steps", "legalApproval", "decision"] },
        "right": { "kind": "literal", "value": "approved" }
      },
      "approvalId": "legalApproval"
    }))
    .unwrap(),
    serde_json::from_value(json!({
      "id": "legalApproval:rejected",
      "from": "legalApproval",
      "to": "__woml_approval__legalApproval__join",
      "condition": {
        "kind": "equals",
        "left": { "kind": "contextReference", "path": ["steps", "legalApproval", "decision"] },
        "right": { "kind": "literal", "value": "rejected" }
      },
      "approvalId": "legalApproval"
    }))
    .unwrap(),
  ]);
  workflow
}

fn unavailable_options(now: DateTime<Utc>) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(
      PathBuf::from("/woml-a5-no-such-bun"),
      PathBuf::from("/woml-a5-no-such-host.ts"),
    ),
    2_000,
  )
  .with_clock(Arc::new(FixedEngineClock::new(now)))
}

fn real_options(now: DateTime<Utc>) -> Option<RuntimeExecutionOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host.exists().then(|| {
    RuntimeExecutionOptions::new(ScriptHostProcessOptions::new(bun, host), 2_000)
      .with_clock(Arc::new(FixedEngineClock::new(now)))
  })
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-a5-{label}-{}.sqlite",
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

fn waiting_parts(
  outcome: WorkflowRuntimeOutcome,
) -> (String, woml_engine::WaitingWorkflowApproval) {
  let WorkflowRuntimeOutcome::Waiting {
    run_id, approval, ..
  } = outcome
  else {
    panic!("workflow must be waiting");
  };
  (run_id, approval)
}

#[tokio::test]
async fn approve_and_reject_are_idempotent_and_execute_only_the_selected_route() {
  let Some(_) = real_options(base_time()) else {
    return;
  };
  for (decision, selected, unselected, expected_published) in [
    (
      ApprovalDecision::Approved,
      "publish",
      "recordRejection",
      true,
    ),
    (
      ApprovalDecision::Rejected,
      "recordRejection",
      "publish",
      false,
    ),
  ] {
    let database = TemporaryDatabase::new(match decision {
      ApprovalDecision::Approved => "approved",
      ApprovalDecision::Rejected => "rejected",
    });
    let started_at = base_time();
    let waiting = woml_engine::execute_workflow_durable_outcome(
      model(APPROVAL_MODEL),
      APPROVAL_HASH.to_string(),
      Map::new(),
      real_options(started_at).unwrap(),
      database.path().to_path_buf(),
    )
    .await
    .unwrap();
    let (run_id, approval) = waiting_parts(waiting);
    let decided_at = started_at + Duration::minutes(1);
    let clock = FixedEngineClock::new(decided_at);
    let accepted = resolve_human_approval_durable(
      database.path().to_path_buf(),
      &approval.token,
      decision,
      &clock,
    )
    .unwrap();
    assert_eq!(accepted.status, ApprovalDecisionOutcomeStatus::Accepted);
    assert_eq!(accepted.decided_at, decided_at);

    let duplicate = resolve_human_approval_durable(
      database.path().to_path_buf(),
      &approval.token,
      decision,
      &FixedEngineClock::new(decided_at + Duration::seconds(1)),
    )
    .unwrap();
    assert_eq!(
      duplicate.status,
      ApprovalDecisionOutcomeStatus::AlreadyResolved
    );
    assert_eq!(duplicate.decided_at, decided_at);
    let opposite = match decision {
      ApprovalDecision::Approved => ApprovalDecision::Rejected,
      ApprovalDecision::Rejected => ApprovalDecision::Approved,
    };
    assert!(matches!(
      resolve_human_approval_durable(
        database.path().to_path_buf(),
        &approval.token,
        opposite,
        &FixedEngineClock::new(decided_at + Duration::seconds(2)),
      ),
      Err(RuntimeExecutionError::DurableStore(
        DurableStoreError::ApprovalDecisionConflict
      ))
    ));

    let resumed = resume_workflow_durable_outcome(
      database.path().to_path_buf(),
      &run_id,
      real_options(decided_at + Duration::seconds(3)).unwrap(),
    )
    .await
    .unwrap();
    let WorkflowRuntimeOutcome::Succeeded { execution, .. } = resumed else {
      panic!("resolved workflow must complete");
    };
    assert_eq!(execution.result["published"], expected_published);
    assert!(execution.execution_order.iter().any(|id| id == selected));
    assert!(!execution.execution_order.iter().any(|id| id == unselected));
    assert_eq!(
      execution
        .events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::ApprovalResolved(_)))
        .count(),
      1
    );

    let event_count = execution.events.len();
    let completed_again = resume_workflow_durable_outcome(
      database.path().to_path_buf(),
      &run_id,
      unavailable_options(decided_at + Duration::seconds(4)),
    )
    .await
    .unwrap();
    let WorkflowRuntimeOutcome::Succeeded {
      execution: repeated,
      ..
    } = completed_again
    else {
      panic!("a completed approval run must stay complete");
    };
    assert_eq!(repeated.events.len(), event_count);
    assert_eq!(repeated.result["published"], expected_published);
  }
}

#[tokio::test]
async fn timeout_reject_selects_the_rejected_route_and_timeout_fail_is_terminal() {
  let Some(_) = real_options(base_time()) else {
    return;
  };
  let started_at = base_time();

  let reject_database = TemporaryDatabase::new("timeout-reject");
  let (reject_run, reject_approval) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      approval_first(model(APPROVAL_MODEL)),
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      reject_database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );
  let deadline = reject_approval.expires_at.unwrap();
  let early = settle_approval_timeout_durable(
    reject_database.path().to_path_buf(),
    &reject_run,
    "editorApproval",
    &FixedEngineClock::new(deadline - Duration::milliseconds(1)),
  )
  .unwrap();
  assert_eq!(early.status, ApprovalTimeoutSettlementStatus::NotDue);
  let settled = settle_approval_timeout_durable(
    reject_database.path().to_path_buf(),
    &reject_run,
    "editorApproval",
    &FixedEngineClock::new(deadline),
  )
  .unwrap();
  assert_eq!(settled.status, ApprovalTimeoutSettlementStatus::Settled);
  assert_eq!(
    settled.resolution,
    Some(ApprovalResolution::Decision {
      decision: ApprovalDecision::Rejected,
      source: ApprovalDecisionSource::Timeout,
    })
  );
  let repeated = settle_approval_timeout_durable(
    reject_database.path().to_path_buf(),
    &reject_run,
    "editorApproval",
    &FixedEngineClock::new(deadline + Duration::seconds(1)),
  )
  .unwrap();
  assert_eq!(
    repeated.status,
    ApprovalTimeoutSettlementStatus::AlreadyResolved
  );
  let rejected = resume_workflow_durable_outcome(
    reject_database.path().to_path_buf(),
    &reject_run,
    real_options(deadline + Duration::seconds(2)).unwrap(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Succeeded { execution, .. } = rejected else {
    panic!("timeout rejection must continue");
  };
  assert_eq!(execution.result["published"], false);
  assert_eq!(execution.result["source"], "timeout");
  assert!(!execution.execution_order.iter().any(|id| id == "publish"));

  let fail_database = TemporaryDatabase::new("timeout-fail");
  let (fail_run, fail_approval) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      approval_first(model(APPROVAL_TIMEOUT_FAIL_MODEL)),
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      fail_database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );
  let fail_deadline = fail_approval.expires_at.unwrap();
  settle_approval_timeout_durable(
    fail_database.path().to_path_buf(),
    &fail_run,
    "editorApproval",
    &FixedEngineClock::new(fail_deadline),
  )
  .unwrap();
  let store = DurableEventStore::open(fail_database.path()).unwrap();
  let events = store.events(&fail_run).unwrap();
  assert_eq!(
    store.projection(&fail_run).unwrap().status,
    RunStatus::Failed
  );
  assert!(matches!(
    (
      &events[events.len() - 2].payload,
      &events[events.len() - 1].payload
    ),
    (
      RunEventPayload::ApprovalResolved(_),
      RunEventPayload::RunFailed(_)
    )
  ));
  assert_eq!(events[events.len() - 2].occurred_at, fail_deadline);
  assert_eq!(events[events.len() - 1].occurred_at, fail_deadline);
  drop(store);
  assert!(matches!(
    resume_workflow_durable_outcome(
      fail_database.path().to_path_buf(),
      &fail_run,
      unavailable_options(fail_deadline + Duration::seconds(1)),
    )
    .await,
    Err(RuntimeExecutionError::ApprovalFailed(_))
  ));

  let rollback_database = TemporaryDatabase::new("timeout-fail-rollback");
  let (rollback_run, rollback_approval) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      approval_first(model(APPROVAL_TIMEOUT_FAIL_MODEL)),
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      rollback_database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );
  Connection::open(rollback_database.path())
    .unwrap()
    .execute_batch(&format!(
      "CREATE TRIGGER reject_timeout_run_failure
       BEFORE INSERT ON woml_run_events
       WHEN NEW.run_id = '{rollback_run}' AND NEW.sequence = 4
       BEGIN
         SELECT RAISE(ABORT, 'test timeout failure rollback');
       END;"
    ))
    .unwrap();
  assert!(settle_approval_timeout_durable(
    rollback_database.path().to_path_buf(),
    &rollback_run,
    "editorApproval",
    &FixedEngineClock::new(rollback_approval.expires_at.unwrap()),
  )
  .is_err());
  let store = DurableEventStore::open(rollback_database.path()).unwrap();
  assert_eq!(store.events(&rollback_run).unwrap().len(), 2);
  assert_eq!(
    store.projection(&rollback_run).unwrap().status,
    RunStatus::Waiting
  );
}

#[tokio::test]
async fn a_selected_route_can_reach_a_nested_approval_then_finish_after_its_decision() {
  let Some(options) = real_options(base_time()) else {
    return;
  };
  let database = TemporaryDatabase::new("nested");
  let workflow = nested_approval_model();
  workflow.validate_for_durable_execution().unwrap();
  let started_at = base_time();
  let (run_id, editorial) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      workflow,
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );
  resolve_human_approval_durable(
    database.path().to_path_buf(),
    &editorial.token,
    ApprovalDecision::Approved,
    &FixedEngineClock::new(started_at + Duration::minutes(1)),
  )
  .unwrap();

  let (same_run_id, legal) = waiting_parts(
    resume_workflow_durable_outcome(
      database.path().to_path_buf(),
      &run_id,
      unavailable_options(started_at + Duration::minutes(2)),
    )
    .await
    .unwrap(),
  );
  assert_eq!(same_run_id, run_id);
  assert_eq!(legal.approval_id, "legalApproval");
  assert_ne!(legal.request_id, editorial.request_id);
  resolve_human_approval_durable(
    database.path().to_path_buf(),
    &legal.token,
    ApprovalDecision::Approved,
    &FixedEngineClock::new(started_at + Duration::minutes(3)),
  )
  .unwrap();

  let finished = resume_workflow_durable_outcome(
    database.path().to_path_buf(),
    &run_id,
    options.with_clock(Arc::new(FixedEngineClock::new(
      started_at + Duration::minutes(4),
    ))),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Succeeded { execution, .. } = finished else {
    panic!("nested approvals must complete");
  };
  assert_eq!(execution.result["published"], true);
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ApprovalRequested(_)))
      .count(),
    2
  );
}

#[tokio::test]
async fn human_decision_and_timeout_races_have_one_durable_winner() {
  for iteration in 0..20 {
    let database = TemporaryDatabase::new(&format!("race-{iteration}"));
    let started_at = base_time() + Duration::days(i64::from(iteration));
    let (run_id, approval) = waiting_parts(
      woml_engine::execute_workflow_durable_outcome(
        approval_first(model(APPROVAL_MODEL)),
        TEST_HASH.to_string(),
        Map::new(),
        unavailable_options(started_at),
        database.path().to_path_buf(),
      )
      .await
      .unwrap(),
    );
    let deadline = approval.expires_at.unwrap();
    let decision_path = database.path().to_path_buf();
    let timeout_path = database.path().to_path_buf();
    let token = approval.token.clone();
    let decision_thread = std::thread::spawn(move || {
      resolve_human_approval_durable(
        decision_path,
        &token,
        ApprovalDecision::Approved,
        &FixedEngineClock::new(deadline - Duration::milliseconds(1)),
      )
    });
    let timeout_run_id = run_id.clone();
    let timeout_thread = std::thread::spawn(move || {
      settle_approval_timeout_durable(
        timeout_path,
        &timeout_run_id,
        "editorApproval",
        &FixedEngineClock::new(deadline),
      )
    });
    let decision_result = decision_thread.join().unwrap();
    let timeout_result = timeout_thread.join().unwrap().unwrap();

    match decision_result {
      Ok(outcome) => {
        assert_eq!(outcome.status, ApprovalDecisionOutcomeStatus::Accepted);
        assert_eq!(
          timeout_result.status,
          ApprovalTimeoutSettlementStatus::AlreadyResolved
        );
      }
      Err(RuntimeExecutionError::DurableStore(DurableStoreError::ApprovalExpired)) => {
        assert_eq!(
          timeout_result.status,
          ApprovalTimeoutSettlementStatus::Settled
        );
      }
      other => panic!("unexpected decision race outcome: {other:?}"),
    }

    let store = DurableEventStore::open(database.path()).unwrap();
    let events = store.events(&run_id).unwrap();
    assert_eq!(
      events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::ApprovalResolved(_)))
        .count(),
      1
    );
    assert!(!events
      .iter()
      .any(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_))));
  }
}

#[tokio::test]
async fn a_human_decision_waits_for_transient_sqlite_contention_without_being_lost() {
  let database = TemporaryDatabase::new("contention");
  let started_at = base_time();
  let (run_id, approval) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      approval_first(model(APPROVAL_MODEL)),
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );

  let connection = Connection::open(database.path()).unwrap();
  connection.execute_batch("BEGIN IMMEDIATE").unwrap();
  let decision_path = database.path().to_path_buf();
  let token = approval.token;
  let decision_thread = std::thread::spawn(move || {
    resolve_human_approval_durable(
      decision_path,
      &token,
      ApprovalDecision::Approved,
      &FixedEngineClock::new(started_at + Duration::minutes(1)),
    )
  });
  std::thread::sleep(std::time::Duration::from_millis(100));
  connection.execute_batch("COMMIT").unwrap();

  let outcome = decision_thread.join().unwrap().unwrap();
  assert_eq!(outcome.status, ApprovalDecisionOutcomeStatus::Accepted);
  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ApprovalResolved(_)))
      .count(),
    1
  );
  assert_eq!(
    store.projection(&run_id).unwrap().status,
    RunStatus::Running
  );
}

#[tokio::test]
async fn recovery_during_a_selected_approval_arm_fails_closed_without_replaying_it() {
  let database = TemporaryDatabase::new("selected-arm-interrupted");
  let started_at = base_time();
  let (run_id, approval) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      approval_first(model(APPROVAL_MODEL)),
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );
  resolve_human_approval_durable(
    database.path().to_path_buf(),
    &approval.token,
    ApprovalDecision::Approved,
    &FixedEngineClock::new(started_at + Duration::minutes(1)),
  )
  .unwrap();

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = woml_engine::DurableDagEngine::resume(store, &run_id).unwrap();
    assert_eq!(engine.ready_node_ids(&run_id).unwrap(), vec!["publish"]);
    engine
      .append_payload(
        "evt_a7_selected_arm_started",
        &run_id,
        started_at + Duration::minutes(2),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: "publish".to_string(),
          attempt: 1,
          invocation_id: "inv_a7_selected_arm".to_string(),
          handler: "runtime.script".to_string(),
          idempotency_key: None,
        }),
      )
      .unwrap();
  }

  assert!(matches!(
    resume_workflow_durable_outcome(
      database.path().to_path_buf(),
      &run_id,
      unavailable_options(started_at + Duration::minutes(3)),
    )
    .await,
    Err(RuntimeExecutionError::RunFailed(_))
  ));
  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ApprovalResolved(_)))
      .count(),
    1
  );
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
      .count(),
    1
  );
  assert!(events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::StepAttemptFailed(_))));
  assert_eq!(store.projection(&run_id).unwrap().status, RunStatus::Failed);
}

#[tokio::test]
async fn recovery_after_the_selected_arm_and_before_the_join_continues_downstream_once() {
  let Some(options) = real_options(base_time()) else {
    return;
  };
  let database = TemporaryDatabase::new("selected-arm-before-join");
  let started_at = base_time();
  let (run_id, approval) = waiting_parts(
    woml_engine::execute_workflow_durable_outcome(
      approval_first(model(APPROVAL_MODEL)),
      TEST_HASH.to_string(),
      Map::new(),
      unavailable_options(started_at),
      database.path().to_path_buf(),
    )
    .await
    .unwrap(),
  );
  resolve_human_approval_durable(
    database.path().to_path_buf(),
    &approval.token,
    ApprovalDecision::Approved,
    &FixedEngineClock::new(started_at + Duration::minutes(1)),
  )
  .unwrap();

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = woml_engine::DurableDagEngine::resume(store, &run_id).unwrap();
    engine
      .append_payload(
        "evt_a7_selected_arm_complete_start",
        &run_id,
        started_at + Duration::minutes(2),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: "publish".to_string(),
          attempt: 1,
          invocation_id: "inv_a7_selected_arm_complete".to_string(),
          handler: "runtime.script".to_string(),
          idempotency_key: None,
        }),
      )
      .unwrap();
    engine
      .append_payload(
        "evt_a7_selected_arm_complete_success",
        &run_id,
        started_at + Duration::minutes(2) + Duration::milliseconds(1),
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: "publish".to_string(),
          attempt: 1,
          invocation_id: "inv_a7_selected_arm_complete".to_string(),
          output: json!({ "published": true }),
        }),
      )
      .unwrap();
  }

  let resumed = resume_workflow_durable_outcome(
    database.path().to_path_buf(),
    &run_id,
    options.with_clock(Arc::new(FixedEngineClock::new(
      started_at + Duration::minutes(3),
    ))),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Succeeded { execution, .. } = resumed else {
    panic!("the completed selected arm must continue through the join");
  };
  assert_eq!(execution.result["decision"], "approved");
  assert_eq!(execution.execution_order, vec!["publish", "finalStatus"]);
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
      .count(),
    2
  );
}
