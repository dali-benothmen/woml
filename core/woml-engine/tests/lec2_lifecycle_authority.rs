use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde_json::{json, Map};
use woml_engine::{
  BusinessOutcome, CompiledWorkflowDefinition, DurableEventStore, InspectedBusinessOutcome,
  LifecycleActionFailedData, LifecycleActionIdentityData, LifecycleFailure, LifecycleFailureKind,
  LifecycleHookCompletedData, LifecycleHookCompletionStatus, LifecycleStatus, PublicRunStatus,
  RunCancellationStatus, RunEventPayload, RunIngress, RunOutcomeDecidedData, RunStartedData,
  RunStatus, RunSucceededData, ScriptRuntimeBindings, DURABLE_STORE_SCHEMA_VERSION,
};

const DEFINITION_HASH: &str =
  "sha256:1111111111111111111111111111111111111111111111111111111111111111";

fn lifecycle_model() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .expect("fixture parses")
}

fn lifecycle_free_model_v11() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/hello.compiled.v1.json"
  ))
  .unwrap();
  workflow.schema_version = 11;
  for node in &mut workflow.graph.nodes {
    if node.handler == "runtime.script" {
      node.script_runtime = Some(ScriptRuntimeBindings {
        binding_version: 1,
        bindings: vec![
          "context".to_string(),
          "attempt".to_string(),
          "services".to_string(),
          "secrets".to_string(),
        ],
        required_secrets: Vec::new(),
      });
    }
  }
  workflow
}

fn legacy_model_v1() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/hello.compiled.v1.json"
  ))
  .unwrap()
}

fn started_store() -> DurableEventStore {
  let mut store = DurableEventStore::open_in_memory().expect("store opens");
  let workflow = lifecycle_model();
  workflow.validate_structure().expect("Model v11 validates");
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .expect("definition registers");
  store
    .append_payload(
      "run_lec2",
      "event_start",
      Utc.with_ymd_and_hms(2026, 8, 11, 10, 0, 0).unwrap(),
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: workflow.workflow_id,
        definition_hash: DEFINITION_HASH.to_string(),
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        ingress: Some(RunIngress::WorkflowCall {
          call_key: "sha256:2222222222222222222222222222222222222222222222222222222222222222"
            .to_string(),
        }),
        trigger: Map::new(),
      }),
    )
    .expect("run starts");
  store
}

#[test]
fn model_v11_maps_to_event_v10_and_store_v11() {
  assert_eq!(DURABLE_STORE_SCHEMA_VERSION, 11);
  assert_eq!(woml_engine::run_event_schema_version_for_model(11), 10);
  assert_eq!(
    started_store().events("run_lec2").unwrap()[0].event_schema_version,
    10
  );
}

#[test]
fn lifecycle_free_v11_terminal_admission_uses_outcome_and_finalized_events() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let workflow = lifecycle_free_model_v11();
  workflow.validate_structure().unwrap();
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .unwrap();
  let at = Utc.with_ymd_and_hms(2026, 8, 11, 10, 0, 0).unwrap();
  store
    .append_payload(
      "run_v11_no_hooks",
      "event_start",
      at,
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: workflow.workflow_id,
        definition_hash: DEFINITION_HASH.to_string(),
        trigger_id: Some("start".to_string()),
        trigger_handler: Some("trigger.manual".to_string()),
        trigger_occurrence_id: Some("occ_v11".to_string()),
        ingress: None,
        trigger: Map::new(),
      }),
    )
    .unwrap();
  store
    .append_payload(
      "run_v11_no_hooks",
      "legacy_success_boundary",
      at,
      RunEventPayload::RunSucceeded(RunSucceededData {
        terminal_node_id: "b".to_string(),
        result: json!({ "message": "Hello World" }),
      }),
    )
    .unwrap();
  let events = store.events("run_v11_no_hooks").unwrap();
  assert_eq!(events.len(), 3);
  assert!(matches!(
    events[1].payload,
    RunEventPayload::RunOutcomeDecided(_)
  ));
  assert!(matches!(
    events[2].payload,
    RunEventPayload::RunFinalized(_)
  ));
  assert_eq!(
    store.projection("run_v11_no_hooks").unwrap().status,
    RunStatus::Succeeded
  );
}

#[test]
fn store_v10_migrates_to_v11_and_rebuilds_summary_rows_from_events() {
  let path = std::env::temp_dir().join(format!(
    "woml-lec2-store-migration-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let at = Utc.with_ymd_and_hms(2026, 8, 11, 10, 0, 0).unwrap();
  {
    let mut store = DurableEventStore::open(&path).unwrap();
    let workflow = legacy_model_v1();
    store
      .register_definition(&workflow, DEFINITION_HASH)
      .unwrap();
    store
      .append_payload(
        "run_before_v11",
        "event_start",
        at,
        RunEventPayload::RunStarted(RunStartedData {
          workflow_id: workflow.workflow_id,
          definition_hash: DEFINITION_HASH.to_string(),
          trigger_id: None,
          trigger_handler: None,
          trigger_occurrence_id: None,
          ingress: None,
          trigger: Map::new(),
        }),
      )
      .unwrap();
  }
  {
    let connection = Connection::open(&path).unwrap();
    connection
      .execute("DROP TABLE woml_run_summaries", [])
      .unwrap();
    connection
      .execute(
        "UPDATE woml_store_metadata SET value = '10' WHERE key = 'schema_version'",
        [],
      )
      .unwrap();
  }
  let store = DurableEventStore::open(&path).unwrap();
  let list = store.list_runs(10).unwrap();
  assert_eq!(list.runs.len(), 1);
  assert_eq!(list.runs[0].run_id, "run_before_v11");
  assert_eq!(list.runs[0].status, PublicRunStatus::Running);
  drop(store);
  let version: String = Connection::open(&path)
    .unwrap()
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, "11");
  std::fs::remove_file(path).unwrap();
}

#[test]
fn store_v11_rejects_a_corrupt_run_summary_shape() {
  let path = std::env::temp_dir().join(format!(
    "woml-lec2-store-corrupt-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  DurableEventStore::open(&path).unwrap();
  {
    let connection = Connection::open(&path).unwrap();
    connection
      .execute("DROP TABLE woml_run_summaries", [])
      .unwrap();
    connection
      .execute_batch(
        "CREATE TABLE woml_run_summaries (
           run_id TEXT PRIMARY KEY,
           workflow_id TEXT NOT NULL,
           status TEXT NOT NULL,
           started_at TEXT NOT NULL
         );
         CREATE INDEX woml_run_summaries_updated
           ON woml_run_summaries(started_at DESC, run_id DESC);",
      )
      .unwrap();
  }
  let error = match DurableEventStore::open(&path) {
    Ok(_) => panic!("corrupt Store v11 must fail closed"),
    Err(error) => error,
  };
  assert!(error
    .to_string()
    .contains("woml_run_summaries columns do not match the frozen schema"));
  std::fs::remove_file(path).unwrap();
}

#[test]
fn cancellation_and_hook_admission_are_atomic_and_idempotent() {
  let mut store = started_store();
  let at = Utc.with_ymd_and_hms(2026, 8, 11, 10, 1, 0).unwrap();
  let accepted = store
    .request_run_cancellation("run_lec2", "cancel_lec2", at)
    .expect("cancellation commits");
  assert_eq!(accepted.status, RunCancellationStatus::Accepted);

  let events = store.events("run_lec2").unwrap();
  assert!(matches!(
    events[1].payload,
    RunEventPayload::RunCancellationRequested(_)
  ));
  assert!(matches!(
    events[2].payload,
    RunEventPayload::LifecycleHookRequested(_)
  ));
  let projection = store.projection("run_lec2").unwrap();
  assert_eq!(projection.status, RunStatus::Cancelling);
  assert_eq!(projection.lifecycle_status, LifecycleStatus::Running);
  assert_eq!(projection.lifecycle_hooks.len(), 1);

  let duplicate = store
    .request_run_cancellation("run_lec2", "cancel_again", at)
    .expect("duplicate is classified");
  assert_eq!(duplicate.status, RunCancellationStatus::AlreadyRequested);
  assert_eq!(store.events("run_lec2").unwrap().len(), 3);
}

#[test]
fn outcome_decision_and_outcome_hook_admission_share_one_authority_commit() {
  let mut store = started_store();
  let at = Utc.with_ymd_and_hms(2026, 8, 11, 10, 1, 0).unwrap();
  let projection = store
    .decide_run_outcome(
      "run_lec2",
      RunOutcomeDecidedData::Succeeded {
        result: json!({ "message": "done" }),
      },
      at,
    )
    .unwrap();
  assert_eq!(projection.status, RunStatus::Finalizing);
  assert_eq!(
    projection.business_outcome,
    Some(BusinessOutcome::Succeeded)
  );
  assert!(projection
    .lifecycle_hooks
    .values()
    .any(|hook| hook.hook_id == "lifecycle:run_success"));
  let events = store.events("run_lec2").unwrap();
  assert!(matches!(
    events[1].payload,
    RunEventPayload::RunOutcomeDecided(_)
  ));
  assert!(matches!(
    events[2].payload,
    RunEventPayload::LifecycleHookRequested(_)
  ));
}

#[test]
fn lifecycle_projection_finalizes_without_leaking_into_summary() {
  let mut store = started_store();
  let at = Utc.with_ymd_and_hms(2026, 8, 11, 10, 1, 0).unwrap();
  store
    .request_run_cancellation("run_lec2", "cancel_lec2", at)
    .unwrap();
  let hook_invocation_id = store
    .projection("run_lec2")
    .unwrap()
    .lifecycle_hooks
    .keys()
    .next()
    .unwrap()
    .clone();
  let action_id = "lifecycle:run_cancel:action:0".to_string();
  store
    .append_payload(
      "run_lec2",
      "event_action_started",
      at,
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id: action_id.clone(),
        attempt: 1,
      }),
    )
    .unwrap();
  store
    .append_payload(
      "run_lec2",
      "event_action_failed",
      at,
      RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id,
        attempt: 1,
        failure: LifecycleFailure {
          kind: LifecycleFailureKind::Interrupted,
          code: "WOML_LIFECYCLE_ACTION_INTERRUPTED".to_string(),
          message: "Action outcome is ambiguous and is not replayed.".to_string(),
        },
      }),
    )
    .unwrap();
  store
    .append_payload(
      "run_lec2",
      "event_hook_completed",
      at,
      RunEventPayload::LifecycleHookCompleted(LifecycleHookCompletedData {
        hook_invocation_id,
        status: LifecycleHookCompletionStatus::CompletedWithWarnings,
        failed_actions: 1,
      }),
    )
    .unwrap();
  store
    .append_payload(
      "run_lec2",
      "event_outcome",
      at,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Cancelled {
        cancellation_request_id: "cancel_lec2".to_string(),
      }),
    )
    .unwrap();
  store.finalize_run_v11("run_lec2", at).unwrap();

  let inspection = store.inspect_run_v2("run_lec2").unwrap();
  assert_eq!(inspection.status, PublicRunStatus::Cancelled);
  assert_eq!(
    inspection.business_outcome,
    InspectedBusinessOutcome::Cancelled
  );
  assert_eq!(inspection.hooks[0].failed_actions, 1);
  assert_eq!(inspection.warnings.len(), 1);
  let list = store.list_runs(10).unwrap();
  assert_eq!(list.runs[0].status, PublicRunStatus::Cancelled);
  assert_eq!(
    serde_json::to_value(list).unwrap()["runs"][0]["runId"],
    json!("run_lec2")
  );

  store.rebuild_run_summaries().unwrap();
  assert_eq!(
    store.list_runs(10).unwrap().runs[0].status,
    PublicRunStatus::Cancelled
  );
}

#[test]
fn recovery_fails_an_ambiguous_lifecycle_action_closed_without_replay() {
  let mut store = started_store();
  let at = Utc.with_ymd_and_hms(2026, 8, 11, 10, 1, 0).unwrap();
  store
    .request_run_cancellation("run_lec2", "cancel_lec2", at)
    .unwrap();
  let hook_invocation_id = store
    .projection("run_lec2")
    .unwrap()
    .lifecycle_hooks
    .keys()
    .next()
    .unwrap()
    .clone();
  store
    .append_payload(
      "run_lec2",
      "event_action_started",
      at,
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id: "lifecycle:run_cancel:action:0".to_string(),
        attempt: 1,
      }),
    )
    .unwrap();

  let report = store.recover_interrupted_runs().unwrap();
  assert_eq!(report.recovered_runs, 1);
  let projection = store.projection("run_lec2").unwrap();
  let hook = projection.lifecycle_hooks.get(&hook_invocation_id).unwrap();
  let action = hook.actions.get("lifecycle:run_cancel:action:0").unwrap();
  assert_eq!(action.status, woml_engine::LifecycleActionStatus::Failed);
  assert_eq!(
    action.failure.as_ref().unwrap().kind,
    LifecycleFailureKind::Interrupted
  );
  assert_eq!(
    hook.status,
    woml_engine::LifecycleHookStatus::CompletedWithWarnings
  );
  assert_eq!(
    store
      .events("run_lec2")
      .unwrap()
      .iter()
      .filter(|event| matches!(
        event.payload,
        RunEventPayload::LifecycleActionAttemptStarted(_)
      ))
      .count(),
    1,
    "recovery must never replay the ambiguous action"
  );
}
