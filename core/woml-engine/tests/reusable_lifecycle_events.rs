use std::path::PathBuf;

use woml_engine::{
  fold_events, run_event_schema_version_for_model, ReusableLifecycleOutcome,
  ReusableLifecycleStatus, RunEvent, RunEventPayload, COMPILED_MODEL_SCHEMA_VERSION_V14,
  RUN_EVENT_SCHEMA_VERSION_V13,
};

fn fixture_path(name: &str) -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../woml/tests/fixtures/reusable-definitions")
    .join(name)
}

#[test]
fn event_v13_folds_reusable_lifecycle_warnings_without_operation_data() {
  let source = std::fs::read_to_string(fixture_path("event-history.v13.json")).unwrap();
  let events: Vec<RunEvent> = serde_json::from_str(&source).unwrap();
  let projection = fold_events(&events).unwrap();
  let hook = projection.reusable_lifecycle_hooks.values().next().unwrap();

  assert_eq!(events.len(), 3);
  assert_eq!(hook.invocation_id, "discount");
  assert_eq!(hook.status, ReusableLifecycleStatus::CompletedWithWarnings);
  assert_eq!(
    hook.warning_codes,
    vec!["WOML_REUSABLE_LIFECYCLE_ACTION_FAILED"]
  );
  let encoded = serde_json::to_string(&events).unwrap();
  assert!(!encoded.contains("props"));
  assert!(!encoded.contains("context"));
  assert!(!encoded.contains("result"));
}

#[test]
fn event_v13_rejects_a_success_event_with_a_failed_outcome() {
  let source = std::fs::read_to_string(fixture_path("event-history.v13.json")).unwrap();
  let mut events: Vec<RunEvent> = serde_json::from_str(&source).unwrap();
  let failed = events.remove(1);
  let RunEventPayload::ReusableLifecycleActionFailed(data) = &failed.payload else {
    panic!("fixture action must be failed");
  };
  let invalid = RunEvent {
    payload: RunEventPayload::ReusableLifecycleActionSucceeded(
      woml_engine::ReusableLifecycleActionSucceededData {
        invocation_id: data.invocation_id.clone(),
        definition_digest: data.definition_digest.clone(),
        hook: data.hook,
        action_id: data.action_id.clone(),
        outcome: ReusableLifecycleOutcome::Failed,
      },
    ),
    ..failed
  };
  assert!(invalid.validate().is_err());
}

#[test]
fn compiled_model_v14_uses_event_v13() {
  assert_eq!(
    run_event_schema_version_for_model(COMPILED_MODEL_SCHEMA_VERSION_V14),
    RUN_EVENT_SCHEMA_VERSION_V13
  );
}
