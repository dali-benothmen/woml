//! Borrowed, immutable query preparation. Not a validation or authority cache.

use crate::model::{CompiledWorkflowEdge, CompiledWorkflowNode};
use crate::projection::AttemptProjection;
use crate::{CompiledWorkflowDefinition, RunProjection};
use std::collections::HashMap;

/// Bound by borrowing to one exact definition, including synthetic scoped graphs.
/// Edge vectors retain source order; no query result or permission is retained.
pub(in crate::engine) struct ReadinessGraph<'a> {
  pub(super) workflow: &'a CompiledWorkflowDefinition,
  nodes: HashMap<&'a str, &'a CompiledWorkflowNode>,
  incoming: HashMap<&'a str, Vec<&'a CompiledWorkflowEdge>>,
  outgoing: HashMap<&'a str, Vec<&'a CompiledWorkflowEdge>>,
  #[cfg(test)]
  pub(super) reference: bool,
}

impl<'a> ReadinessGraph<'a> {
  pub(in crate::engine) fn new(workflow: &'a CompiledWorkflowDefinition) -> Self {
    let mut nodes = HashMap::with_capacity(workflow.graph.nodes.len());
    for node in &workflow.graph.nodes {
      // Match `node()`'s first occurrence even for a caller's invalid graph.
      nodes.entry(node.id.as_str()).or_insert(node);
    }
    let mut incoming: HashMap<&str, Vec<_>> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<_>> = HashMap::new();
    for edge in &workflow.graph.edges {
      incoming.entry(edge.to.as_str()).or_default().push(edge);
      outgoing.entry(edge.from.as_str()).or_default().push(edge);
    }
    Self {
      workflow,
      nodes,
      incoming,
      outgoing,
      #[cfg(test)]
      reference: false,
    }
  }

  pub(super) fn node(&self, id: &str) -> Option<&'a CompiledWorkflowNode> {
    self.nodes.get(id).copied()
  }

  pub(super) fn incoming(&self, id: &str) -> &[&'a CompiledWorkflowEdge] {
    self.incoming.get(id).map(Vec::as_slice).unwrap_or_default()
  }

  pub(super) fn outgoing(&self, id: &str) -> &[&'a CompiledWorkflowEdge] {
    self.outgoing.get(id).map(Vec::as_slice).unwrap_or_default()
  }

  #[cfg(test)]
  pub(in crate::engine) fn reference(workflow: &'a CompiledWorkflowDefinition) -> Self {
    Self {
      workflow,
      nodes: HashMap::new(),
      incoming: HashMap::new(),
      outgoing: HashMap::new(),
      reference: true,
    }
  }
}

/// Rebuilt for the exact borrowed projection used by ONE query. Last vector
/// occurrence wins, just as `latest_attempt`'s reverse scan (not largest number).
/// Does not merge iterations, invocations, histories, or tentative append state.
pub(super) struct LatestAttempts<'a>(HashMap<&'a str, &'a AttemptProjection>);

impl<'a> LatestAttempts<'a> {
  pub(super) fn new(projection: &'a RunProjection) -> Self {
    let mut latest = HashMap::new();
    for attempt in &projection.attempts {
      latest.insert(attempt.identity.node_id.as_str(), attempt);
    }
    Self(latest)
  }

  pub(super) fn get(&self, node_id: &str) -> Option<&'a AttemptProjection> {
    self.0.get(node_id).copied()
  }
}
