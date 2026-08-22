use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::{BackoffPolicy, RetryPolicy};
use woml_engine::{
  create_online_backup, execute_retention, execute_workflow_durable, plan_retention,
  prepare_restored_store, resume_workflow_durable, AttemptFailureKind, CompiledWorkflowDefinition,
  DurableEventStore, RetentionPolicyV1, RunEvent, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const REVIEWED_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/for-each/model.v16.reviewed.json");

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

fn model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  value["graph"]["forEach"][0]["concurrency"] = json!(1);
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

fn retry_model() -> CompiledWorkflowDefinition {
  let mut workflow = model();
  let node = &mut workflow.graph.for_each.as_mut().unwrap()[0].body.nodes[0];
  let woml_engine::model::ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: Value::String(
        "if (attempt.number === 1) throw new Error('retry me'); return { value: context.item, index: context.iteration.index };"
          .to_string(),
      ),
    },
  );
  node.retry_policy = Some(RetryPolicy {
    max_attempts: 2,
    backoff: BackoffPolicy::Fixed { delay_ms: 1 },
  });
  workflow.validate_for_durable_execution().unwrap();
  workflow
}

fn options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 3_000)
}

fn payload(items: Value) -> Map<String, Value> {
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), items);
  trigger
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-for-each-recovery-{label}-{}.sqlite",
      Uuid::new_v4().simple()
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

fn restore_prefix(
  database: &TemporaryDatabase,
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &[RunEvent],
  end_inclusive: usize,
) {
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .register_definition(workflow, definition_hash)
    .unwrap();
  for event in &events[..=end_inclusive] {
    store
      .append_payload_scoped(
        event.run_id.clone(),
        event.event_id.clone(),
        event.occurred_at,
        event.iteration.clone(),
        event.payload.clone(),
      )
      .unwrap();
  }
}

fn position(events: &[RunEvent], predicate: impl Fn(&RunEvent) -> bool) -> usize {
  events.iter().position(predicate).unwrap()
}

#[tokio::test]
async fn restart_continues_pending_indexes_without_replaying_successes() {
  let Some(host) = host_options() else { return };
  let workflow = model();
  let definition_hash = "sha256:6161616161616161616161616161616161616161616161616161616161616161";
  let baseline_database = TemporaryDatabase::new("safe-baseline");
  let baseline = execute_workflow_durable(
    workflow.clone(),
    definition_hash.to_string(),
    payload(json!(["first", "second", "third"])),
    options(host.clone()),
    baseline_database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let boundary = position(&baseline.events, |event| {
    event
      .iteration
      .as_ref()
      .is_some_and(|scope| scope.index == 0)
      && matches!(event.payload, RunEventPayload::ForEachIterationSucceeded(_))
  });
  let database = TemporaryDatabase::new("safe-resume");
  restore_prefix(
    &database,
    &workflow,
    definition_hash,
    &baseline.events,
    boundary,
  );

  let resumed = resume_workflow_durable(
    database.path().to_path_buf(),
    &baseline.run_id,
    options(host),
  )
  .await
  .unwrap();
  assert_eq!(resumed.result, baseline.result);
  for index in 0..3 {
    assert_eq!(
      resumed
        .events
        .iter()
        .filter(|event| {
          event
            .iteration
            .as_ref()
            .is_some_and(|scope| scope.index == index)
            && matches!(event.payload, RunEventPayload::StepAttemptStarted(_))
        })
        .count(),
      1,
      "iteration {index} was replayed"
    );
  }
}

#[tokio::test]
async fn restart_fails_closed_for_an_ambiguous_iteration_attempt() {
  let Some(host) = host_options() else { return };
  let workflow = model();
  let definition_hash = "sha256:6262626262626262626262626262626262626262626262626262626262626262";
  let baseline_database = TemporaryDatabase::new("ambiguous-baseline");
  let baseline = execute_workflow_durable(
    workflow.clone(),
    definition_hash.to_string(),
    payload(json!(["first", "second", "third"])),
    options(host.clone()),
    baseline_database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let boundary = position(&baseline.events, |event| {
    event
      .iteration
      .as_ref()
      .is_some_and(|scope| scope.index == 0)
      && matches!(event.payload, RunEventPayload::StepAttemptStarted(_))
  });
  let database = TemporaryDatabase::new("ambiguous-resume");
  restore_prefix(
    &database,
    &workflow,
    definition_hash,
    &baseline.events,
    boundary,
  );

  let error = resume_workflow_durable(
    database.path().to_path_buf(),
    &baseline.run_id,
    options(host),
  )
  .await
  .unwrap_err();
  match error {
    RuntimeExecutionError::RunFailed(details)
      if details.failure.kind == AttemptFailureKind::Interrupted => {}
    other => panic!("expected an interrupted run failure, received {other:?}"),
  }
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store.projection(&baseline.run_id).unwrap().status,
    RunStatus::Failed
  );
  let events = store.events(&baseline.run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationSkipped(_)))
      .count(),
    2
  );
  assert_eq!(
    events
      .iter()
      .filter(|event| {
        event
          .iteration
          .as_ref()
          .is_some_and(|scope| scope.index == 0)
          && matches!(event.payload, RunEventPayload::StepAttemptStarted(_))
      })
      .count(),
    1
  );
}

#[tokio::test]
async fn restart_waits_for_the_durable_retry_and_starts_only_the_next_attempt() {
  let Some(host) = host_options() else { return };
  let workflow = retry_model();
  let definition_hash = "sha256:6363636363636363636363636363636363636363636363636363636363636363";
  let baseline_database = TemporaryDatabase::new("retry-baseline");
  let baseline = execute_workflow_durable(
    workflow.clone(),
    definition_hash.to_string(),
    payload(json!(["only-item"])),
    options(host.clone()),
    baseline_database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let boundary = position(&baseline.events, |event| {
    matches!(event.payload, RunEventPayload::StepRetryScheduled(_))
  });
  let database = TemporaryDatabase::new("retry-resume");
  restore_prefix(
    &database,
    &workflow,
    definition_hash,
    &baseline.events,
    boundary,
  );

  let resumed = resume_workflow_durable(
    database.path().to_path_buf(),
    &baseline.run_id,
    options(host),
  )
  .await
  .unwrap();
  let attempts = resumed
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptStarted(data) if event.iteration.is_some() => Some(data.attempt),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(attempts, [1, 2]);
}

#[tokio::test]
async fn cancellation_closes_the_open_iteration_and_skips_pending_indexes() {
  let Some(host) = host_options() else { return };
  let workflow = model();
  let definition_hash = "sha256:6464646464646464646464646464646464646464646464646464646464646464";
  let baseline_database = TemporaryDatabase::new("cancel-baseline");
  let baseline = execute_workflow_durable(
    workflow.clone(),
    definition_hash.to_string(),
    payload(json!(["first", "second", "third"])),
    options(host.clone()),
    baseline_database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let boundary = position(&baseline.events, |event| {
    event
      .iteration
      .as_ref()
      .is_some_and(|scope| scope.index == 0)
      && matches!(event.payload, RunEventPayload::StepAttemptStarted(_))
  });
  let database = TemporaryDatabase::new("cancel-resume");
  restore_prefix(
    &database,
    &workflow,
    definition_hash,
    &baseline.events,
    boundary,
  );
  DurableEventStore::open(database.path())
    .unwrap()
    .request_run_cancellation(&baseline.run_id, "cancel-loop", Utc::now())
    .unwrap();

  let error = resume_workflow_durable(
    database.path().to_path_buf(),
    &baseline.run_id,
    options(host),
  )
  .await
  .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store.projection(&baseline.run_id).unwrap().status,
    RunStatus::Cancelled
  );
  let events = store.events(&baseline.run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachCancelled(_)))
      .count(),
    1
  );
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationSkipped(_)))
      .count(),
    2
  );
}

#[tokio::test]
async fn backup_restore_projection_rebuild_and_retention_preserve_loop_history() {
  let Some(host) = host_options() else { return };
  let workflow = model();
  let definition_hash = "sha256:6565656565656565656565656565656565656565656565656565656565656565";
  let database = TemporaryDatabase::new("operations-source");
  let result = execute_workflow_durable(
    workflow,
    definition_hash.to_string(),
    payload(json!(["first", "second"])),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let backup = database.path().with_extension("backup.sqlite");
  let restored = database.path().with_extension("restored.sqlite");
  create_online_backup(
    database.path(),
    &backup,
    "for-each-backup-lease",
    "for-each-tests",
    "for-each-test-deployment",
  )
  .unwrap();
  std::fs::copy(&backup, &restored).unwrap();
  prepare_restored_store(
    &restored,
    &[definition_hash.to_string()],
    "for-each-backup",
    Utc::now(),
  )
  .unwrap();
  let mut restored_store = DurableEventStore::open(&restored).unwrap();
  assert_eq!(
    restored_store.events(&result.run_id).unwrap(),
    result.events
  );
  assert_eq!(
    restored_store.projection(&result.run_id).unwrap().context,
    result.context
  );
  restored_store.rebuild_run_summaries().unwrap();
  assert_eq!(
    restored_store.projection(&result.run_id).unwrap().status,
    RunStatus::Succeeded
  );
  drop(restored_store);

  let now = Utc::now() + chrono::Duration::days(31);
  let policy = RetentionPolicyV1 {
    policy_id: "for-each-terminal-history".to_string(),
    succeeded_before: now,
    failed_before: now,
    cancelled_before: now,
  };
  assert_eq!(
    plan_retention(&restored, &policy, now)
      .unwrap()
      .eligible_runs,
    1
  );
  assert_eq!(
    execute_retention(
      &restored,
      &policy,
      "for-each-retention-lease",
      "for-each-tests",
      false,
      now,
    )
    .unwrap()
    .result
    .deleted_runs,
    1
  );
  assert!(DurableEventStore::open(&restored)
    .unwrap()
    .events(&result.run_id)
    .is_err());
  let _ = std::fs::remove_file(backup);
  let _ = std::fs::remove_file(restored);
}
