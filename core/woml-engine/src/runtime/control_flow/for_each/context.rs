//! Iteration-scoped reference resolution, routing, readiness, and effect keys.

use crate::engine::resolve_context_reference;
use crate::model::ValueExpression;
use crate::protocol::RuntimeModuleBinding;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::WorkflowContext;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::HashSet;

pub(in crate::runtime) fn for_each_digest(value: &Value) -> Result<String, RuntimeExecutionError> {
  let bytes = serde_json_canonicalizer::to_vec(value)
    .map_err(|error| RuntimeExecutionError::Stalled(error.to_string()))?;
  Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub(crate) fn for_each_effect_key(
  run_id: &str,
  definition_hash: &str,
  for_each_id: &str,
  index: u32,
  node_id: &str,
) -> String {
  let identity = serde_json::json!({
    "contract": "woml.for-each-effect",
    "version": 1,
    "runId": run_id,
    "definitionHash": definition_hash,
    "forEachId": for_each_id,
    "index": index,
    "nodeId": node_id,
  });
  let bytes = serde_json_canonicalizer::to_vec(&identity)
    .expect("a static for-each effect identity is canonical JSON");
  format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(in crate::runtime) fn resolve_for_each_reference(
  expression: &ValueExpression,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<Value, RuntimeExecutionError> {
  let ValueExpression::ContextReference { path } = expression else {
    return Err(RuntimeExecutionError::Stalled(
      "for-each result is not a context reference".to_string(),
    ));
  };
  if matches!(path.first().map(String::as_str), Some("trigger" | "steps")) {
    return resolve_context_reference(expression, context).map_err(|_| {
      RuntimeExecutionError::Stalled(format!(
        "for-each result reference context.{} is unavailable",
        path.join(".")
      ))
    });
  }
  let (mut value, remaining): (Value, &[String]) = match path.first().map(String::as_str) {
    Some("item") => (item.clone(), &path[1..]),
    Some("iteration") => (
      serde_json::json!({ "index": index, "total": total }),
      &path[1..],
    ),
    _ => {
      return Err(RuntimeExecutionError::Stalled(
        "for-each result has an unsupported context root".to_string(),
      ));
    }
  };
  for segment in remaining {
    value = value
      .as_object()
      .and_then(|object| object.get(segment))
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "for-each result reference context.{} is unavailable",
          path.join(".")
        ))
      })?;
  }
  Ok(value)
}

pub(in crate::runtime) fn for_each_selected_choice(
  descriptor: &crate::model::CompiledForEach,
  selector_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<(String, String), RuntimeExecutionError> {
  let choice = descriptor
    .body
    .choices
    .iter()
    .find(|choice| choice.selector_node_id == selector_id)
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "for-each choice selector {selector_id:?} has no descriptor"
      ))
    })?;
  if let Some(selector) = &choice.string_selector {
    let selected = resolve_for_each_reference(selector, context, item, index, total)?;
    let Value::String(selected) = selected else {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "for-each switch {:?} must resolve to a string",
        choice.choice_id
      )));
    };
    let arm_id = choice
      .string_cases
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|case| case.value == selected)
      .map(|case| case.arm_id.clone())
      .or_else(|| choice.default_arm_id.clone())
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(format!(
          "for-each switch {:?} has no matching case or default",
          choice.choice_id
        ))
      })?;
    return Ok((choice.choice_id.clone(), arm_id));
  }
  for edge in descriptor
    .body
    .edges
    .iter()
    .filter(|edge| edge.from == selector_id && choice.arm_ids.contains(&edge.id))
  {
    match &edge.condition {
      crate::model::EdgeCondition::Boolean { value } => {
        match resolve_for_each_reference(value, context, item, index, total)? {
          Value::Bool(true) => return Ok((choice.choice_id.clone(), edge.id.clone())),
          Value::Bool(false) => {}
          _ => {
            return Err(RuntimeExecutionError::InvalidConfiguration(format!(
              "for-each choice {:?} condition must resolve to a boolean",
              choice.choice_id
            )))
          }
        }
      }
      crate::model::EdgeCondition::Always => {
        return Ok((choice.choice_id.clone(), edge.id.clone()))
      }
      _ => {
        return Err(RuntimeExecutionError::Stalled(format!(
          "for-each choice {:?} contains an unsupported condition",
          choice.choice_id
        )))
      }
    }
  }
  Err(RuntimeExecutionError::InvalidConfiguration(format!(
    "for-each choice {:?} has no selectable route",
    choice.choice_id
  )))
}

pub(in crate::runtime) fn for_each_selected_branch(
  descriptor: &crate::model::CompiledForEach,
  selector_node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<(String, String), RuntimeExecutionError> {
  let branch_id = selector_node_id
    .strip_prefix("__woml_branch__")
    .and_then(|value| value.strip_suffix("__select"))
    .ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(format!(
        "for-each conditional selector {selector_node_id:?} has no canonical identity"
      ))
    })?;
  for edge in descriptor
    .body
    .edges
    .iter()
    .filter(|edge| edge.from == selector_node_id)
  {
    match &edge.condition {
      crate::model::EdgeCondition::Boolean { value } => {
        let selected = resolve_for_each_reference(value, context, item, index, total)?;
        match selected {
          Value::Bool(true) => return Ok((branch_id.to_string(), edge.id.clone())),
          Value::Bool(false) => {}
          _ => {
            return Err(RuntimeExecutionError::InvalidConfiguration(format!(
              "for-each condition for branch {branch_id:?} must resolve to a boolean"
            )))
          }
        }
      }
      crate::model::EdgeCondition::Always => return Ok((branch_id.to_string(), edge.id.clone())),
      _ => {
        return Err(RuntimeExecutionError::InvalidConfiguration(format!(
          "for-each branch {branch_id:?} uses an unsupported condition"
        )))
      }
    }
  }
  Err(RuntimeExecutionError::InvalidConfiguration(format!(
    "for-each branch {branch_id:?} has no selectable route"
  )))
}

fn for_each_edge_is_active(
  descriptor: &crate::model::CompiledForEach,
  edge: &crate::model::CompiledWorkflowEdge,
  selections: &BTreeMap<String, String>,
) -> bool {
  if let Some(branch_id) = edge.branch_id.as_deref() {
    return selections
      .get(branch_id)
      .is_some_and(|selected| selected == &edge.id);
  }
  if let Some(result_node) = descriptor
    .body
    .nodes
    .iter()
    .find(|node| node.id == edge.to && node.handler == "engine.branch-result")
  {
    if let Some(selected_arm) = selections.get(&result_node.id) {
      if let ValueExpression::Object { fields } = &result_node.inputs {
        if let Some(ValueExpression::ContextReference { path }) = fields.get(selected_arm) {
          if let [root, selected_node, ..] = path.as_slice() {
            if root == "steps" {
              return edge.from == *selected_node;
            }
          }
        }
      }
    }
  }
  for choice in &descriptor.body.choices {
    if choice.arm_ids.contains(&edge.id) {
      return selections
        .get(&choice.choice_id)
        .is_some_and(|selected| selected == &edge.id);
    }
    if let Some(selected) = selections.get(&choice.choice_id) {
      if edge.id == format!("{selected}:join") {
        return true;
      }
      if edge.to == choice.join_node_id && edge.id.ends_with(":join") {
        return false;
      }
    } else if edge.to == choice.join_node_id && edge.id.ends_with(":join") {
      return false;
    }
  }
  true
}

pub(in crate::runtime) fn for_each_ready_nodes(
  descriptor: &crate::model::CompiledForEach,
  completed: &HashSet<String>,
  selections: &BTreeMap<String, String>,
) -> Vec<String> {
  descriptor
    .body
    .nodes
    .iter()
    .filter(|node| !completed.contains(&node.id))
    .filter(|node| {
      let incoming = descriptor
        .body
        .edges
        .iter()
        .filter(|edge| edge.to == node.id)
        .filter(|edge| for_each_edge_is_active(descriptor, edge, selections))
        .collect::<Vec<_>>();
      if incoming.is_empty() {
        descriptor.body.entry_node_ids.contains(&node.id)
      } else {
        incoming.iter().all(|edge| completed.contains(&edge.from))
      }
    })
    .map(|node| node.id.clone())
    .collect()
}

pub(in crate::runtime) fn for_each_module_bindings(
  options: &RuntimeExecutionOptions,
) -> Vec<RuntimeModuleBinding> {
  options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect()
}
