//! One exact, completely checked history. Never a cache key or effect authority.

use crate::performance::PerformanceSpan;
use crate::projection::ProjectionFoldState;
use crate::{CompiledWorkflowDefinition, RunEvent, RunProjection};

/// Construction stays inside the validator. The input borrows bind the owned
/// full state to its immutable source, not an equal count, tail, or public view.
/// Model validity remains the existing definition-loader/engine precondition.
#[must_use]
pub(crate) struct ValidatedHistory<'a> {
  _events: &'a [RunEvent],
  _definition: &'a CompiledWorkflowDefinition,
  _definition_hash: &'a str,
  state: ProjectionFoldState,
}

impl ValidatedHistory<'_> {
  pub(crate) fn projection(&self) -> &RunProjection {
    self.state.projection()
  }

  /// Consumes the witness. Future appends still need their existing checks.
  pub(crate) fn into_state(self) -> ProjectionFoldState {
    self.state
  }

  pub(crate) fn into_projection(self) -> RunProjection {
    self.into_state().into_projection()
  }

  #[cfg(test)]
  pub(super) fn matches_inputs(
    &self,
    definition: &CompiledWorkflowDefinition,
    hash: &str,
    events: &[RunEvent],
  ) -> bool {
    std::ptr::eq(self._definition, definition)
      && std::ptr::eq(self._definition_hash, hash)
      && std::ptr::eq(self._events, events)
  }
}

pub(super) fn check<'a>(
  definition: &'a CompiledWorkflowDefinition,
  hash: &'a str,
  events: &'a [RunEvent],
) -> Result<ValidatedHistory<'a>, String> {
  // Preserve the former fold_events span and structural-error precedence, but
  // keep ALL fold bookkeeping instead of discarding it into a public view.
  let state = {
    let mut span = PerformanceSpan::new("runtime", "runtime.fold_events");
    span.count("events", events.len());
    if let Some(event) = events.first() {
      span.run_id(event.run_id.clone());
    }
    let result = ProjectionFoldState::from_events(events);
    if result.is_ok() {
      span.succeed();
    }
    result.map_err(|error| error.to_string())?
  };
  super::validate_history_definition_rules(
    definition,
    hash,
    events,
    super::HistoryPrefix::new,
    super::ReadinessGraph::new,
  )?;
  Ok(ValidatedHistory {
    _events: events,
    _definition: definition,
    _definition_hash: hash,
    state,
  })
}
