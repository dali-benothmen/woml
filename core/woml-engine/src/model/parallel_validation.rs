//! model parallel validation.

use super::{
  issue, parallel_join_inputs, parallel_start_id, parallel_start_inputs,
  valid_public_structural_id, CompiledWorkflowDefinition, CompiledWorkflowEdge,
  CompiledWorkflowNode, EdgeCondition, ModelIssue, ModelIssueCode,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V17;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V18;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V3;
use std::collections::BTreeMap;
use std::collections::HashMap;

pub(super) fn inspect_parallel_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  let nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let mut groups: BTreeMap<&str, Vec<&CompiledWorkflowEdge>> = BTreeMap::new();
  for edge in &workflow.graph.edges {
    if let Some(parallel_id) = edge.parallel_id.as_deref() {
      groups.entry(parallel_id).or_default().push(edge);
      if edge.branch_id.is_some()
        || edge.approval_id.is_some()
        || !matches!(edge.condition, EdgeCondition::Always)
      {
        issues.push(issue(
          ModelIssueCode::InvalidParallelGroup,
          format!(
            "Parallel edge {:?} must be unconditional and cannot also belong to a branch.",
            edge.id
          ),
        ));
      }
    }
  }

  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V3 && !groups.is_empty() {
    issues.push(issue(
      ModelIssueCode::InvalidParallelGroup,
      "Compiled model v1/v2 cannot contain model-v3 parallel groups.",
    ));
  }

  let mut child_owners: HashMap<&str, &str> = HashMap::new();
  for (parallel_id, edges) in &groups {
    let start_id = format!("__woml_parallel__{parallel_id}__start");
    let start = nodes.get(start_id.as_str()).copied();
    let join = nodes.get(*parallel_id).copied();
    let child_edges: Vec<_> = edges
      .iter()
      .copied()
      .filter(|edge| edge.from == start_id)
      .collect();
    let join_edges: Vec<_> = edges
      .iter()
      .copied()
      .filter(|edge| edge.to.as_str() == *parallel_id)
      .collect();
    let children: Vec<_> = child_edges.iter().map(|edge| edge.to.as_str()).collect();

    let ordered_edges_are_valid = !child_edges.is_empty()
      && join_edges.len() == child_edges.len()
      && edges.len() == child_edges.len() * 2
      && child_edges.iter().enumerate().all(|(index, edge)| {
        edge.id == format!("{parallel_id}:child:{index}")
          && edge.parallel_id.as_deref() == Some(*parallel_id)
          && edge.branch_id.is_none()
          && matches!(edge.condition, EdgeCondition::Always)
          && nodes.get(edge.to.as_str()).is_some_and(|node| {
            node.handler == "runtime.script"
              || workflow.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V17
                && node.handler == "engine.durable-wait"
              || workflow.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V18
                && matches!(
                  node.handler.as_str(),
                  "engine.lock-acquire" | "engine.lock-release" | "engine.lock-result"
                )
          })
      })
      && join_edges.iter().enumerate().all(|(index, edge)| {
        edge.id == format!("{parallel_id}:join:{index}")
          && edge.from == children[index]
          && edge.parallel_id.as_deref() == Some(*parallel_id)
          && edge.branch_id.is_none()
          && matches!(edge.condition, EdgeCondition::Always)
      });
    let start_outgoing: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == start_id)
      .collect();
    let join_incoming: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.to.as_str() == *parallel_id)
      .collect();
    let child_boundaries_are_closed = children.iter().all(|child_id| {
      let incoming: Vec<_> = workflow
        .graph
        .edges
        .iter()
        .filter(|edge| edge.to == *child_id)
        .collect();
      let outgoing: Vec<_> = workflow
        .graph
        .edges
        .iter()
        .filter(|edge| edge.from == *child_id)
        .collect();
      incoming.len() == 1
        && incoming[0].from == start_id
        && incoming[0].parallel_id.as_deref() == Some(*parallel_id)
        && outgoing.len() == 1
        && outgoing[0].to.as_str() == *parallel_id
        && outgoing[0].parallel_id.as_deref() == Some(*parallel_id)
    });
    let boundaries_are_valid = start_outgoing.len() == child_edges.len()
      && start_outgoing
        .iter()
        .all(|edge| edge.parallel_id.as_deref() == Some(*parallel_id))
      && join_incoming.len() == join_edges.len()
      && join_incoming
        .iter()
        .all(|edge| edge.parallel_id.as_deref() == Some(*parallel_id))
      && child_boundaries_are_closed
      && workflow
        .graph
        .edges
        .iter()
        .any(|edge| edge.from.as_str() == *parallel_id);

    let mut duplicate_child = false;
    for child_id in &children {
      if child_owners.insert(child_id, parallel_id).is_some() {
        duplicate_child = true;
      }
    }

    if !valid_public_structural_id(parallel_id)
      || !parallel_start_inputs(start, child_edges.len())
      || !parallel_join_inputs(join)
      || !ordered_edges_are_valid
      || !boundaries_are_valid
      || duplicate_child
    {
      issues.push(issue(
        ModelIssueCode::InvalidParallelGroup,
        format!(
          "Parallel group {parallel_id:?} does not match the frozen start, ordered child, join, policy, and concurrency contract."
        ),
      ));
    }
  }

  for node in &workflow.graph.nodes {
    if node.handler == "engine.parallel-start" {
      if parallel_start_id(&node.id).is_none_or(|parallel_id| !groups.contains_key(parallel_id)) {
        issues.push(issue(
          ModelIssueCode::InvalidParallelGroup,
          format!("Parallel start {:?} has no matching edge group.", node.id),
        ));
      }
    } else if node.handler == "engine.parallel-join" && !groups.contains_key(node.id.as_str()) {
      issues.push(issue(
        ModelIssueCode::InvalidParallelGroup,
        format!("Parallel join {:?} has no matching edge group.", node.id),
      ));
    }
  }
}
