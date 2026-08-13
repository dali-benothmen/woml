use std::path::{Path, PathBuf};

use serde_json::Map;
use uuid::Uuid;
use woml_engine::{
  execute_workflow_durable, resume_workflow_durable, AttemptFailureKind,
  CompiledWorkflowDefinition, DurableEventStore, RunEvent, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");
const DEFINITION_HASH: &str =
  "sha256:8888888888888888888888888888888888888888888888888888888888888888";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
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

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-fork-recovery-{label}-{}.sqlite",
      Uuid::new_v4()
    )))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn restore_prefix(database: &TemporaryDatabase, events: &[RunEvent], end_inclusive: usize) {
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .register_definition(&model(), DEFINITION_HASH)
    .unwrap();
  for event in &events[..=end_inclusive] {
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

fn position(events: &[RunEvent], predicate: impl Fn(&RunEventPayload) -> bool) -> usize {
  events
    .iter()
    .position(|event| predicate(&event.payload))
    .unwrap()
}

fn last_position(events: &[RunEvent], predicate: impl Fn(&RunEventPayload) -> bool) -> usize {
  events
    .iter()
    .rposition(|event| predicate(&event.payload))
    .unwrap()
}

#[tokio::test]
async fn every_safe_fork_boundary_recovers_without_replaying_completed_work() {
  let Some(host) = host_options() else {
    return;
  };
  let baseline_database = TemporaryDatabase::new("baseline");
  let baseline = execute_workflow_durable(
    model(),
    DEFINITION_HASH.to_string(),
    Map::new(),
    options(host.clone()),
    baseline_database.path().to_path_buf(),
  )
  .await
  .unwrap();

  let boundaries = [
    (
      "before-open",
      position(
        &baseline.events,
        |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "prepare"),
      ),
    ),
    (
      "after-open",
      position(&baseline.events, |payload| {
        matches!(payload, RunEventPayload::ForkOpened(_))
      }),
    ),
    (
      "after-branch-effects",
      last_position(
        &baseline.events,
        |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishInstagram" || data.node_id == "publishFacebook"),
      ),
    ),
    (
      "after-branch-settlements",
      last_position(&baseline.events, |payload| {
        matches!(payload, RunEventPayload::ForkBranchSettled(_))
      }),
    ),
    (
      "after-join",
      position(&baseline.events, |payload| {
        matches!(payload, RunEventPayload::ForkJoinSettled(_))
      }),
    ),
    (
      "after-main-continuation",
      position(
        &baseline.events,
        |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish"),
      ),
    ),
    (
      "after-outcome-decision",
      position(&baseline.events, |payload| {
        matches!(payload, RunEventPayload::RunOutcomeDecided(_))
      }),
    ),
  ];

  for (label, boundary) in boundaries {
    let database = TemporaryDatabase::new(label);
    restore_prefix(&database, &baseline.events, boundary);
    let resumed = resume_workflow_durable(
      database.path().to_path_buf(),
      &baseline.run_id,
      options(host.clone()),
    )
    .await
    .unwrap();
    assert_eq!(resumed.result, baseline.result, "boundary {label}");
    for node_id in ["prepare", "publishInstagram", "publishFacebook", "finish"] {
      assert_eq!(
        resumed
          .events
          .iter()
          .filter(|event| matches!(
            &event.payload,
            RunEventPayload::StepAttemptSucceeded(data) if data.node_id == node_id
          ))
          .count(),
        1,
        "boundary {label} replayed {node_id}"
      );
    }
  }
}

#[tokio::test]
async fn an_ambiguous_branch_effect_fails_closed_and_is_never_replayed() {
  let Some(host) = host_options() else {
    return;
  };
  let baseline_database = TemporaryDatabase::new("ambiguous-baseline");
  let baseline = execute_workflow_durable(
    model(),
    DEFINITION_HASH.to_string(),
    Map::new(),
    options(host.clone()),
    baseline_database.path().to_path_buf(),
  )
  .await
  .unwrap();

  for node_id in ["publishInstagram", "publishFacebook"] {
    let boundary = position(
      &baseline.events,
      |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == node_id),
    );
    let database = TemporaryDatabase::new(node_id);
    restore_prefix(&database, &baseline.events, boundary);
    let error = resume_workflow_durable(
      database.path().to_path_buf(),
      &baseline.run_id,
      options(host.clone()),
    )
    .await
    .unwrap_err();
    assert!(
      matches!(
        &error,
        RuntimeExecutionError::RunFailed(details)
          if details.failure.kind == AttemptFailureKind::Interrupted
      ),
      "unexpected recovery error for {node_id}: {error:?}"
    );
    let store = DurableEventStore::open(database.path()).unwrap();
    assert_eq!(
      store.projection(&baseline.run_id).unwrap().status,
      RunStatus::Failed
    );
    assert_eq!(
      store
        .events(&baseline.run_id)
        .unwrap()
        .iter()
        .filter(|event| matches!(
          &event.payload,
          RunEventPayload::StepAttemptStarted(data) if data.node_id == node_id
        ))
        .count(),
      1,
      "ambiguous effect {node_id} was replayed"
    );
  }
}
