//! VSR1 executable specification. This entire module is test-only.
//! The reference deliberately repeats folding; it is NOT the VSR2 optimization.

use super::{histories, History};
use crate::engine::history::{validate_history_with_prefixes, HistoryPrefix};
use crate::projection::ProjectionFoldState;
use crate::{fold_events, CompiledWorkflowDefinition, RunEvent, RunEventPayload, RunProjection};

/// Own complete bookkeeping, but borrow the exact inputs that were checked.
/// No Clone, serialization, mutable accessor or constructor outside this module.
struct ReferenceCheckedHistory<'a> {
  events: &'a [RunEvent],
  definition: &'a CompiledWorkflowDefinition,
  definition_hash: &'a str,
  state: ProjectionFoldState,
}

impl<'a> ReferenceCheckedHistory<'a> {
  fn check(
    definition: &'a CompiledWorkflowDefinition,
    definition_hash: &'a str,
    events: &'a [RunEvent],
  ) -> Result<Self, String> {
    // Independent complete fold first: structural errors retain priority.
    let state = ProjectionFoldState::from_events(events).map_err(|error| error.to_string())?;
    // Preserve the existing definition checks and original per-prefix replay.
    // This executes another full gate and is intentionally NOT a performance path.
    validate_history_with_prefixes(definition, definition_hash, events, HistoryPrefix::replay)?;
    Ok(Self {
      events,
      definition,
      definition_hash,
      state,
    })
  }

  fn projection(&self) -> &RunProjection {
    self.state.projection()
  }

  fn matches_inputs(
    &self,
    definition: &CompiledWorkflowDefinition,
    definition_hash: &str,
    events: &[RunEvent],
  ) -> bool {
    // Identity checks are executable specification guards, not digest-based
    // certificates. Lifetimes keep the originally borrowed inputs immutable.
    std::ptr::eq(self.definition, definition)
      && std::ptr::eq(self.definition_hash, definition_hash)
      && std::ptr::eq(self.events, events)
  }

  fn into_state(self) -> ProjectionFoldState {
    // Moving out the state destroys this definition-checked witness. Its later
    // incremental mutation is governed by existing store/transaction rules.
    self.state
  }
}

fn checked(history: &History, events: &[RunEvent]) -> Result<RunProjection, String> {
  ReferenceCheckedHistory::check(&history.model, &history.definition_hash, events)
    .map(|checked| checked.into_state().into_projection())
}

#[test]
fn reference_matches_all_recorded_and_historical_prefix_outcomes() {
  for history in histories() {
    for end in 0..=history.events.len() {
      let events = &history.events[..end];
      let expected = super::compare(&history, events)
        .and_then(|()| fold_events(events).map_err(|error| error.to_string()));
      assert_eq!(
        checked(&history, events),
        expected,
        "{} at {end}",
        history.label
      );
    }
  }
}

#[test]
fn complete_result_keeps_the_suffix_after_the_last_requested_prefix() {
  let history = histories()
    .into_iter()
    .find(|item| item.label == "sequential-32")
    .unwrap();
  // The last readiness prefix stops at 66; outcome/finalization still belong to
  // the checked result. Never return the prefix cursor's last state instead.
  let proof =
    ReferenceCheckedHistory::check(&history.model, &history.definition_hash, &history.events)
      .unwrap();
  assert_eq!(history.events.len(), 68);
  assert_eq!(proof.projection().last_sequence, 68);
  assert_eq!(proof.projection(), &fold_events(&history.events).unwrap());
  assert_eq!(
    proof.into_state().last_event_id(),
    history.events.last().map(|event| event.event_id.as_str())
  );
}

#[test]
fn consumed_state_retains_append_bookkeeping_not_only_public_projection() {
  for history in histories()
    .into_iter()
    .filter(|item| !item.expected_definition_rejection)
  {
    for end in 0..history.events.len() {
      let events = &history.events[..end];
      let Ok(proof) =
        ReferenceCheckedHistory::check(&history.model, &history.definition_hash, events)
      else {
        continue;
      };
      let mut actual = proof.into_state();
      let mut expected = ProjectionFoldState::from_events(events).unwrap();
      for event in &history.events[end..] {
        assert_eq!(actual.apply(event), expected.apply(event));
        assert_eq!(actual.projection(), expected.projection());
        assert_eq!(actual.last_event_id(), expected.last_event_id());
      }
      // Reapplying an existing event still rejects; extracting a public view
      // alone would lose duplicate-event/attempt/previous-event bookkeeping.
      assert!(actual.apply(&history.events[end]).is_err());
    }
  }
}

#[test]
fn scoped_skips_and_repeated_prefix_requests_do_not_omit_events() {
  let history = histories()
    .into_iter()
    .find(|item| item.label == "foreach-8")
    .unwrap();
  assert!(history.events.iter().any(|event| event.iteration.is_some()));
  let mut cursor = HistoryPrefix::new(&history.events);
  for end in 0..=history.events.len() {
    cursor.at(end).unwrap();
    cursor.at(end).unwrap();
    assert_eq!(cursor.applied_events(), end);
    assert_eq!(
      cursor.at(end).unwrap(),
      &fold_events(&history.events[..end]).unwrap()
    );
  }
  assert_eq!(
    checked(&history, &history.events).unwrap(),
    fold_events(&history.events).unwrap()
  );
}

#[test]
fn later_structural_corruption_wins_over_earlier_graph_failure() {
  let mut history = histories()
    .into_iter()
    .find(|item| item.label == "hello")
    .unwrap();
  history.events[1..5].rotate_left(2);
  for (index, event) in history.events.iter_mut().enumerate() {
    event.sequence = index as u64 + 1;
  }
  assert!(checked(&history, &history.events)
    .unwrap_err()
    .contains("before it was ready"));
  history.events.last_mut().unwrap().sequence = 999;
  assert_eq!(
    checked(&history, &history.events),
    Err(fold_events(&history.events).unwrap_err().to_string())
  );
}

#[test]
fn version_identity_and_definition_substitution_preserve_reference_errors() {
  let history = histories()
    .into_iter()
    .find(|item| item.label == "hello")
    .unwrap();
  for index in 0..history.events.len() {
    let mut events = history.events.clone();
    events[index].event_schema_version = 999;
    assert_eq!(
      checked(&history, &events).map(|_| ()),
      super::compare(&history, &events)
    );
    assert!(checked(&history, &events).is_err());
    events = history.events.clone();
    events[index].run_id = "run_substituted".to_string();
    assert!(checked(&history, &events).is_err());
  }
  let wrong_hash = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
  let expected = validate_history_with_prefixes(
    &history.model,
    wrong_hash,
    &history.events,
    HistoryPrefix::replay,
  );
  let actual =
    ReferenceCheckedHistory::check(&history.model, wrong_hash, &history.events).map(|_| ());
  assert_eq!(actual, expected);
  assert!(actual.is_err());
}

#[test]
fn proof_is_bound_to_original_inputs_not_equal_count_tail_or_serialization() {
  let history = histories()
    .into_iter()
    .find(|item| item.label == "hello")
    .unwrap();
  let proof =
    ReferenceCheckedHistory::check(&history.model, &history.definition_hash, &history.events)
      .unwrap();
  assert!(proof.matches_inputs(&history.model, &history.definition_hash, &history.events));
  let other_definition = history.model.clone();
  let other_hash = history.definition_hash.clone();
  let mut other_events = history.events.clone();
  assert!(!proof.matches_inputs(&other_definition, &history.definition_hash, &history.events));
  assert!(!proof.matches_inputs(&history.model, &other_hash, &history.events));
  assert!(!proof.matches_inputs(&history.model, &history.definition_hash, &other_events));
  assert!(!proof.matches_inputs(
    &history.model,
    &history.definition_hash,
    &history.events[..history.events.len() - 1]
  ));
  if let RunEventPayload::StepAttemptSucceeded(data) = &mut other_events[2].payload {
    data.output = serde_json::json!({"changed": true});
  } else {
    panic!("expected completed first step")
  }
  assert_eq!(other_events.len(), history.events.len());
  assert_eq!(other_events.last(), history.events.last());
  assert!(!proof.matches_inputs(&history.model, &history.definition_hash, &other_events));
  // This guard tests the new witness, NOT arbitrary earlier-event tampering
  // detection by the existing count/tail-based durable cache.
}

#[test]
fn empty_history_is_structurally_empty_not_a_started_run_or_model_certificate() {
  let history = histories().remove(0);
  let proof =
    ReferenceCheckedHistory::check(&history.model, &history.definition_hash, &[]).unwrap();
  assert_eq!(proof.projection(), &RunProjection::default());
  assert_eq!(proof.into_state().last_event_id(), None);
  // Full model validation remains an existing caller precondition: this
  // history checker must not claim to validate a graph it never examines.
  let mut invalid_model = history.model.clone();
  invalid_model.schema_version = 999;
  assert!(ReferenceCheckedHistory::check(&invalid_model, &history.definition_hash, &[]).is_ok());
}
