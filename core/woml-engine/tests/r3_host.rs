use std::path::PathBuf;

use serde_json::{json, Map};
use woml_engine::protocol::{ExecuteMessage, HostOutcome};
use woml_engine::{
  execute_workflow, AttemptFailureKind, CompiledWorkflowDefinition, RuntimeExecutionError,
  RuntimeExecutionOptions, ScriptHostClient, ScriptHostProcessOptions, WorkflowContext,
};

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";

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
    1_000,
    slow_source,
    &context,
  );
  let fast = ExecuteMessage::runtime_script(
    "inv_rust_fast",
    "run_rust_multiplex",
    "fast",
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
        woml_engine::RunEventPayload::RunSucceeded(_) => "run_succeeded",
        woml_engine::RunEventPayload::RunFailed(_) => "run_failed",
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
