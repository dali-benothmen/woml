use std::path::PathBuf;

use serde_json::{json, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use woml_engine::{CompiledWorkflowDefinition, ModelIssueCode};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json");

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).expect("reviewed Model v21 fixture")
}

fn issue_codes(workflow: &CompiledWorkflowDefinition) -> Vec<ModelIssueCode> {
  workflow
    .validate_structure()
    .expect_err("mutated placement must fail closed")
    .issues
    .into_iter()
    .map(|issue| issue.code)
    .collect()
}

#[test]
fn reviewed_model_v21_is_structurally_valid_but_not_locally_executable() {
  let workflow = model();
  workflow.validate_structure().expect("Model v21 structure");

  let error = workflow
    .validate_for_durable_execution()
    .expect_err("RET1 must not silently run a targeted step locally");
  assert!(error
    .issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedRemoteExecution));
}

#[test]
fn rust_rejects_backported_malformed_and_misplaced_execution_descriptors() {
  let mut future: Value = serde_json::from_str(MODEL).unwrap();
  future["schemaVersion"] = Value::from(22);
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(future).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::UnsupportedSchemaVersion));

  let mut backported: Value = serde_json::from_str(MODEL).unwrap();
  backported["schemaVersion"] = Value::from(20);
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(backported).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::InvalidExecutionPlacement));

  let mut malformed: Value = serde_json::from_str(MODEL).unwrap();
  let forward = malformed["graph"]["nodes"]
    .as_array_mut()
    .unwrap()
    .iter_mut()
    .find(|node| node["id"] == "applyRelease")
    .unwrap();
  forward["execution"]["target"] = Value::String("Production/unsafe".to_string());
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(malformed).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::InvalidExecutionPlacement));

  let mut unsorted: Value = serde_json::from_str(MODEL).unwrap();
  let forward = unsorted["graph"]["nodes"]
    .as_array_mut()
    .unwrap()
    .iter_mut()
    .find(|node| node["id"] == "applyRelease")
    .unwrap();
  forward["execution"]["requiredCapabilities"] = json!(["services.process", "runtime.script"]);
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(unsorted).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::InvalidExecutionPlacement));

  let mut engine_placement: Value = serde_json::from_str(MODEL).unwrap();
  let engine_node = engine_placement["graph"]["nodes"]
    .as_array_mut()
    .unwrap()
    .iter_mut()
    .find(|node| node["handler"] != "runtime.script")
    .unwrap();
  engine_node["execution"] = json!({
    "profileVersion": 1,
    "target": "production-private",
    "requiredCapabilities": ["runtime.script"]
  });
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(engine_placement).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::InvalidExecutionPlacement));
}

#[test]
fn correction_placement_cannot_drift_from_the_completed_forward_step() {
  let mut drifted: Value = serde_json::from_str(MODEL).unwrap();
  drifted["graph"]["sagas"][0]["compensations"][0]["node"]["execution"]["target"] =
    Value::String("maintenance-runner".to_string());
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(drifted).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::InvalidExecutionPlacement));
}

#[test]
fn model_v21_requires_at_least_one_explicit_execution_placement() {
  let mut unplaced: Value = serde_json::from_str(MODEL).unwrap();
  for node in unplaced["graph"]["nodes"].as_array_mut().unwrap() {
    node.as_object_mut().unwrap().remove("execution");
  }
  for saga in unplaced["graph"]["sagas"].as_array_mut().unwrap() {
    for action in saga["compensations"].as_array_mut().unwrap() {
      action["node"].as_object_mut().unwrap().remove("execution");
    }
    for action in saga["cleanup"].as_array_mut().unwrap() {
      action["node"].as_object_mut().unwrap().remove("execution");
    }
  }
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(unplaced).unwrap();
  assert!(issue_codes(&workflow).contains(&ModelIssueCode::InvalidExecutionPlacement));
}

#[test]
fn typescript_and_rust_share_the_exact_reviewed_model_v21_json() {
  let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
  let fixture = repository.join("woml/tests/fixtures/remote-execution-targets/model.reviewed.woml");
  let output = std::process::Command::new("bun")
    .current_dir(&repository)
    .env("WOML_FIXTURE", fixture)
    .args([
      "-e",
      "import {readFileSync} from 'node:fs'; import {parseWoml,compileWoml} from './woml/src/index.ts'; const source=readFileSync(process.env.WOML_FIXTURE,'utf8').replace(/\\r\\n/g,'\\n'); process.stdout.write(JSON.stringify(compileWoml(parseWoml(source))));",
    ])
    .output()
    .expect("run TypeScript compiler");
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );

  let typescript: Value = serde_json::from_slice(&output.stdout).unwrap();
  let reviewed: Value = serde_json::from_str(MODEL).unwrap();
  assert_eq!(typescript, reviewed);
  let rust: CompiledWorkflowDefinition = serde_json::from_value(reviewed.clone()).unwrap();
  assert_eq!(
    canonical_json(&serde_json::to_value(rust).unwrap()).unwrap(),
    canonical_json(&reviewed).unwrap()
  );
}
