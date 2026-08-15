use std::path::{Path, PathBuf};

use uuid::Uuid;
use woml_engine::{
  project_run_presentation_v1, recent_run_presentations_from_store_v1,
  run_presentation_from_store_v1, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  PresentationRunStatus, PresentationStepKind, PresentationStepStatus, PresentationTriggerType,
  RunEvent, RunEventPayload, RUN_PRESENTATION_LIST_PROFILE, RUN_PRESENTATION_PROFILE,
};

const HELLO_MODEL: &str = include_str!("../../../woml/tests/fixtures/hello.compiled.v1.json");
const HELLO_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/hello.events.v1.json");
const HELLO_HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";
const RETRY_MODEL: &str = include_str!("../../../woml/tests/fixtures/retry.compiled.v6.json");
const RETRY_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-success.events.v6.json");
const RETRY_EXHAUSTED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-exhausted.events.v6.json");
const RETRY_HASH: &str = "sha256:27606cefeebc5b6d45c965969b621a2f74ae2ebebe2b94edec80d97bfeb8378c";
const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const BRANCH_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/branch-selected.events.v2.json");
const BRANCH_HASH: &str = "sha256:6a9b3aa53e81ae0e95414f80df0192de5ff11489e9b65b1254b69b71a496155a";
const PARALLEL_MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const PARALLEL_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-succeeded.events.v3.json");
const PARALLEL_FAIL_FAST_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-fail-fast.events.v3.json");
const PARALLEL_WAIT_ALL_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-wait-all-failed.events.v3.json");
const PARALLEL_HASH: &str =
  "sha256:d58dfcefdcd6c40db659042c41e17ca6c8d652033f90f120734d5cd95819b45c";
const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const APPROVAL_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-approved.events.v4.json");
const APPROVAL_HASH: &str =
  "sha256:c85377270773c4abb178ba2811109843be53df66c91fedea04bb37d586901aa9";
const APPROVAL_SLACK_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/approval-slack.compiled.v5.json");
const APPROVAL_SLACK_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-slack-approved.events.v5.json");
const APPROVAL_SLACK_HASH: &str =
  "sha256:a02f094f7200f0e7e33bef7de2aba9b52638ac24adb9f017fd292764fbcb6988";
const RETRY_SCHEDULED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-scheduled-recovery.events.v6.json");
const RUNTIME_POLICY_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json");
const RUNTIME_POLICY_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/runtime-policies/events.v11.json");
const RUNTIME_POLICY_HASH: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new() -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-run-presentation-{}.sqlite",
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

fn model(json: &str) -> CompiledWorkflowDefinition {
  serde_json::from_str(json).unwrap()
}

fn events(json: &str) -> Vec<RunEvent> {
  serde_json::from_str(json).unwrap()
}

fn persist_fixture(
  path: &Path,
  workflow: CompiledWorkflowDefinition,
  definition_hash: &str,
  history: &[RunEvent],
) {
  let store = DurableEventStore::open(path).unwrap();
  let mut engine = DurableDagEngine::new(workflow, definition_hash, store).unwrap();
  for event in history {
    match &event.payload {
      RunEventPayload::RunStarted(data) => {
        engine
          .start_run(
            event.event_id.clone(),
            event.run_id.clone(),
            event.occurred_at,
            data.trigger.clone(),
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

#[test]
fn presentation_is_a_deterministic_read_model_across_store_restart() {
  let database = TemporaryDatabase::new();
  let history = events(HELLO_EVENTS);
  persist_fixture(database.path(), model(HELLO_MODEL), HELLO_HASH, &history);

  let first = {
    let store = DurableEventStore::open(database.path()).unwrap();
    run_presentation_from_store_v1(&store, "run_hello_01").unwrap()
  };
  let reopened = DurableEventStore::open(database.path()).unwrap();
  let second = run_presentation_from_store_v1(&reopened, "run_hello_01").unwrap();

  assert_eq!(first, second);
  assert_eq!(first.profile, RUN_PRESENTATION_PROFILE);
  assert_eq!(first.workflow.id, "hello");
  assert_eq!(first.workflow.name.as_deref(), Some("Hello WOML"));
  assert_eq!(first.trigger.id, "start");
  assert_eq!(first.trigger.kind, PresentationTriggerType::Manual);
  assert_eq!(first.status, PresentationRunStatus::Succeeded);
  assert_eq!(first.duration_ms, Some(5));
  assert_eq!(first.steps.len(), 2);
  assert_eq!(first.steps[0].name.as_deref(), Some("Choose greeting name"));
  assert_eq!(first.steps[0].kind, PresentationStepKind::Script);
  assert_eq!(first.steps[0].duration_ms, Some(1));
  assert_eq!(
    first.steps[1].result,
    Some(serde_json::json!({ "message": "Hello World" }))
  );
  assert_eq!(first.summary.succeeded, 2);
  assert_eq!(first.summary.total, 2);

  let list = recent_run_presentations_from_store_v1(&reopened, "hello", 10).unwrap();
  assert_eq!(list.profile, RUN_PRESENTATION_LIST_PROFILE);
  assert_eq!(list.workflow_id, "hello");
  assert_eq!(list.runs, [first]);
  assert!(reopened.has_definition_for_workflow("hello").unwrap());
  assert!(!reopened
    .has_definition_for_workflow("missing-workflow")
    .unwrap());
}

#[test]
fn retries_are_folded_into_one_author_visible_step() {
  let presentation =
    project_run_presentation_v1(&model(RETRY_MODEL), RETRY_HASH, &events(RETRY_EVENTS)).unwrap();
  let retried = presentation
    .steps
    .iter()
    .find(|step| step.id == "greet")
    .unwrap();
  assert_eq!(retried.attempts, 3);
  assert_eq!(retried.status, PresentationStepStatus::Succeeded);
  assert!(retried.failure.is_none());
}

#[test]
fn exhausted_retries_produce_one_failed_step_and_one_final_failure() {
  let presentation = project_run_presentation_v1(
    &model(RETRY_MODEL),
    RETRY_HASH,
    &events(RETRY_EXHAUSTED_EVENTS),
  )
  .unwrap();
  assert_eq!(presentation.status, PresentationRunStatus::Failed);
  let failed = presentation
    .steps
    .iter()
    .find(|step| step.id == "greet")
    .unwrap();
  assert_eq!(failed.attempts, 2);
  assert_eq!(failed.status, PresentationStepStatus::Failed);
  assert!(failed.failure.is_some());
  assert!(presentation.failure.is_some());
  assert!(presentation.result.is_none());
}

#[test]
fn selected_routes_are_explained_and_unselected_work_is_skipped() {
  let presentation =
    project_run_presentation_v1(&model(BRANCH_MODEL), BRANCH_HASH, &events(BRANCH_EVENTS)).unwrap();
  let choice = presentation
    .steps
    .iter()
    .find(|step| step.id == "decision")
    .unwrap();
  assert_eq!(choice.kind, PresentationStepKind::Choose);
  assert_eq!(choice.detail.as_deref(), Some("Selected condition 1."));
  assert_eq!(choice.status, PresentationStepStatus::Succeeded);
  let unselected = presentation
    .steps
    .iter()
    .find(|step| step.id == "acceptContent")
    .unwrap();
  assert_eq!(unselected.status, PresentationStepStatus::Skipped);
}

#[test]
fn parallel_work_is_grouped_without_exposing_engine_nodes() {
  let presentation = project_run_presentation_v1(
    &model(PARALLEL_MODEL),
    PARALLEL_HASH,
    &events(PARALLEL_EVENTS),
  )
  .unwrap();
  assert_eq!(
    presentation
      .steps
      .iter()
      .map(|step| step.id.as_str())
      .collect::<Vec<_>>(),
    [
      "loadField",
      "fieldData",
      "loadWeather",
      "loadSoil",
      "buildReport"
    ]
  );
  let parallel = presentation
    .steps
    .iter()
    .find(|step| step.id == "fieldData")
    .unwrap();
  assert_eq!(parallel.kind, PresentationStepKind::Parallel);
  assert_eq!(parallel.status, PresentationStepStatus::Succeeded);
  assert_eq!(parallel.duration_ms, Some(84));
  assert_eq!(
    parallel.detail.as_deref(),
    Some("2 children · up to 2 at once · wait for all · all children completed.")
  );
  assert_eq!(presentation.summary.total, 5);
  assert_eq!(
    presentation
      .steps
      .iter()
      .find(|step| step.id == "loadWeather")
      .unwrap()
      .depth,
    1
  );
  assert!(!serde_json::to_string(&presentation)
    .unwrap()
    .contains("engine.parallel"));
}

#[test]
fn parallel_failure_policy_and_cancelled_siblings_are_visible() {
  let fail_fast = project_run_presentation_v1(
    &model(PARALLEL_MODEL),
    PARALLEL_HASH,
    &events(PARALLEL_FAIL_FAST_EVENTS),
  )
  .unwrap();
  let group = fail_fast
    .steps
    .iter()
    .find(|step| step.id == "fieldData")
    .unwrap();
  assert_eq!(group.status, PresentationStepStatus::Failed);
  assert_eq!(
    group.failure.as_ref().map(|failure| failure.code.as_str()),
    Some("WOML_PARALLEL_CHILD_FAILED")
  );
  assert!(group
    .detail
    .as_deref()
    .is_some_and(|detail| detail.contains("1 failed, 1 cancelled")));
  assert_eq!(
    fail_fast
      .steps
      .iter()
      .find(|step| step.id == "loadSoil")
      .unwrap()
      .status,
    PresentationStepStatus::Cancelled
  );

  let wait_all = project_run_presentation_v1(
    &model(PARALLEL_MODEL),
    PARALLEL_HASH,
    &events(PARALLEL_WAIT_ALL_EVENTS),
  )
  .unwrap();
  let group = wait_all
    .steps
    .iter()
    .find(|step| step.id == "fieldData")
    .unwrap();
  assert!(group.detail.as_deref().is_some_and(
    |detail| detail.contains("wait for all") && detail.contains("1 failed, 0 cancelled")
  ));
}

#[test]
fn approval_waiting_and_resolution_are_one_author_visible_item() {
  let presentation = project_run_presentation_v1(
    &model(APPROVAL_MODEL),
    APPROVAL_HASH,
    &events(APPROVAL_EVENTS),
  )
  .unwrap();
  let approval = presentation
    .steps
    .iter()
    .find(|step| step.id == "editorApproval")
    .unwrap();
  assert_eq!(approval.kind, PresentationStepKind::Approval);
  assert_eq!(approval.status, PresentationStepStatus::Succeeded);
  assert_eq!(approval.duration_ms, Some(299_997));
  assert_eq!(approval.name.as_deref(), Some("Editorial approval"));
  assert!(approval
    .detail
    .as_deref()
    .is_some_and(|detail| detail.contains("Decision approved by human")));
  assert!(!presentation
    .steps
    .iter()
    .any(|step| step.id.contains("approval__")));
}

#[test]
fn approval_presentation_reports_safe_delivery_and_decision_details() {
  let presentation = project_run_presentation_v1(
    &model(APPROVAL_SLACK_MODEL),
    APPROVAL_SLACK_HASH,
    &events(APPROVAL_SLACK_EVENTS),
  )
  .unwrap();
  let approval = presentation
    .steps
    .iter()
    .find(|step| step.id == "releaseApproval")
    .unwrap();
  let detail = approval.detail.as_deref().unwrap();
  assert!(detail.contains("Decision approved by slack"));
  assert!(detail.contains("Notifications 2/2 delivered via slack"));
  let encoded = serde_json::to_string(&presentation).unwrap();
  assert!(!encoded.contains("providerActorId"));
  assert!(!encoded.contains("U12345678"));
  assert!(!encoded.contains("capability"));
}

#[test]
fn scheduled_retry_is_one_retrying_row_with_the_next_attempt() {
  let presentation = project_run_presentation_v1(
    &model(RETRY_MODEL),
    RETRY_HASH,
    &events(RETRY_SCHEDULED_EVENTS),
  )
  .unwrap();
  let step = presentation
    .steps
    .iter()
    .find(|step| step.id == "greet")
    .unwrap();
  assert_eq!(presentation.status, PresentationRunStatus::Retrying);
  assert_eq!(step.status, PresentationStepStatus::Retrying);
  assert_eq!(step.attempts, 1);
  assert!(step
    .detail
    .as_deref()
    .is_some_and(|detail| detail.contains("Attempt 2 scheduled")));
  assert!(step.failure.is_some());
}

#[test]
fn queued_policy_and_workflow_timeout_are_explained_without_scheduler_logs() {
  let history = events(RUNTIME_POLICY_EVENTS);
  let queued = project_run_presentation_v1(
    &model(RUNTIME_POLICY_MODEL),
    RUNTIME_POLICY_HASH,
    &history[..1],
  )
  .unwrap();
  assert_eq!(queued.status, PresentationRunStatus::Queued);
  let entry = queued
    .steps
    .iter()
    .find(|step| step.id == "processOrder")
    .unwrap();
  assert_eq!(entry.status, PresentationStepStatus::Queued);
  assert_eq!(
    entry.detail.as_deref(),
    Some("Waiting in queue orders for concurrency and rate limit capacity.")
  );

  let timed_out =
    project_run_presentation_v1(&model(RUNTIME_POLICY_MODEL), RUNTIME_POLICY_HASH, &history)
      .unwrap();
  assert_eq!(timed_out.status, PresentationRunStatus::TimedOut);
  assert_eq!(
    timed_out
      .failure
      .as_ref()
      .map(|failure| failure.code.as_str()),
    Some("WOML_WORKFLOW_TIMED_OUT")
  );
}

#[test]
fn results_are_bounded_and_credential_shaped_fields_are_redacted() {
  let mut history = events(HELLO_EVENTS);
  let unsafe_result = serde_json::json!({
    "message": "ok",
    "token": "must-not-leak",
    "nested": { "authorization": "Bearer must-not-leak" },
    "values": (0..100).collect::<Vec<_>>(),
    "long": "x".repeat(2_000)
  });
  for event in &mut history {
    match &mut event.payload {
      RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "b" => {
        data.output = unsafe_result.clone();
      }
      RunEventPayload::RunSucceeded(data) => data.result = unsafe_result.clone(),
      _ => {}
    }
  }
  let presentation =
    project_run_presentation_v1(&model(HELLO_MODEL), HELLO_HASH, &history).unwrap();
  let encoded = serde_json::to_string(&presentation).unwrap();
  assert!(!encoded.contains("must-not-leak"));
  assert!(presentation.result_truncated.is_some());
  assert_eq!(presentation.result.as_ref().unwrap()["token"], "[redacted]");
  assert_eq!(
    presentation.result.as_ref().unwrap()["values"]
      .as_array()
      .unwrap()
      .len(),
    21
  );
}
