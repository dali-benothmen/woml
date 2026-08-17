use std::path::PathBuf;

use serde_json::{json, Map};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, RunEventPayload, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new(name: &str) -> Self {
    let path = std::env::temp_dir().join(format!(
      "woml-sc10-integration-{name}-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }

  fn state(&self) -> PathBuf {
    self.0.join("state.sqlite")
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(())?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(PathBuf::from("bun"), host))
}

fn workflow(workflow_id: &str, source: &str) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  workflow.workflow_id = workflow_id.to_string();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  workflow.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  let woml_engine::model::ValueExpression::Object { fields } = &mut workflow.graph.nodes[0].inputs
  else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(source),
    },
  );
  workflow
}

fn hash(character: char) -> String {
  format!("sha256:{}", character.to_string().repeat(64))
}

#[tokio::test]
async fn cache_v1_runs_end_to_end_and_redacts_keys_and_values_from_operation_events() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("operations");
  let source = r#"
    const initial = await services.cache.get("private/customer:42");
    const stored = await services.cache.set(
      "private/customer:42",
      { message: "sensitive-cache-value" },
      { ttl: "15m", name: "store-customer" }
    );
    const loaded = await services.cache.get("private/customer:42");
    if (!loaded.hit || loaded.value.message !== "sensitive-cache-value") {
      throw new Error("cache round trip failed");
    }
    const present = await services.cache.has("private/customer:42");
    const counter = await services.cache.increment("counter", 2, {
      ttl: "1h", name: "increment-counter"
    });
    const winner = await services.cache.setIfAbsent("counter", 99, {
      ttl: "1h", name: "initialize-counter"
    });
    const removed = await services.cache.delete("private/customer:42", {
      name: "delete-customer"
    });
    const final = await services.cache.get("private/customer:42");
    return {
      initialMiss: !initial.hit,
      stored: stored.stored,
      present: present.present,
      counter: counter.value,
      existingWon: !winner.stored && winner.value === 2,
      deleted: removed.deleted,
      finalMiss: !final.hit
    };
  "#;
  let result = execute_workflow_durable(
    workflow("cache-operation-contract", source),
    hash('a'),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.state(),
  )
  .await
  .unwrap();

  assert_eq!(
    result.result,
    json!({
      "initialMiss": true,
      "stored": true,
      "present": true,
      "counter": 2,
      "existingWon": true,
      "deleted": true,
      "finalMiss": true,
    })
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
      .count(),
    8
  );
  for event in &result.events {
    if matches!(
      event.payload,
      RunEventPayload::OperationStarted(_)
        | RunEventPayload::OperationSucceeded(_)
        | RunEventPayload::OperationFailed(_)
    ) {
      let encoded = serde_json::to_string(event).unwrap();
      assert!(!encoded.contains("private/customer:42"));
      assert!(!encoded.contains("sensitive-cache-value"));
    }
  }
  assert!(directory.0.join("cache-v1.sqlite").is_file());
}

#[tokio::test]
async fn workflow_definition_updates_share_cache_but_other_workflow_ids_are_isolated() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("scope");
  let state = directory.state();
  execute_workflow_durable(
    workflow(
      "stable-workflow-id",
      r#"return await services.cache.set("warm", { revision: 1 }, { ttl: "1h" });"#,
    ),
    hash('b'),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    state.clone(),
  )
  .await
  .unwrap();

  let updated = execute_workflow_durable(
    workflow(
      "stable-workflow-id",
      r#"return await services.cache.get("warm");"#,
    ),
    hash('c'),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    state.clone(),
  )
  .await
  .unwrap();
  assert_eq!(updated.result["hit"], true);
  assert_eq!(updated.result["value"], json!({ "revision": 1 }));

  let isolated = execute_workflow_durable(
    workflow(
      "another-workflow-id",
      r#"return await services.cache.get("warm");"#,
    ),
    hash('d'),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    state.clone(),
  )
  .await
  .unwrap();
  assert_eq!(isolated.result, json!({ "hit": false }));

  let other_state_location = execute_workflow_durable(
    workflow(
      "stable-workflow-id",
      r#"return await services.cache.get("warm");"#,
    ),
    hash('e'),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.0.join("other-state.sqlite"),
  )
  .await
  .unwrap();
  assert_eq!(other_state_location.result, json!({ "hit": false }));
  let cache_files = std::fs::read_dir(&directory.0)
    .unwrap()
    .filter_map(Result::ok)
    .filter(|entry| entry.file_name().to_string_lossy().starts_with("cache-v1"))
    .count();
  assert_eq!(cache_files, 2);
}
