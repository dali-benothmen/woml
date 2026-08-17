use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::{BackoffPolicy, RetryPolicy, ValueExpression};
use woml_engine::{
  execute_workflow_durable, execute_workflow_durable_outcome, resolve_human_approval_durable,
  resume_workflow_durable_outcome, ApprovalDecision, CompiledWorkflowDefinition, DurableEventStore,
  FixedEngineClock, ParallelFailurePolicy, ParallelGroupOutcome, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions, WorkflowRuntimeOutcome,
};

const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const PARALLEL_MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const NOTIFICATION_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/approval-slack.compiled.v5.json");
const HASH: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn model(source: &str) -> CompiledWorkflowDefinition {
  serde_json::from_str(source).unwrap()
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

fn options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 3_000)
}

fn approval_options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 3_000).with_clock(Arc::new(FixedEngineClock::new(Utc::now())))
}

fn node_mut<'a>(
  workflow: &'a mut CompiledWorkflowDefinition,
  node_id: &str,
) -> &'a mut woml_engine::model::CompiledWorkflowNode {
  workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap()
}

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let ValueExpression::Object { fields } = &mut node_mut(workflow, node_id).inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source.to_string()),
    },
  );
}

fn set_retry(
  workflow: &mut CompiledWorkflowDefinition,
  node_id: &str,
  max_attempts: u32,
  delay_ms: u64,
) {
  node_mut(workflow, node_id).retry_policy = Some(RetryPolicy {
    max_attempts,
    backoff: BackoffPolicy::Fixed { delay_ms },
  });
}

fn install_notification(workflow: &mut CompiledWorkflowDefinition) {
  let ValueExpression::Object { fields } = &mut node_mut(workflow, "editorApproval").inputs else {
    panic!("approval inputs must be an object");
  };
  fields.insert(
    "notifications".to_string(),
    serde_json::from_value(json!({
      "kind": "array",
      "items": [{
        "kind": "object",
        "fields": {
          "deliveryId": { "kind": "literal", "value": "editorApproval:notify:0:channel:0" },
          "provider": { "kind": "literal", "value": "slack" },
          "destination": { "kind": "literal", "value": "#approvals" },
          "credentials": {
            "kind": "object",
            "fields": {
              "botToken": { "kind": "secretReference", "name": "SLACK_BOT_TOKEN" },
              "appToken": { "kind": "secretReference", "name": "SLACK_APP_TOKEN" }
            }
          }
        }
      }]
    }))
    .unwrap(),
  );
}

fn attempts(events: &[woml_engine::RunEvent], node_id: &str) -> Vec<u32> {
  events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptStarted(data) if data.node_id == node_id => Some(data.attempt),
      _ => None,
    })
    .collect()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-ri5-{label}-{}.sqlite",
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

#[tokio::test]
async fn selected_branch_route_retries_without_reselecting_either_arm() {
  let Some(host) = host_options() else {
    return;
  };
  for (needs_review, selected, unselected, expected_status) in [
    (true, "reviewContent", "acceptContent", "reviewed"),
    (
      false,
      "acceptContent",
      "reviewContent",
      "accepted-automatically",
    ),
  ] {
    let database = TemporaryDatabase::new(selected);
    let mut workflow = model(BRANCH_MODEL);
    workflow.schema_version = 6;
    set_script(
      &mut workflow,
      "checkContent",
      &format!("return {{ needsReview: {needs_review} }};"),
    );
    set_retry(&mut workflow, selected, 2, 1);
    set_script(
      &mut workflow,
      selected,
      &format!(
        "if (attempt.number === 1) throw new Error('temporary route failure'); return {{ status: '{expected_status}', accepted: true }};"
      ),
    );

    let execution = execute_workflow_durable(
      workflow,
      HASH.to_string(),
      Map::new(),
      options(host.clone()),
      database.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert_eq!(
      execution.result,
      json!({ "message": format!("Final status: {expected_status}") })
    );
    assert_eq!(attempts(&execution.events, selected), [1, 2]);
    assert!(attempts(&execution.events, unselected).is_empty());
    assert_eq!(
      execution
        .events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::BranchSelected(_)))
        .count(),
      1
    );
  }
}

#[tokio::test]
async fn wait_all_parallel_children_retry_independently_then_join_once() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("parallel-wait-all");
  let mut workflow = model(PARALLEL_MODEL);
  workflow.schema_version = 6;
  set_retry(&mut workflow, "loadWeather", 2, 1);
  set_retry(&mut workflow, "loadSoil", 3, 2);
  set_script(
    &mut workflow,
    "loadWeather",
    "if (attempt.number === 1) throw new Error('weather unavailable'); return { temperature: 22 };",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "if (attempt.number < 3) throw new Error('soil unavailable'); return { moisture: 41 };",
  );

  let execution = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(
    execution.result,
    json!({ "summary": "Weather 22°C, soil 41%" })
  );
  assert_eq!(attempts(&execution.events, "loadWeather"), [1, 2]);
  assert_eq!(attempts(&execution.events, "loadSoil"), [1, 2, 3]);
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::ParallelGroupCompleted(data)
          if data.outcome == ParallelGroupOutcome::Succeeded
      ))
      .count(),
    1
  );
  assert_eq!(
    execution
      .execution_order
      .iter()
      .filter(|node_id| *node_id == "loadWeather")
      .count(),
    1
  );
  assert_eq!(
    execution
      .execution_order
      .iter()
      .filter(|node_id| *node_id == "loadSoil")
      .count(),
    1
  );
}

#[tokio::test]
async fn wait_all_aggregates_only_after_retries_reach_final_outcomes() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("parallel-wait-all-final");
  let mut workflow = model(PARALLEL_MODEL);
  workflow.schema_version = 6;
  set_retry(&mut workflow, "loadWeather", 2, 1);
  set_retry(&mut workflow, "loadSoil", 2, 20);
  set_script(
    &mut workflow,
    "loadWeather",
    "throw new Error('weather final');",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "if (attempt.number === 1) throw new Error('soil temporary'); return { moisture: 41 };",
  );

  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::ParallelFailed(details) = error else {
    panic!("expected parallel failure");
  };

  assert_eq!(details.policy, ParallelFailurePolicy::WaitAll);
  assert_eq!(details.failed_node_ids, ["loadWeather"]);
  assert_eq!(attempts(&details.events, "loadWeather"), [1, 2]);
  assert_eq!(attempts(&details.events, "loadSoil"), [1, 2]);
  let soil_success = details
    .events
    .iter()
    .position(|event| {
      matches!(
        &event.payload,
        RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "loadSoil"
      )
    })
    .unwrap();
  let group_failure = details
    .events
    .iter()
    .position(|event| {
      matches!(
        &event.payload,
        RunEventPayload::ParallelGroupCompleted(data)
          if data.outcome == ParallelGroupOutcome::Failed
      )
    })
    .unwrap();
  assert!(soil_success < group_failure);
  assert!(!details.events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::StepAttemptStarted(data) if data.node_id == "buildReport"
  )));
}

#[tokio::test]
async fn fail_fast_waits_for_final_failure_and_abandons_scheduled_sibling_retry() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("parallel-fail-fast");
  let mut workflow = model(PARALLEL_MODEL);
  workflow.schema_version = 6;
  let ValueExpression::Object { fields } =
    &mut node_mut(&mut workflow, "__woml_parallel__fieldData__start").inputs
  else {
    panic!("parallel inputs must be an object");
  };
  fields.insert(
    "onError".to_string(),
    ValueExpression::Literal {
      value: Value::String("fail-fast".to_string()),
    },
  );
  set_retry(&mut workflow, "loadWeather", 2, 5);
  set_retry(&mut workflow, "loadSoil", 2, 1_000);
  set_script(
    &mut workflow,
    "loadWeather",
    "throw new Error('weather final');",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "throw new Error('soil pending');",
  );

  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::ParallelFailed(details) = error else {
    panic!("expected parallel failure");
  };

  assert_eq!(details.policy, ParallelFailurePolicy::FailFast);
  assert_eq!(details.failed_node_ids, ["loadWeather"]);
  assert!(details.cancelled_node_ids.is_empty());
  assert_eq!(attempts(&details.events, "loadWeather"), [1, 2]);
  assert_eq!(attempts(&details.events, "loadSoil"), [1]);
  let run_id = details.events[0].run_id.clone();
  let store = DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection(&run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(projection.pending_retries.is_empty());
}

#[tokio::test]
async fn both_approval_arms_retry_without_reopening_the_human_decision() {
  let Some(host) = host_options() else {
    return;
  };
  for (decision, selected, unselected) in [
    (ApprovalDecision::Approved, "publish", "recordRejection"),
    (ApprovalDecision::Rejected, "recordRejection", "publish"),
  ] {
    let database = TemporaryDatabase::new(selected);
    let mut workflow = model(APPROVAL_MODEL);
    workflow.schema_version = 6;
    set_retry(&mut workflow, selected, 2, 1);
    set_script(
      &mut workflow,
      selected,
      "if (attempt.number === 1) throw new Error('temporary approval route failure'); return { completed: true };",
    );

    let waiting = execute_workflow_durable_outcome(
      workflow,
      HASH.to_string(),
      Map::new(),
      approval_options(host.clone()),
      database.path().to_path_buf(),
    )
    .await
    .unwrap();
    let WorkflowRuntimeOutcome::Waiting {
      run_id, approval, ..
    } = waiting
    else {
      panic!("approval workflow must wait");
    };
    resolve_human_approval_durable(
      database.path().to_path_buf(),
      &approval.token,
      decision,
      &FixedEngineClock::new(Utc::now() + Duration::seconds(1)),
    )
    .unwrap();
    let resumed = resume_workflow_durable_outcome(
      database.path().to_path_buf(),
      &run_id,
      approval_options(host.clone()),
    )
    .await
    .unwrap();
    let WorkflowRuntimeOutcome::Succeeded { execution, .. } = resumed else {
      panic!("resolved approval workflow must succeed");
    };

    assert_eq!(attempts(&execution.events, selected), [1, 2]);
    assert!(attempts(&execution.events, unselected).is_empty());
    assert_eq!(
      execution
        .events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::ApprovalRequested(_)))
        .count(),
      1
    );
    assert_eq!(
      execution
        .events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::ApprovalResolved(_)))
        .count(),
      1
    );
    assert!(!execution.events.iter().any(|event| matches!(
      event.payload,
      RunEventPayload::NotificationDeliveryRequested(_)
    )));
  }
}

#[test]
fn model_v6_keeps_notification_provider_capabilities_when_a_step_retries() {
  let mut workflow = model(NOTIFICATION_MODEL);
  workflow.schema_version = 6;
  set_retry(&mut workflow, "finalStatus", 2, 1);
  workflow.validate_for_durable_execution().unwrap();
}

#[tokio::test]
async fn approval_arm_retry_does_not_requeue_notification_delivery() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("approval-notification-once");
  let mut workflow = model(APPROVAL_MODEL);
  workflow.schema_version = 6;
  install_notification(&mut workflow);
  set_retry(&mut workflow, "publish", 2, 1);
  set_script(
    &mut workflow,
    "publish",
    "if (attempt.number === 1) throw new Error('temporary publish failure'); return { published: true };",
  );

  let waiting = execute_workflow_durable_outcome(
    workflow,
    HASH.to_string(),
    Map::new(),
    approval_options(host.clone()),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Waiting {
    run_id, approval, ..
  } = waiting
  else {
    panic!("notification approval must wait");
  };
  resolve_human_approval_durable(
    database.path().to_path_buf(),
    &approval.token,
    ApprovalDecision::Approved,
    &FixedEngineClock::new(Utc::now() + Duration::seconds(1)),
  )
  .unwrap();
  let resumed = resume_workflow_durable_outcome(
    database.path().to_path_buf(),
    &run_id,
    approval_options(host),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Succeeded { execution, .. } = resumed else {
    panic!("approved notification workflow must finish");
  };

  assert_eq!(attempts(&execution.events, "publish"), [1, 2]);
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ApprovalRequested(_)))
      .count(),
    1
  );
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(
        event.payload,
        RunEventPayload::NotificationDeliveryRequested(_)
      ))
      .count(),
    1
  );
}
