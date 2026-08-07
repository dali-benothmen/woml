use std::path::PathBuf;

use serde_json::{json, Map};
use woml_engine::protocol::{ExecuteMessage, HostOutcome, ScriptAttempt};
use woml_engine::{
  execute_workflow, step_effect_idempotency_key, AttemptFailureKind, CompiledWorkflowDefinition,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostClient, ScriptHostProcessOptions,
  WorkflowContext,
};

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";
const TEST_EFFECT_KEY: &str =
  "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn attempt() -> ScriptAttempt<'static> {
  ScriptAttempt::new(1, 1, TEST_EFFECT_KEY).unwrap()
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

fn context() -> WorkflowContext {
  WorkflowContext {
    trigger: Map::new(),
    steps: Map::new(),
  }
}

#[tokio::test]
async fn protocol_v3_attempt_identity_is_stable_across_host_restart() {
  let Some(options) = host_options() else {
    return;
  };
  let run_id = "run_retry_host_restart";
  let node_id = "greet";
  let key = step_effect_idempotency_key(run_id, HELLO_HASH, node_id);
  let other_node_key = step_effect_idempotency_key(run_id, HELLO_HASH, "other");
  let other_run_key = step_effect_idempotency_key("run_other", HELLO_HASH, node_id);
  assert_ne!(key, other_node_key);
  assert_ne!(key, other_run_key);

  let context = context();
  let source = "return { number: attempt.number, maxAttempts: attempt.maxAttempts, idempotencyKey: attempt.idempotencyKey, frozen: Object.isFrozen(attempt) };";
  let first_invocation = "inv_retry_restart_01";
  let first_attempt = ScriptAttempt::new(1, 3, &key).unwrap();
  let first_request = ExecuteMessage::runtime_script(
    first_invocation,
    run_id,
    node_id,
    first_attempt,
    1_000,
    source,
    &context,
  );
  let first_host = ScriptHostClient::spawn(options.clone()).await.unwrap();
  let first = first_host.execute(&first_request).await.unwrap();
  let first_host_id = first_host.host_instance_id.clone();
  first_host.shutdown().await;

  let second_invocation = "inv_retry_restart_02";
  let second_attempt = ScriptAttempt::new(2, 3, &key).unwrap();
  let second_request = ExecuteMessage::runtime_script(
    second_invocation,
    run_id,
    node_id,
    second_attempt,
    1_000,
    source,
    &context,
  );
  let second_host = ScriptHostClient::spawn(options).await.unwrap();
  assert_ne!(first_host_id, second_host.host_instance_id);
  let second = second_host.execute(&second_request).await.unwrap();
  second_host.shutdown().await;

  assert_ne!(first_invocation, second_invocation);
  assert_eq!(
    first.outcome,
    HostOutcome::Success {
      value: json!({
        "number": 1,
        "maxAttempts": 3,
        "idempotencyKey": key,
        "frozen": true
      })
    }
  );
  assert_eq!(
    second.outcome,
    HostOutcome::Success {
      value: json!({
        "number": 2,
        "maxAttempts": 3,
        "idempotencyKey": key,
        "frozen": true
      })
    }
  );
}

#[test]
fn protocol_v3_rejects_invalid_attempt_metadata_before_transport() {
  assert!(ScriptAttempt::new(0, 3, TEST_EFFECT_KEY).is_err());
  assert!(ScriptAttempt::new(4, 3, TEST_EFFECT_KEY).is_err());
  assert!(ScriptAttempt::new(1, 11, TEST_EFFECT_KEY).is_err());
  assert!(ScriptAttempt::new(1, 3, "secret-token").is_err());
}

#[tokio::test]
async fn rust_client_multiplexes_and_correlates_out_of_order_utf8_frames() {
  let Some(options) = host_options() else {
    return;
  };
  let host = ScriptHostClient::spawn(options).await.unwrap();
  let context = context();
  let slow_source =
    "await new Promise((resolve) => setTimeout(resolve, 120)); return { value: 'slow' };";
  let fast_source = "return { value: 'مرحبا 🌱\\r\\nfast' };";
  let slow = ExecuteMessage::runtime_script(
    "inv_rust_slow",
    "run_rust_multiplex",
    "slow",
    attempt(),
    1_000,
    slow_source,
    &context,
  );
  let fast = ExecuteMessage::runtime_script(
    "inv_rust_fast",
    "run_rust_multiplex",
    "fast",
    attempt(),
    1_000,
    fast_source,
    &context,
  );

  let (slow_result, fast_result) = tokio::join!(host.execute(&slow), host.execute(&fast));
  let slow_result = slow_result.unwrap();
  let fast_result = fast_result.unwrap();
  assert_eq!(slow_result.invocation_id, "inv_rust_slow");
  assert_eq!(fast_result.invocation_id, "inv_rust_fast");
  assert!(fast_result.duration_ms < slow_result.duration_ms);
  assert_eq!(
    fast_result.outcome,
    HostOutcome::Success {
      value: json!({ "value": "مرحبا 🌱\r\nfast" })
    }
  );
  host.shutdown().await;
}

#[tokio::test]
async fn rust_client_cancels_only_the_target_invocation() {
  let Some(options) = host_options() else {
    return;
  };
  let host = ScriptHostClient::spawn(options).await.unwrap();
  let context = context();
  let cancelled = ExecuteMessage::runtime_script(
    "inv_rust_cancelled",
    "run_rust_cancel",
    "cancelled",
    attempt(),
    3_000,
    "await new Promise(resolve => setTimeout(resolve, 1500)); return { value: 'too late' };",
    &context,
  );
  let survivor = ExecuteMessage::runtime_script(
    "inv_rust_survivor",
    "run_rust_cancel",
    "survivor",
    attempt(),
    3_000,
    "await new Promise(resolve => setTimeout(resolve, 160)); return { value: 'survived' };",
    &context,
  );

  let cancel = async {
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    host.cancel("inv_rust_cancelled").await
  };
  let (cancelled_result, survivor_result, cancel_result) =
    tokio::join!(host.execute(&cancelled), host.execute(&survivor), cancel);
  cancel_result.unwrap();
  assert!(matches!(
    cancelled_result.unwrap().outcome,
    HostOutcome::Failure { error }
      if error.kind == woml_engine::protocol::HostReportedFailureKind::InvocationCancelled
        && error.code == "WOML_SCRIPT_CANCELLED"
  ));
  assert_eq!(
    survivor_result.unwrap().outcome,
    HostOutcome::Success {
      value: json!({ "value": "survived" })
    }
  );
  host.shutdown().await;
}

#[tokio::test]
async fn host_loss_during_cancellation_stays_a_host_crash() {
  let Some(mut options) = host_options() else {
    return;
  };
  options.host_script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../woml-cli/tests/fixtures/crashing-script-host.ts");
  let host = ScriptHostClient::spawn(options).await.unwrap();
  let context = context();
  let first = ExecuteMessage::runtime_script(
    "inv_host_loss_first",
    "run_host_loss",
    "first",
    attempt(),
    3_000,
    "await new Promise(resolve => setTimeout(resolve, 1000)); return {};",
    &context,
  );
  let second = ExecuteMessage::runtime_script(
    "inv_host_loss_second",
    "run_host_loss",
    "second",
    attempt(),
    3_000,
    "await new Promise(resolve => setTimeout(resolve, 1000)); return {};",
    &context,
  );
  let cancel = async {
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    host.cancel("inv_host_loss_second").await
  };

  let (first_result, second_result, _) =
    tokio::join!(host.execute(&first), host.execute(&second), cancel);
  assert!(matches!(
    first_result,
    Err(woml_engine::ScriptHostClientError::HostCrashed(_))
  ));
  assert!(matches!(
    second_result,
    Err(woml_engine::ScriptHostClientError::HostCrashed(_))
  ));
  host.shutdown().await;
}

#[tokio::test]
async fn runtime_appends_canonical_failure_events_for_script_errors() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(HELLO_MODEL).unwrap();
  let first = workflow.graph.nodes.first_mut().unwrap();
  let woml_engine::model::ValueExpression::Object { fields } = &mut first.inputs else {
    panic!("expected object inputs");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!("throw new Error('r3 failure');"),
    },
  );
  let options = RuntimeExecutionOptions::new(host, 1_000);

  let error = execute_workflow(workflow, HELLO_HASH.to_string(), Map::new(), options)
    .await
    .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected a failed run");
  };
  assert_eq!(details.failure.kind, AttemptFailureKind::ScriptThrew);
  assert_eq!(details.failure.code, "WOML_SCRIPT_THROWN");
  assert_eq!(
    details
      .events
      .iter()
      .map(|event| match &event.payload {
        woml_engine::RunEventPayload::RunStarted(_) => "run_started",
        woml_engine::RunEventPayload::StepAttemptStarted(_) => "step_attempt_started",
        woml_engine::RunEventPayload::StepAttemptSucceeded(_) => "step_attempt_succeeded",
        woml_engine::RunEventPayload::StepAttemptFailed(_) => "step_attempt_failed",
        woml_engine::RunEventPayload::BranchSelected(_) => "branch_selected",
        woml_engine::RunEventPayload::ParallelGroupStarted(_) => "parallel_group_started",
        woml_engine::RunEventPayload::ParallelGroupCompleted(_) => "parallel_group_completed",
        woml_engine::RunEventPayload::ApprovalRequested(_) => "approval_requested",
        woml_engine::RunEventPayload::ApprovalResolved(_) => "approval_resolved",
        woml_engine::RunEventPayload::RunSucceeded(_) => "run_succeeded",
        woml_engine::RunEventPayload::RunFailed(_) => "run_failed",
        _ => "notification_event",
      })
      .collect::<Vec<_>>(),
    [
      "run_started",
      "step_attempt_started",
      "step_attempt_failed",
      "run_failed"
    ]
  );
}
