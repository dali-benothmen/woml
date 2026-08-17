use woml_engine::{CompiledWorkflowDefinition, ModelIssueCode};

const REVIEWED_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/reusable-definitions/model-v14.reviewed.json");

fn reviewed_model() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(REVIEWED_MODEL)
    .expect("the reviewed reusable-operation model must decode")
}

fn has_issue(workflow: &CompiledWorkflowDefinition, code: ModelIssueCode) -> bool {
  workflow
    .validate_structure()
    .expect_err("the malformed reusable-operation model must be rejected")
    .issues
    .iter()
    .any(|issue| issue.code == code)
}

#[test]
fn rust_accepts_the_reviewed_model_v14_reusable_operation_shape() {
  let original: serde_json::Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  let workflow = reviewed_model();

  workflow.validate_structure().unwrap();
  assert_eq!(workflow.schema_version, 14);
  assert_eq!(workflow.reusable_definitions.as_ref().unwrap().len(), 2);
  assert_eq!(serde_json::to_value(&workflow).unwrap(), original);
}

#[test]
fn reusable_steps_are_valid_for_durable_execution() {
  reviewed_model()
    .validate_for_durable_execution()
    .expect("the published reusable-step model must support durable execution");
}

#[test]
fn rust_rejects_malformed_props_artifacts_and_source_provenance() {
  let mut malformed_prop = reviewed_model();
  let definitions = malformed_prop.reusable_definitions.as_mut().unwrap();
  if let woml_engine::model::CompiledReusableInvocation::Step { props, .. } = &mut definitions[0] {
    props[0].secret = true;
  }
  assert!(has_issue(
    &malformed_prop,
    ModelIssueCode::InvalidReusableDefinition
  ));

  let mut malformed_artifact = reviewed_model();
  let definitions = malformed_artifact.reusable_definitions.as_mut().unwrap();
  if let woml_engine::model::CompiledReusableInvocation::Step {
    script_artifact_id, ..
  } = &mut definitions[0]
  {
    script_artifact_id.clear();
  }
  assert!(has_issue(
    &malformed_artifact,
    ModelIssueCode::InvalidReusableDefinition
  ));

  let mut malformed_source = reviewed_model();
  let definitions = malformed_source.reusable_definitions.as_mut().unwrap();
  if let woml_engine::model::CompiledReusableInvocation::Step { source, .. } = &mut definitions[0] {
    *source = "../outside.woml".to_string();
  }
  assert!(has_issue(
    &malformed_source,
    ModelIssueCode::InvalidReusableDefinition
  ));
}

#[test]
fn rust_rejects_a_binding_v3_node_that_does_not_match_its_descriptor() {
  let mut workflow = reviewed_model();
  workflow.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .binding_version = 1;

  assert!(has_issue(
    &workflow,
    ModelIssueCode::InvalidReusableDefinition
  ));
}

#[test]
fn rust_requires_one_generic_delivery_for_each_provider_descriptor() {
  let mut workflow = reviewed_model();
  let definitions = workflow.reusable_definitions.as_mut().unwrap();
  if let woml_engine::model::CompiledReusableInvocation::NotificationProvider {
    provider_id, ..
  } = &mut definitions[1]
  {
    *provider_id = "unknown-provider-delivery".to_string();
  }

  assert!(has_issue(
    &workflow,
    ModelIssueCode::InvalidReusableDefinition
  ));
}
