//! model saga validation.

use super::{
  empty_control_node, issue, valid_public_structural_id, valid_secret_name,
  CompiledCorrectionIdentity, CompiledCorrectiveAction, CompiledWorkflowDefinition, ModelIssue,
  ModelIssueCode, ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V19;
use std::collections::HashMap;
use std::collections::HashSet;

pub(super) fn inspect_saga_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V19 {
    if workflow.graph.sagas.is_some() {
      issues.push(issue(
        ModelIssueCode::InvalidSaga,
        "Saga descriptors are available only in compiled Model v19.",
      ));
    }
    return;
  }
  let Some(sagas) = workflow.graph.sagas.as_deref() else {
    issues.push(issue(
      ModelIssueCode::InvalidSaga,
      "Compiled Model v19 requires a sagas descriptor collection.",
    ));
    return;
  };
  if (workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V19 && sagas.is_empty())
    || sagas.len() > 64
  {
    issues.push(issue(
      ModelIssueCode::InvalidSaga,
      "Compiled Model v19 requires between 1 and 64 sagas; Model v20 permits an empty inherited collection.",
    ));
    return;
  }
  let nodes = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect::<HashMap<_, _>>();
  let mut saga_ids = HashSet::new();
  let mut action_ids = HashSet::new();
  let mut described_controls = HashSet::new();
  let valid_action = |action: &CompiledCorrectiveAction, compensation: bool| {
    let identity_valid = match &action.identity {
      CompiledCorrectionIdentity::Static { action_id } => action_id == &action.action_id,
      CompiledCorrectionIdentity::ForEachIteration {
        for_each_id,
        action_id_template,
      } => {
        valid_public_structural_id(for_each_id)
          && action_id_template.contains("{iterationKey}")
          && action_id_template.len() <= 1024
      }
    };
    let node = &action.node;
    identity_valid
      && !action.action_id.is_empty()
      && action.action_id.len() <= 768
      && node.id == action.action_id
      && node.handler == "runtime.script"
      && matches!(&node.inputs, ValueExpression::Object { fields }
        if fields.len() == 1
          && matches!(fields.get("source"), Some(ValueExpression::Literal { value }) if value.is_string()))
      && node.script_runtime.as_ref().is_some_and(|runtime| {
        runtime.binding_version == 1
          && runtime.bindings == ["context", "attempt", "services", "secrets"]
          && runtime.required_secrets.len() <= 64
          && runtime
            .required_secrets
            .windows(2)
            .all(|pair| pair[0] < pair[1])
          && runtime
            .required_secrets
            .iter()
            .all(|name| valid_secret_name(name))
      })
      && node.retry_policy.as_ref().is_none_or(|policy| {
        (1..=10).contains(&policy.max_attempts)
          && policy
            .delay_before_attempt(policy.max_attempts.max(2))
            .is_some()
      })
      && node.timeout_ms.is_none_or(|value| value > 0)
      && action
        .visible_step_ids
        .iter()
        .all(|id| valid_public_structural_id(id))
      && action.visible_step_ids.iter().collect::<HashSet<_>>().len()
        == action.visible_step_ids.len()
      && if compensation {
        action
          .for_step_id
          .as_deref()
          .is_some_and(valid_public_structural_id)
      } else {
        action.for_step_id.is_none()
      }
  };
  for saga in sagas {
    described_controls.insert(saga.open_node_id.as_str());
    described_controls.insert(saga.commit_node_id.as_str());
    described_controls.insert(saga.result_node_id.as_str());
    let controls_valid =
      empty_control_node(nodes.get(saga.open_node_id.as_str()), "engine.saga-open")
        && empty_control_node(
          nodes.get(saga.commit_node_id.as_str()),
          "engine.saga-commit",
        )
        && nodes.get(saga.result_node_id.as_str()).is_some_and(|node| {
          node.handler == "engine.saga-result"
            && node.timeout_ms.is_none()
            && node.retry_policy.is_none()
            && node.script_runtime.is_none()
            && matches!(&node.inputs, ValueExpression::Object { fields } if fields.is_empty())
        });
    let actions_valid = saga
      .compensations
      .iter()
      .all(|action| action_ids.insert(action.action_id.as_str()) && valid_action(action, true))
      && saga
        .cleanup
        .iter()
        .all(|action| action_ids.insert(action.action_id.as_str()) && valid_action(action, false));
    let valid = valid_public_structural_id(&saga.saga_id)
      && saga_ids.insert(saga.saga_id.as_str())
      && saga.open_node_id == format!("__woml_saga__{}__open", saga.saga_id)
      && saga.commit_node_id == format!("__woml_saga__{}__commit", saga.saga_id)
      && saga.result_node_id == saga.saga_id
      && controls_valid
      && nodes.contains_key(saga.body_entry_node_id.as_str())
      && nodes.contains_key(saga.body_terminal_node_id.as_str())
      && nodes.contains_key(saga.body_result_node_id.as_str())
      && saga.compensations.len() <= 256
      && saga.cleanup.len() <= 64
      && actions_valid;
    if !valid {
      issues.push(issue(
        ModelIssueCode::InvalidSaga,
        format!(
          "Saga {:?} does not match the frozen Model v19 descriptor contract.",
          saga.saga_id
        ),
      ));
    }
  }
  let controls = workflow
    .graph
    .nodes
    .iter()
    .filter(|node| {
      matches!(
        node.handler.as_str(),
        "engine.saga-open" | "engine.saga-commit" | "engine.saga-result"
      )
    })
    .map(|node| node.id.as_str())
    .collect::<HashSet<_>>();
  if controls != described_controls {
    issues.push(issue(
      ModelIssueCode::InvalidSaga,
      "Model v19 sagas must describe every saga control node exactly once.",
    ));
  }
}
