use woml_engine::{CompiledWorkflowDefinition, ModelIssueCode, COMPILED_MODEL_SCHEMA_VERSION_V13};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");
const CONTROL_CHOICE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/control-choice.compiled.v13.json");

fn reviewed_model() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(MODEL).expect("the reviewed Model v13 fixture must decode")
}

fn has_issue(workflow: &CompiledWorkflowDefinition, code: ModelIssueCode) -> bool {
  workflow
    .validate_structure()
    .expect_err("the malformed Model v13 graph must be rejected")
    .issues
    .iter()
    .any(|issue| issue.code == code)
}

#[test]
fn rust_accepts_the_reviewed_model_v13_graph_without_rewriting_it() {
  let original: serde_json::Value = serde_json::from_str(MODEL).unwrap();
  let model = reviewed_model();

  model.validate_structure().unwrap();
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V13);
  assert_eq!(model.graph.forks.as_ref().unwrap().len(), 1);
  assert_eq!(
    model.graph.settlement.as_ref().unwrap().main_result_node_id,
    "finish"
  );
  assert_eq!(serde_json::to_value(&model).unwrap(), original);
}

#[test]
fn rust_rejects_malformed_ownership_visibility_and_settlement() {
  let mut unknown_join = reviewed_model();
  unknown_join.graph.forks.as_mut().unwrap()[0]
    .joined_branch_ids
    .push("missing".to_string());
  assert!(has_issue(&unknown_join, ModelIssueCode::InvalidForkGraph));

  let mut missing_visibility = reviewed_model();
  missing_visibility
    .graph
    .context_visibility
    .as_mut()
    .unwrap()
    .pop();
  assert!(has_issue(
    &missing_visibility,
    ModelIssueCode::InvalidContextVisibility
  ));

  let mut wrong_result = reviewed_model();
  wrong_result
    .graph
    .settlement
    .as_mut()
    .unwrap()
    .main_result_node_id = "missing".to_string();
  assert!(has_issue(
    &wrong_result,
    ModelIssueCode::InvalidWorkflowSettlement
  ));
}

#[test]
fn rust_structurally_accepts_v13_but_explicitly_gates_execution_until_fj5() {
  let model = reviewed_model();
  let error = model.validate_for_durable_execution().unwrap_err();
  assert_eq!(error.issues.len(), 1);
  assert_eq!(
    error.issues[0].code,
    ModelIssueCode::UnsupportedForkExecution
  );
}

#[test]
fn rust_validates_control_only_choice_structure_independently() {
  let choice = CompiledWorkflowDefinition::from_json(CONTROL_CHOICE_MODEL).unwrap();
  choice.validate_structure().unwrap();
  assert!(choice.graph.forks.as_ref().unwrap().is_empty());
  assert_eq!(choice.graph.choices.as_ref().unwrap().len(), 1);

  let mut malformed = choice;
  malformed.graph.choices.as_mut().unwrap()[0].arm_ids[0] = "wrong".to_string();
  assert!(has_issue(&malformed, ModelIssueCode::InvalidControlChoice));
}
