use std::path::PathBuf;

use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use woml_engine::model::{BackoffPolicy, RetryPolicy, ScriptRuntimeBindings, ValueExpression};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, LifecycleEventName, RunEventPayload,
  RuntimeExecutionOptions, RuntimeModuleArtifact, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const PARALLEL_MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const MODULE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/modules/customer-import.compiled.v9.json");
const LIFECYCLE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json");

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new(name: &str) -> Self {
    let path =
      std::env::temp_dir().join(format!("woml-ds3-{name}-{}", uuid::Uuid::new_v4().simple()));
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
  let ValueExpression::Object { fields } = &mut workflow.graph.nodes[0].inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
  workflow
}

fn hash(character: char) -> String {
  format!("sha256:{}", character.to_string().repeat(64))
}

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
}

fn sha256(value: &str) -> String {
  format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

fn promote_script_runtime_to_v8(workflow: &mut CompiledWorkflowDefinition) {
  workflow.schema_version = 8;
  for node in &mut workflow.graph.nodes {
    if node.handler == "runtime.script" && node.script_runtime.is_none() {
      node.script_runtime = Some(ScriptRuntimeBindings {
        binding_version: 1,
        bindings: vec![
          "context".to_string(),
          "attempt".to_string(),
          "services".to_string(),
          "secrets".to_string(),
        ],
        required_secrets: Vec::new(),
      });
    }
  }
}

#[tokio::test]
async fn state_v1_runs_through_bun_and_rust_and_persists_across_runs() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("round-trip");
  let state = directory.state();
  let first = execute_workflow_durable(
    workflow(
      "durable-state-demo",
      r#"
        const initialized = await services.state.setIfAbsent(
          "private/customer:42",
          { visits: 0, secret: "never-log-this" },
          { name: "initialize-customer" }
        );
        const initial = await services.state.get("private/customer:42");
        const saved = await services.state.set(
          "private/customer:42",
          { visits: initial.value.visits + 1, secret: initial.value.secret },
          { name: "save-customer", ifVersion: initial.version }
        );
        const present = await services.state.has("private/customer:42");
        let conflictCode = null;
        try {
          await services.state.set("private/customer:42", { visits: 99 }, {
            name: "stale-customer-write",
            ifVersion: 1
          });
        } catch (error) {
          conflictCode = error.code;
        }
        const temporary = await services.state.set("temporary", true, {
          name: "create-temporary",
          ifVersion: 0
        });
        const removed = await services.state.delete("temporary", {
          name: "delete-temporary",
          ifVersion: temporary.version
        });
        const loaded = await services.state.get("private/customer:42");
        return {
          initialized: initialized.stored,
          present: present.present,
          visits: loaded.value.visits,
          version: saved.version,
          conflictCode,
          deleted: removed.deleted
        };
      "#,
    ),
    hash('a'),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    state.clone(),
  )
  .await
  .unwrap();
  assert_eq!(
    first.result,
    json!({
      "initialized": true,
      "present": true,
      "visits": 1,
      "version": 2,
      "conflictCode": "WOML_STATE_CONFLICT",
      "deleted": true,
    })
  );

  let second = execute_workflow_durable(
    workflow(
      "durable-state-demo",
      r#"
        const loaded = await services.state.get("private/customer:42");
        const saved = await services.state.increment("visit-counter", 1, {
          name: "count-run"
        });
        return { visits: loaded.value.visits, counter: saved.value };
      "#,
    ),
    hash('b'),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    state,
  )
  .await
  .unwrap();
  assert_eq!(second.result, json!({ "visits": 1, "counter": 1 }));

  let isolated = execute_workflow_durable(
    workflow(
      "another-workflow",
      r#"return await services.state.get("private/customer:42");"#,
    ),
    hash('d'),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.state(),
  )
  .await
  .unwrap();
  assert_eq!(isolated.result, json!({ "found": false }));

  for execution in [&first, &second, &isolated] {
    for event in &execution.events {
      if matches!(
        event.payload,
        RunEventPayload::OperationStarted(_)
          | RunEventPayload::OperationSucceeded(_)
          | RunEventPayload::OperationFailed(_)
      ) {
        let encoded = serde_json::to_string(event).unwrap();
        assert!(!encoded.contains("private/customer:42"));
        assert!(!encoded.contains("never-log-this"));
        assert!(!encoded.contains("temporary"));
      }
    }
  }
  assert!(first.events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::OperationSucceeded(data)
      if data.capability == "state"
        && data.metadata.get("profile") == Some(&json!("woml.state-operation-metadata/v1"))
        && data.metadata.contains_key("keyDigest")
        && data.metadata.contains_key("inputDigest")
        && data.metadata.contains_key("resultDigest")
        && data.metadata.contains_key("durationMs")
  )));
}

#[tokio::test]
async fn retry_reattaches_named_mutation_instead_of_applying_it_twice() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("retry");
  let mut model = workflow(
    "durable-state-retry",
    r#"
      const counter = await services.state.increment("attempt-counter", 1, {
        name: "increment-once"
      });
      if (attempt.number === 1) throw new Error("fail after the state commit");
      return counter;
    "#,
  );
  model.graph.nodes[0].retry_policy = Some(RetryPolicy {
    max_attempts: 2,
    backoff: BackoffPolicy::Fixed { delay_ms: 1 },
  });
  let result = execute_workflow_durable(
    model,
    hash('c'),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.state(),
  )
  .await
  .unwrap();
  assert_eq!(result.result["value"], 1);
  assert_eq!(result.result["version"], 1);
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::OperationSucceeded(data)
          if data.capability == "state" && data.operation == "increment"
      ))
      .count(),
    2
  );
}

#[tokio::test]
async fn state_is_available_inside_selected_branches_and_parallel_children() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("dag-composition");

  let mut branch: CompiledWorkflowDefinition = serde_json::from_str(BRANCH_MODEL).unwrap();
  promote_script_runtime_to_v8(&mut branch);
  set_script(
    &mut branch,
    "reviewContent",
    r#"
      const saved = await services.state.setIfAbsent("decision", "reviewed", {
        name: "remember-decision"
      });
      return { status: saved.value, accepted: true };
    "#,
  );
  set_script(
    &mut branch,
    "publishDecision",
    r#"
      const durable = await services.state.get("decision");
      return { message: `Final status: ${durable.value}` };
    "#,
  );
  let branch_result = execute_workflow_durable(
    branch,
    hash('e'),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    directory.state(),
  )
  .await
  .unwrap();
  assert_eq!(
    branch_result.result,
    json!({ "message": "Final status: reviewed" })
  );

  let mut parallel: CompiledWorkflowDefinition = serde_json::from_str(PARALLEL_MODEL).unwrap();
  promote_script_runtime_to_v8(&mut parallel);
  set_script(
    &mut parallel,
    "loadWeather",
    r#"return await services.state.increment("weather-loads", 1, { name: "weather-load" });"#,
  );
  set_script(
    &mut parallel,
    "loadSoil",
    r#"return await services.state.increment("soil-loads", 1, { name: "soil-load" });"#,
  );
  set_script(
    &mut parallel,
    "buildReport",
    r#"
      const weather = await services.state.get("weather-loads");
      const soil = await services.state.get("soil-loads");
      return { weather: weather.value, soil: soil.value };
    "#,
  );
  let parallel_result = execute_workflow_durable(
    parallel,
    hash('f'),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.state(),
  )
  .await
  .unwrap();
  assert_eq!(parallel_result.result, json!({ "weather": 1, "soil": 1 }));
}

#[tokio::test]
async fn state_is_available_inside_lifecycle_scripts_and_local_modules() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("script-locations");
  let mut lifecycle: CompiledWorkflowDefinition = serde_json::from_str(LIFECYCLE_MODEL).unwrap();
  lifecycle.lifecycle.as_mut().unwrap().hooks.retain(|hook| {
    matches!(
      hook.event,
      LifecycleEventName::RunStart | LifecycleEventName::RunComplete
    )
  });
  for hook in &mut lifecycle.lifecycle.as_mut().unwrap().hooks {
    let source = if hook.event == LifecycleEventName::RunStart {
      r#"await services.state.increment("lifecycle-runs", 1, { name: "count-lifecycle-run" });"#
    } else {
      r#"
        const count = await services.state.get("lifecycle-runs");
        if (!count.found || count.value !== 1) throw new Error("lifecycle state was unavailable");
      "#
    };
    let ValueExpression::Object { fields } = &mut hook.actions[0].inputs else {
      panic!("expected lifecycle script inputs");
    };
    fields.insert(
      "source".to_string(),
      ValueExpression::Literal {
        value: json!(source),
      },
    );
  }
  lifecycle.validate_structure().unwrap();
  execute_workflow_durable(
    lifecycle,
    hash('1'),
    Map::from_iter([("orderId".to_string(), json!("order-1"))]),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    directory.state(),
  )
  .await
  .unwrap();

  let bundle = r#"
    async function removeEmptyRows(rows) {
      const count = await services.state.increment("module-runs", 1, {
        name: "count-module-run"
      });
      return { rows, count: count.value };
    }
    function read(rows) { return rows; }
    export { read, removeEmptyRows };
  "#;
  let source_map =
    r#"{"version":3,"sources":["memory.ts"],"sourcesContent":[""],"names":[],"mappings":""}"#;
  let mut module_workflow: CompiledWorkflowDefinition = serde_json::from_str(MODULE_MODEL).unwrap();
  let binding = &mut module_workflow.module_runtime.as_mut().unwrap().modules[0];
  binding.bundle_digest = sha256(bundle);
  binding.source_map_digest = sha256(source_map);
  let artifact = RuntimeModuleArtifact {
    name: "spreadsheet".to_string(),
    bundle_digest: binding.bundle_digest.clone(),
    source_map_digest: binding.source_map_digest.clone(),
    exports: binding.exports.clone(),
    bundle: bundle.to_string(),
    source_map: source_map.to_string(),
  };
  let result = execute_workflow_durable(
    module_workflow,
    hash('2'),
    Map::from_iter([("rows".to_string(), json!([["Ada"], ["Grace"]]))]),
    RuntimeExecutionOptions::new(host, 10_000).with_runtime_modules(vec![artifact]),
    directory.state(),
  )
  .await
  .unwrap();
  assert_eq!(
    result.result,
    json!({ "rows": { "rows": [["Ada"], ["Grace"]], "count": 1 } })
  );
}
