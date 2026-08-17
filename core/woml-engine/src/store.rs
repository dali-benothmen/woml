use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::{fold_events, FoldError, RunEvent, RunProjection};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EventStoreError {
  #[error("event ID {0:?} already exists")]
  DuplicateEventId(String),
  #[error(transparent)]
  InvalidHistory(#[from] FoldError),
}

#[derive(Debug, Default)]
pub struct InMemoryEventStore {
  runs: HashMap<String, Vec<RunEvent>>,
  event_ids: HashSet<String>,
}

impl InMemoryEventStore {
  pub fn append(&mut self, event: RunEvent) -> Result<RunProjection, EventStoreError> {
    if self.event_ids.contains(&event.event_id) {
      return Err(EventStoreError::DuplicateEventId(event.event_id));
    }

    let mut candidate = self.runs.get(&event.run_id).cloned().unwrap_or_default();
    candidate.push(event.clone());
    let projection = fold_events(&candidate)?;

    self.event_ids.insert(event.event_id.clone());
    self.runs.insert(event.run_id.clone(), candidate);
    Ok(projection)
  }

  pub fn events(&self, run_id: &str) -> &[RunEvent] {
    self.runs.get(run_id).map(Vec::as_slice).unwrap_or_default()
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, FoldError> {
    fold_events(self.events(run_id))
  }
}
