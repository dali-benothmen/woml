use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;
use woml_engine::event::StepAttemptFailedData;
use woml_engine::{
  resume_workflow_durable, AttemptFailure, AttemptFailureKind, CompiledWorkflowDefinition,
  DurableEventStore, ParallelGroupOutcome, RunEvent, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/parallel-succeeded.events.v3.json");
const HASH: &str = "sha256:d58dfcefdcd6c40db659042c41e17ca6c8d652033f90f120734d5cd95819b45c";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
}

fn events() -> Vec<RunEvent> {
  serde_json::from_str(EVENTS).unwrap()
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(bun, host))
}

fn options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 3_000)
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new() -> Self {
    Self {
      path: std::env::temp_dir().join(format!("woml-p7-{}.sqlite", Uuid::new_v4())),
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

fn append_prefix(database: &TemporaryDatabase, fixture: &[RunEvent], length: usize) {
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&model(), HASH).unwrap();
  let RunEventPayload::RunStarted(start) = &fixture[0].payload else {
    panic!("fixture must start the run");
  };
  store
    .start_run(
      fixture[0].event_id.clone(),
      fixture[0].run_id.clone(),
      fixture[0].occurred_at,
      start.workflow_id.clone(),
      start.definition_hash.clone(),
      start.trigger.clone(),
    )
    .unwrap();
  for event in &fixture[1..length] {
    store
      .append_payload(
        event.run_id.clone(),
        event.event_id.clone(),
        event.occurred_at,
        event.payload.clone(),
      )
      .unwrap();
  }
}

#[tokio::test]
async fn safe_parallel_crash_boundaries_resume_without_replaying_successes() {
  let Some(host) = host_options() else {
    return;
  };
  let fixture = events();
  for prefix in [3_usize, 4, 8, 9, 12] {
    let database = TemporaryDatabase::new();
    append_prefix(&database, &fixture, prefix);
    let result = resume_workflow_durable(
      database.path().to_path_buf(),
      "run_parallel_01",
      options(host.clone()),
    )
    .await
    .unwrap();

    assert_eq!(
      result.result,
      json!({ "summary": "Weather 22°C, soil 41%" })
    );
    for node_id in ["loadField", "loadWeather", "loadSoil", "buildReport"] {
      assert_eq!(
        result
          .events
          .iter()
          .filter(|event| matches!(
            &event.payload,
            RunEventPayload::StepAttemptSucceeded(data) if data.node_id == node_id
          ))
          .count(),
        1,
        "prefix {prefix} must not replay {node_id}"
      );
    }
    assert_eq!(result.events.last().unwrap().sequence, 12);
  }
}

#[tokio::test]
async fn partial_success_resumes_only_the_pending_parallel_child() {
  let Some(host) = host_options() else {
    return;
  };
  let fixture = events();
  let database = TemporaryDatabase::new();
  append_prefix(&database, &fixture, 5);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .append_payload(
      "run_parallel_01",
      "evt_weather_success_before_crash",
      Utc::now(),
      fixture[7].payload.clone(),
    )
    .unwrap();
  drop(store);

  let result = resume_workflow_durable(
    database.path().to_path_buf(),
    "run_parallel_01",
    options(host),
  )
  .await
  .unwrap();
  assert_eq!(
    result.result,
    json!({ "summary": "Weather 22°C, soil 41%" })
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadWeather"
      ))
      .count(),
    1
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadSoil"
      ))
      .count(),
    1
  );
}

#[tokio::test]
async fn ambiguous_active_parallel_attempt_fails_closed_and_never_replays() {
  let Some(host) = host_options() else {
    return;
  };
  let fixture = events();
  let database = TemporaryDatabase::new();
  append_prefix(&database, &fixture, 5);

  let error = resume_workflow_durable(
    database.path().to_path_buf(),
    "run_parallel_01",
    options(host),
  )
  .await
  .unwrap_err();
  assert!(matches!(
    error,
    RuntimeExecutionError::RunFailed(details)
      if details.failure.kind == AttemptFailureKind::Interrupted
  ));
  let store = DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection("run_parallel_01").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert_eq!(
    store
      .events("run_parallel_01")
      .unwrap()
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::StepAttemptStarted(data) if data.node_id == "loadWeather"
      ))
      .count(),
    1
  );
}

#[tokio::test]
async fn recovery_derives_missing_wait_all_group_and_run_failure_atomically() {
  let Some(host) = host_options() else {
    return;
  };
  let fixture = events();
  let database = TemporaryDatabase::new();
  append_prefix(&database, &fixture, 6);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .append_payload(
      "run_parallel_01",
      "evt_weather_failed_before_crash",
      Utc::now(),
      RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
        node_id: "loadWeather".to_string(),
        attempt: 1,
        invocation_id: "inv_weather_01".to_string(),
        failure: AttemptFailure {
          kind: AttemptFailureKind::ScriptThrew,
          code: AttemptFailureKind::ScriptThrew.code().to_string(),
          message: "weather failed before the runtime recorded the group outcome".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      }),
    )
    .unwrap();
  store
    .append_payload(
      "run_parallel_01",
      "evt_soil_success_before_crash",
      Utc::now(),
      fixture[6].payload.clone(),
    )
    .unwrap();
  drop(store);

  let error = resume_workflow_durable(
    database.path().to_path_buf(),
    "run_parallel_01",
    options(host),
  )
  .await
  .unwrap_err();
  assert!(matches!(
    error,
    RuntimeExecutionError::ParallelFailed(details)
      if details.failed_node_ids == ["loadWeather"]
        && details.cancelled_node_ids.is_empty()
  ));
  let store = DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection("run_parallel_01").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(matches!(
    projection.parallel_groups["fieldData"].status,
    woml_engine::ParallelGroupStatus::Completed {
      outcome: ParallelGroupOutcome::Failed,
      ..
    }
  ));
  let recovered = store.events("run_parallel_01").unwrap();
  assert!(matches!(
    recovered[recovered.len() - 2].payload,
    RunEventPayload::ParallelGroupCompleted(_)
  ));
  assert!(matches!(
    recovered[recovered.len() - 1].payload,
    RunEventPayload::RunFailed(_)
  ));
}

#[tokio::test]
async fn wait_all_resumes_pending_siblings_after_a_recorded_failure_then_fails_the_group() {
  let Some(host) = host_options() else {
    return;
  };
  let fixture = events();
  for (failed_node_id, pending_node_id, started_index) in [
    ("loadWeather", "loadSoil", 4),
    ("loadSoil", "loadWeather", 5),
  ] {
    let database = TemporaryDatabase::new();
    append_prefix(&database, &fixture, 4);
    let mut store = DurableEventStore::open(database.path()).unwrap();
    let RunEventPayload::StepAttemptStarted(started) = &fixture[started_index].payload else {
      panic!("fixture must start the failed child");
    };
    store
      .append_payload(
        "run_parallel_01",
        "evt_child_started_before_crash",
        Utc::now(),
        fixture[started_index].payload.clone(),
      )
      .unwrap();
    store
      .append_payload(
        "run_parallel_01",
        "evt_child_failed_with_sibling_pending",
        Utc::now(),
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: failed_node_id.to_string(),
          attempt: started.attempt,
          invocation_id: started.invocation_id.clone(),
          failure: AttemptFailure {
            kind: AttemptFailureKind::ScriptThrew,
            code: AttemptFailureKind::ScriptThrew.code().to_string(),
            message: "child failed before its sibling was scheduled".to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        }),
      )
      .unwrap();
    drop(store);

    let error = resume_workflow_durable(
      database.path().to_path_buf(),
      "run_parallel_01",
      options(host.clone()),
    )
    .await
    .unwrap_err();
    let RuntimeExecutionError::ParallelFailed(details) = error else {
      panic!("expected the resumed wait-all group to fail with its parallel failure: {error:?}");
    };
    assert_eq!(details.primary_node_id, failed_node_id);
    assert_eq!(details.failed_node_ids, [failed_node_id]);
    assert!(details.cancelled_node_ids.is_empty());
    for node_id in [failed_node_id, pending_node_id] {
      assert_eq!(
        details
          .events
          .iter()
          .filter(|event| matches!(
            &event.payload,
            RunEventPayload::StepAttemptStarted(data) if data.node_id == node_id
          ))
          .count(),
        1,
        "recovery must not replay {node_id}"
      );
    }
    assert_eq!(
      details
        .events
        .iter()
        .filter(|event| matches!(
          &event.payload,
          RunEventPayload::StepAttemptSucceeded(data) if data.node_id == pending_node_id
        ))
        .count(),
      1
    );
    assert!(!details.events.iter().any(|event| matches!(
      &event.payload,
      RunEventPayload::StepAttemptStarted(data) if data.node_id == "buildReport"
    )));
    assert_eq!(
      details
        .events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::ParallelGroupCompleted(_)))
        .count(),
      1
    );
    assert!(matches!(
      &details.events[details.events.len() - 2].payload,
      RunEventPayload::ParallelGroupCompleted(data)
        if data.outcome == ParallelGroupOutcome::Failed
          && data.failed_node_ids == [failed_node_id]
          && data.cancelled_node_ids.is_empty()
    ));
    assert!(matches!(
      details.events.last().unwrap().payload,
      RunEventPayload::RunFailed(_)
    ));

    let store = DurableEventStore::open(database.path()).unwrap();
    store.audit_integrity().unwrap();
    let projection = store.projection("run_parallel_01").unwrap();
    assert_eq!(projection.status, RunStatus::Failed);
    assert!(projection.context.steps.contains_key(pending_node_id));
    assert!(!projection.context.steps.contains_key(failed_node_id));
    assert_eq!(store.events("run_parallel_01").unwrap(), details.events);
    drop(store);

    let repeated = resume_workflow_durable(
      database.path().to_path_buf(),
      "run_parallel_01",
      options(host.clone()),
    )
    .await
    .unwrap_err();
    let RuntimeExecutionError::ParallelFailed(repeated_details) = repeated else {
      panic!("a settled group must retain its parallel failure: {repeated:?}");
    };
    assert_eq!(repeated_details.events, details.events);
  }
}
