//! Definition-owned or borrowed topology. Not a validation or authority cache.

use crate::model::{CompiledWorkflowEdge, CompiledWorkflowNode};
use crate::projection::AttemptProjection;
use crate::{CompiledWorkflowDefinition, RunProjection};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Owns the immutable definition and its lazy, ordinal-only topology together.
/// No self-reference, mutable definition accessor, global cache or eligibility.
pub(crate) struct ReadinessDefinition {
  workflow: CompiledWorkflowDefinition,
  topology: OnceLock<OwnedTopology>,
}

impl std::fmt::Debug for ReadinessDefinition {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.workflow.fmt(formatter)
  }
}

impl std::ops::Deref for ReadinessDefinition {
  type Target = CompiledWorkflowDefinition;
  fn deref(&self) -> &Self::Target {
    &self.workflow
  }
}

impl ReadinessDefinition {
  pub(crate) fn new(workflow: CompiledWorkflowDefinition) -> Self {
    Self {
      workflow,
      topology: OnceLock::new(),
    }
  }

  pub(crate) fn ready_at(
    &self,
    hash: &str,
    projection: &RunProjection,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<Vec<String>, String> {
    // Do not charge terminal or wrongly-bound queries for retained preparation.
    if !super::readiness_binding(&self.workflow, hash, projection)? {
      return Ok(Vec::new());
    }
    self.graph().ready_at(hash, projection, now)
  }

  fn graph(&self) -> ReadinessGraph<'_> {
    let topology = self
      .topology
      .get_or_init(|| OwnedTopology::new(&self.workflow));
    ReadinessGraph {
      workflow: &self.workflow,
      index: GraphIndex::Prepared(topology),
      #[cfg(test)]
      reference: false,
    }
  }
}

struct OwnedTopology {
  nodes: HashMap<String, usize>,
  incoming: HashMap<String, Vec<usize>>,
  outgoing: HashMap<String, Vec<usize>>,
}

impl OwnedTopology {
  fn new(workflow: &CompiledWorkflowDefinition) -> Self {
    #[cfg(test)]
    super::incremental::note(|work| work.graph_preparations += 1);
    let mut nodes = HashMap::with_capacity(workflow.graph.nodes.len());
    for (index, node) in workflow.graph.nodes.iter().enumerate() {
      nodes.entry(node.id.clone()).or_insert(index);
    }
    let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
    let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, edge) in workflow.graph.edges.iter().enumerate() {
      incoming.entry(edge.to.clone()).or_default().push(index);
      outgoing.entry(edge.from.clone()).or_default().push(index);
    }
    Self {
      nodes,
      incoming,
      outgoing,
    }
  }
}

#[derive(Default)]
struct BorrowedTopology<'a> {
  nodes: HashMap<&'a str, &'a CompiledWorkflowNode>,
  incoming: HashMap<&'a str, Vec<&'a CompiledWorkflowEdge>>,
  outgoing: HashMap<&'a str, Vec<&'a CompiledWorkflowEdge>>,
}

enum GraphIndex<'a> {
  Borrowed(BorrowedTopology<'a>),
  Prepared(&'a OwnedTopology),
}

enum Edges<'s, 'w> {
  Borrowed(std::slice::Iter<'s, &'w CompiledWorkflowEdge>),
  Prepared {
    edges: &'w [CompiledWorkflowEdge],
    ordinals: std::slice::Iter<'s, usize>,
  },
}

impl<'w> Iterator for Edges<'_, 'w> {
  type Item = &'w CompiledWorkflowEdge;
  fn next(&mut self) -> Option<Self::Item> {
    match self {
      Self::Borrowed(edges) => edges.next().copied(),
      Self::Prepared { edges, ordinals } => ordinals.next().map(|index| &edges[*index]),
    }
  }
}

/// Bound by borrowing to one exact definition, including synthetic scoped graphs.
/// Edge vectors retain source order; no query result or permission is retained.
pub(in crate::engine) struct ReadinessGraph<'a> {
  pub(super) workflow: &'a CompiledWorkflowDefinition,
  index: GraphIndex<'a>,
  #[cfg(test)]
  pub(super) reference: bool,
}

impl<'a> ReadinessGraph<'a> {
  pub(in crate::engine) fn new(workflow: &'a CompiledWorkflowDefinition) -> Self {
    #[cfg(test)]
    super::incremental::note(|work| work.graph_preparations += 1);
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
      index: GraphIndex::Borrowed(BorrowedTopology {
        nodes,
        incoming,
        outgoing,
      }),
      #[cfg(test)]
      reference: false,
    }
  }

  pub(super) fn node(&self, id: &str) -> Option<&'a CompiledWorkflowNode> {
    match &self.index {
      GraphIndex::Borrowed(index) => index.nodes.get(id).copied(),
      GraphIndex::Prepared(index) => index
        .nodes
        .get(id)
        .map(|ordinal| &self.workflow.graph.nodes[*ordinal]),
    }
  }

  pub(super) fn incoming(&self, id: &str) -> impl Iterator<Item = &'a CompiledWorkflowEdge> + '_ {
    self.edges(id, true)
  }

  pub(super) fn outgoing(&self, id: &str) -> impl Iterator<Item = &'a CompiledWorkflowEdge> + '_ {
    self.edges(id, false)
  }

  fn edges(&self, id: &str, incoming: bool) -> Edges<'_, 'a> {
    match &self.index {
      GraphIndex::Borrowed(index) => {
        let map = if incoming {
          &index.incoming
        } else {
          &index.outgoing
        };
        Edges::Borrowed(map.get(id).map(Vec::as_slice).unwrap_or_default().iter())
      }
      GraphIndex::Prepared(index) => {
        let map = if incoming {
          &index.incoming
        } else {
          &index.outgoing
        };
        Edges::Prepared {
          edges: &self.workflow.graph.edges,
          ordinals: map.get(id).map(Vec::as_slice).unwrap_or_default().iter(),
        }
      }
    }
  }

  #[cfg(test)]
  pub(in crate::engine) fn reference(workflow: &'a CompiledWorkflowDefinition) -> Self {
    Self {
      workflow,
      index: GraphIndex::Borrowed(BorrowedTopology::default()),
      reference: true,
    }
  }
}

/// Uses either a query-local map or the exact cursor's borrowed ordinal view.
/// Last vector occurrence wins, as in `latest_attempt`'s reverse scan (not
/// largest number). No merging iterations, histories or tentative append state.
enum AttemptLookup<'a> {
  Query(HashMap<&'a str, &'a AttemptProjection>),
  Prefix(&'a crate::engine::history::ReadinessPrefix<'a>),
}

pub(super) struct LatestAttempts<'a>(AttemptLookup<'a>);

impl<'a> LatestAttempts<'a> {
  pub(super) fn new(projection: &'a RunProjection) -> Self {
    #[cfg(test)]
    super::incremental::note(|work| work.attempt_visits += projection.attempts.len());
    let mut latest = HashMap::new();
    for attempt in &projection.attempts {
      latest.insert(attempt.identity.node_id.as_str(), attempt);
    }
    Self(AttemptLookup::Query(latest))
  }

  pub(super) fn from_prefix(prefix: &'a crate::engine::history::ReadinessPrefix<'a>) -> Self {
    Self(AttemptLookup::Prefix(prefix))
  }

  pub(super) fn get(&self, node_id: &str) -> Option<&'a AttemptProjection> {
    match &self.0 {
      AttemptLookup::Query(latest) => latest.get(node_id).copied(),
      AttemptLookup::Prefix(prefix) => prefix.latest_attempt(node_id),
    }
  }
}

#[cfg(test)]
mod ownership_tests {
  use super::*;
  use crate::engine::readiness::{incremental::counted, investigation::chain, reference};

  #[test]
  fn owner_prepares_once_after_binding_gate_and_survives_moves() {
    let (model, hash, events) = chain(16);
    let owner = ReadinessDefinition::new(model);
    let mut projection = crate::fold_events(&events[..1]).unwrap();
    projection.status = crate::RunStatus::Cancelled;
    assert!(owner
      .ready_at("wrong", &projection, events[0].occurred_at)
      .unwrap()
      .is_empty());
    assert!(owner.topology.get().is_none());
    projection.status = crate::RunStatus::Running;
    assert!(owner
      .ready_at("wrong", &projection, events[0].occurred_at)
      .is_err());
    assert!(owner.topology.get().is_none());
    let (_, work) = counted(|| {
      for _ in 0..5 {
        assert_eq!(
          owner
            .ready_at(&hash, &projection, events[0].occurred_at)
            .unwrap(),
          vec!["node0"]
        );
      }
    });
    assert_eq!(work.graph_preparations, 1);
    let moved = Box::new(owner);
    let (_, work) = counted(|| {
      for end in 0..=events.len() {
        let projection = crate::fold_events(&events[..end]).unwrap();
        assert_eq!(
          moved.ready_at(&hash, &projection, events[0].occurred_at),
          reference::ready_node_ids_for_projection_at(
            &moved,
            &hash,
            &projection,
            events[0].occurred_at
          )
        );
      }
    });
    assert_eq!(work.graph_preparations, 0);
  }

  #[test]
  fn equal_ids_or_hashes_do_not_share_indexes_across_definitions() {
    let (model, hash, events) = chain(4);
    let projection = crate::fold_events(&events[..1]).unwrap();
    let a = ReadinessDefinition::new(model.clone());
    let mut other = model;
    other.graph.nodes.reverse();
    other.graph.edges.clear();
    other.graph.entry_node_ids = other
      .graph
      .nodes
      .iter()
      .map(|node| node.id.clone())
      .collect();
    let b = ReadinessDefinition::new(other);
    let (_, work) = counted(|| {
      assert_eq!(
        a.ready_at(&hash, &projection, events[0].occurred_at)
          .unwrap(),
        vec!["node0"]
      );
      assert_eq!(
        b.ready_at(&hash, &projection, events[0].occurred_at)
          .unwrap(),
        vec!["node3", "node2", "node1", "node0"]
      );
    });
    assert_eq!(work.graph_preparations, 2);
    assert!(!std::ptr::eq(
      a.topology.get().unwrap(),
      b.topology.get().unwrap()
    ));
  }

  #[test]
  fn ordinal_edges_preserve_duplicates_missing_endpoints_and_membership() {
    let (mut model, hash, events) = chain(4);
    let mut duplicate = model.graph.nodes[0].clone();
    duplicate.handler = "engine.workflow-settlement".into();
    model.graph.nodes.insert(0, duplicate);
    model.graph.edges.push(model.graph.edges[0].clone());
    model.graph.edges[0].from = "missing".into();
    let owner = ReadinessDefinition::new(model);
    let graph = owner.graph();
    assert_eq!(graph.node("node0"), owner.node("node0"));
    for node in &owner.graph.nodes {
      assert_eq!(
        graph.incoming(&node.id).collect::<Vec<_>>(),
        owner
          .graph
          .edges
          .iter()
          .filter(|edge| edge.to == node.id)
          .collect::<Vec<_>>()
      );
      assert_eq!(
        graph.outgoing(&node.id).collect::<Vec<_>>(),
        owner
          .graph
          .edges
          .iter()
          .filter(|edge| edge.from == node.id)
          .collect::<Vec<_>>()
      );
    }
    for end in 0..=events.len() {
      let projection = crate::fold_events(&events[..end]).unwrap();
      let expected = reference::ready_node_ids_for_projection_at(
        &owner,
        &hash,
        &projection,
        events[0].occurred_at,
      );
      assert_eq!(
        graph.ready_at(&hash, &projection, events[0].occurred_at),
        expected
      );
      for node in &owner.graph.nodes {
        assert_eq!(
          graph.is_ready_at(&hash, &projection, events[0].occurred_at, &node.id),
          expected.clone().map(|ids| ids.contains(&node.id))
        );
      }
    }
  }

  #[test]
  fn concurrent_readers_initialize_only_one_topology() {
    let (model, hash, events) = chain(16);
    let owner = ReadinessDefinition::new(model);
    let projection = crate::fold_events(&events[..1]).unwrap();
    std::thread::scope(|scope| {
      let handles = (0..4)
        .map(|_| {
          scope.spawn(|| {
            counted(|| {
              owner
                .ready_at(&hash, &projection, events[0].occurred_at)
                .unwrap()
            })
          })
        })
        .collect::<Vec<_>>();
      let mut preparations = 0;
      for handle in handles {
        let (ready, work) = handle.join().unwrap();
        assert_eq!(ready, vec!["node0"]);
        preparations += work.graph_preparations;
      }
      assert_eq!(preparations, 1);
    });
  }
}
