//! Call-local, monotone replay of one immutable history. Never a durable cache.

use crate::performance::PerformanceSpan;
use crate::projection::ProjectionFoldState;
use crate::{RunEvent, RunProjection};

pub(super) struct HistoryPrefix<'a> {
  events: &'a [RunEvent],
  state: ProjectionFoldState,
  next: usize,
  requests: usize,
  span: PerformanceSpan,
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
      #[cfg(test)]
      replay: false,
      #[cfg(test)]
      replayed: None,
    }
  }

  pub(super) fn at(&mut self, end: usize) -> Result<&RunProjection, String> {
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
      self
        .state
        .apply(&self.events[self.next])
        .map_err(|error| error.to_string())?;
      self.next += 1;
    }
    self.span.count("prefix_event_visits", self.next);
    Ok(self.state.projection())
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
