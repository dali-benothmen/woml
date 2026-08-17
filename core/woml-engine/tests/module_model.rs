use woml_engine::CompiledWorkflowDefinition;

const MODEL_V9: &str =
  include_str!("../../../woml/tests/fixtures/modules/customer-import.compiled.v9.json");

#[test]
fn rust_accepts_the_frozen_model_v9_module_identity_contract() {
  let workflow = serde_json::from_str::<CompiledWorkflowDefinition>(MODEL_V9).unwrap();
  workflow.validate_for_durable_execution().unwrap();
  let runtime = workflow.module_runtime.expect("Model v9 moduleRuntime");
  assert_eq!(runtime.profile_version, 1);
  assert_eq!(runtime.modules[0].name, "spreadsheet");
  assert_eq!(runtime.modules[0].exports, ["read", "removeEmptyRows"]);
}
