//! model lock validation.

use super::{
  issue, valid_public_structural_id, CompiledWorkflowDefinition, ModelIssue, ModelIssueCode,
  ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V18;
use std::collections::HashMap;
use std::collections::HashSet;

pub(super) fn inspect_lock_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V18 {
    if workflow.graph.locks.is_some() {
      issues.push(issue(
        ModelIssueCode::InvalidLock,
        "Lock descriptors are available only in compiled Model v18.",
      ));
    }
    return;
  }
  let Some(locks) = workflow.graph.locks.as_deref() else {
    issues.push(issue(
      ModelIssueCode::InvalidLock,
      "Compiled Model v18 requires a locks descriptor collection.",
    ));
    return;
  };
  if locks.len() > 1024 {
    issues.push(issue(
      ModelIssueCode::InvalidLock,
      "Compiled Model v18 supports at most 1024 locks.",
    ));
    return;
  }
  let root_node_ids = workflow
    .graph
    .nodes
    .iter()
    .map(|node| node.id.as_str())
    .collect::<HashSet<_>>();
  let body_node_ids = workflow
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .map(|descriptor| {
      (
        descriptor.for_each_id.as_str(),
        descriptor
          .body
          .nodes
          .iter()
          .map(|node| node.id.as_str())
          .collect::<HashSet<_>>(),
      )
    })
    .collect::<HashMap<_, _>>();
  let nodes = workflow
    .graph
    .nodes
    .iter()
    .chain(
      workflow
        .graph
        .for_each
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|descriptor| descriptor.body.nodes.iter()),
    )
    .map(|node| (node.id.as_str(), node))
    .collect::<HashMap<_, _>>();
  let mut lock_ids = HashSet::new();
  let mut owned_nodes = HashSet::new();
  for lock in locks {
    let valid_control = |id: &str, handler: &str| {
      nodes.get(id).is_some_and(|node| {
        node.handler == handler
          && node.timeout_ms.is_none()
          && node.retry_policy.is_none()
          && node.script_runtime.is_none()
          && matches!(&node.inputs, ValueExpression::Object { fields } if fields.is_empty())
      })
    };
    let key_valid = match &lock.key {
      ValueExpression::Literal { value } => value.as_str().is_some_and(|key| {
        !key.is_empty()
          && key.len() <= 256
          && key.trim() == key
          && !key.chars().any(char::is_control)
      }),
      ValueExpression::Template { parts } => !parts.is_empty() && parts.len() <= 65,
      ValueExpression::ContextReference { path } => !path.is_empty() && path.len() <= 64,
      _ => false,
    };
    let node_ids = [
      &lock.acquire_node_id,
      &lock.release_node_id,
      &lock.result_node_id,
    ];
    let ownership_ids = [
      lock.acquire_node_id.as_str(),
      lock.release_node_id.as_str(),
      lock.result_node_id.as_str(),
      lock.body_entry_node_id.as_str(),
      lock.body_terminal_node_id.as_str(),
    ];
    let one_graph_owns_every_node = ownership_ids.iter().all(|id| root_node_ids.contains(id))
      || body_node_ids
        .values()
        .any(|body| ownership_ids.iter().all(|id| body.contains(id)));
    let valid = valid_public_structural_id(&lock.lock_id)
      && lock_ids.insert(lock.lock_id.as_str())
      && node_ids.iter().all(|id| owned_nodes.insert(id.as_str()))
      && lock.acquire_node_id == format!("__woml_lock__{}__acquire", lock.lock_id)
      && lock.release_node_id == format!("__woml_lock__{}__release", lock.lock_id)
      && lock.result_node_id == lock.lock_id
      && valid_control(&lock.acquire_node_id, "engine.lock-acquire")
      && valid_control(&lock.release_node_id, "engine.lock-release")
      && valid_control(&lock.result_node_id, "engine.lock-result")
      && nodes.contains_key(lock.body_entry_node_id.as_str())
      && nodes.contains_key(lock.body_terminal_node_id.as_str())
      && one_graph_owns_every_node
      && key_valid
      && (1_000..=604_800_000).contains(&lock.acquire_timeout_ms)
      && (5_000..=600_000).contains(&lock.lease_ms);
    if !valid {
      issues.push(issue(
        ModelIssueCode::InvalidLock,
        format!(
          "Lock {:?} does not match the frozen Model v18 descriptor contract.",
          lock.lock_id
        ),
      ));
    }
  }
  let control_count = workflow
    .graph
    .nodes
    .iter()
    .chain(
      workflow
        .graph
        .for_each
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|descriptor| descriptor.body.nodes.iter()),
    )
    .filter(|node| {
      matches!(
        node.handler.as_str(),
        "engine.lock-acquire" | "engine.lock-release" | "engine.lock-result"
      )
    })
    .count();
  if control_count != locks.len() * 3 {
    issues.push(issue(
      ModelIssueCode::InvalidLock,
      "Model v18 locks must describe every lock control node exactly once.",
    ));
  }
}
