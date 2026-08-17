use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::event::StepAttemptFailedData;
use woml_engine::model::{BackoffPolicy, RetryPolicy, ValueExpression};
use woml_engine::{
  execute_workflow_durable, resume_workflow_durable_any_outcome, AttemptFailure,
  AttemptFailureKind, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  LifecycleEventName, RunEvent, RunEventPayload, RuntimeExecutionOptions, ScriptHostProcessOptions,
  TriggerAdmissionRequest,
};

const HASH: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-lec4-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
  }
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

fn workflow() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap()
}

fn set_node_source(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
}

fn set_hook_source(
  workflow: &mut CompiledWorkflowDefinition,
  event: LifecycleEventName,
  source: &str,
) {
  let action = &mut workflow
    .lifecycle
    .as_mut()
    .unwrap()
    .hooks
    .iter_mut()
    .find(|hook| hook.event == event)
    .unwrap()
    .actions[0];
  let ValueExpression::Object { fields } = &mut action.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
}

fn retain_step_hooks(workflow: &mut CompiledWorkflowDefinition) {
  let lifecycle = workflow.lifecycle.as_mut().unwrap();
  lifecycle.hooks.retain(|hook| hook.event.is_step());
  let mut script_action = lifecycle
    .hooks
    .iter()
    .find(|hook| hook.event == LifecycleEventName::StepComplete)
    .unwrap()
    .actions[0]
    .clone();
  script_action.action_id = "lifecycle:step_failure:action:0".to_string();
  lifecycle
    .hooks
    .iter_mut()
    .find(|hook| hook.event == LifecycleEventName::StepFailure)
    .unwrap()
    .actions = vec![script_action];
}

fn filter_step_hooks(workflow: &mut CompiledWorkflowDefinition, step_id: &str) {
  for hook in &mut workflow.lifecycle.as_mut().unwrap().hooks {
    hook.step_ids = Some(vec![step_id.to_string()]);
  }
}

fn hook_requests(events: &[RunEvent], event: LifecycleEventName, step_id: &str) -> usize {
  events
    .iter()
    .filter(|entry| {
      matches!(
        &entry.payload,
        RunEventPayload::LifecycleHookRequested(request)
          if request.event == event && request.subject.id == step_id
      )
    })
    .count()
}

#[tokio::test]
async fn step_hooks_apply_filters_and_receive_the_frozen_settlement_snapshot() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("success");
  let mut workflow = workflow();
  retain_step_hooks(&mut workflow);
  set_hook_source(
    &mut workflow,
    LifecycleEventName::StepStart,
    "if (!lifecycle.step || lifecycle.step.attempts !== 1 || lifecycle.step.outcome !== undefined) throw new Error('bad start binding');",
  );
  set_hook_source(
    &mut workflow,
    LifecycleEventName::StepSuccess,
    "if (lifecycle.step.id !== 'prepare' || lifecycle.step.outcome !== 'succeeded' || lifecycle.step.attempts !== 1 || !context.steps.prepare.ready) throw new Error('bad success binding');",
  );
  set_hook_source(
    &mut workflow,
    LifecycleEventName::StepComplete,
    "if (lifecycle.step.outcome !== 'succeeded' || lifecycle.step.attempts !== 1) throw new Error('bad complete binding');",
  );
  workflow.validate_structure().unwrap();

  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-1"))]),
    RuntimeExecutionOptions::new(host, 2_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepStart, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepStart, "finish"),
    1
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepSuccess, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepSuccess, "finish"),
    0
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepComplete, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepComplete, "finish"),
    1
  );
  assert!(matches!(
    result.events.last().unwrap().payload,
    RunEventPayload::RunFinalized(_)
  ));
}

#[tokio::test]
async fn retries_do_not_duplicate_logical_step_hooks() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("retry");
  let mut workflow = workflow();
  retain_step_hooks(&mut workflow);
  filter_step_hooks(&mut workflow, "prepare");
  workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "prepare")
    .unwrap()
    .retry_policy = Some(RetryPolicy {
    max_attempts: 2,
    backoff: BackoffPolicy::Fixed { delay_ms: 1 },
  });
  set_node_source(
    &mut workflow,
    "prepare",
    "if (attempt.number === 1) throw new Error('retry me'); return { ready: true, orderId: context.payload.orderId };",
  );
  set_hook_source(
    &mut workflow,
    LifecycleEventName::StepSuccess,
    "if (lifecycle.step.attempts !== 2) throw new Error('hook observed an attempt instead of the logical step');",
  );
  set_hook_source(
    &mut workflow,
    LifecycleEventName::StepComplete,
    "if (lifecycle.step.attempts !== 2 || lifecycle.step.outcome !== 'succeeded') throw new Error('bad retry settlement');",
  );
  workflow.validate_structure().unwrap();

  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-2"))]),
    RuntimeExecutionOptions::new(host, 2_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepStart, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepSuccess, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepFailure, "prepare"),
    0
  );
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepComplete, "prepare"),
    1
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(ref data) if data.node_id == "prepare"))
      .count(),
    2
  );
}

#[tokio::test]
async fn restart_recovers_one_logical_failure_without_duplicate_hook_admission() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("recovery");
  let mut workflow = workflow();
  retain_step_hooks(&mut workflow);
  filter_step_hooks(&mut workflow, "prepare");
  workflow.validate_structure().unwrap();
  let now = Utc::now();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&workflow, HASH).unwrap();
  let admitted = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: format!("lec4:{}", Uuid::new_v4().simple()),
      payload: Map::from_iter([("orderId".to_string(), json!("order-3"))]),
      received_at: now,
    })
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &admitted.run_id).unwrap();
  engine
    .start_step_attempt(&admitted.run_id, "prepare", 1, "inv_interrupted", now)
    .unwrap();
  drop(engine.into_store());

  let outcome = resume_workflow_durable_any_outcome(
    database.path().to_path_buf(),
    &admitted.run_id,
    RuntimeExecutionOptions::new(host, 2_000),
  )
  .await;
  assert!(outcome.is_err());
  let events = DurableEventStore::open(database.path())
    .unwrap()
    .events(&admitted.run_id)
    .unwrap();
  assert_eq!(
    hook_requests(&events, LifecycleEventName::StepStart, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&events, LifecycleEventName::StepFailure, "prepare"),
    1
  );
  assert_eq!(
    hook_requests(&events, LifecycleEventName::StepComplete, "prepare"),
    1
  );
  assert!(matches!(
    events.last().unwrap().payload,
    RunEventPayload::RunFinalized(_)
  ));
}

#[tokio::test]
async fn engine_cancelled_step_gets_complete_without_permanent_failure_hook() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("cancelled-step");
  let mut workflow = workflow();
  retain_step_hooks(&mut workflow);
  filter_step_hooks(&mut workflow, "prepare");
  workflow.validate_structure().unwrap();
  let now = Utc::now();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&workflow, HASH).unwrap();
  let admitted = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: format!("lec4-cancel:{}", Uuid::new_v4().simple()),
      payload: Map::from_iter([("orderId".to_string(), json!("order-4"))]),
      received_at: now,
    })
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &admitted.run_id).unwrap();
  engine
    .start_step_attempt(&admitted.run_id, "prepare", 1, "inv_cancelled", now)
    .unwrap();
  engine
    .record_step_attempt_failure(
      &admitted.run_id,
      now,
      StepAttemptFailedData {
        node_id: "prepare".to_string(),
        attempt: 1,
        invocation_id: "inv_cancelled".to_string(),
        failure: AttemptFailure {
          kind: AttemptFailureKind::InvocationCancelled,
          code: AttemptFailureKind::InvocationCancelled.code().to_string(),
          message: "cancelled by fail-fast sibling settlement".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      },
    )
    .unwrap();
  drop(engine.into_store());

  let outcome = resume_workflow_durable_any_outcome(
    database.path().to_path_buf(),
    &admitted.run_id,
    RuntimeExecutionOptions::new(host, 2_000),
  )
  .await;
  assert!(outcome.is_err());
  let events = DurableEventStore::open(database.path())
    .unwrap()
    .events(&admitted.run_id)
    .unwrap();
  assert_eq!(
    hook_requests(&events, LifecycleEventName::StepFailure, "prepare"),
    0
  );
  assert_eq!(
    hook_requests(&events, LifecycleEventName::StepComplete, "prepare"),
    1
  );
}

#[tokio::test]
async fn step_hook_failure_becomes_a_warning_without_changing_business_output() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("hook-warning");
  let mut workflow = workflow();
  retain_step_hooks(&mut workflow);
  filter_step_hooks(&mut workflow, "prepare");
  set_hook_source(
    &mut workflow,
    LifecycleEventName::StepSuccess,
    "throw new Error('observer failed');",
  );
  workflow.validate_structure().unwrap();

  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-5"))]),
    RuntimeExecutionOptions::new(host, 2_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result, json!({ "done": true }));
  let finalized = result
    .events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::RunFinalized(data) => Some(data),
      _ => None,
    })
    .unwrap();
  assert_eq!(finalized.warnings.len(), 1);
  assert_eq!(finalized.warnings[0].step_id.as_deref(), Some("prepare"));
  assert_eq!(
    hook_requests(&result.events, LifecycleEventName::StepComplete, "prepare"),
    1
  );
}
