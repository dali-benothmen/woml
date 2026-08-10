use std::path::PathBuf;

use chrono::Utc;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::{
  resume_workflow_durable, CompiledWorkflowDefinition, DurableEventStore, RuntimeExecutionOptions,
  RuntimeModuleArtifact, ScriptHostProcessOptions, TriggerAdmissionRequest,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/modules/customer-import.compiled.v9.json");
const BUNDLE: &str = include_str!("../../../woml/tests/fixtures/modules/spreadsheet.bundle.v1.mjs");
const SOURCE_MAP: &str =
  include_str!("../../../woml/tests/fixtures/modules/spreadsheet.bundle.v1.mjs.map");
const DEFINITION_HASH: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn workflow() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).expect("the frozen Model v9 fixture must decode")
}

fn artifact() -> RuntimeModuleArtifact {
  RuntimeModuleArtifact {
    name: "spreadsheet".to_string(),
    bundle_digest: "sha256:8a474bbae11ac5793e2bcaaede1d78e0a691592f98c165c6ffa2a93b2319dcb1"
      .to_string(),
    source_map_digest: "sha256:8ad1e63c024bce96ac458f62543d93cec5523889c706c52e79f309a9a70a3bed"
      .to_string(),
    exports: vec!["read".to_string(), "removeEmptyRows".to_string()],
    bundle: BUNDLE.to_string(),
    source_map: SOURCE_MAP.trim_end().to_string(),
  }
}

fn runtime_options() -> RuntimeExecutionOptions {
  let bun = PathBuf::from("bun");
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  RuntimeExecutionOptions::new(ScriptHostProcessOptions::new(bun, host), 5_000)
}

#[tokio::test]
async fn stored_artifacts_resume_without_any_source_or_caller_supplied_bundle() {
  let database = std::env::temp_dir().join(format!("woml-ms4-{}.sqlite", Uuid::new_v4().simple()));
  let workflow = workflow();
  let mut store = DurableEventStore::open(&database).expect("open durable store");
  store
    .register_definition_module_artifacts(&workflow, DEFINITION_HASH, &[artifact()])
    .expect("store exact immutable module artifacts");
  let admission = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: DEFINITION_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "ms4-recovery-test".to_string(),
      payload: Map::from_iter([(
        "rows".to_string(),
        json!([["Ada", "active"], ["", ""], ["Grace", "active"]]),
      )]),
      received_at: Utc::now(),
    })
    .expect("create a pending durable run");
  drop(store);

  let execution = resume_workflow_durable(database.clone(), &admission.run_id, runtime_options())
    .await
    .expect("recovery must load the stored bundle and source map");

  assert_eq!(
    execution.result,
    json!({"rows": [["Ada", "active"], ["Grace", "active"]]})
  );
  assert_eq!(execution.execution_order, vec!["cleanRows"]);
  let _ = std::fs::remove_file(database);
}

#[test]
fn artifact_identity_is_immutable_and_corruption_fails_before_execution() {
  let mut store = DurableEventStore::open_in_memory().expect("open durable store");
  let workflow = workflow();
  store
    .register_definition_module_artifacts(&workflow, DEFINITION_HASH, &[artifact()])
    .expect("store the reviewed artifact");
  assert_eq!(
    store
      .definition_module_artifacts(DEFINITION_HASH)
      .expect("load stored artifacts"),
    vec![artifact()]
  );

  let mut corrupt = artifact();
  corrupt.bundle.push_str("\n// changed");
  let error = store
    .register_definition_module_artifacts(&workflow, DEFINITION_HASH, &[corrupt])
    .expect_err("the same identity cannot acquire different bytes");
  assert!(error.to_string().contains("failed its immutable identity"));

  assert!(!artifact().bundle.contains("sk-"));
  assert!(!artifact().source_map.contains("Bearer "));
}
