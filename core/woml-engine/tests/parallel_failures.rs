use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::{
  execute_workflow, execute_workflow_durable, AttemptFailureKind, CompiledWorkflowDefinition,
  ParallelFailurePolicy, ParallelGroupOutcome, RunEventPayload, RunStatus, RuntimeExecutionError,
  RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const HASH: &str = "sha256:d58dfcefdcd6c40db659042c41e17ca6c8d652033f90f120734d5cd95819b45c";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
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

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source.to_string()),
    },
  );
}

fn set_policy(workflow: &mut CompiledWorkflowDefinition, policy: &str, concurrency: u64) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "__woml_parallel__fieldData__start")
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("parallel inputs must be an object");
  };
  fields.insert(
    "onError".to_string(),
    ValueExpression::Literal {
      value: Value::String(policy.to_string()),
    },
  );
  fields.insert(
    "concurrency".to_string(),
    ValueExpression::Literal {
      value: Value::from(concurrency),
    },
  );
}

fn add_queued_third_child(workflow: &mut CompiledWorkflowDefinition) {
  let mut child = workflow
    .graph
    .nodes
    .iter()
    .find(|node| node.id == "loadSoil")
    .unwrap()
    .clone();
  child.id = "loadCrop".to_string();
  workflow.graph.nodes.insert(4, child);
  set_script(workflow, "loadCrop", "return { crop: 'wheat' };");

  let child_template = workflow
    .graph
    .edges
    .iter()
    .find(|edge| edge.id == "fieldData:child:1")
    .unwrap()
    .clone();
  let join_template = workflow
    .graph
    .edges
    .iter()
    .find(|edge| edge.id == "fieldData:join:1")
    .unwrap()
    .clone();
  let mut child_edge = child_template;
  child_edge.id = "fieldData:child:2".to_string();
  child_edge.to = "loadCrop".to_string();
  let mut join_edge = join_template;
  join_edge.id = "fieldData:join:2".to_string();
  join_edge.from = "loadCrop".to_string();

  let tail = workflow.graph.edges.pop().unwrap();
  let join_one = workflow.graph.edges.pop().unwrap();
  let join_zero = workflow.graph.edges.pop().unwrap();
  workflow.graph.edges.push(child_edge);
  workflow.graph.edges.push(join_zero);
  workflow.graph.edges.push(join_one);
  workflow.graph.edges.push(join_edge);
  workflow.graph.edges.push(tail);
  workflow.validate_for_execution().unwrap();
}

fn failed_details(error: RuntimeExecutionError) -> woml_engine::FailedParallelDetails {
  match error {
    RuntimeExecutionError::ParallelFailed(details) => *details,
    other => panic!("expected a parallel failure, received {other:?}"),
  }
}

fn child_terminal_count(events: &[woml_engine::RunEvent], node_id: &str) -> usize {
  events
    .iter()
    .filter(|event| match &event.payload {
      RunEventPayload::StepAttemptSucceeded(data) => data.node_id == node_id,
      RunEventPayload::StepAttemptFailed(data) => data.node_id == node_id,
      _ => false,
    })
    .count()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new() -> Self {
    Self {
      path: std::env::temp_dir().join(format!("woml-p5-{}.sqlite", Uuid::new_v4())),
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
async fn wait_all_runs_every_child_preserves_successes_and_blocks_downstream_work() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = model();
  set_script(
    &mut workflow,
    "loadWeather",
    "await new Promise(resolve => setTimeout(resolve, 15)); throw new Error('no weather');",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "await new Promise(resolve => setTimeout(resolve, 80)); return { moisture: 41 };",
  );

  let details = failed_details(
    execute_workflow(workflow, HASH.to_string(), Map::new(), options(host))
      .await
      .unwrap_err(),
  );
  assert_eq!(details.policy, ParallelFailurePolicy::WaitAll);
  assert_eq!(details.primary_node_id, "loadWeather");
  assert_eq!(details.failed_node_ids, ["loadWeather"]);
  assert!(details.cancelled_node_ids.is_empty());
  assert_eq!(child_terminal_count(&details.events, "loadWeather"), 1);
  assert_eq!(child_terminal_count(&details.events, "loadSoil"), 1);
  assert!(details.events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "loadSoil")
  }));
  assert!(!details.events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "buildReport")
  }));
}

#[tokio::test]
async fn wait_all_reports_multiple_failures_in_document_order() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = model();
  set_script(
    &mut workflow,
    "loadWeather",
    "await new Promise(resolve => setTimeout(resolve, 70)); throw new Error('weather failed');",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "await new Promise(resolve => setTimeout(resolve, 10)); throw new Error('soil failed');",
  );

  let details = failed_details(
    execute_workflow(workflow, HASH.to_string(), Map::new(), options(host))
      .await
      .unwrap_err(),
  );
  assert_eq!(details.primary_node_id, "loadSoil");
  assert_eq!(details.failed_node_ids, ["loadWeather", "loadSoil"]);
  assert_eq!(details.message, "2 parallel children failed.");
}

#[tokio::test]
async fn fail_fast_cancels_active_work_suppresses_queued_work_and_emits_one_terminal_each() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = model();
  add_queued_third_child(&mut workflow);
  set_policy(&mut workflow, "fail-fast", 2);
  set_script(
    &mut workflow,
    "loadWeather",
    "await new Promise(resolve => setTimeout(resolve, 20)); throw new Error('no weather');",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "await new Promise(resolve => setTimeout(resolve, 1500)); return { moisture: 41 };",
  );

  let details = failed_details(
    execute_workflow(
      workflow,
      "sha256:5555555555555555555555555555555555555555555555555555555555555555".to_string(),
      Map::new(),
      options(host),
    )
    .await
    .unwrap_err(),
  );
  assert_eq!(details.policy, ParallelFailurePolicy::FailFast);
  assert_eq!(details.failed_node_ids, ["loadWeather"]);
  assert_eq!(details.cancelled_node_ids, ["loadSoil"]);
  assert_eq!(child_terminal_count(&details.events, "loadWeather"), 1);
  assert_eq!(child_terminal_count(&details.events, "loadSoil"), 1);
  assert_eq!(child_terminal_count(&details.events, "loadCrop"), 0);
  assert!(!details.events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadCrop" || data.node_id == "buildReport")
  }));
  assert!(details.events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepAttemptFailed(data)
      if data.node_id == "loadSoil" && data.failure.kind == AttemptFailureKind::InvocationCancelled)
  }));
}

#[tokio::test]
async fn a_sibling_that_finishes_before_fail_fast_cancellation_keeps_its_real_success() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = model();
  set_policy(&mut workflow, "fail-fast", 2);
  set_script(
    &mut workflow,
    "loadWeather",
    "await new Promise(resolve => setTimeout(resolve, 100)); throw new Error('no weather');",
  );
  set_script(&mut workflow, "loadSoil", "return { moisture: 41 };");

  let details = failed_details(
    execute_workflow(workflow, HASH.to_string(), Map::new(), options(host))
      .await
      .unwrap_err(),
  );
  assert_eq!(details.failed_node_ids, ["loadWeather"]);
  assert!(details.cancelled_node_ids.is_empty());
  assert!(details.events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "loadSoil")
  }));
  assert_eq!(child_terminal_count(&details.events, "loadSoil"), 1);
}

#[tokio::test]
async fn durable_parallel_failure_reopens_as_one_coherent_failed_run() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  set_script(
    &mut workflow,
    "loadWeather",
    "throw new Error('no weather');",
  );

  let details = failed_details(
    execute_workflow_durable(
      workflow,
      HASH.to_string(),
      Map::new(),
      options(host),
      database.path().to_path_buf(),
    )
    .await
    .unwrap_err(),
  );
  let run_id = details.events.first().unwrap().run_id.clone();
  let store = woml_engine::DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection(&run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(matches!(
    projection.parallel_groups["fieldData"].status,
    woml_engine::ParallelGroupStatus::Completed {
      outcome: ParallelGroupOutcome::Failed,
      ..
    }
  ));
  let events = store.events(&run_id).unwrap();
  assert!(matches!(
    events[events.len() - 2].payload,
    RunEventPayload::ParallelGroupCompleted(_)
  ));
  assert!(matches!(
    events[events.len() - 1].payload,
    RunEventPayload::RunFailed(_)
  ));
}
