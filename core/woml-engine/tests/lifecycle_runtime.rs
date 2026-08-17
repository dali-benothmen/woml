use std::{collections::BTreeMap, path::PathBuf};

use serde_json::{json, Map};
use woml_engine::event::StepAttemptStartedData;
use woml_engine::{
  execute_workflow_durable, resume_workflow_durable_any_outcome, CompiledWorkflowDefinition,
  DurableEventStore, LifecycleActionIdentityData, LifecycleEventName, LifecycleHookCompletedData,
  LifecycleHookCompletionStatus, RunEventPayload, RunStartedData, RuntimeExecutionError,
  RuntimeExecutionOptions, ScriptHostProcessOptions, WorkflowRuntimeOutcome,
};

const HASH: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

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
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap();
  workflow.lifecycle.as_mut().unwrap().hooks.retain(|hook| {
    matches!(
      hook.event,
      LifecycleEventName::RunStart
        | LifecycleEventName::RunSuccess
        | LifecycleEventName::RunFailure
        | LifecycleEventName::RunComplete
    )
  });
  workflow.validate_structure().unwrap();
  workflow
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
  let woml_engine::model::ValueExpression::Object { fields } = &mut action.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(source),
    },
  );
}

#[tokio::test]
async fn workflow_hooks_execute_in_durable_order_and_ignore_return_values() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = workflow();
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunStart,
    "if (!Object.isFrozen(lifecycle) || !Object.isFrozen(lifecycle.workflow)) throw new Error('binding is mutable'); try { context.steps.injected = true; } catch {} await services.cache.set('lec3-hook', { ready: true }); const cached = await services.cache.get('lec3-hook'); if (!cached.hit || !cached.value.ready || secrets.LEC3_TOKEN !== 'safe-value') throw new Error('capability binding failed');",
  );
  workflow
    .lifecycle
    .as_mut()
    .unwrap()
    .hooks
    .iter_mut()
    .find(|hook| hook.event == LifecycleEventName::RunStart)
    .unwrap()
    .actions[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets = vec!["LEC3_TOKEN".to_string()];
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunSuccess,
    "return { thisValueIsIgnored: true };",
  );
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunComplete,
    "if (lifecycle.workflow.outcome !== 'succeeded') throw new Error('wrong outcome');",
  );
  let path = std::env::temp_dir().join(format!(
    "woml-lec3-success-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-1"))]),
    RuntimeExecutionOptions::new(host, 2_000).with_resolved_secrets(BTreeMap::from([(
      "LEC3_TOKEN".to_string(),
      "safe-value".to_string(),
    )])),
    path.clone(),
  )
  .await
  .unwrap();
  assert_eq!(result.result, json!({ "done": true }));
  assert!(!result.context.steps.contains_key("injected"));
  let kinds = result
    .events
    .iter()
    .map(|event| match event.payload {
      RunEventPayload::RunStarted(_) => "run_started",
      RunEventPayload::LifecycleHookRequested(_) => "hook_requested",
      RunEventPayload::LifecycleActionAttemptStarted(_) => "action_started",
      RunEventPayload::LifecycleActionSucceeded(_) => "action_succeeded",
      RunEventPayload::LifecycleHookCompleted(_) => "hook_completed",
      RunEventPayload::RunOutcomeDecided(_) => "outcome_decided",
      RunEventPayload::RunFinalized(_) => "run_finalized",
      RunEventPayload::StepAttemptStarted(_) => "step_started",
      RunEventPayload::StepAttemptSucceeded(_) => "step_succeeded",
      _ => "other",
    })
    .collect::<Vec<_>>();
  assert_eq!(kinds.first(), Some(&"run_started"));
  assert_eq!(kinds.get(1), Some(&"hook_requested"));
  assert_eq!(kinds.last(), Some(&"run_finalized"));
  assert_eq!(
    kinds
      .iter()
      .filter(|kind| **kind == "hook_completed")
      .count(),
    3
  );
  let finalized = result
    .events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::RunFinalized(data) => Some(data),
      _ => None,
    })
    .unwrap();
  assert!(
    finalized.warnings.is_empty(),
    "{:#?} / {:#?}",
    finalized.warnings,
    result
      .events
      .iter()
      .filter_map(|event| match &event.payload {
        RunEventPayload::LifecycleActionFailed(data) => Some(data),
        _ => None,
      })
      .collect::<Vec<_>>()
  );
  assert!(result
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::OperationSucceeded(_))));
  assert!(!serde_json::to_string(&result.events)
    .unwrap()
    .contains("safe-value"));
  let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn failed_outcome_hook_becomes_a_warning_without_changing_success() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = workflow();
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunSuccess,
    "throw new Error('hook failed safely');",
  );
  let path = std::env::temp_dir().join(format!(
    "woml-lec3-warning-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-2"))]),
    RuntimeExecutionOptions::new(host, 2_000),
    path.clone(),
  )
  .await
  .unwrap();
  assert_eq!(result.result, json!({ "done": true }));
  assert!(result
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::LifecycleActionFailed(_))));
  let final_event = result.events.last().unwrap();
  let RunEventPayload::RunFinalized(finalized) = &final_event.payload else {
    panic!("expected durable finalization")
  };
  assert_eq!(finalized.warnings.len(), 1, "{:#?}", finalized.warnings);
  let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn resume_executes_a_requested_but_never_started_hook_exactly_once() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = workflow();
  set_hook_source(&mut workflow, LifecycleEventName::RunStart, "return;");
  set_hook_source(&mut workflow, LifecycleEventName::RunSuccess, "return;");
  set_hook_source(&mut workflow, LifecycleEventName::RunComplete, "return;");
  let path = std::env::temp_dir().join(format!(
    "woml-lec3-resume-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  {
    let mut store = DurableEventStore::open(&path).unwrap();
    store.register_definition(&workflow, HASH).unwrap();
    store
      .append_payload(
        "run_lec3_resume",
        "event_start",
        chrono::Utc::now(),
        RunEventPayload::RunStarted(RunStartedData {
          workflow_id: workflow.workflow_id.clone(),
          definition_hash: HASH.to_string(),
          trigger_id: Some("start".to_string()),
          trigger_handler: Some("trigger.manual".to_string()),
          trigger_occurrence_id: Some("occ_lec3_resume".to_string()),
          ingress: None,
          trigger: Map::from_iter([("orderId".to_string(), json!("resume-order"))]),
        }),
      )
      .unwrap();
    assert_eq!(store.events("run_lec3_resume").unwrap().len(), 2);
  }
  let outcome = resume_workflow_durable_any_outcome(
    path.clone(),
    "run_lec3_resume",
    RuntimeExecutionOptions::new(host, 2_000),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Succeeded { execution, .. } = outcome else {
    panic!("expected resumed success")
  };
  assert_eq!(execution.result, json!({ "done": true }));
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::LifecycleHookRequested(_)))
      .count(),
    3
  );
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::LifecycleActionSucceeded(_)))
      .count(),
    3
  );
  let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn failed_business_run_executes_failure_then_complete_without_changing_outcome() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = workflow();
  set_hook_source(&mut workflow, LifecycleEventName::RunStart, "return;");
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunFailure,
    "if (!lifecycle.failure.code.startsWith('WOML_')) throw new Error('missing failure');",
  );
  set_hook_source(&mut workflow, LifecycleEventName::RunComplete, "return;");
  let first = workflow.graph.nodes.first_mut().unwrap();
  let woml_engine::model::ValueExpression::Object { fields } = &mut first.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!("throw new Error('business failed');"),
    },
  );
  let path = std::env::temp_dir().join(format!(
    "woml-lec3-business-failure-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("failed-order"))]),
    RuntimeExecutionOptions::new(host, 2_000),
    path.clone(),
  )
  .await
  .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunFailed(_)));
  let store = DurableEventStore::open(&path).unwrap();
  let run_id = store.list_runs(1).unwrap().runs[0].run_id.clone();
  let events = store.events(&run_id).unwrap();
  assert!(matches!(
    events.last().unwrap().payload,
    RunEventPayload::RunFinalized(_)
  ));
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::LifecycleHookCompleted(_)))
      .count(),
    3
  );
  let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn recovery_runs_failure_lifecycle_after_an_interrupted_business_attempt() {
  let Some(host) = host_options() else { return };
  let mut workflow = workflow();
  for event in [
    LifecycleEventName::RunStart,
    LifecycleEventName::RunFailure,
    LifecycleEventName::RunComplete,
  ] {
    set_hook_source(&mut workflow, event, "return;");
  }
  let path = std::env::temp_dir().join(format!(
    "woml-lec3-interrupted-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  {
    let mut store = DurableEventStore::open(&path).unwrap();
    store.register_definition(&workflow, HASH).unwrap();
    store
      .append_payload(
        "run_lec3_interrupted",
        "event_start",
        chrono::Utc::now(),
        RunEventPayload::RunStarted(RunStartedData {
          workflow_id: workflow.workflow_id.clone(),
          definition_hash: HASH.to_string(),
          trigger_id: Some("start".to_string()),
          trigger_handler: Some("trigger.manual".to_string()),
          trigger_occurrence_id: Some("occ_lec3_interrupted".to_string()),
          ingress: None,
          trigger: Map::new(),
        }),
      )
      .unwrap();
    let hook_invocation_id = store
      .projection("run_lec3_interrupted")
      .unwrap()
      .lifecycle_hooks
      .keys()
      .next()
      .unwrap()
      .clone();
    let action_id = "lifecycle:run_start:action:0".to_string();
    for payload in [
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id: action_id.clone(),
        attempt: 1,
      }),
      RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id,
        attempt: 1,
      }),
      RunEventPayload::LifecycleHookCompleted(LifecycleHookCompletedData {
        hook_invocation_id,
        status: LifecycleHookCompletionStatus::Completed,
        failed_actions: 0,
      }),
    ] {
      store
        .append_payload(
          "run_lec3_interrupted",
          format!("event_{}", uuid::Uuid::new_v4().simple()),
          chrono::Utc::now(),
          payload,
        )
        .unwrap();
    }
    store
      .append_payload(
        "run_lec3_interrupted",
        "event_business_start",
        chrono::Utc::now(),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: "prepare".to_string(),
          attempt: 1,
          invocation_id: "inv_lec3_interrupted".to_string(),
          handler: "runtime.script".to_string(),
          idempotency_key: Some(woml_engine::step_effect_idempotency_key(
            "run_lec3_interrupted",
            HASH,
            "prepare",
          )),
        }),
      )
      .unwrap();
  }
  let error = resume_workflow_durable_any_outcome(
    path.clone(),
    "run_lec3_interrupted",
    RuntimeExecutionOptions::new(host, 2_000),
  )
  .await
  .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunFailed(_)));
  let events = DurableEventStore::open(&path)
    .unwrap()
    .events("run_lec3_interrupted")
    .unwrap();
  assert!(matches!(
    events.last().unwrap().payload,
    RunEventPayload::RunFinalized(_)
  ));
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::LifecycleHookRequested(hook)
      if hook.event == LifecycleEventName::RunFailure
  )));
  let _ = std::fs::remove_file(path);
}
