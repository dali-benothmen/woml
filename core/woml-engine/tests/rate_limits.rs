use std::sync::{Arc, Barrier};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::Map;
use uuid::Uuid;
use woml_engine::model::{CompiledRateLimitPolicy, RateLimitAlgorithm};
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, PolicyClaimWaitReason, PolicyExecutionClaimResult,
  PolicyWaitingFor, RunEventPayload, TriggerAdmissionRequest,
};

struct TestDatabase(std::path::PathBuf);

impl TestDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-rp4-{}.sqlite", Uuid::new_v4().simple())))
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn rate_model(workflow_id: &str, count: u32, window_ms: u64) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  workflow.workflow_id = workflow_id.to_string();
  let policy = workflow.runtime_policy.as_mut().unwrap();
  policy.concurrency = None;
  policy.timeout_ms = None;
  policy.rate_limit = Some(CompiledRateLimitPolicy {
    count,
    window_ms,
    algorithm: RateLimitAlgorithm::RollingWindow,
  });
  policy.queue.as_mut().unwrap().name = workflow_id.to_string();
  workflow
}

fn admit(
  store: &mut DurableEventStore,
  workflow: &CompiledWorkflowDefinition,
  hash: &str,
  source: &str,
  now: chrono::DateTime<Utc>,
) -> String {
  store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: hash.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: source.to_string(),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap()
    .run_id
}

#[test]
fn rolling_window_is_exact_at_the_lower_boundary_and_inspectable() {
  let database = TestDatabase::new();
  let hash = "sha256:4444444444444444444444444444444444444444444444444444444444444444";
  let start = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let workflow = rate_model("rate-boundary", 3, 10_000);
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(&workflow, hash).unwrap();
  let runs = (0..4)
    .map(|index| {
      admit(
        &mut store,
        &workflow,
        hash,
        &format!("run-{index}"),
        start + chrono::Duration::milliseconds(index),
      )
    })
    .collect::<Vec<_>>();
  let first_started_at = start + chrono::Duration::milliseconds(3);

  for (index, run_id) in runs.iter().take(3).enumerate() {
    let claim = store
      .claim_policy_run(
        run_id,
        &format!("owner-{index}"),
        first_started_at,
        Duration::from_secs(30),
      )
      .unwrap();
    store
      .release_policy_claim(run_id, &format!("owner-{index}"), &claim.claim_id)
      .unwrap();
  }

  let before_boundary = first_started_at + chrono::Duration::milliseconds(9_999);
  let waiting = store
    .try_claim_policy_run(
      &runs[3],
      "owner-four",
      before_boundary,
      Duration::from_secs(30),
    )
    .unwrap();
  assert!(matches!(
    waiting,
    PolicyExecutionClaimResult::Waiting {
      reason: PolicyClaimWaitReason::RateLimit,
      eligible_at: Some(value),
      ..
    } if value == first_started_at + chrono::Duration::seconds(10)
  ));
  let inspection = store.inspect_run_v3(&runs[3]).unwrap();
  assert_eq!(
    inspection.policy.waiting_for,
    Some(PolicyWaitingFor::RateLimit)
  );
  assert_eq!(
    inspection.policy.eligible_at,
    Some(first_started_at + chrono::Duration::seconds(10))
  );

  let claim = store
    .claim_policy_run(
      &runs[3],
      "owner-four",
      first_started_at + chrono::Duration::seconds(10),
      Duration::from_secs(30),
    )
    .unwrap();
  assert_eq!(
    store
      .events(&runs[3])
      .unwrap()
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::RunExecutionStarted(_)))
      .count(),
    1
  );
  store
    .release_policy_claim(&runs[3], "owner-four", &claim.claim_id)
    .unwrap();
}

#[test]
fn resume_does_not_consume_rate_capacity_again() {
  let database = TestDatabase::new();
  let hash = "sha256:4545454545454545454545454545454545454545454545454545454545454545";
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 13, 0, 0).unwrap();
  let workflow = rate_model("resume-rate", 1, 60_000);
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(&workflow, hash).unwrap();
  let first = admit(&mut store, &workflow, hash, "first", now);
  let second = admit(
    &mut store,
    &workflow,
    hash,
    "second",
    now + chrono::Duration::milliseconds(1),
  );
  let execution_at = now + chrono::Duration::milliseconds(1);

  let initial = store
    .claim_policy_run(&first, "initial", execution_at, Duration::from_secs(30))
    .unwrap();
  store
    .release_policy_claim(&first, "initial", &initial.claim_id)
    .unwrap();
  assert!(matches!(
    store
      .try_claim_policy_run(&second, "second", execution_at, Duration::from_secs(30))
      .unwrap(),
    PolicyExecutionClaimResult::Waiting {
      reason: PolicyClaimWaitReason::RateLimit,
      ..
    }
  ));
  assert!(matches!(
    store
      .try_claim_policy_run(&first, "resume", execution_at, Duration::from_secs(30))
      .unwrap(),
    PolicyExecutionClaimResult::Claimed {
      first_start: false,
      ..
    }
  ));
}

#[test]
fn clock_rollback_fails_closed_and_restart_keeps_rate_history() {
  let database = TestDatabase::new();
  let hash = "sha256:4646464646464646464646464646464646464646464646464646464646464646";
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 14, 0, 0).unwrap();
  let execution_at = now + chrono::Duration::milliseconds(1);
  let workflow = rate_model("restart-rate", 1, 10_000);
  let second = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store.register_definition(&workflow, hash).unwrap();
    let first = admit(&mut store, &workflow, hash, "first", now);
    let second = admit(&mut store, &workflow, hash, "second", execution_at);
    let claim = store
      .claim_policy_run(&first, "first", execution_at, Duration::from_secs(30))
      .unwrap();
    store
      .release_policy_claim(&first, "first", &claim.claim_id)
      .unwrap();
    second
  };
  let mut reopened = DurableEventStore::open(&database.0).unwrap();
  assert!(matches!(
    reopened
      .try_claim_policy_run_at(
        &second,
        "second",
        execution_at,
        now - chrono::Duration::seconds(5),
        Duration::from_secs(30),
      )
      .unwrap(),
    PolicyExecutionClaimResult::Waiting {
      reason: PolicyClaimWaitReason::RateLimit,
      eligible_at: Some(value),
      ..
    } if value == execution_at + chrono::Duration::seconds(10)
  ));
  reopened.rebuild_runtime_policy_indexes().unwrap();
  assert!(reopened
    .claim_policy_run(
      &second,
      "second",
      execution_at + chrono::Duration::seconds(10),
      Duration::from_secs(30),
    )
    .is_ok());
}

#[test]
fn cross_process_race_can_commit_only_one_rate_start() {
  let database = TestDatabase::new();
  let hash = "sha256:4747474747474747474747474747474747474747474747474747474747474747";
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 15, 0, 0).unwrap();
  let workflow = rate_model("race-rate", 1, 10_000);
  let (first, second) = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store.register_definition(&workflow, hash).unwrap();
    (
      admit(&mut store, &workflow, hash, "first", now),
      admit(&mut store, &workflow, hash, "second", now),
    )
  };
  let run_ids = [first.clone(), second.clone()];
  let barrier = Arc::new(Barrier::new(3));
  let handles = [(first, "one"), (second, "two")]
    .into_iter()
    .map(|(run_id, owner)| {
      let path = database.0.clone();
      let barrier = Arc::clone(&barrier);
      std::thread::spawn(move || {
        let mut store = DurableEventStore::open(path).unwrap();
        barrier.wait();
        store
          .try_claim_policy_run(&run_id, owner, now, Duration::from_secs(30))
          .unwrap()
      })
    })
    .collect::<Vec<_>>();
  barrier.wait();
  let outcomes = handles
    .into_iter()
    .map(|handle| handle.join().unwrap())
    .collect::<Vec<_>>();
  assert_eq!(
    outcomes
      .iter()
      .filter(|outcome| matches!(outcome, PolicyExecutionClaimResult::Claimed { .. }))
      .count(),
    1
  );
  assert_eq!(
    outcomes
      .iter()
      .filter(|outcome| matches!(outcome, PolicyExecutionClaimResult::Waiting { .. }))
      .count(),
    1
  );
  let store = DurableEventStore::open(&database.0).unwrap();
  let durable_starts = run_ids
    .iter()
    .flat_map(|run_id| store.events(run_id).unwrap())
    .filter(|event| matches!(event.payload, RunEventPayload::RunExecutionStarted(_)))
    .count();
  assert_eq!(durable_starts, 1);
}
