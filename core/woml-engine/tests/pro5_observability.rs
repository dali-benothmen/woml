use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};
use woml_engine::event::{StepAttemptFailedData, StepAttemptSucceededData};
use woml_engine::{
  AttemptFailure, AttemptFailureKind, CompiledWorkflowDefinition, DurableDagEngine,
  DurableEventStore, RunEventPayload, StepFailureDisposition, TriggerAdmissionRequest,
};

const RETRY_HASH: &str = "sha256:27606cefeebc5b6d45c965969b621a2f74ae2ebebe2b94edec80d97bfeb8378c";
const POLICY_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn retry_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/retry.compiled.v6.json"
  ))
  .unwrap()
}

fn policy_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap()
}

#[test]
fn durable_observation_survives_process_memory_and_contains_no_payloads() {
  let started_at = Utc.with_ymd_and_hms(2026, 8, 12, 9, 0, 0).unwrap();
  let store = DurableEventStore::open_in_memory().unwrap();
  let mut engine = DurableDagEngine::new(retry_model(), RETRY_HASH, store).unwrap();
  let run_id = "run_pro5_retry";
  engine
    .start_run("event_run", run_id, started_at, Map::new())
    .unwrap();
  engine
    .start_step_attempt(run_id, "prepare", 1, "invoke_prepare", started_at)
    .unwrap();
  engine
    .append_payload(
      "event_prepare",
      run_id,
      started_at,
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: "prepare".to_string(),
        attempt: 1,
        invocation_id: "invoke_prepare".to_string(),
        output: json!({ "name": "must-not-appear-in-observation" }),
      }),
    )
    .unwrap();
  engine
    .start_step_attempt(run_id, "greet", 1, "invoke_greet", started_at)
    .unwrap();
  let failure = engine
    .record_step_attempt_failure(
      run_id,
      started_at,
      StepAttemptFailedData {
        node_id: "greet".to_string(),
        attempt: 1,
        invocation_id: "invoke_greet".to_string(),
        failure: AttemptFailure {
          kind: AttemptFailureKind::ScriptThrew,
          code: AttemptFailureKind::ScriptThrew.code().to_string(),
          message: "private error detail".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      },
    )
    .unwrap();
  assert!(matches!(
    failure.disposition,
    StepFailureDisposition::RetryScheduled { .. }
  ));

  let mut store = engine.into_store();
  store
    .register_definition(&policy_model(), POLICY_HASH)
    .unwrap();
  store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: "policy-demo".to_string(),
      definition_hash: POLICY_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "manual:pro5".to_string(),
      payload: Map::from_iter([(
        "orderId".to_string(),
        Value::String("private-order".to_string()),
      )]),
      received_at: started_at,
    })
    .unwrap();

  let observation = store.runtime_observation_v1().unwrap();
  assert_eq!(observation.profile, "woml.runtime-observation/v1");
  assert_eq!(observation.retries_total, 1);
  assert_eq!(observation.retrying_run_ids, [run_id]);
  assert_eq!(observation.triggers_total, 1);
  assert_eq!(observation.status_totals.get("queued"), Some(&1));
  let encoded = serde_json::to_string(&observation).unwrap();
  assert!(!encoded.contains("must-not-appear-in-observation"));
  assert!(!encoded.contains("private error detail"));
  assert!(!encoded.contains("private-order"));
}
