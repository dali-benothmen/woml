use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::{
  execute_workflow, execute_workflow_durable, CompiledWorkflowDefinition, ParallelGroupOutcome,
  RunEventPayload, RuntimeExecutionOptions, ScriptHostProcessOptions,
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
  RuntimeExecutionOptions::new(host, 2_000)
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

fn set_concurrency(workflow: &mut CompiledWorkflowDefinition, concurrency: u64) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "__woml_parallel__fieldData__start")
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("parallel start inputs must be an object");
  };
  fields.insert(
    "concurrency".to_string(),
    ValueExpression::Literal {
      value: Value::from(concurrency),
    },
  );
}

fn event_position(
  events: &[woml_engine::RunEvent],
  predicate: impl Fn(&RunEventPayload) -> bool,
) -> usize {
  events
    .iter()
    .position(|event| predicate(&event.payload))
    .unwrap()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new() -> Self {
    Self {
      path: std::env::temp_dir().join(format!("woml-p4-{}.sqlite", Uuid::new_v4())),
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
async fn reviewed_parallel_workflow_runs_joins_and_feeds_downstream_context() {
  let Some(host) = host_options() else {
    return;
  };
  let result = execute_workflow(model(), HASH.to_string(), Map::new(), options(host))
    .await
    .unwrap();

  assert_eq!(
    result.result,
    json!({ "summary": "Weather 22°C, soil 41%" })
  );
  assert_eq!(
    result.context.steps.get("loadWeather"),
    Some(&json!({ "fieldId": "field-42", "temperature": 22 }))
  );
  assert_eq!(
    result.context.steps.get("loadSoil"),
    Some(&json!({ "fieldId": "field-42", "moisture": 41 }))
  );
  assert!(!result.context.steps.contains_key("fieldData"));
  assert!(result
    .events
    .iter()
    .all(|event| event.event_schema_version == 3));

  let weather_start = event_position(
    &result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadWeather"),
  );
  let soil_start = event_position(
    &result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadSoil"),
  );
  let first_terminal = event_position(&result.events, |payload| match payload {
    RunEventPayload::StepAttemptSucceeded(data) => {
      data.node_id == "loadWeather" || data.node_id == "loadSoil"
    }
    RunEventPayload::StepAttemptFailed(data) => {
      data.node_id == "loadWeather" || data.node_id == "loadSoil"
    }
    _ => false,
  });
  assert!(weather_start < first_terminal);
  assert!(soil_start < first_terminal);
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ParallelGroupStarted(_)))
      .count(),
    1
  );
  assert!(result.events.iter().any(|event| {
    matches!(
      &event.payload,
      RunEventPayload::ParallelGroupCompleted(data)
        if data.parallel_id == "fieldData" && data.outcome == ParallelGroupOutcome::Succeeded
    )
  }));
}

#[tokio::test]
async fn slow_first_child_finishes_after_fast_second_child_and_neither_sees_its_sibling() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = model();
  set_script(
    &mut workflow,
    "loadWeather",
    "await new Promise(resolve => setTimeout(resolve, 180)); return { sawSoil: context.steps.loadSoil !== undefined };",
  );
  set_script(
    &mut workflow,
    "loadSoil",
    "await new Promise(resolve => setTimeout(resolve, 10)); return { sawWeather: context.steps.loadWeather !== undefined };",
  );
  set_script(
    &mut workflow,
    "buildReport",
    "return { weatherSawSoil: context.steps.loadWeather.sawSoil, soilSawWeather: context.steps.loadSoil.sawWeather };",
  );
  let result = execute_workflow(workflow, HASH.to_string(), Map::new(), options(host))
    .await
    .unwrap();

  assert_eq!(
    result.result,
    json!({ "weatherSawSoil": false, "soilSawWeather": false })
  );
  let soil = result
    .execution_order
    .iter()
    .position(|node_id| node_id == "loadSoil")
    .unwrap();
  let weather = result
    .execution_order
    .iter()
    .position(|node_id| node_id == "loadWeather")
    .unwrap();
  assert!(soil < weather, "the fast second child must complete first");
}

#[tokio::test]
async fn concurrency_one_serializes_children_and_one_child_groups_remain_valid() {
  let Some(host) = host_options() else {
    return;
  };
  let mut serial = model();
  set_concurrency(&mut serial, 1);
  let serial_result = execute_workflow(
    serial,
    "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_string(),
    Map::new(),
    options(host.clone()),
  )
  .await
  .unwrap();
  let weather_terminal = event_position(
    &serial_result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "loadWeather"),
  );
  let soil_start = event_position(
    &serial_result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadSoil"),
  );
  assert!(weather_terminal < soil_start);

  let mut one = model();
  one.graph.nodes.retain(|node| node.id != "loadSoil");
  one
    .graph
    .edges
    .retain(|edge| edge.id != "fieldData:child:1" && edge.id != "fieldData:join:1");
  set_concurrency(&mut one, 1);
  set_script(
    &mut one,
    "buildReport",
    "return { temperature: context.steps.loadWeather.temperature };",
  );
  one.validate_for_execution().unwrap();
  let one_result = execute_workflow(
    one,
    "sha256:2222222222222222222222222222222222222222222222222222222222222222".to_string(),
    Map::new(),
    options(host),
  )
  .await
  .unwrap();
  assert_eq!(one_result.result, json!({ "temperature": 22 }));
  assert!(!one_result.context.steps.contains_key("loadSoil"));
}

#[tokio::test]
async fn durable_parallel_execution_serializes_events_and_reopens_as_complete() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let result = execute_workflow_durable(
    model(),
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  assert_eq!(
    result.result,
    json!({ "summary": "Weather 22°C, soil 41%" })
  );

  let store = woml_engine::DurableEventStore::open(database.path()).unwrap();
  let reopened = store.projection(&result.run_id).unwrap();
  assert_eq!(reopened.status, woml_engine::RunStatus::Succeeded);
  assert_eq!(reopened.context, result.context);
}
