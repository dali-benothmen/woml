use woml_engine::CompiledWorkflowDefinition;

const MODEL_V9: &str =
  include_str!("../../../woml/tests/fixtures/modules/customer-import.compiled.v9.json");

#[test]
fn ms2_model_v9_remains_fail_closed_until_artifact_registration_exists() {
  let error = serde_json::from_str::<CompiledWorkflowDefinition>(MODEL_V9).unwrap_err();
  assert!(
    error.to_string().contains("moduleRuntime"),
    "Rust must reject the new module field instead of silently executing Model v9: {error}"
  );
}
