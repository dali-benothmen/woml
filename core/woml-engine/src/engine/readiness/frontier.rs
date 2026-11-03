//! Cursor-local advice for ordinary script DAGs. Never persisted or live authority.

use crate::model::EdgeCondition;
use crate::projection::AttemptStatus;
use crate::{CompiledWorkflowDefinition, RunEvent, RunEventPayload, RunProjection};
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap, VecDeque};

/// Every supported delta is read AFTER structural apply succeeds. All other
/// events permanently invalidate this cursor's frontier, including scoped work.
pub(in crate::engine) fn changed_node(event: &RunEvent) -> Result<Option<&str>, ()> {
  if event.iteration.is_some() {
    return Err(());
  }
  match &event.payload {
    RunEventPayload::RunStarted(_) => Ok(None),
    RunEventPayload::StepAttemptStarted(data) => Ok(Some(&data.node_id)),
    RunEventPayload::StepAttemptSucceeded(data) => Ok(Some(&data.node_id)),
    RunEventPayload::StepAttemptFailed(data) => Ok(Some(&data.node_id)),
    RunEventPayload::StepRetryScheduled(data) => Ok(Some(&data.node_id)),
    // Fail closed for control, authority, managed-effect, outcome, terminal and
    // future event variants. No event is assumed irrelevant from its name.
    _ => Err(()),
  }
}

pub(in crate::engine) struct Frontier<'a> {
  definition: &'a CompiledWorkflowDefinition,
  ids: HashMap<&'a str, usize>,
  outgoing: Vec<Vec<usize>>,
  active: Vec<bool>,
  complete: Vec<bool>,
  remaining: Vec<usize>,
  eligible: Vec<bool>,
  retry_at: Vec<Option<DateTime<Utc>>>,
  candidates: BTreeSet<usize>,
}

impl<'a> Frontier<'a> {
  pub(in crate::engine) fn new(
    definition: &'a CompiledWorkflowDefinition,
    projection: &RunProjection,
    latest: &HashMap<String, usize>,
  ) -> Option<Self> {
    let graph = &definition.graph;
    if !(1..=crate::COMPILED_MODEL_SCHEMA_VERSION_V21).contains(&definition.schema_version)
      || definition.lifecycle.is_some()
      || definition.runtime_policy.is_some()
      || definition.reusable_definitions.is_some()
      || definition.communication.is_some()
      || graph.forks.is_some()
      || graph.choices.is_some()
      || graph.context_visibility.is_some()
      || graph.settlement.is_some()
      || graph.for_each.is_some()
      || graph.waits.is_some()
      || graph.locks.is_some()
      || graph.sagas.is_some()
      || graph
        .nodes
        .iter()
        .any(|node| node.handler != "runtime.script" || node.execution.is_some())
      || graph.edges.iter().any(|edge| {
        !matches!(edge.condition, EdgeCondition::Always)
          || edge.branch_id.is_some()
          || edge.parallel_id.is_some()
          || edge.approval_id.is_some()
          || edge.id.starts_with("__woml_choice__")
      })
    {
      return None;
    }
    let count = graph.nodes.len();
    let ids: HashMap<_, _> = graph
      .nodes
      .iter()
      .enumerate()
      .map(|(i, node)| (node.id.as_str(), i))
      .collect();
    if ids.len() != count {
      return None;
    }
    if latest.keys().any(|id| !ids.contains_key(id.as_str())) {
      return None;
    }
    let mut outgoing = vec![Vec::new(); count];
    let mut indegree = vec![0usize; count];
    for edge in &graph.edges {
      let (&from, &to) = (ids.get(edge.from.as_str())?, ids.get(edge.to.as_str())?);
      outgoing[from].push(to);
      indegree[to] += 1;
    }
    // Reject malformed direct inputs as an optimization scope decision, not a
    // substitute model validation error. Existing validators/oracle keep priority.
    let mut queue: VecDeque<_> = indegree
      .iter()
      .enumerate()
      .filter_map(|(i, n)| (*n == 0).then_some(i))
      .collect();
    let mut visited = 0;
    while let Some(node) = queue.pop_front() {
      visited += 1;
      for &next in &outgoing[node] {
        indegree[next] -= 1;
        if indegree[next] == 0 {
          queue.push_back(next);
        }
      }
    }
    if visited != count {
      return None;
    }
    let mut active = vec![false; count];
    for entry in &graph.entry_node_ids {
      queue.push_back(*ids.get(entry.as_str())?);
    }
    while let Some(node) = queue.pop_front() {
      if active[node] {
        continue;
      }
      active[node] = true;
      queue.extend(outgoing[node].iter().copied());
    }
    let complete: Vec<_> = graph
      .nodes
      .iter()
      .map(|node| projection.context.steps.contains_key(&node.id))
      .collect();
    let mut remaining = vec![0usize; count];
    for (from, edges) in outgoing.iter().enumerate() {
      if active[from] && !complete[from] {
        for &to in edges {
          remaining[to] += 1;
        }
      }
    }
    let mut result = Self {
      definition,
      ids,
      outgoing,
      active,
      complete,
      remaining,
      eligible: vec![false; count],
      retry_at: vec![None; count],
      candidates: BTreeSet::new(),
    };
    for index in 0..count {
      result.refresh(index, projection, latest);
    }
    #[cfg(test)]
    super::incremental::note(|work| {
      work.frontier_builds += 1;
      work.frontier_initial_nodes += count;
    });
    Some(result)
  }

  pub(in crate::engine) fn matches(&self, definition: &CompiledWorkflowDefinition) -> bool {
    std::ptr::eq(self.definition, definition)
  }

  pub(in crate::engine) fn update(
    &mut self,
    node: &str,
    projection: &RunProjection,
    latest: &HashMap<String, usize>,
  ) -> bool {
    let Some(&index) = self.ids.get(node) else {
      return false;
    };
    let complete = projection.context.steps.contains_key(node);
    if complete != self.complete[index] {
      // Completion only appears in the supported fold. A future transition
      // removing output invalidates rather than guessing new predecessor state.
      if !complete {
        return false;
      }
      self.complete[index] = true;
      if self.active[index] {
        for &next in &self.outgoing[index] {
          self.remaining[next] -= 1;
          if self.remaining[next] == 0 && self.active[next] && self.eligible[next] {
            self.candidates.insert(next);
          }
          #[cfg(test)]
          super::incremental::note(|work| work.frontier_affected_edges += 1);
        }
      }
    }
    self.refresh(index, projection, latest);
    true
  }

  fn refresh(&mut self, index: usize, projection: &RunProjection, latest: &HashMap<String, usize>) {
    let id = self.definition.graph.nodes[index].id.as_str();
    self.retry_at[index] = None;
    self.eligible[index] = !self.complete[index]
      && match latest.get(id).map(|i| &projection.attempts[*i].status) {
        None => true,
        Some(AttemptStatus::Started | AttemptStatus::Succeeded { .. }) => false,
        Some(AttemptStatus::Failed { .. }) => {
          self.retry_at[index] = projection
            .pending_retries
            .get(id)
            .map(|retry| retry.scheduled_at);
          self.retry_at[index].is_some()
        }
      };
    if self.active[index] && self.remaining[index] == 0 && self.eligible[index] {
      self.candidates.insert(index);
    } else {
      self.candidates.remove(&index);
    }
    #[cfg(test)]
    super::incremental::note(|work| work.frontier_affected_nodes += 1);
  }

  fn due(&self, index: usize, now: DateTime<Utc>) -> bool {
    self.retry_at[index].is_none_or(|deadline| deadline <= now)
  }

  pub(in crate::engine) fn contains(&self, node: &str, now: DateTime<Utc>) -> bool {
    #[cfg(test)]
    super::incremental::note(|work| work.frontier_queries += 1);
    self
      .ids
      .get(node)
      .is_some_and(|index| self.candidates.contains(index) && self.due(*index, now))
  }

  pub(in crate::engine) fn ready_at(&self, now: DateTime<Utc>) -> Vec<String> {
    #[cfg(test)]
    super::incremental::note(|work| work.frontier_queries += 1);
    self
      .candidates
      .iter()
      .filter(|index| self.due(**index, now))
      .map(|index| self.definition.graph.nodes[*index].id.clone())
      .collect()
  }
}
