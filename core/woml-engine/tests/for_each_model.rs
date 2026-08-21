use serde_json::{json, Value};
use woml_engine::{CompiledWorkflowDefinition, ModelIssueCode};

const REVIEWED_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/for-each/model.v16.reviewed.json");

fn reviewed_value() -> Value {
  serde_json::from_str(REVIEWED_MODEL).expect("reviewed Model v16 fixture must be JSON")
}

fn assert_for_each_rejected(value: Value) {
  let model: CompiledWorkflowDefinition =
    serde_json::from_value(value).expect("mutated fixture must remain deserializable");
  let error = model
    .validate_structure()
    .expect_err("malformed Model v16 for-each contract must be rejected");
  assert!(
    error
      .issues
      .iter()
      .any(|issue| issue.code == ModelIssueCode::InvalidForEach),
    "expected INVALID_FOR_EACH, received {:?}",
    error.issues
  );
}

#[test]
fn reviewed_model_v16_deserializes_and_passes_structural_validation() {
  let model = CompiledWorkflowDefinition::from_json(REVIEWED_MODEL)
    .expect("Rust must deserialize the shared Model v16 fixture");
  model
    .validate_structure()
    .expect("Rust must independently accept the reviewed loop body DAG");

  let descriptor = &model.graph.for_each.as_ref().unwrap()[0];
  assert_eq!(descriptor.for_each_id, "organize");
  assert_eq!(descriptor.body.entry_node_ids, ["normalize"]);
  assert_eq!(descriptor.body.terminal_node_id, "normalize");
  assert_eq!(descriptor.outer_step_ids, ["load"]);
}

#[test]
fn rejects_invalid_concurrency_and_identity_collisions() {
  let mut excessive = reviewed_value();
  excessive["graph"]["forEach"][0]["concurrency"] = json!(65);
  assert_for_each_rejected(excessive);

  let mut collision = reviewed_value();
  collision["graph"]["forEach"][0]["body"]["nodes"][0]["id"] = json!("load");
  collision["graph"]["forEach"][0]["body"]["entryNodeIds"][0] = json!("load");
  collision["graph"]["forEach"][0]["body"]["terminalNodeId"] = json!("load");
  collision["graph"]["forEach"][0]["body"]["contextVisibility"][0]["nodeId"] = json!("load");
  collision["graph"]["forEach"][0]["result"]["path"][1] = json!("load");
  assert_for_each_rejected(collision);
}

#[test]
fn rejects_cycles_visibility_escapes_and_invalid_items_references() {
  let mut cycle = reviewed_value();
  cycle["graph"]["forEach"][0]["body"]["edges"] = json!([{
    "id": "normalize-to-normalize",
    "from": "normalize",
    "to": "normalize",
    "condition": { "kind": "always" }
  }]);
  assert_for_each_rejected(cycle);

  let mut escape = reviewed_value();
  escape["graph"]["forEach"][0]["body"]["contextVisibility"][0]["stepIds"] = json!(["load"]);
  assert_for_each_rejected(escape);

  let mut forward_reference = reviewed_value();
  forward_reference["graph"]["forEach"][0]["items"]["path"][1] = json!("summary");
  assert_for_each_rejected(forward_reference);
}

#[test]
fn model_v15_cannot_smuggle_a_for_each_descriptor() {
  let mut value = reviewed_value();
  value["schemaVersion"] = json!(15);
  assert_for_each_rejected(value);
}

#[test]
fn fe2_does_not_make_model_v16_executable() {
  let model = CompiledWorkflowDefinition::from_json(REVIEWED_MODEL).unwrap();
  let error = model
    .validate_for_durable_execution()
    .expect_err("loop runtime handlers are intentionally implemented in FE3");
  assert!(error.issues.iter().any(|issue| {
    matches!(
      issue.code,
      ModelIssueCode::UnknownHandler | ModelIssueCode::UnsupportedForkExecution
    )
  }));
}
