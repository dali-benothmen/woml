use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::Map;
use uuid::Uuid;
use woml_engine::event::{BranchSelectedData, RunStartedData};
use woml_engine::{
  fold_events, BranchFailure, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  InMemoryDagEngine, RunEvent, RunEventPayload, RunFailedData, RunFailedDataV2, RunFailure,
  RunStatus,
};

const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const BRANCH_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/branch-selected.events.v2.json");
const BRANCH_NOT_BOOLEAN: &str =
  include_str!("../../../woml/tests/fixtures/run-events/branch-test-not-boolean.event.v2.json");
const REFERENCE_NOT_AVAILABLE: &str =
  include_str!("../../../woml/tests/fixtures/run-events/reference-not-available.event.v2.json");
const BRANCH_HASH: &str = "sha256:6a9b3aa53e81ae0e95414f80df0192de5ff11489e9b65b1254b69b71a496155a";

fn branch_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(BRANCH_MODEL).unwrap()
}

fn branch_events() -> Vec<RunEvent> {
  serde_json::from_str(BRANCH_EVENTS).unwrap()
}

fn engine_with_ready_selector() -> InMemoryDagEngine {
  let events = branch_events();
  let mut engine = InMemoryDagEngine::new_for_event_history(branch_model(), BRANCH_HASH).unwrap();
  for event in events.into_iter().take(3) {
    engine.append_event(event).unwrap();
  }
  engine
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-b3-{label}-{}.sqlite",
        Uuid::new_v4().simple()
      )),
    }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

#[test]
fn frozen_v2_history_folds_selection_and_context_from_events_alone() {
  let events = branch_events();
  for event in &events {
    event.validate().unwrap();
  }

  let projection = fold_events(&events).unwrap();
  assert_eq!(projection.event_schema_version, Some(2));
  assert_eq!(projection.status, RunStatus::Succeeded);
  assert_eq!(
    projection
      .branch_selections
      .get("decision")
      .map(String::as_str),
    Some("decision:when:0")
  );
  assert!(projection.context.steps.contains_key("reviewContent"));
  assert!(!projection.context.steps.contains_key("acceptContent"));
  assert_eq!(
    projection.result,
    Some(serde_json::json!({ "message": "Final status: reviewed" }))
  );
}

#[test]
fn v2_branch_failure_fixtures_are_a_separate_failure_scope() {
  for fixture in [BRANCH_NOT_BOOLEAN, REFERENCE_NOT_AVAILABLE] {
    let failure_event: RunEvent = serde_json::from_str(fixture).unwrap();
    failure_event.validate().unwrap();

    let mut prefix = branch_events().into_iter().take(3).collect::<Vec<_>>();
    for event in &mut prefix {
      event.run_id = failure_event.run_id.clone();
    }
    prefix.push(failure_event.clone());
    let projection = fold_events(&prefix).unwrap();
    assert!(matches!(projection.failure, Some(RunFailure::Branch(_))));
    assert!(matches!(
      failure_event.payload,
      RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Branch { .. }))
    ));
  }

  let event: RunEvent = serde_json::from_str(BRANCH_NOT_BOOLEAN).unwrap();
  let RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Branch { failure, .. })) =
    event.payload
  else {
    panic!("expected branch-scoped failure");
  };
  assert!(matches!(
    failure,
    BranchFailure::BranchTestNotBoolean { .. }
  ));
}

#[test]
fn one_run_never_mixes_event_schema_versions() {
  let mut events = branch_events();
  events[1].event_schema_version = 1;
  let error = fold_events(&events[..2]).unwrap_err();
  assert!(error.to_string().contains("mixes event schema versions"));
}

#[test]
fn branch_selection_requires_a_ready_known_selector_and_known_arm() {
  let events = branch_events();
  let mut too_early =
    InMemoryDagEngine::new_for_event_history(branch_model(), BRANCH_HASH).unwrap();
  too_early.append_event(events[0].clone()).unwrap();
  let mut selection = events[3].clone();
  selection.sequence = 2;
  assert!(too_early
    .append_event(selection)
    .unwrap_err()
    .to_string()
    .contains("not ready"));

  let mut unknown_branch = engine_with_ready_selector();
  let mut selection = events[3].clone();
  let RunEventPayload::BranchSelected(data) = &mut selection.payload else {
    panic!("expected branch selection");
  };
  data.branch_id = "otherDecision".to_string();
  data.arm_id = "otherDecision:when:0".to_string();
  assert!(unknown_branch
    .append_event(selection)
    .unwrap_err()
    .to_string()
    .contains("unknown branch"));

  let mut unknown_arm = engine_with_ready_selector();
  let mut selection = events[3].clone();
  let RunEventPayload::BranchSelected(data) = &mut selection.payload else {
    panic!("expected branch selection");
  };
  data.arm_id = "decision:when:9".to_string();
  assert!(unknown_arm
    .append_event(selection)
    .unwrap_err()
    .to_string()
    .contains("not selectable"));
}

#[test]
fn a_recorded_selection_rejects_both_duplicate_and_different_arms() {
  let events = branch_events();
  let mut engine = engine_with_ready_selector();
  engine.append_event(events[3].clone()).unwrap();

  for (event_id, arm_id) in [
    ("evt_branch_duplicate", "decision:when:0"),
    ("evt_branch_changed", "decision:otherwise"),
  ] {
    let mut duplicate = events[3].clone();
    duplicate.event_id = event_id.to_string();
    duplicate.sequence = 5;
    duplicate.payload = RunEventPayload::BranchSelected(BranchSelectedData {
      branch_id: "decision".to_string(),
      arm_id: arm_id.to_string(),
    });
    assert!(engine
      .append_event(duplicate)
      .unwrap_err()
      .to_string()
      .contains("immutable selection"));
  }
  assert_eq!(engine.events("run_branch_01").len(), 4);
}

#[test]
fn sqlite_reopen_and_recovery_preserve_the_exact_selected_arm() {
  let database = TemporaryDatabase::new("selection-reopen");
  let events = branch_events();
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine =
      DurableDagEngine::new_for_event_history(branch_model(), BRANCH_HASH, store).unwrap();
    for event in events.iter().take(4) {
      match &event.payload {
        RunEventPayload::RunStarted(RunStartedData { trigger, .. }) => {
          engine
            .start_run(
              event.event_id.clone(),
              event.run_id.clone(),
              event.occurred_at,
              trigger.clone(),
            )
            .unwrap();
        }
        payload => {
          engine
            .append_payload(
              event.event_id.clone(),
              &event.run_id,
              event.occurred_at,
              payload.clone(),
            )
            .unwrap();
        }
      }
    }
  }

  let mut reopened = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    reopened
      .projection("run_branch_01")
      .unwrap()
      .branch_selections
      .get("decision")
      .map(String::as_str),
    Some("decision:when:0")
  );
  let report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(report.resumable_runs, 1);

  let mut resumed = DurableDagEngine::resume(reopened, "run_branch_01").unwrap();
  assert_eq!(
    resumed.ready_node_ids("run_branch_01").unwrap(),
    ["reviewContent"]
  );
  let changed = RunEventPayload::BranchSelected(BranchSelectedData {
    branch_id: "decision".to_string(),
    arm_id: "decision:otherwise".to_string(),
  });
  assert!(resumed
    .append_payload(
      "evt_branch_changed_after_restart",
      "run_branch_01",
      Utc::now(),
      changed,
    )
    .unwrap_err()
    .to_string()
    .contains("immutable selection"));
  assert_eq!(resumed.events("run_branch_01").unwrap().len(), 4);
}

#[test]
fn sqlite_round_trips_the_complete_reviewed_v2_history() {
  let database = TemporaryDatabase::new("complete-history");
  let fixture = branch_events();
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&branch_model(), BRANCH_HASH)
      .unwrap();
    for event in &fixture {
      match &event.payload {
        RunEventPayload::RunStarted(data) => {
          store
            .start_run(
              event.event_id.clone(),
              event.run_id.clone(),
              event.occurred_at,
              data.workflow_id.clone(),
              data.definition_hash.clone(),
              data.trigger.clone(),
            )
            .unwrap();
        }
        payload => {
          store
            .append_payload(
              event.run_id.clone(),
              event.event_id.clone(),
              event.occurred_at,
              payload.clone(),
            )
            .unwrap();
        }
      }
    }
  }

  let reopened = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(reopened.events("run_branch_01").unwrap(), fixture);
  assert_eq!(
    reopened.projection("run_branch_01").unwrap(),
    fold_events(&branch_events()).unwrap()
  );
}

#[test]
fn v2_recovery_keeps_selection_and_uses_attempt_scoped_run_failure() {
  let database = TemporaryDatabase::new("interrupted-selected-route");
  let fixture = branch_events();
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&branch_model(), BRANCH_HASH)
      .unwrap();
    for event in fixture.iter().take(5) {
      match &event.payload {
        RunEventPayload::RunStarted(data) => {
          store
            .start_run(
              event.event_id.clone(),
              event.run_id.clone(),
              event.occurred_at,
              data.workflow_id.clone(),
              data.definition_hash.clone(),
              data.trigger.clone(),
            )
            .unwrap();
        }
        payload => {
          store
            .append_payload(
              event.run_id.clone(),
              event.event_id.clone(),
              event.occurred_at,
              payload.clone(),
            )
            .unwrap();
        }
      }
    }
  }

  let mut reopened = DurableEventStore::open(database.path()).unwrap();
  let report = reopened.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 1);
  let events = reopened.events("run_branch_01").unwrap();
  assert!(events.iter().all(|event| event.event_schema_version == 2));
  assert!(matches!(
    events.last().map(|event| &event.payload),
    Some(RunEventPayload::RunFailed(RunFailedData::V2(
      RunFailedDataV2::Attempt { .. }
    )))
  ));
  let projection = fold_events(&events).unwrap();
  assert_eq!(
    projection
      .branch_selections
      .get("decision")
      .map(String::as_str),
    Some("decision:when:0")
  );
  assert!(matches!(projection.failure, Some(RunFailure::Attempt(_))));
}

#[test]
fn a_model_v2_run_starts_with_event_schema_v2() {
  let mut engine = InMemoryDagEngine::new_for_event_history(branch_model(), BRANCH_HASH).unwrap();
  let projection = engine
    .start_run(
      "evt_branch_start_generated",
      "run_branch_generated",
      Utc::now(),
      Map::new(),
    )
    .unwrap();
  assert_eq!(projection.event_schema_version, Some(2));
  assert_eq!(
    engine.events("run_branch_generated")[0].event_schema_version,
    2
  );
}
