use serde_json::{json, Value};
use woml_engine::model::{EdgeCondition, ModelIssueCode, ValueExpression};
use woml_engine::{CompiledWorkflowDefinition, COMPILED_MODEL_SCHEMA_VERSION_V4};

const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");

fn approval_model() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(APPROVAL_MODEL)
    .expect("the reviewed approval model must deserialize")
}

fn has_issue(workflow: &CompiledWorkflowDefinition, code: ModelIssueCode) -> bool {
  workflow
    .validate_structure()
    .expect_err("the malformed approval model must be rejected")
    .issues
    .iter()
    .any(|issue| issue.code == code)
}

#[test]
fn accepts_the_reviewed_model_v4_approval_dag_without_rewriting_it() {
  let original: Value = serde_json::from_str(APPROVAL_MODEL).unwrap();
  let model = approval_model();

  model.validate_structure().unwrap();
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V4);
  assert_eq!(model.graph.entry_node_ids, ["prepareArticle"]);
  assert_eq!(model.terminal_node_id(), Some("finalStatus"));
  assert_eq!(serde_json::to_value(&model).unwrap(), original);

  let wait = model.node("editorApproval").unwrap();
  assert_eq!(wait.handler, "engine.approval-wait");
  assert_eq!(
    wait.metadata.as_ref().unwrap().get("name"),
    Some(&json!("Editorial approval"))
  );
  assert_eq!(
    model
      .node("__woml_approval__editorApproval__join")
      .unwrap()
      .handler,
    "engine.approval-join"
  );
}

#[test]
fn rejects_unowned_or_malformed_approval_routes() {
  let mut missing_owner = approval_model();
  missing_owner.graph.edges[1].approval_id = None;
  assert!(has_issue(
    &missing_owner,
    ModelIssueCode::InvalidApprovalGroup
  ));

  let mut wrong_decision_path = approval_model();
  wrong_decision_path.graph.edges[1].condition = EdgeCondition::Equals {
    left: ValueExpression::ContextReference {
      path: vec![
        "steps".to_string(),
        "otherApproval".to_string(),
        "decision".to_string(),
      ],
    },
    right: ValueExpression::Literal {
      value: json!("approved"),
    },
  };
  assert!(has_issue(
    &wrong_decision_path,
    ModelIssueCode::InvalidApprovalGroup
  ));

  let mut mixed_owner = approval_model();
  mixed_owner.graph.edges[1].branch_id = Some("otherGroup".to_string());
  assert!(has_issue(
    &mixed_owner,
    ModelIssueCode::InvalidApprovalGroup
  ));
}

#[test]
fn rejects_bad_wait_inputs_join_identity_and_pre_v4_ownership() {
  let mut leaked_runtime_value = approval_model();
  let wait = leaked_runtime_value
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "editorApproval")
    .unwrap();
  let ValueExpression::Object { fields } = &mut wait.inputs else {
    panic!("approval wait inputs must be an object");
  };
  fields.insert(
    "token".to_string(),
    ValueExpression::Literal {
      value: json!("forbidden"),
    },
  );
  assert!(has_issue(
    &leaked_runtime_value,
    ModelIssueCode::InvalidApprovalGroup
  ));

  let mut bad_join = approval_model();
  bad_join
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.handler == "engine.approval-join")
    .unwrap()
    .id = "generatedJoin".to_string();
  assert!(has_issue(&bad_join, ModelIssueCode::InvalidApprovalGroup));

  let mut wrong_version = approval_model();
  wrong_version.schema_version = 3;
  assert!(has_issue(
    &wrong_version,
    ModelIssueCode::InvalidApprovalGroup
  ));
}

#[test]
fn keeps_execution_gated_until_the_approval_runtime_phases() {
  let issues = approval_model()
    .validate_for_execution()
    .unwrap_err()
    .issues;

  assert!(issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnknownHandler));
  assert!(issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedEdgeCondition));
}
