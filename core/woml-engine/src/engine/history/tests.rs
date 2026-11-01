use super::{
  validate_event_history_against_definition, validate_history_with_prefixes, HistoryPrefix,
};
use crate::{fold_events, CompiledWorkflowDefinition, RunEvent, RunEventPayload};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
  label: String,
  model: CompiledWorkflowDefinition,
  definition_hash: String,
  events: Vec<RunEvent>,
  #[serde(default)]
  expected_definition_rejection: bool,
}

fn histories() -> Vec<History> {
  #[derive(Deserialize)]
  struct Recorded {
    histories: Vec<History>,
  }
  let mut histories: Recorded =
    serde_json::from_str(include_str!("tests/profile-histories.json")).unwrap();
  for (label, model, events) in [
    (
      "hello",
      include_str!("../../../../../woml/tests/fixtures/hello.compiled.v1.json"),
      include_str!("../../../../../woml/tests/fixtures/run-events/hello.events.v1.json"),
    ),
    (
      "branch",
      include_str!("../../../../../woml/tests/fixtures/branch.compiled.v2.json"),
      include_str!("../../../../../woml/tests/fixtures/run-events/branch-selected.events.v2.json"),
    ),
    (
      "parallel",
      include_str!("../../../../../woml/tests/fixtures/parallel.compiled.v3.json"),
      include_str!(
        "../../../../../woml/tests/fixtures/run-events/parallel-succeeded.events.v3.json"
      ),
    ),
    (
      "parallel-failure",
      include_str!("../../../../../woml/tests/fixtures/parallel.compiled.v3.json"),
      include_str!(
        "../../../../../woml/tests/fixtures/run-events/parallel-wait-all-failed.events.v3.json"
      ),
    ),
    (
      "retry",
      include_str!("../../../../../woml/tests/fixtures/retry.compiled.v6.json"),
      include_str!("../../../../../woml/tests/fixtures/run-events/retry-success.events.v6.json"),
    ),
    (
      "approval",
      include_str!("../../../../../woml/tests/fixtures/approval.compiled.v4.json"),
      include_str!(
        "../../../../../woml/tests/fixtures/run-events/approval-approved.events.v4.json"
      ),
    ),
  ] {
    let events: Vec<RunEvent> = serde_json::from_str(events).unwrap();
    let RunEventPayload::RunStarted(start) = &events[0].payload else {
      panic!("legacy start");
    };
    histories.histories.push(History {
      label: label.to_string(),
      model: serde_json::from_str(model).unwrap(),
      definition_hash: start.definition_hash.clone(),
      events,
      // Projection-only fixtures omit prerequisite loadField/prepare attempts.
      // Keep their original structural acceptance and definition rejection.
      expected_definition_rejection: matches!(label, "parallel-failure" | "retry"),
    });
  }
  // Also test a complete retry history without rewriting the frozen fixture.
  let retry = histories
    .histories
    .iter()
    .find(|history| history.label == "retry")
    .unwrap();
  let mut complete = History {
    label: "retry-complete".to_string(),
    model: retry.model.clone(),
    definition_hash: retry.definition_hash.clone(),
    events: retry.events.clone(),
    expected_definition_rejection: false,
  };
  let start = complete.events[0].clone();
  let prepare_started = RunEvent {
    event_id: "evt_retry_prepare_start".to_string(),
    sequence: 2,
    payload: RunEventPayload::StepAttemptStarted(crate::StepAttemptStartedData {
      node_id: "prepare".to_string(),
      attempt: 1,
      invocation_id: "inv_retry_prepare".to_string(),
      handler: "runtime.script".to_string(),
      idempotency_key: Some(crate::step_effect_idempotency_key(
        &start.run_id,
        &complete.definition_hash,
        "prepare",
      )),
    }),
    ..start.clone()
  };
  let prepare_succeeded = RunEvent {
    event_id: "evt_retry_prepare_done".to_string(),
    sequence: 3,
    payload: RunEventPayload::StepAttemptSucceeded(crate::StepAttemptSucceededData {
      node_id: "prepare".to_string(),
      attempt: 1,
      invocation_id: "inv_retry_prepare".to_string(),
      output: serde_json::json!({ "name": "World" }),
    }),
    ..start
  };
  complete
    .events
    .splice(1..1, [prepare_started, prepare_succeeded]);
  for (index, event) in complete.events.iter_mut().enumerate() {
    event.sequence = index as u64 + 1;
  }
  histories.histories.push(complete);
  histories.histories
}

fn compare(history: &History, events: &[RunEvent]) -> Result<(), String> {
  let actual =
    validate_event_history_against_definition(&history.model, &history.definition_hash, events);
  let original = validate_history_with_prefixes(
    &history.model,
    &history.definition_hash,
    events,
    HistoryPrefix::replay,
  );
  assert_eq!(
    actual, original,
    "{}: cursor/replay outcome or error drift",
    history.label
  );
  actual
}

#[test]
fn every_projection_prefix_matches_the_original_fold() {
  for history in histories() {
    let mut cursor = HistoryPrefix::new(&history.events);
    for end in 0..=history.events.len() {
      assert_eq!(
        cursor.at(end).unwrap(),
        &fold_events(&history.events[..end]).unwrap(),
        "{} prefix {end}",
        history.label
      );
      // Asking for the same prefix must not replay its events.
      assert_eq!(
        cursor.at(end).unwrap(),
        &fold_events(&history.events[..end]).unwrap()
      );
      assert_eq!(cursor.applied_events(), end);
    }
  }
}

#[test]
fn historical_and_recorded_histories_preserve_every_validation_outcome() {
  for history in histories() {
    let result = compare(&history, &history.events);
    if history.expected_definition_rejection {
      assert!(result.unwrap_err().contains("ready"));
    } else {
      result.unwrap_or_else(|error| panic!("{}: {error}", history.label));
    }
    for end in 0..=history.events.len() {
      // A retry failure without its following schedule can deliberately be an
      // invalid atomic checkpoint. Compare errors, not just successful prefixes.
      let _ = compare(&history, &history.events[..end]);
    }
  }
}

#[test]
fn corrupted_histories_preserve_rejection_and_error_precedence() {
  for history in histories() {
    for index in 0..history.events.len() {
      let mut removed = history.events.clone();
      removed.remove(index);
      for (position, event) in removed.iter_mut().enumerate() {
        event.sequence = position as u64 + 1;
      }
      let _ = compare(&history, &removed);
      let mut duplicate = history.events.clone();
      duplicate.insert(index, duplicate[index].clone());
      for (position, event) in duplicate.iter_mut().enumerate() {
        event.sequence = position as u64 + 1;
      }
      assert!(compare(&history, &duplicate).is_err());
      let mut identity = history.events.clone();
      identity[index].run_id = "run_wrong_history".to_string();
      assert!(compare(&history, &identity).is_err());
      let mut version = history.events.clone();
      version[index].event_schema_version = 999;
      assert!(compare(&history, &version).is_err());
      if index + 1 < history.events.len() {
        let mut swapped = history.events.clone();
        swapped.swap(index, index + 1);
        for (position, event) in swapped.iter_mut().enumerate() {
          event.sequence = position as u64 + 1;
        }
        let _ = compare(&history, &swapped);
      }
    }
  }
}

#[test]
fn structurally_valid_but_unready_steps_still_fail_before_execution() {
  let mut history = histories()
    .into_iter()
    .find(|history| history.label == "hello")
    .unwrap();
  // Exchange whole successful attempt pairs; structural folding still accepts
  // them, but the definition forbids running the second step before the first.
  history.events[1..5].rotate_left(2);
  for (index, event) in history.events.iter_mut().enumerate() {
    event.sequence = index as u64 + 1;
  }
  fold_events(&history.events).unwrap();
  assert!(compare(&history, &history.events)
    .unwrap_err()
    .contains("before it was ready"));
  // An independent structural error must retain priority over that graph error.
  history.events.last_mut().unwrap().sequence = 999;
  let structural_error = fold_events(&history.events).unwrap_err().to_string();
  assert_eq!(compare(&history, &history.events), Err(structural_error));
}

#[test]
fn cursor_rejects_backwards_and_out_of_bounds_without_poisoning_the_next_read() {
  let history = histories().remove(0);
  let mut cursor = HistoryPrefix::new(&history.events);
  cursor.at(1).unwrap();
  assert!(cursor.at(0).is_err());
  assert!(cursor.at(history.events.len() + 1).is_err());
  assert_eq!(cursor.applied_events(), 1);
  assert_eq!(
    cursor.at(history.events.len()).unwrap(),
    &fold_events(&history.events).unwrap()
  );
}

#[test]
fn sequential_thirty_two_validation_prefixes_apply_only_sixty_six_events() {
  let history = histories()
    .into_iter()
    .find(|history| history.label == "sequential-32")
    .unwrap();
  let mut cursor = HistoryPrefix::new(&history.events);
  let mut visits = 0;
  for (index, event) in history.events.iter().enumerate() {
    if matches!(
      event.payload,
      RunEventPayload::StepAttemptStarted(_) | RunEventPayload::RunOutcomeDecided(_)
    ) {
      cursor.at(index).unwrap();
      visits += index;
    }
  }
  assert_eq!(history.events.len(), 68);
  assert_eq!(visits, 1122);
  assert_eq!(cursor.applied_events(), 66);
}
