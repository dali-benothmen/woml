use std::path::PathBuf;

use chrono::{Duration, TimeZone, Utc};
use serde_json::{json, Map};
use woml_engine::{
  fold_events, step_effect_idempotency_key, ChoiceSelectedData, CompiledWorkflowDefinition,
  DurableEventStore, ForkBranchOutcome, ForkBranchSettledData, ForkJoinOutcome,
  ForkJoinSettledData, ForkOpenedData, InMemoryEventStore, RunAdmissionQueue, RunAdmissionTrigger,
  RunAdmittedData, RunEvent, RunEventPayload, RunExecutionStartedData, StepAttemptStartedData,
  StepAttemptSucceededData, RUN_EVENT_SCHEMA_VERSION_V12,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");
const HISTORIES: &str = include_str!("../../../woml/tests/fixtures/fork-branch/histories.v12.json");
const DEFINITION_HASH: &str =
  "sha256:1313131313131313131313131313131313131313131313131313131313131313";

struct TestDatabase(PathBuf);

impl TestDatabase {
  fn new(name: &str) -> Self {
    Self(std::env::temp_dir().join(format!("woml-fj4-{name}-{}.sqlite", uuid::Uuid::new_v4())))
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
}

fn at(second: i64) -> chrono::DateTime<Utc> {
  Utc.with_ymd_and_hms(2026, 8, 13, 10, 0, 0).unwrap() + Duration::seconds(second)
}

fn event(sequence: u64, run_id: &str, payload: RunEventPayload) -> RunEvent {
  RunEvent {
    event_schema_version: RUN_EVENT_SCHEMA_VERSION_V12,
    event_id: format!("evt-fj4-{run_id}-{sequence}"),
    run_id: run_id.to_string(),
    sequence,
    occurred_at: at(sequence as i64),
    iteration: None,
    payload,
  }
}

fn append(store: &mut DurableEventStore, run_id: &str, sequence: u64, payload: RunEventPayload) {
  store
    .append_payload(
      run_id,
      format!("evt-fj4-{run_id}-{sequence}"),
      at(sequence as i64),
      payload,
    )
    .unwrap();
}

fn admit_and_start(
  store: &mut DurableEventStore,
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
) {
  append(
    store,
    run_id,
    1,
    RunEventPayload::RunAdmitted(RunAdmittedData {
      definition_hash: DEFINITION_HASH.to_string(),
      policy_hash: workflow.runtime_policy_hash().unwrap(),
      trigger: RunAdmissionTrigger {
        id: "start".to_string(),
        handler: "trigger.manual".to_string(),
      },
      payload: Map::new(),
      queue: RunAdmissionQueue {
        name: workflow.runtime_policy_queue_name().unwrap(),
        discipline: woml_engine::QueueDiscipline::WorkConservingFifo,
      },
      admitted_at: at(1),
      occurrence_sequence: 1,
    }),
  );
  append(
    store,
    run_id,
    2,
    RunEventPayload::RunExecutionStarted(RunExecutionStartedData {
      started_at: at(2),
      timeout_at: None,
    }),
  );
}

fn succeed_step(
  store: &mut DurableEventStore,
  run_id: &str,
  sequence: &mut u64,
  node_id: &str,
  output: serde_json::Value,
) {
  let invocation_id = format!("inv-{run_id}-{node_id}");
  append(
    store,
    run_id,
    *sequence,
    RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
      node_id: node_id.to_string(),
      attempt: 1,
      invocation_id: invocation_id.clone(),
      handler: "runtime.script".to_string(),
      idempotency_key: Some(step_effect_idempotency_key(
        run_id,
        DEFINITION_HASH,
        node_id,
      )),
    }),
  );
  *sequence += 1;
  append(
    store,
    run_id,
    *sequence,
    RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
      node_id: node_id.to_string(),
      attempt: 1,
      invocation_id,
      output,
    }),
  );
  *sequence += 1;
}

#[test]
fn frozen_event_v12_payloads_are_closed_and_fold_rejects_contradictions() {
  let histories: serde_json::Value = serde_json::from_str(HISTORIES).unwrap();
  for history in histories.as_object().unwrap().values() {
    for encoded in history.as_array().unwrap() {
      let event: RunEvent = serde_json::from_value(encoded.clone()).unwrap();
      event.validate().unwrap();
      assert_eq!(event.event_schema_version, RUN_EVENT_SCHEMA_VERSION_V12);
    }
  }

  let run_id = "run-fold";
  let history = vec![
    event(
      1,
      run_id,
      RunEventPayload::RunAdmitted(RunAdmittedData {
        definition_hash: DEFINITION_HASH.to_string(),
        policy_hash: "sha256:abababababababababababababababababababababababababababababababab"
          .to_string(),
        trigger: RunAdmissionTrigger {
          id: "start".to_string(),
          handler: "trigger.manual".to_string(),
        },
        payload: Map::new(),
        queue: RunAdmissionQueue {
          name: "workflow:distribution-all".to_string(),
          discipline: woml_engine::QueueDiscipline::WorkConservingFifo,
        },
        admitted_at: at(1),
        occurrence_sequence: 1,
      }),
    ),
    event(
      2,
      run_id,
      RunEventPayload::RunExecutionStarted(RunExecutionStartedData {
        started_at: at(2),
        timeout_at: None,
      }),
    ),
    event(
      3,
      run_id,
      RunEventPayload::ForkOpened(ForkOpenedData {
        fork_id: "distribution".to_string(),
      }),
    ),
  ];
  let projection = fold_events(&history).unwrap();
  assert!(projection.forks.contains_key("distribution"));
  let mut memory = InMemoryEventStore::default();
  for stored in &history {
    memory.append(stored.clone()).unwrap();
  }
  assert_eq!(memory.projection(run_id).unwrap(), projection);
  let mut duplicate = history;
  duplicate.push(event(
    4,
    run_id,
    RunEventPayload::ForkOpened(ForkOpenedData {
      fork_id: "distribution".to_string(),
    }),
  ));
  assert!(fold_events(&duplicate)
    .unwrap_err()
    .to_string()
    .contains("opened more than once"));

  let mut choice_history = duplicate[..2].to_vec();
  choice_history.push(event(
    3,
    run_id,
    RunEventPayload::ChoiceSelected(ChoiceSelectedData {
      choice_id: "__woml_choice__root_1".to_string(),
      arm_id: "__woml_choice__root_1:otherwise".to_string(),
    }),
  ));
  assert_eq!(
    fold_events(&choice_history).unwrap().choice_selections["__woml_choice__root_1"],
    "__woml_choice__root_1:otherwise"
  );
}

#[test]
fn sqlite_reopen_rebuilds_the_same_fork_projection_inspection_and_safe_work() {
  let database = TestDatabase::new("reopen");
  let workflow = model();
  let run_id = "run-fj4-reopen";
  let before = {
    let mut store = DurableEventStore::open(&database.0).unwrap();
    store
      .register_definition(&workflow, DEFINITION_HASH)
      .unwrap();
    admit_and_start(&mut store, &workflow, run_id);
    let mut sequence = 3;
    succeed_step(
      &mut store,
      run_id,
      &mut sequence,
      "prepare",
      json!({ "enabled": true }),
    );
    append(
      &mut store,
      run_id,
      sequence,
      RunEventPayload::ForkOpened(ForkOpenedData {
        fork_id: "distribution".to_string(),
      }),
    );
    let projection = store.projection(run_id).unwrap();
    let recovery = store.fork_recovery_work_v1(run_id).unwrap();
    let inspection = store.inspect_run_v4(run_id).unwrap();
    (
      projection,
      recovery,
      serde_json::to_value(inspection).unwrap(),
    )
  };

  let reopened = DurableEventStore::open(&database.0).unwrap();
  assert_eq!(reopened.projection(run_id).unwrap(), before.0);
  assert_eq!(reopened.fork_recovery_work_v1(run_id).unwrap(), before.1);
  assert_eq!(
    serde_json::to_value(reopened.inspect_run_v4(run_id).unwrap()).unwrap(),
    before.2
  );
  assert_eq!(before.1.pending_branches.len(), 2);
  assert_eq!(before.2["forks"]["counts"]["active"], 1);
  assert!(serde_json::to_string(&before.2)
    .unwrap()
    .find("enabled")
    .is_none());
}

#[test]
fn model_bound_history_rejects_unknown_identities_impossible_joins_and_ambiguous_work() {
  let database = TestDatabase::new("invalid");
  let workflow = model();
  let run_id = "run-fj4-invalid";
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .unwrap();
  admit_and_start(&mut store, &workflow, run_id);
  let mut sequence = 3;
  succeed_step(
    &mut store,
    run_id,
    &mut sequence,
    "prepare",
    json!({ "enabled": true }),
  );
  append(
    &mut store,
    run_id,
    sequence,
    RunEventPayload::ForkOpened(ForkOpenedData {
      fork_id: "distribution".to_string(),
    }),
  );
  sequence += 1;

  let impossible = store.append_payload(
    run_id,
    "evt-impossible-join",
    at(sequence as i64),
    RunEventPayload::ForkJoinSettled(ForkJoinSettledData {
      fork_id: "distribution".to_string(),
      outcome: ForkJoinOutcome::Succeeded,
      blocking_branch_id: None,
    }),
  );
  assert!(impossible
    .unwrap_err()
    .to_string()
    .contains("before every joined branch"));

  let unknown = store.append_payload(
    run_id,
    "evt-unknown-branch",
    at(sequence as i64),
    RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
      fork_id: "distribution".to_string(),
      branch_id: "unknown".to_string(),
      terminal_node_id: "unknown-terminal".to_string(),
      outcome: ForkBranchOutcome::Failed,
    }),
  );
  assert!(unknown.unwrap_err().to_string().contains("unknown branch"));

  append(
    &mut store,
    run_id,
    sequence,
    RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
      node_id: "publishInstagram".to_string(),
      attempt: 1,
      invocation_id: "inv-ambiguous-instagram".to_string(),
      handler: "runtime.script".to_string(),
      idempotency_key: Some(step_effect_idempotency_key(
        run_id,
        DEFINITION_HASH,
        "publishInstagram",
      )),
    }),
  );
  let recovery = store.fork_recovery_work_v1(run_id).unwrap();
  assert_eq!(
    recovery.ambiguous_active_node_ids,
    vec!["publishInstagram".to_string()]
  );
  assert_eq!(recovery.pending_branches.len(), 1);
  assert_eq!(recovery.pending_branches[0].branch_id, "facebook");
}

#[test]
fn successful_join_requires_durable_branch_settlement_in_any_completion_order() {
  let database = TestDatabase::new("join");
  let workflow = model();
  let run_id = "run-fj4-join";
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .unwrap();
  admit_and_start(&mut store, &workflow, run_id);
  let mut sequence = 3;
  succeed_step(
    &mut store,
    run_id,
    &mut sequence,
    "prepare",
    json!({ "enabled": true }),
  );
  append(
    &mut store,
    run_id,
    sequence,
    RunEventPayload::ForkOpened(ForkOpenedData {
      fork_id: "distribution".to_string(),
    }),
  );
  sequence += 1;
  succeed_step(
    &mut store,
    run_id,
    &mut sequence,
    "publishFacebook",
    json!({ "platform": "facebook" }),
  );
  append(
    &mut store,
    run_id,
    sequence,
    RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
      fork_id: "distribution".to_string(),
      branch_id: "facebook".to_string(),
      terminal_node_id: "__woml_fork__distribution__facebook__terminal".to_string(),
      outcome: ForkBranchOutcome::Succeeded,
    }),
  );
  sequence += 1;
  succeed_step(
    &mut store,
    run_id,
    &mut sequence,
    "publishInstagram",
    json!({ "platform": "instagram" }),
  );
  append(
    &mut store,
    run_id,
    sequence,
    RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
      fork_id: "distribution".to_string(),
      branch_id: "instagram".to_string(),
      terminal_node_id: "__woml_fork__distribution__instagram__terminal".to_string(),
      outcome: ForkBranchOutcome::Succeeded,
    }),
  );
  sequence += 1;
  assert_eq!(
    store
      .fork_recovery_work_v1(run_id)
      .unwrap()
      .joinable_fork_ids,
    vec!["distribution".to_string()]
  );
  append(
    &mut store,
    run_id,
    sequence,
    RunEventPayload::ForkJoinSettled(ForkJoinSettledData {
      fork_id: "distribution".to_string(),
      outcome: ForkJoinOutcome::Succeeded,
      blocking_branch_id: None,
    }),
  );
  let projection = store.projection(run_id).unwrap();
  assert_eq!(
    projection.forks["distribution"].join_status,
    woml_engine::ForkJoinStatus::Succeeded
  );
}
