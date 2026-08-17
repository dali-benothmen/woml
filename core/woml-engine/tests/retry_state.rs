use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::event::{StepAttemptFailedData, StepAttemptSucceededData};
use woml_engine::model::{BackoffPolicy, ModelIssueCode, RetryPolicy};
use woml_engine::projection::{AttemptStatus, RunStatus};
use woml_engine::{
  fold_events, run_event_schema_version_for_model, step_effect_idempotency_key, AttemptFailure,
  AttemptFailureKind, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, RunEvent,
  RunEventPayload, StepFailureDisposition, COMPILED_MODEL_SCHEMA_VERSION_V6,
  RUN_EVENT_SCHEMA_VERSION_V6,
};

const RETRY_MODEL: &str = include_str!("../../../woml/tests/fixtures/retry.compiled.v6.json");
const RETRY_HASH: &str = "sha256:27606cefeebc5b6d45c965969b621a2f74ae2ebebe2b94edec80d97bfeb8378c";
const RETRY_SUCCESS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-success.events.v6.json");
const RETRY_EXHAUSTED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-exhausted.events.v6.json");
const RETRY_SCHEDULED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-scheduled-recovery.events.v6.json");
const RETRY_AMBIGUOUS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/retry-ambiguous-recovery.events.v6.json");

fn retry_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(RETRY_MODEL).unwrap()
}

fn script_failure(message: &str) -> AttemptFailure {
  AttemptFailure {
    kind: AttemptFailureKind::ScriptThrew,
    code: AttemptFailureKind::ScriptThrew.code().to_string(),
    message: message.to_string(),
    details: None,
    ..AttemptFailure::legacy_defaults()
  }
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-ri2-{label}-{}.sqlite",
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

fn seed_retry_step(engine: &mut DurableDagEngine, run_id: &str, started_at: chrono::DateTime<Utc>) {
  engine
    .start_run(
      format!("evt_{run_id}_started"),
      run_id,
      started_at,
      Map::new(),
    )
    .unwrap();
  engine
    .start_step_attempt(
      run_id,
      "prepare",
      1,
      format!("inv_{run_id}_prepare_1"),
      started_at + chrono::Duration::milliseconds(1),
    )
    .unwrap();
  engine
    .append_payload(
      format!("evt_{run_id}_prepare_succeeded"),
      run_id,
      started_at + chrono::Duration::milliseconds(2),
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: "prepare".to_string(),
        attempt: 1,
        invocation_id: format!("inv_{run_id}_prepare_1"),
        output: json!({ "name": "World" }),
      }),
    )
    .unwrap();
  engine
    .start_step_attempt(
      run_id,
      "greet",
      1,
      format!("inv_{run_id}_greet_1"),
      started_at + chrono::Duration::milliseconds(3),
    )
    .unwrap();
}

#[test]
fn model_v6_accepts_only_the_frozen_script_retry_contract() {
  let model = retry_model();
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V6);
  model.validate_structure().unwrap();
  model.validate_for_durable_execution().unwrap();
  assert!(model.validate_for_execution().is_err());
  assert_eq!(
    run_event_schema_version_for_model(model.schema_version),
    RUN_EVENT_SCHEMA_VERSION_V6
  );

  let mut structural = model.clone();
  structural.graph.nodes[1].handler = "engine.parallel-join".to_string();
  let issues = structural.validate_structure().unwrap_err().issues;
  assert!(issues
    .iter()
    .any(|issue| issue.code == ModelIssueCode::UnsupportedRetry));

  let mut invalid_backoff = model;
  invalid_backoff.graph.nodes[1].retry_policy = Some(RetryPolicy {
    max_attempts: 3,
    backoff: BackoffPolicy::Exponential {
      initial_delay_ms: 2_000,
      multiplier: 3.0,
      maximum_delay_ms: Some(1_000),
    },
  });
  assert!(invalid_backoff.validate_structure().is_err());
}

#[test]
fn frozen_v6_histories_fold_to_their_reviewed_retry_boundaries() {
  let success: Vec<RunEvent> = serde_json::from_str(RETRY_SUCCESS).unwrap();
  let exhausted: Vec<RunEvent> = serde_json::from_str(RETRY_EXHAUSTED).unwrap();
  let scheduled: Vec<RunEvent> = serde_json::from_str(RETRY_SCHEDULED).unwrap();
  let ambiguous: Vec<RunEvent> = serde_json::from_str(RETRY_AMBIGUOUS).unwrap();

  let success = fold_events(&success).unwrap();
  assert_eq!(success.status, RunStatus::Succeeded);
  assert!(success.pending_retries.is_empty());
  assert_eq!(
    success.context.steps["greet"],
    json!({ "message": "Hello World" })
  );

  let exhausted = fold_events(&exhausted).unwrap();
  assert_eq!(exhausted.status, RunStatus::Failed);
  assert!(exhausted.pending_retries.is_empty());

  let scheduled = fold_events(&scheduled).unwrap();
  assert_eq!(scheduled.status, RunStatus::Running);
  assert_eq!(scheduled.pending_retries["greet"].next_attempt, 2);

  let ambiguous = fold_events(&ambiguous).unwrap();
  assert_eq!(ambiguous.status, RunStatus::Failed);
  assert!(ambiguous.pending_retries.is_empty());
  assert_eq!(ambiguous.attempts.len(), 1);
}

#[test]
fn durable_retry_schedule_is_atomic_due_aware_and_survives_reopen() {
  let database = TemporaryDatabase::new("scheduled");
  let run_id = "run_ri2_scheduled";
  let started_at = Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
  let scheduled_at;

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(retry_model(), RETRY_HASH, store).unwrap();
    seed_retry_step(&mut engine, run_id, started_at);
    let failed_at = started_at + chrono::Duration::milliseconds(4);
    let commit = engine
      .record_step_attempt_failure(
        run_id,
        failed_at,
        StepAttemptFailedData {
          node_id: "greet".to_string(),
          attempt: 1,
          invocation_id: format!("inv_{run_id}_greet_1"),
          failure: script_failure("temporary"),
        },
      )
      .unwrap();
    let StepFailureDisposition::RetryScheduled {
      next_attempt,
      scheduled_at: due,
    } = commit.disposition
    else {
      panic!("attempt 1 should schedule attempt 2")
    };
    assert_eq!(next_attempt, 2);
    assert_eq!(due, failed_at + chrono::Duration::seconds(1));
    scheduled_at = due;
    assert_eq!(commit.projection.pending_retries["greet"].scheduled_at, due);
    assert!(engine
      .ready_node_ids_at(run_id, due - chrono::Duration::milliseconds(1))
      .unwrap()
      .is_empty());
    assert_eq!(engine.ready_node_ids_at(run_id, due).unwrap(), ["greet"]);

    let events = engine.events(run_id).unwrap();
    let end = &events[events.len() - 2..];
    assert!(matches!(
      end[0].payload,
      RunEventPayload::StepAttemptFailed(_)
    ));
    assert!(matches!(
      end[1].payload,
      RunEventPayload::StepRetryScheduled(_)
    ));
    assert_eq!(end[0].sequence + 1, end[1].sequence);
  }

  let mut store = DurableEventStore::open(database.path()).unwrap();
  let recovery = store.recover_interrupted_runs().unwrap();
  assert_eq!(recovery.recovered_runs, 0);
  assert_eq!(recovery.resumable_runs, 1);
  let projection = store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Running);
  assert_eq!(projection.pending_retries["greet"].next_attempt, 2);
  let mut reopened = DurableDagEngine::resume(store, run_id).unwrap();
  assert_eq!(
    reopened.projection(run_id).unwrap().pending_retries["greet"].scheduled_at,
    scheduled_at
  );
  assert!(reopened
    .start_step_attempt(
      run_id,
      "greet",
      2,
      format!("inv_{run_id}_greet_early"),
      scheduled_at - chrono::Duration::milliseconds(1),
    )
    .is_err());
  reopened
    .start_step_attempt(
      run_id,
      "greet",
      2,
      format!("inv_{run_id}_greet_2"),
      scheduled_at,
    )
    .unwrap();
  let projection = reopened.projection(run_id).unwrap();
  assert!(projection.pending_retries.is_empty());
  assert_eq!(projection.attempts.len(), 3);
  assert_eq!(
    projection.attempts[1].idempotency_key,
    projection.attempts[2].idempotency_key
  );
}

#[test]
fn exponential_retry_exhaustion_fails_atomically_without_attempt_four() {
  let store = DurableEventStore::open_in_memory().unwrap();
  let mut engine = DurableDagEngine::new(retry_model(), RETRY_HASH, store).unwrap();
  let run_id = "run_ri2_exhausted";
  let started_at = Utc.with_ymd_and_hms(2026, 8, 7, 13, 0, 0).unwrap();
  seed_retry_step(&mut engine, run_id, started_at);

  let first = engine
    .record_step_attempt_failure(
      run_id,
      started_at + chrono::Duration::milliseconds(4),
      StepAttemptFailedData {
        node_id: "greet".to_string(),
        attempt: 1,
        invocation_id: format!("inv_{run_id}_greet_1"),
        failure: script_failure("first"),
      },
    )
    .unwrap();
  let StepFailureDisposition::RetryScheduled {
    scheduled_at: second_at,
    ..
  } = first.disposition
  else {
    unreachable!()
  };
  engine
    .start_step_attempt(run_id, "greet", 2, "inv_exhausted_greet_2", second_at)
    .unwrap();
  let second = engine
    .record_step_attempt_failure(
      run_id,
      second_at + chrono::Duration::milliseconds(1),
      StepAttemptFailedData {
        node_id: "greet".to_string(),
        attempt: 2,
        invocation_id: "inv_exhausted_greet_2".to_string(),
        failure: script_failure("second"),
      },
    )
    .unwrap();
  let StepFailureDisposition::RetryScheduled {
    scheduled_at: third_at,
    ..
  } = second.disposition
  else {
    unreachable!()
  };
  assert_eq!(
    third_at,
    second_at + chrono::Duration::milliseconds(1) + chrono::Duration::seconds(2)
  );
  engine
    .start_step_attempt(run_id, "greet", 3, "inv_exhausted_greet_3", third_at)
    .unwrap();
  let final_commit = engine
    .record_step_attempt_failure(
      run_id,
      third_at + chrono::Duration::milliseconds(1),
      StepAttemptFailedData {
        node_id: "greet".to_string(),
        attempt: 3,
        invocation_id: "inv_exhausted_greet_3".to_string(),
        failure: script_failure("third"),
      },
    )
    .unwrap();
  assert_eq!(final_commit.disposition, StepFailureDisposition::RunFailed);
  assert_eq!(final_commit.projection.status, RunStatus::Failed);
  assert!(final_commit.projection.pending_retries.is_empty());
  assert!(engine
    .start_step_attempt(run_id, "greet", 4, "inv_forbidden", Utc::now())
    .is_err());
}

#[test]
fn recovery_fails_an_active_v6_attempt_closed_without_scheduling_it() {
  let database = TemporaryDatabase::new("ambiguous");
  let run_id = "run_ri2_ambiguous";
  let started_at = Utc.with_ymd_and_hms(2026, 8, 7, 14, 0, 0).unwrap();
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(retry_model(), RETRY_HASH, store).unwrap();
    seed_retry_step(&mut engine, run_id, started_at);
  }

  let mut store = DurableEventStore::open(database.path()).unwrap();
  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  assert_eq!(report.interrupted_attempts, 1);
  let projection = store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(projection.pending_retries.is_empty());
  assert!(matches!(
    projection.attempts.last().unwrap().status,
    AttemptStatus::Failed {
      failure: AttemptFailure {
        kind: AttemptFailureKind::Interrupted,
        ..
      }
    }
  ));
}

#[test]
fn recovery_closes_a_successful_retry_instead_of_reviving_its_old_failure() {
  let database = TemporaryDatabase::new("successful-retry-recovery");
  let run_id = "run_ri6_successful_retry";
  let started_at = Utc.with_ymd_and_hms(2026, 8, 7, 15, 0, 0).unwrap();
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(retry_model(), RETRY_HASH, store).unwrap();
    seed_retry_step(&mut engine, run_id, started_at);
    let commit = engine
      .record_step_attempt_failure(
        run_id,
        started_at + chrono::Duration::milliseconds(4),
        StepAttemptFailedData {
          node_id: "greet".to_string(),
          attempt: 1,
          invocation_id: format!("inv_{run_id}_greet_1"),
          failure: script_failure("temporary"),
        },
      )
      .unwrap();
    let StepFailureDisposition::RetryScheduled { scheduled_at, .. } = commit.disposition else {
      panic!("attempt 1 should schedule attempt 2")
    };
    let invocation_id = format!("inv_{run_id}_greet_2");
    engine
      .start_step_attempt(run_id, "greet", 2, &invocation_id, scheduled_at)
      .unwrap();
    engine
      .append_payload(
        format!("evt_{run_id}_greet_succeeded"),
        run_id,
        scheduled_at + chrono::Duration::milliseconds(1),
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: "greet".to_string(),
          attempt: 2,
          invocation_id,
          output: json!({ "message": "Hello World" }),
        }),
      )
      .unwrap();
  }

  let mut store = DurableEventStore::open(database.path()).unwrap();
  let recovery = store.recover_interrupted_runs().unwrap();
  assert_eq!(recovery.recovered_runs, 1);
  assert_eq!(recovery.interrupted_attempts, 0);
  let projection = store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Succeeded);
  assert_eq!(
    projection.context.steps["greet"],
    json!({ "message": "Hello World" })
  );
  assert_eq!(
    projection
      .attempts
      .iter()
      .filter(|attempt| attempt.identity.node_id == "greet")
      .count(),
    2
  );
}

#[test]
fn recovery_fails_closed_when_the_retry_itself_was_interrupted() {
  let database = TemporaryDatabase::new("interrupted-retry");
  let run_id = "run_ri6_interrupted_retry";
  let started_at = Utc.with_ymd_and_hms(2026, 8, 7, 16, 0, 0).unwrap();
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(retry_model(), RETRY_HASH, store).unwrap();
    seed_retry_step(&mut engine, run_id, started_at);
    let commit = engine
      .record_step_attempt_failure(
        run_id,
        started_at + chrono::Duration::milliseconds(4),
        StepAttemptFailedData {
          node_id: "greet".to_string(),
          attempt: 1,
          invocation_id: format!("inv_{run_id}_greet_1"),
          failure: script_failure("temporary"),
        },
      )
      .unwrap();
    let StepFailureDisposition::RetryScheduled { scheduled_at, .. } = commit.disposition else {
      panic!("attempt 1 should schedule attempt 2")
    };
    engine
      .start_step_attempt(
        run_id,
        "greet",
        2,
        format!("inv_{run_id}_greet_2"),
        scheduled_at,
      )
      .unwrap();
  }

  let mut store = DurableEventStore::open(database.path()).unwrap();
  let recovery = store.recover_interrupted_runs().unwrap();
  assert_eq!(recovery.recovered_runs, 1);
  assert_eq!(recovery.interrupted_attempts, 1);
  let projection = store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(projection.pending_retries.is_empty());
  let latest = projection.latest_attempt("greet").unwrap();
  assert_eq!(latest.identity.attempt, 2);
  assert!(matches!(
    latest.status,
    AttemptStatus::Failed {
      failure: AttemptFailure {
        kind: AttemptFailureKind::Interrupted,
        ..
      }
    }
  ));
}

#[test]
fn stable_effect_key_matches_the_frozen_rfc8785_fixture() {
  assert_eq!(
    step_effect_idempotency_key("run_retry_success", RETRY_HASH, "greet"),
    "sha256:35278a8c79c5843d1fc3015aac65ea3ee7579559463214234e16624b5bbf609c"
  );
}
