//! model placement validation.

use super::{issue, CompiledWorkflowDefinition, ModelIssue, ModelIssueCode};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V21;
use std::collections::HashMap;

fn valid_execution_target(target: &str) -> bool {
  if target.is_empty() || target.len() > 128 {
    return false;
  }
  let bytes = target.as_bytes();
  bytes[0].is_ascii_lowercase()
    && bytes
      .last()
      .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    && !target.contains("--")
    && bytes
      .iter()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_execution_capability(capability: &str) -> bool {
  if capability.is_empty() || capability.len() > 128 {
    return false;
  }
  let mut previous_separator = true;
  for byte in capability.bytes() {
    if byte == b'.' || byte == b'-' {
      if previous_separator {
        return false;
      }
      previous_separator = true;
    } else if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
      previous_separator = false;
    } else {
      return false;
    }
  }
  !previous_separator
}

pub(super) fn inspect_execution_placement_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  let mut forward_nodes = HashMap::new();
  let mut nodes = Vec::new();
  for node in &workflow.graph.nodes {
    forward_nodes.insert(node.id.as_str(), node);
    nodes.push(node);
  }
  for descriptor in workflow.graph.for_each.iter().flatten() {
    for node in &descriptor.body.nodes {
      forward_nodes.insert(node.id.as_str(), node);
      nodes.push(node);
    }
  }
  for saga in workflow.graph.sagas.iter().flatten() {
    nodes.extend(saga.compensations.iter().map(|action| &action.node));
    nodes.extend(saga.cleanup.iter().map(|action| &action.node));
  }

  let placed = nodes.iter().filter(|node| node.execution.is_some()).count();
  if workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V21 && placed == 0 {
    issues.push(issue(
      ModelIssueCode::InvalidExecutionPlacement,
      "Compiled Model v21 requires at least one explicit execution placement.",
    ));
  }

  for node in nodes {
    let Some(execution) = &node.execution else {
      continue;
    };
    if workflow.schema_version != COMPILED_MODEL_SCHEMA_VERSION_V21
      || node.handler != "runtime.script"
      || execution.profile_version != 1
      || !valid_execution_target(&execution.target)
      || execution.required_capabilities.is_empty()
      || execution.required_capabilities.len() > 128
      || execution.required_capabilities[0] != "runtime.script"
      || execution
        .required_capabilities
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
      || execution
        .required_capabilities
        .iter()
        .any(|capability| !valid_execution_capability(capability))
    {
      issues.push(issue(
        ModelIssueCode::InvalidExecutionPlacement,
        format!(
          "Node {:?} does not match the Model v21 Execution Placement v1 contract.",
          node.id
        ),
      ));
    }
  }

  for saga in workflow.graph.sagas.iter().flatten() {
    for compensation in &saga.compensations {
      if compensation
        .for_step_id
        .as_deref()
        .and_then(|for_step_id| forward_nodes.get(for_step_id))
        .is_none_or(|forward| {
          compensation
            .node
            .execution
            .as_ref()
            .map(|value| &value.target)
            != forward.execution.as_ref().map(|value| &value.target)
        })
      {
        issues.push(issue(
          ModelIssueCode::InvalidExecutionPlacement,
          format!(
            "Compensation {:?} must inherit the execution placement of forward step {:?}.",
            compensation.action_id, compensation.for_step_id
          ),
        ));
      }
    }
  }
}
