//! Exact lock key and compiled protected-scope resolution, including iteration identities.

use crate::event::ForEachIterationScope;
use crate::projection::ForEachIterationStatus;
use crate::CompiledWorkflowDefinition;
use crate::RunProjection;
use std::collections::HashSet;

pub(in crate::durable) fn resolve_lock_execution<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  projection: &RunProjection,
  execution_identity: &str,
) -> Option<(&'a crate::CompiledLock, Option<ForEachIterationScope>)> {
  workflow
    .graph
    .locks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find_map(|lock| {
      if lock.acquire_node_id == execution_identity {
        return Some((lock, None));
      }
      workflow
        .graph
        .for_each
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|loop_| {
          loop_
            .body
            .nodes
            .iter()
            .any(|node| node.id == lock.acquire_node_id)
        })
        .find_map(|loop_| {
          let prefix = format!("{}:", loop_.for_each_id);
          let suffix = format!(":{}", lock.acquire_node_id);
          let index = execution_identity
            .strip_prefix(&prefix)?
            .strip_suffix(&suffix)?
            .parse::<u32>()
            .ok()?;
          projection
            .for_each
            .get(&loop_.for_each_id)
            .and_then(|state| state.iterations.get(&index))
            .filter(|status| matches!(status, ForEachIterationStatus::Started))?;
          Some((
            lock,
            Some(ForEachIterationScope {
              for_each_id: loop_.for_each_id.clone(),
              index,
            }),
          ))
        })
    })
}

pub(in crate::durable) fn valid_lock_key(key: &str) -> bool {
  !key.is_empty()
    && key.len() <= 256
    && key.trim() == key
    && !key.chars().any(char::is_control)
    && !key.split('/').any(str::is_empty)
}

pub(in crate::durable) fn lock_descriptor_for_protected_node<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<&'a crate::CompiledLock> {
  // Corrective actions live outside the forward DAG. Their enclosing scope is
  // the saga, not the original forward step (whose inner lock may be released).
  let node_id = workflow
    .graph
    .sagas
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|saga| {
      saga
        .compensations
        .iter()
        .chain(&saga.cleanup)
        .any(|action| match &action.identity {
          crate::model::CompiledCorrectionIdentity::Static { action_id } => action_id == node_id,
          crate::model::CompiledCorrectionIdentity::ForEachIteration {
            action_id_template, ..
          } => action_id_template
            .split_once("{iterationKey}")
            .is_some_and(|(prefix, suffix)| {
              node_id
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                .is_some_and(|index| index.parse::<u32>().is_ok())
            }),
        })
    })
    .map_or(node_id, |saga| saga.open_node_id.as_str());
  fn contains_node(
    lock: &crate::CompiledLock,
    node_id: &str,
    edges: &[crate::model::CompiledWorkflowEdge],
  ) -> bool {
    let mut pending = vec![lock.body_entry_node_id.as_str()];
    let mut visited = HashSet::new();
    while let Some(candidate) = pending.pop() {
      if !visited.insert(candidate) {
        continue;
      }
      if candidate == node_id {
        return true;
      }
      if candidate == lock.body_terminal_node_id {
        continue;
      }
      pending.extend(
        edges
          .iter()
          .filter(|edge| edge.from == candidate)
          .map(|edge| edge.to.as_str()),
      );
    }
    false
  }
  workflow
    .graph
    .locks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|lock| {
      contains_node(lock, node_id, &workflow.graph.edges)
        || workflow
          .graph
          .for_each
          .as_deref()
          .unwrap_or_default()
          .iter()
          .any(|loop_| contains_node(lock, node_id, &loop_.body.edges))
    })
}
