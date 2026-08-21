use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, DurableEventStore, RunEventPayload,
  RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const REVIEWED_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/for-each/model.v16.reviewed.json");

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

fn sequential_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  value["graph"]["forEach"][0]["concurrency"] = json!(1);
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

#[tokio::test]
async fn sequential_iterations_publish_ordered_results_for_later_steps() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = sequential_model();
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), json!(["alpha", "beta", "gamma"]));

  let result = execute_workflow_durable(
    model,
    "sha256:1616161616161616161616161616161616161616161616161616161616161616".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result, json!({ "processed": 3 }));
  assert_eq!(result.context.steps["organize"]["total"], 3);
  assert_eq!(result.context.steps["organize"]["succeeded"], 3);
  assert_eq!(
    result.context.steps["organize"]["results"],
    json!([
      { "value": "alpha", "index": 0 },
      { "value": "beta", "index": 1 },
      { "value": "gamma", "index": 2 }
    ])
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationSucceeded(_)))
      .count(),
    3
  );
  assert!(result.events.iter().all(|event| {
    !matches!(
      event.payload,
      RunEventPayload::ForEachIterationStarted(_) | RunEventPayload::ForEachIterationSucceeded(_)
    ) || event.iteration.is_some()
  }));
  let reopened = DurableEventStore::open(database.path())
    .unwrap()
    .projection(&result.run_id)
    .unwrap();
  assert_eq!(
    reopened.context.steps["organize"],
    result.context.steps["organize"]
  );
}

#[tokio::test]
async fn an_empty_items_array_settles_without_an_iteration() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = sequential_model();
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), json!([]));

  let result = execute_workflow_durable(
    model,
    "sha256:2626262626262626262626262626262626262626262626262626262626262626".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result, json!({ "processed": 0 }));
  assert_eq!(
    result.context.steps["organize"],
    json!({ "total": 0, "succeeded": 0 })
  );
  assert!(!result
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::ForEachIterationStarted(_))));
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!("woml-for-each-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    Self(directory.join("state.sqlite"))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    if let Some(directory) = self.0.parent() {
      let _ = std::fs::remove_dir_all(directory);
    }
  }
}
