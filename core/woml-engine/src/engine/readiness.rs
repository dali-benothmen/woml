//! Readiness queries over immutable definition indexes and exact projection snapshots.

mod frontier;
mod index;
pub(in crate::engine) use frontier::{changed_node, Frontier};
use index::LatestAttempts;
pub(crate) use index::ReadinessDefinition;
pub(super) use index::ReadinessGraph;
#[cfg(test)]
mod incremental;
#[cfg(test)]
mod investigation;
#[cfg(test)]
mod reference;
#[cfg(test)]
mod reuse_tests;
#[cfg(test)]
mod tests;

use super::history::ReadinessPrefix;
use super::selector_branch_id;
use crate::event::ParallelGroupOutcome;
use crate::{CompiledWorkflowDefinition, ParallelGroupStatus, RunProjection, RunStatus};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

#[cfg(test)]
pub(super) fn record_prefix_attempt_visit() {
  incremental::note(|work| work.prefix_attempt_visits += 1);
}

#[cfg(test)]
pub(super) fn record_frontier_invalidation() {
  incremental::note(|work| work.frontier_invalidations += 1);
}

#[cfg(test)]
pub(super) fn observe_frontier<T>(operation: impl FnOnce() -> T) -> (T, [usize; 7]) {
  let (value, work) = incremental::counted(operation);
  (
    value,
    [
      work.frontier_builds,
      work.frontier_initial_nodes,
      work.frontier_affected_nodes,
      work.frontier_affected_edges,
      work.frontier_queries,
      work.frontier_fallback_queries,
      work.frontier_invalidations,
    ],
  )
}

#[cfg(test)]
pub(super) fn chain_for_frontier_tests(
  nodes: usize,
) -> (CompiledWorkflowDefinition, String, Vec<crate::RunEvent>) {
  investigation::chain(nodes)
}

#[cfg(test)]
pub(super) fn observe_prefix_visits<T>(operation: impl FnOnce() -> T) -> (T, usize) {
  let (value, work) = incremental::counted(operation);
  (value, work.prefix_attempt_visits)
}

#[derive(Clone, Copy)]
enum QueryProjection<'a> {
  Current(&'a RunProjection),
  Prefix(&'a ReadinessPrefix<'a>),
}

impl<'a> QueryProjection<'a> {
  fn projection(self) -> &'a RunProjection {
    match self {
      Self::Current(projection) => projection,
      Self::Prefix(prefix) => prefix.projection(),
    }
  }
  fn latest(self) -> LatestAttempts<'a> {
    match self {
      Self::Current(projection) => LatestAttempts::new(projection),
      Self::Prefix(prefix) => LatestAttempts::from_prefix(prefix),
    }
  }
}

pub(crate) fn ready_node_ids_for_projection(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  projection: &RunProjection,
) -> Result<Vec<String>, String> {
  ready_node_ids_for_projection_at(workflow, definition_hash, projection, Utc::now())
}

pub(crate) fn ready_node_ids_for_projection_at(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  projection: &RunProjection,
  now: DateTime<Utc>,
) -> Result<Vec<String>, String> {
  // Terminal/unbound queries retain their cheap gate before allocating indexes.
  if !readiness_binding(workflow, definition_hash, projection)? {
    return Ok(Vec::new());
  }
  ReadinessGraph::new(workflow).ready_at(definition_hash, projection, now)
}

pub(super) fn readiness_binding(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  projection: &RunProjection,
) -> Result<bool, String> {
  if projection.status != RunStatus::Running {
    return Ok(false);
  }
  let workflow_identity_matches = projection.workflow_id.as_deref()
    == Some(workflow.workflow_id.as_str())
    || (projection
      .event_schema_version
      .is_some_and(|version| version >= crate::RUN_EVENT_SCHEMA_VERSION_V11)
      && projection.workflow_id.is_none());
  if !workflow_identity_matches || projection.definition_hash.as_deref() != Some(definition_hash) {
    return Err("The run is not bound to this compiled workflow definition.".to_string());
  }
  Ok(true)
}

impl ReadinessGraph<'_> {
  pub(super) fn ready_from_prefix(
    &self,
    definition_hash: &str,
    prefix: &ReadinessPrefix<'_>,
  ) -> Result<Vec<String>, String> {
    self.ready_from_prefix_at(definition_hash, prefix, Utc::now())
  }

  pub(super) fn ready_from_prefix_at(
    &self,
    definition_hash: &str,
    prefix: &ReadinessPrefix<'_>,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, String> {
    self.ready_at_view(definition_hash, QueryProjection::Prefix(prefix), now)
  }

  pub(super) fn ready_at(
    &self,
    definition_hash: &str,
    projection: &RunProjection,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, String> {
    self.ready_at_view(definition_hash, QueryProjection::Current(projection), now)
  }

  fn ready_at_view(
    &self,
    definition_hash: &str,
    view: QueryProjection<'_>,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, String> {
    let projection = view.projection();
    #[cfg(test)]
    if self.reference {
      return reference::ready_node_ids_for_projection_at(
        self.workflow,
        definition_hash,
        projection,
        now,
      );
    }
    let workflow = self.workflow;
    if !readiness_binding(workflow, definition_hash, projection)? {
      return Ok(Vec::new());
    }
    if let QueryProjection::Prefix(prefix) = view {
      if let Some(frontier) = prefix.frontier_for(workflow) {
        return Ok(frontier.ready_at(now));
      }
      #[cfg(test)]
      incremental::note(|work| work.frontier_fallback_queries += 1);
    }

    // These are query-local views, not cached branch decisions or retry eligibility.
    let active = self.active_node_ids(projection);
    let latest = view.latest();
    #[cfg(test)]
    incremental::note(|work| work.ready_vectors += 1);
    Ok(
      workflow
        .graph
        .nodes
        .iter()
        .filter(|node| self.candidate_is_ready(node, projection, now, &active, &latest))
        .map(|node| node.id.clone())
        .collect(),
    )
  }

  /// Exact full-list membership without evaluating unrelated candidates or
  /// allocating a ready-result vector. Dynamic query views remain call-local.
  pub(super) fn is_ready_at(
    &self,
    definition_hash: &str,
    projection: &RunProjection,
    now: DateTime<Utc>,
    node_id: &str,
  ) -> Result<bool, String> {
    self.is_ready_at_view(
      definition_hash,
      QueryProjection::Current(projection),
      now,
      node_id,
    )
  }

  pub(super) fn is_ready_from_prefix_at(
    &self,
    definition_hash: &str,
    prefix: &ReadinessPrefix<'_>,
    now: DateTime<Utc>,
    node_id: &str,
  ) -> Result<bool, String> {
    if prefix.status != RunStatus::Running {
      return self.is_ready_at(definition_hash, prefix.projection(), now, node_id);
    }
    self.is_ready_at_view(
      definition_hash,
      QueryProjection::Prefix(prefix),
      now,
      node_id,
    )
  }

  fn is_ready_at_view(
    &self,
    definition_hash: &str,
    view: QueryProjection<'_>,
    now: DateTime<Utc>,
    node_id: &str,
  ) -> Result<bool, String> {
    let projection = view.projection();
    #[cfg(test)]
    if self.reference {
      return reference::ready_node_ids_for_projection_at(
        self.workflow,
        definition_hash,
        projection,
        now,
      )
      .map(|ids| ids.iter().any(|id| id == node_id));
    }
    #[cfg(test)]
    if incremental::full_membership_mode() {
      return self
        .ready_at_view(definition_hash, view, now)
        .map(|ids| ids.iter().any(|id| id == node_id));
    }
    if !readiness_binding(self.workflow, definition_hash, projection)? {
      return Ok(false);
    }
    if let QueryProjection::Prefix(prefix) = view {
      if let Some(frontier) = prefix.frontier_for(self.workflow) {
        return Ok(frontier.contains(node_id, now));
      }
      #[cfg(test)]
      incremental::note(|work| work.frontier_fallback_queries += 1);
    }
    let active = self.active_node_ids(projection);
    let latest = view.latest();
    // Deliberately scan IDs, not just `node(node_id)`: direct malformed inputs
    // can contain duplicates with different handlers. Full-list membership is
    // true if ANY matching source candidate is ready, not only the first one.
    Ok(self.workflow.graph.nodes.iter().any(|node| {
      node.id == node_id && self.candidate_is_ready(node, projection, now, &active, &latest)
    }))
  }

  fn candidate_is_ready(
    &self,
    node: &crate::model::CompiledWorkflowNode,
    projection: &RunProjection,
    now: DateTime<Utc>,
    active: &HashSet<&str>,
    latest: &LatestAttempts<'_>,
  ) -> bool {
    #[cfg(test)]
    incremental::note(|work| work.candidate_checks += 1);
    active.contains(node.id.as_str())
      && !self.complete(node, projection)
      && (node.handler != "engine.workflow-settlement"
        || self.settlement_can_decide(projection, latest))
      && match latest.get(&node.id) {
        None => true,
        Some(attempt)
          if matches!(
            attempt.status,
            crate::projection::AttemptStatus::Started
              | crate::projection::AttemptStatus::Succeeded { .. }
          ) =>
        {
          false
        }
        Some(_) => projection
          .pending_retries
          .get(&node.id)
          .is_some_and(|schedule| schedule.scheduled_at <= now),
      }
      && self
        .incoming(&node.id)
        .filter(|edge| active.contains(edge.from.as_str()) && edge_is_active(edge, projection))
        .all(|edge| {
          self
            .node(&edge.from)
            .is_some_and(|predecessor| self.complete(predecessor, projection))
        })
  }

  fn complete(
    &self,
    node: &crate::model::CompiledWorkflowNode,
    projection: &RunProjection,
  ) -> bool {
    node_is_complete_with_graph(self.workflow, node, projection, Some(self))
  }

  fn settlement_can_decide(&self, projection: &RunProjection, latest: &LatestAttempts<'_>) -> bool {
    let Some(settlement) = self.workflow.graph.settlement.as_ref() else {
      return false;
    };
    if projection
      .context
      .steps
      .contains_key(&settlement.main_result_node_id)
    {
      return true;
    }
    projection.attempts.iter().rev().any(|attempt| {
      let is_final_failure = latest
        .get(&attempt.identity.node_id)
        .is_some_and(|latest| latest.identity == attempt.identity)
        && matches!(
          attempt.status,
          crate::projection::AttemptStatus::Failed { .. }
        )
        && !projection
          .pending_retries
          .contains_key(&attempt.identity.node_id);
      is_final_failure && self.failure_can_inactivate_main_route(&attempt.identity.node_id)
    })
  }

  fn failure_can_inactivate_main_route(&self, failed_node_id: &str) -> bool {
    for fork in self.workflow.graph.forks.as_deref().unwrap_or_default() {
      for branch in &fork.branches {
        let mut visited = HashSet::new();
        let mut pending = VecDeque::from([branch.entry_node_id.as_str()]);
        while let Some(node_id) = pending.pop_front() {
          if !visited.insert(node_id) {
            continue;
          }
          if node_id == failed_node_id {
            return fork.joined_branch_ids.contains(&branch.branch_id);
          }
          if node_id == branch.terminal_node_id {
            continue;
          }
          pending.extend(self.outgoing(node_id).map(|edge| edge.to.as_str()));
        }
      }
    }
    true
  }

  fn active_node_ids<'a>(&'a self, projection: &RunProjection) -> HashSet<&'a str> {
    let mut active = HashSet::new();
    let mut queue: VecDeque<&str> = self
      .workflow
      .graph
      .entry_node_ids
      .iter()
      .map(String::as_str)
      .collect();
    while let Some(node_id) = queue.pop_front() {
      if !active.insert(node_id) {
        continue;
      }
      #[cfg(test)]
      incremental::note(|work| work.active_node_visits += 1);
      for edge in self
        .outgoing(node_id)
        .filter(|edge| edge_is_active(edge, projection))
      {
        queue.push_back(edge.to.as_str());
      }
    }
    active
  }
}

fn edge_is_active(edge: &crate::model::CompiledWorkflowEdge, projection: &RunProjection) -> bool {
  if let Some(approval_id) = edge.approval_id.as_deref() {
    let decision = projection
      .context
      .steps
      .get(approval_id)
      .and_then(Value::as_object)
      .and_then(|output| output.get("decision"))
      .and_then(Value::as_str);
    if edge.id == format!("{approval_id}:approved")
      || edge.id == format!("{approval_id}:approved:join")
    {
      return decision == Some("approved");
    }
    if edge.id == format!("{approval_id}:rejected")
      || edge.id == format!("{approval_id}:rejected:join")
    {
      return decision == Some("rejected");
    }
    return false;
  }
  if let Some((_, selected_arm_id)) = projection
    .choice_selections
    .iter()
    .find(|(choice_id, _)| edge.id.starts_with(&format!("{choice_id}:")))
  {
    return selected_arm_id == &edge.id || edge.id == format!("{selected_arm_id}:join");
  }
  if edge.id.starts_with("__woml_choice__")
    && (edge.id.contains(":when:")
      || edge.id.ends_with(":otherwise")
      || edge.id.contains(":case:")
      || edge.id.ends_with(":default"))
  {
    return false;
  }
  match edge.branch_id.as_deref() {
    Some(branch_id) => projection
      .branch_selections
      .get(branch_id)
      .is_some_and(|arm_id| arm_id == &edge.id),
    None => true,
  }
}

pub(crate) fn node_is_complete(
  workflow: &CompiledWorkflowDefinition,
  node: &crate::model::CompiledWorkflowNode,
  projection: &RunProjection,
) -> bool {
  node_is_complete_with_graph(workflow, node, projection, None)
}

fn node_is_complete_with_graph(
  workflow: &CompiledWorkflowDefinition,
  node: &crate::model::CompiledWorkflowNode,
  projection: &RunProjection,
  graph: Option<&ReadinessGraph<'_>>,
) -> bool {
  if node.handler == "engine.saga-open" {
    return workflow
      .saga_for_control_node(&node.id)
      .is_some_and(|saga| projection.sagas.contains_key(&saga.saga_id));
  }
  if node.handler == "engine.saga-commit" {
    return workflow
      .saga_for_control_node(&node.id)
      .and_then(|saga| projection.sagas.get(&saga.saga_id))
      .is_some_and(|saga| saga.phase == crate::SagaPhase::CorrectionCompleted);
  }
  if node.handler == "engine.saga-result" {
    return projection.context.steps.contains_key(&node.id);
  }
  if node.handler == "engine.for-each-open" {
    return workflow
      .graph
      .for_each
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|descriptor| descriptor.open_node_id == node.id)
      .and_then(|descriptor| projection.for_each.get(&descriptor.for_each_id))
      .is_some_and(|loop_state| {
        matches!(
          loop_state.status,
          crate::projection::ForEachStatus::Succeeded
            | crate::projection::ForEachStatus::Failed
            | crate::projection::ForEachStatus::Cancelled
        )
      });
  }
  if node.handler == "engine.for-each-result" {
    return projection.context.steps.contains_key(&node.id);
  }
  if node.handler == "engine.choice-select" {
    return workflow
      .graph
      .choices
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|choice| choice.selector_node_id == node.id)
      .is_some_and(|choice| projection.choice_selections.contains_key(&choice.choice_id));
  }
  if node.handler == "engine.choice-join" {
    return workflow
      .graph
      .choices
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|choice| choice.join_node_id == node.id)
      .and_then(|choice| {
        let selected = projection.choice_selections.get(&choice.choice_id)?;
        let matches = |edge: &&crate::model::CompiledWorkflowEdge| {
          edge.to == node.id && (edge.id == format!("{selected}:join") || edge.id == *selected)
        };
        match graph {
          Some(graph) => graph.incoming(&node.id).find(matches),
          None => workflow.graph.edges.iter().find(matches),
        }
      })
      .and_then(|edge| match graph {
        Some(graph) => graph.node(&edge.from),
        None => workflow.node(&edge.from),
      })
      .is_some_and(|predecessor| {
        node_is_complete_with_graph(workflow, predecessor, projection, graph)
      });
  }
  if node.handler == "engine.fork-open" {
    return workflow
      .graph
      .forks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|fork| fork.open_node_id == node.id)
      .is_some_and(|fork| projection.forks.contains_key(&fork.fork_id));
  }
  if node.handler == "engine.fork-branch-terminal" {
    return workflow
      .graph
      .forks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find_map(|fork| {
        fork
          .branches
          .iter()
          .find(|branch| branch.terminal_node_id == node.id)
          .map(|branch| (fork, branch))
      })
      .is_some_and(|(fork, branch)| {
        projection
          .forks
          .get(&fork.fork_id)
          .and_then(|projected| projected.branches.get(&branch.branch_id))
          .is_some()
      });
  }
  if node.handler == "engine.fork-join" {
    return workflow
      .graph
      .forks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|fork| fork.join_node_id == node.id)
      .and_then(|fork| projection.forks.get(&fork.fork_id))
      .is_some_and(|fork| fork.join_status == crate::projection::ForkJoinStatus::Succeeded);
  }
  if node.handler == "engine.branch-select" {
    return selector_branch_id(&node.id)
      .is_some_and(|branch_id| projection.branch_selections.contains_key(branch_id));
  }
  if node.handler == "engine.parallel-start" {
    return node
      .id
      .strip_prefix("__woml_parallel__")
      .and_then(|id| id.strip_suffix("__start"))
      .is_some_and(|parallel_id| projection.parallel_groups.contains_key(parallel_id));
  }
  if node.handler == "engine.parallel-join" {
    return projection
      .parallel_groups
      .get(&node.id)
      .is_some_and(|group| {
        matches!(
          group.status,
          ParallelGroupStatus::Completed {
            outcome: ParallelGroupOutcome::Succeeded,
            ..
          }
        )
      });
  }
  if node.handler == "engine.approval-wait" {
    return projection.context.steps.contains_key(&node.id);
  }
  if node.handler == "engine.approval-join" {
    let complete = |edge: &crate::model::CompiledWorkflowEdge| {
      edge_is_active(edge, projection)
        && match graph {
          Some(graph) => graph.node(&edge.from),
          None => workflow.node(&edge.from),
        }
        .is_some_and(|predecessor| {
          node_is_complete_with_graph(workflow, predecessor, projection, graph)
        })
    };
    return match graph {
      Some(graph) => graph.incoming(&node.id).any(complete),
      None => workflow
        .graph
        .edges
        .iter()
        .filter(|edge| edge.to == node.id)
        .any(complete),
    };
  }
  if node.handler == "engine.lock-acquire" {
    return workflow
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.acquire_node_id == node.id)
      .and_then(|lock| projection.locks.get(&lock.lock_id))
      .is_some_and(|lock| {
        matches!(
          lock.status,
          crate::LockStatus::Held | crate::LockStatus::Released
        )
      });
  }
  if node.handler == "engine.lock-release" {
    return workflow
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.release_node_id == node.id)
      .and_then(|lock| projection.locks.get(&lock.lock_id))
      .is_some_and(|lock| lock.status == crate::LockStatus::Released);
  }
  if node.handler == "engine.lock-result" {
    return projection.context.steps.contains_key(&node.id);
  }
  projection.context.steps.contains_key(&node.id)
}
