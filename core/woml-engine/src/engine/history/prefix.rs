//! Call-local, monotone replay of one immutable history. Never a durable cache.

use crate::performance::PerformanceSpan;
use crate::projection::ProjectionFoldState;
use crate::{RunEvent, RunProjection};
use std::collections::HashMap;

/// Only this cursor constructs the projection/ordinal pair. Both borrows end
/// before another event can mutate the cursor. Not a committed-state witness.
pub(in crate::engine) struct ReadinessPrefix<'a> {
  projection: &'a RunProjection,
  latest: Option<&'a HashMap<String, usize>>,
}

impl<'a> ReadinessPrefix<'a> {
  pub(in crate::engine) fn projection(&self) -> &'a RunProjection {
    self.projection
  }
  pub(in crate::engine) fn latest_attempt(
    &self,
    node: &str,
  ) -> Option<&'a crate::projection::AttemptProjection> {
    match self.latest {
      Some(latest) => latest
        .get(node)
        .map(|index| &self.projection.attempts[*index]),
      None => self.projection.latest_attempt(node),
    }
  }
}

impl std::ops::Deref for ReadinessPrefix<'_> {
  type Target = RunProjection;
  fn deref(&self) -> &Self::Target {
    self.projection
  }
}

pub(super) struct HistoryPrefix<'a> {
  events: &'a [RunEvent],
  state: ProjectionFoldState,
  next: usize,
  requests: usize,
  span: PerformanceSpan,
  latest: Option<HashMap<String, usize>>,
  poisoned: Option<String>,
  #[cfg(test)]
  replay: bool,
  #[cfg(test)]
  replayed: Option<RunProjection>,
}

impl<'a> HistoryPrefix<'a> {
  pub(super) fn new(events: &'a [RunEvent]) -> Self {
    let mut span = PerformanceSpan::new("runtime", "runtime.history_prefix_cursor");
    if let Some(event) = events.first() {
      span.run_id(event.run_id.clone());
    }
    span.count("prefix_event_visits", 0);
    span.count("prefix_requests", 0);
    Self {
      events,
      state: ProjectionFoldState::default(),
      next: 0,
      requests: 0,
      span,
      latest: None,
      poisoned: None,
      #[cfg(test)]
      replay: false,
      #[cfg(test)]
      replayed: None,
    }
  }

  pub(super) fn at(&mut self, end: usize) -> Result<&RunProjection, String> {
    if let Some(error) = &self.poisoned {
      return Err(error.clone());
    }
    if end < self.next || end > self.events.len() {
      return Err("History prefix cursor must advance within its bound event slice.".to_string());
    }
    self.requests += 1;
    self.span.count("prefix_requests", self.requests);
    // A test-only oracle uses the original full replay at each request. All
    // definition checks and their ordering are shared with production.
    #[cfg(test)]
    if self.replay {
      self.replayed =
        Some(crate::fold_events(&self.events[..end]).map_err(|error| error.to_string())?);
      self.next = end;
      return Ok(self.replayed.as_ref().expect("replayed prefix"));
    }
    while self.next < end {
      // Consume scoped and non-scoped events alike, even when the validator
      // skipped their match arms. Reuse the existing transition implementation.
      let previous_attempts = self.state.projection().attempts.len();
      if let Err(error) = self.state.apply(&self.events[self.next]) {
        // A rejected apply can already have mutated tentative fold state.
        let error = error.to_string();
        self.latest = None;
        self.state = ProjectionFoldState::default();
        self.poisoned = Some(error.clone());
        return Err(error);
      }
      if self.state.projection().attempts.len() < previous_attempts {
        self.latest = None;
      }
      if let Some(latest) = &mut self.latest {
        for index in previous_attempts..self.state.projection().attempts.len() {
          let node = &self.state.projection().attempts[index].identity.node_id;
          if let Some(ordinal) = latest.get_mut(node.as_str()) {
            *ordinal = index;
          } else {
            latest.insert(node.clone(), index);
          }
          #[cfg(test)]
          crate::engine::readiness::record_prefix_attempt_visit();
        }
      }
      self.next += 1;
    }
    self.span.count("prefix_event_visits", self.next);
    Ok(self.state.projection())
  }

  pub(super) fn query_at(&mut self, end: usize) -> Result<ReadinessPrefix<'_>, String> {
    self.at(end)?;
    #[cfg(test)]
    if self.replay {
      return Ok(ReadinessPrefix {
        projection: self.replayed.as_ref().unwrap(),
        latest: None,
      });
    }
    if self.latest.is_none() && self.state.projection().status == crate::RunStatus::Running {
      let mut latest = HashMap::new();
      for (index, attempt) in self.state.projection().attempts.iter().enumerate() {
        latest.insert(attempt.identity.node_id.clone(), index);
        #[cfg(test)]
        crate::engine::readiness::record_prefix_attempt_visit();
      }
      self.latest = Some(latest);
    }
    Ok(ReadinessPrefix {
      projection: self.state.projection(),
      latest: self.latest.as_ref(),
    })
  }

  pub(super) fn succeed(&mut self) {
    self.span.succeed();
  }

  #[cfg(test)]
  pub(super) fn replay(events: &'a [RunEvent]) -> Self {
    let mut prefixes = Self::new(events);
    prefixes.replay = true;
    prefixes
  }

  #[cfg(test)]
  pub(super) fn applied_events(&self) -> usize {
    self.next
  }
}
