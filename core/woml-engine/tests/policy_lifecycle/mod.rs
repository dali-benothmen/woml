//! Policy-managed run-start ordering, warning, cancellation, and crash boundaries.

use super::{host_options, set_hook_source, HASH};
use serde_json::{json, Map};
use std::{path::PathBuf, time::Duration};
use woml_engine::{
  execute_admitted_trigger_run_durable, execute_workflow_durable,
  resume_workflow_durable_any_outcome, CompiledWorkflowDefinition, DurableEventStore,
  LifecycleActionIdentityData, LifecycleActionStatus, LifecycleEventName,
  LifecycleHookCompletedData, LifecycleHookCompletionStatus, RunEvent, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, TriggerAdmissionRequest, WorkflowRuntimeOutcome,
};

struct TestDatabase(PathBuf, PathBuf);

impl TestDatabase {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!(
      "woml-policy-lifecycle-{}",
      uuid::Uuid::new_v4().simple(),
    ));
    std::fs::create_dir(&directory).unwrap();
    Self(directory.join("state.sqlite"), directory)
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    // Managed services can create sibling stores; remove only this test's directory.
    let _ = std::fs::remove_dir_all(&self.1);
  }
}

fn policy_workflow(schema_version: u32) -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap();
  workflow.lifecycle.as_mut().unwrap().hooks.retain(|hook| {
    matches!(
      hook.event,
      LifecycleEventName::RunStart
        | LifecycleEventName::RunSuccess
        | LifecycleEventName::RunFailure
        | LifecycleEventName::RunCancel
        | LifecycleEventName::RunComplete
    )
  });
  let fixture: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  workflow.schema_version = schema_version;
  workflow.runtime_policy = fixture.runtime_policy;
  let policy = workflow.runtime_policy.as_mut().unwrap();
  policy.timeout_ms = None;
  policy.rate_limit = None;
  policy.concurrency = Some(1);
  for event in [
    LifecycleEventName::RunStart,
    LifecycleEventName::RunSuccess,
    LifecycleEventName::RunFailure,
    LifecycleEventName::RunCancel,
    LifecycleEventName::RunComplete,
  ] {
    set_hook_source(&mut workflow, event, "return;");
  }
  if schema_version == 21 {
    // Model v21 inherits structural descriptors from the intervening models.
    // Keep this a valid, explicitly local, two-step graph with final settlement.
    let mut value = serde_json::to_value(&workflow).unwrap();
    let graph = &mut value["graph"];
    for name in ["forks", "choices", "waits", "locks", "sagas"] {
      graph[name] = json!([]);
    }
    graph["contextVisibility"] = json!([
      { "nodeId": "prepare", "stepIds": [] },
      { "nodeId": "finish", "stepIds": ["prepare"] },
    ]);
    graph["settlement"] = json!({
      "nodeId": "__woml_workflow__settlement",
      "mainResultNodeId": "finish", "ownedBranchTerminalNodeIds": [],
    });
    graph["nodes"][0]["execution"] = json!({
      "profileVersion": 1, "target": "local", "requiredCapabilities": ["runtime.script"],
    });
    graph["nodes"].as_array_mut().unwrap().push(json!({
      "id": "__woml_workflow__settlement", "handler": "engine.workflow-settlement",
      "inputs": { "kind": "object", "fields": {} },
    }));
    graph["edges"].as_array_mut().unwrap().push(json!({
      "id": "finish-to-settlement", "from": "finish", "to": "__woml_workflow__settlement",
      "condition": { "kind": "always" },
    }));
    workflow = serde_json::from_value(value).unwrap();
  }
  workflow.validate_structure().unwrap();
  workflow
}

fn admit(store: &mut DurableEventStore, workflow: &CompiledWorkflowDefinition) -> String {
  store.register_definition(workflow, HASH).unwrap();
  store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "manual:policy-lifecycle".to_string(),
      payload: Map::from_iter([("orderId".to_string(), json!("policy-order"))]),
      received_at: chrono::Utc::now(),
    })
    .unwrap()
    .run_id
}

fn start_request(events: &[RunEvent]) -> &RunEvent {
  let requests: Vec<_> = events
    .iter()
    .filter(|event| {
      matches!(
        &event.payload, RunEventPayload::LifecycleHookRequested(hook)
          if hook.event == LifecycleEventName::RunStart
      )
    })
    .collect();
  assert_eq!(
    requests.len(),
    1,
    "the run needs exactly one durable start-hook request"
  );
  requests[0]
}

fn start_hook_id(events: &[RunEvent]) -> &str {
  let RunEventPayload::LifecycleHookRequested(request) = &start_request(events).payload else {
    unreachable!()
  };
  &request.hook_invocation_id
}

fn assert_start_settled_before_body(events: &[RunEvent]) {
  let hook_id = start_hook_id(events);
  let completed = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload, RunEventPayload::LifecycleHookCompleted(hook)
          if hook.hook_invocation_id == hook_id
      )
    })
    .expect("the start hook settles durably");
  let body = events
    .iter()
    .find(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
    .expect("the business DAG executes");
  assert!(completed.sequence < body.sequence);
}

#[tokio::test]
async fn policy_start_hook_runs_before_the_business_dag() {
  let Some(host) = host_options() else { return };
  for schema_version in [12, 21] {
    let database = TestDatabase::new();
    let mut workflow = policy_workflow(schema_version);
    set_hook_source(&mut workflow, LifecycleEventName::RunStart,
      "if (lifecycle.event !== 'run_start' || context.payload.orderId !== 'policy-order') throw new Error('wrong start binding'); await services.cache.set('policy-start-hook', { ready: true }); return { ignored: true };");
    let woml_engine::model::ValueExpression::Object { fields } =
      &mut workflow.graph.nodes[0].inputs
    else {
      unreachable!()
    };
    fields.insert("source".to_string(), woml_engine::model::ValueExpression::Literal {
      value: json!("const cached = await services.cache.get('policy-start-hook'); if (!cached.hit || !cached.value.ready) throw new Error('start observer did not run before the body'); return { ready: true, orderId: context.payload.orderId };"),
    });
    let result = execute_workflow_durable(
      workflow,
      HASH.to_string(),
      Map::from_iter([("orderId".to_string(), json!("policy-order"))]),
      RuntimeExecutionOptions::new(host.clone(), 5_000),
      database.0.clone(),
    )
    .await
    .unwrap();
    assert_eq!(result.result, json!({ "done": true }));
    assert!(!result.context.steps.contains_key("ignored"));
    assert_start_settled_before_body(&result.events);
    let request = start_request(&result.events);
    assert_eq!(request.sequence, 3);
    assert!(matches!(
      result.events[1].payload,
      RunEventPayload::RunExecutionStarted(_)
    ));
    let RunEventPayload::RunFinalized(finalized) = &result.events.last().unwrap().payload else {
      panic!("the run must finalize")
    };
    assert!(finalized.warnings.is_empty());
    let store = DurableEventStore::open(&database.0).unwrap();
    assert_eq!(store.events(&result.run_id).unwrap(), result.events);
  }
}

#[tokio::test]
async fn failed_policy_start_hook_is_a_warning_not_a_business_failure() {
  let Some(host) = host_options() else { return };
  let database = TestDatabase::new();
  let mut workflow = policy_workflow(21);
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunStart,
    "throw new Error('start observer failed');",
  );
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("policy-order"))]),
    RuntimeExecutionOptions::new(host, 5_000),
    database.0.clone(),
  )
  .await
  .unwrap();
  assert_eq!(result.result, json!({ "done": true }));
  assert_start_settled_before_body(&result.events);
  let RunEventPayload::RunFinalized(finalized) = &result.events.last().unwrap().payload else {
    panic!("the run must finalize")
  };
  assert_eq!(
    finalized.lifecycle_status,
    woml_engine::FinalLifecycleStatus::CompletedWithWarnings
  );
  assert_eq!(finalized.warnings.len(), 1);
  assert_eq!(finalized.warnings[0].hook_id, "lifecycle:run_start");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartCheckpoint {
  Requested,
  ActionStarted,
  ActionSucceeded,
  HookCompleted,
}

#[tokio::test]
async fn policy_start_hook_restart_respects_every_durable_action_checkpoint() {
  let Some(host) = host_options() else { return };
  for checkpoint in [
    StartCheckpoint::Requested,
    StartCheckpoint::ActionStarted,
    StartCheckpoint::ActionSucceeded,
    StartCheckpoint::HookCompleted,
  ] {
    let database = TestDatabase::new();
    let mut workflow = policy_workflow(21);
    if checkpoint != StartCheckpoint::Requested {
      set_hook_source(
        &mut workflow,
        LifecycleEventName::RunStart,
        "throw new Error('a completed or ambiguous start action must not be replayed');",
      );
    }
    let run_id;
    {
      let mut store = DurableEventStore::open(&database.0).unwrap();
      run_id = admit(&mut store, &workflow);
      let now = chrono::Utc::now();
      let claim = store
        .claim_policy_run(&run_id, "previous-owner", now, Duration::from_secs(30))
        .unwrap();
      let hook_id = start_hook_id(&store.events(&run_id).unwrap()).to_string();
      let identity = LifecycleActionIdentityData {
        hook_invocation_id: hook_id.clone(),
        action_id: "lifecycle:run_start:action:0".to_string(),
        attempt: 1,
      };
      let mut payloads = Vec::new();
      if checkpoint != StartCheckpoint::Requested {
        payloads.push(RunEventPayload::LifecycleActionAttemptStarted(
          identity.clone(),
        ));
      }
      if matches!(
        checkpoint,
        StartCheckpoint::ActionSucceeded | StartCheckpoint::HookCompleted
      ) {
        payloads.push(RunEventPayload::LifecycleActionSucceeded(identity));
      }
      if checkpoint == StartCheckpoint::HookCompleted {
        payloads.push(RunEventPayload::LifecycleHookCompleted(
          LifecycleHookCompletedData {
            hook_invocation_id: hook_id,
            status: LifecycleHookCompletionStatus::Completed,
            failed_actions: 0,
          },
        ));
      }
      for payload in payloads {
        store
          .append_payload(
            &run_id,
            format!("evt_{}", uuid::Uuid::new_v4().simple()),
            now,
            payload,
          )
          .unwrap();
      }
      store
        .release_policy_claim(&run_id, "previous-owner", &claim.claim_id)
        .unwrap();
    }
    let outcome = resume_workflow_durable_any_outcome(
      database.0.clone(),
      &run_id,
      RuntimeExecutionOptions::new(host.clone(), 5_000),
    )
    .await
    .unwrap();
    let WorkflowRuntimeOutcome::Succeeded { execution, .. } = outcome else {
      panic!("expected successful continuation from {checkpoint:?}")
    };
    assert_eq!(execution.result, json!({ "done": true }));
    assert_start_settled_before_body(&execution.events);
    let hook_id = start_hook_id(&execution.events);
    assert_eq!(
      execution
        .events
        .iter()
        .filter(|event| matches!(
          &event.payload, RunEventPayload::LifecycleActionAttemptStarted(action)
            if action.hook_invocation_id == hook_id
        ))
        .count(),
      1,
      "{checkpoint:?} must not duplicate the start action"
    );
    let failures: Vec<_> = execution
      .events
      .iter()
      .filter_map(|event| match &event.payload {
        RunEventPayload::LifecycleActionFailed(action) if action.hook_invocation_id == hook_id => {
          Some(action)
        }
        _ => None,
      })
      .collect();
    if checkpoint == StartCheckpoint::ActionStarted {
      assert_eq!(failures.len(), 1);
      assert_eq!(
        failures[0].failure.kind,
        woml_engine::LifecycleFailureKind::Interrupted
      );
    } else {
      assert!(
        failures.is_empty(),
        "{checkpoint:?} must not replay a completed action"
      );
    }
    let second = resume_workflow_durable_any_outcome(
      database.0.clone(),
      &run_id,
      RuntimeExecutionOptions::new(host.clone(), 5_000),
    )
    .await
    .unwrap();
    let WorkflowRuntimeOutcome::Succeeded {
      execution: resumed, ..
    } = second
    else {
      panic!("a finalized run remains successful")
    };
    assert_eq!(resumed.events, execution.events);
  }
}

#[tokio::test]
async fn cancellation_during_a_policy_start_hook_settles_it_without_running_the_body() {
  let Some(host) = host_options() else { return };
  let database = TestDatabase::new();
  let mut workflow = policy_workflow(21);
  set_hook_source(
    &mut workflow,
    LifecycleEventName::RunStart,
    "await new Promise(resolve => setTimeout(resolve, 20000));",
  );
  let mut store = DurableEventStore::open(&database.0).unwrap();
  let run_id = admit(&mut store, &workflow);
  let runtime_run_id = run_id.clone();
  let runtime_path = database.0.clone();
  let mut runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_path,
      &runtime_run_id,
      RuntimeExecutionOptions::new(host, 30_000),
    )
    .await
  });
  let observed = tokio::time::timeout(Duration::from_secs(10), async {
    loop {
      if store
        .projection(&run_id)
        .unwrap()
        .lifecycle_hooks
        .values()
        .any(|hook| {
          hook.event == LifecycleEventName::RunStart
            && hook
              .actions
              .values()
              .any(|action| action.status == LifecycleActionStatus::Started)
        })
      {
        break;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  })
  .await;
  if observed.is_err() {
    runtime.abort();
  }
  observed.expect("the policy-managed start action begins");
  store
    .request_run_cancellation(&run_id, "cancel_policy_start", chrono::Utc::now())
    .unwrap();
  let settled = tokio::time::timeout(Duration::from_secs(10), &mut runtime).await;
  if settled.is_err() {
    runtime.abort();
  }
  let error = settled
    .expect("start-hook cancellation settles")
    .unwrap()
    .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  assert_eq!(
    store.projection(&run_id).unwrap().status,
    RunStatus::Cancelled
  );
  let events = store.events(&run_id).unwrap();
  let hook_id = start_hook_id(&events);
  assert!(events.iter().any(|event| matches!(
    &event.payload, RunEventPayload::LifecycleActionFailed(action)
      if action.hook_invocation_id == hook_id && action.failure.kind == woml_engine::LifecycleFailureKind::Cancelled
  )));
  let failure_sequence = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload, RunEventPayload::LifecycleActionFailed(action)
          if action.hook_invocation_id == hook_id
      )
    })
    .unwrap()
    .sequence;
  let cancel_action = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload, RunEventPayload::LifecycleActionAttemptStarted(action)
          if action.action_id == "lifecycle:run_cancel:action:0"
      )
    })
    .expect("on-cancel executes after the start action settles");
  assert!(failure_sequence < cancel_action.sequence);
  assert!(!events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_))));
  assert!(matches!(
    events.last().unwrap().payload,
    RunEventPayload::RunFinalized(_)
  ));
}

#[tokio::test]
async fn cancellation_before_execution_skips_the_start_script_without_starting_business_work() {
  let Some(host) = host_options() else { return };
  for claimed in [false, true] {
    let database = TestDatabase::new();
    let mut workflow = policy_workflow(21);
    set_hook_source(
      &mut workflow,
      LifecycleEventName::RunStart,
      "throw new Error('the cancelled pending start observer must not execute');",
    );
    let mut store = DurableEventStore::open(&database.0).unwrap();
    let run_id = admit(&mut store, &workflow);
    let now = chrono::Utc::now();
    if claimed {
      let claim = store
        .claim_policy_run(&run_id, "previous-owner", now, Duration::from_secs(30))
        .unwrap();
      store
        .release_policy_claim(&run_id, "previous-owner", &claim.claim_id)
        .unwrap();
    }
    store
      .request_run_cancellation(&run_id, "cancel_before_execution", now)
      .unwrap();
    let error = execute_admitted_trigger_run_durable(
      database.0.clone(),
      &run_id,
      RuntimeExecutionOptions::new(host.clone(), 5_000),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
    let events = store.events(&run_id).unwrap();
    assert!(!events
      .iter()
      .any(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_))));
    if claimed {
      let hook_id = start_hook_id(&events);
      assert!(events.iter().any(|event| matches!(
        &event.payload, RunEventPayload::LifecycleActionFailed(action)
          if action.hook_invocation_id == hook_id
            && action.failure.kind == woml_engine::LifecycleFailureKind::Cancelled
      )));
    } else {
      assert!(!events
        .iter()
        .any(|event| matches!(event.payload, RunEventPayload::RunExecutionStarted(_))));
      assert!(!events.iter().any(|event| matches!(
        &event.payload, RunEventPayload::LifecycleHookRequested(hook)
          if hook.event == LifecycleEventName::RunStart
      )));
    }
    assert_eq!(
      store.projection(&run_id).unwrap().status,
      RunStatus::Cancelled
    );
    assert!(matches!(
      events.last().unwrap().payload,
      RunEventPayload::RunFinalized(_)
    ));
  }
}
