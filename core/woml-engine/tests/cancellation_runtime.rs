use std::path::PathBuf;

use chrono::Utc;
use serde_json::{json, Map};
use woml_engine::event::StepAttemptStartedData;
use woml_engine::model::{BackoffPolicy, RetryPolicy, ValueExpression};
use woml_engine::{
  execute_admitted_trigger_run_durable, execute_workflow_durable_outcome,
  resume_workflow_durable_any_outcome, AttemptFailureKind, BusinessOutcome, CapabilityFailureKind,
  CompiledWorkflowDefinition, DurableEventStore, DurableStoreError, LifecycleEventName,
  OperationExecutionMode, OperationStartedData, OperationStatus, RunEventPayload, RunStartedData,
  RunStatus, RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
  ScriptRuntimeBindings, WorkflowRuntimeOutcome,
};

const HASH: &str = "sha256:6666666666666666666666666666666666666666666666666666666666666666";

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

fn lifecycle_workflow() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap();
  workflow.lifecycle.as_mut().unwrap().hooks.retain(|hook| {
    matches!(
      hook.event,
      LifecycleEventName::StepComplete
        | LifecycleEventName::RunCancel
        | LifecycleEventName::RunComplete
    )
  });
  for hook in &mut workflow.lifecycle.as_mut().unwrap().hooks {
    let action = &mut hook.actions[0];
    let woml_engine::model::ValueExpression::Object { fields } = &mut action.inputs else {
      unreachable!()
    };
    fields.insert(
      "source".to_string(),
      woml_engine::model::ValueExpression::Literal {
        value: json!("return null;"),
      },
    );
  }
  let prepare = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "prepare")
    .unwrap();
  let woml_engine::model::ValueExpression::Object { fields } = &mut prepare.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(
        "await new Promise(resolve => setTimeout(resolve, 5000)); return { ready: true };"
      ),
    },
  );
  workflow.validate_structure().unwrap();
  workflow
}

fn active_lifecycle_workflow() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap();
  workflow.lifecycle.as_mut().unwrap().hooks.retain(|hook| {
    matches!(
      hook.event,
      LifecycleEventName::RunStart
        | LifecycleEventName::RunCancel
        | LifecycleEventName::RunComplete
    )
  });
  for hook in &mut workflow.lifecycle.as_mut().unwrap().hooks {
    let action = &mut hook.actions[0];
    let ValueExpression::Object { fields } = &mut action.inputs else {
      unreachable!()
    };
    fields.insert(
      "source".to_string(),
      ValueExpression::Literal {
        value: if hook.event == LifecycleEventName::RunStart {
          json!("await new Promise(resolve => setTimeout(resolve, 5000)); return null;")
        } else {
          json!("return null;")
        },
      },
    );
  }
  workflow.validate_structure().unwrap();
  workflow
}

fn lifecycle_free_workflow() -> CompiledWorkflowDefinition {
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
  workflow.validate_structure().unwrap();
  workflow
}

fn approval_workflow() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/approval.compiled.v4.json"
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
  workflow.validate_structure().unwrap();
  workflow
}

fn parallel_workflow() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/parallel.compiled.v3.json"
  ))
  .unwrap();
  workflow.schema_version = 11;
  for node in &mut workflow.graph.nodes {
    if node.handler != "runtime.script" {
      continue;
    }
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
    if matches!(node.id.as_str(), "loadWeather" | "loadSoil") {
      let ValueExpression::Object { fields } = &mut node.inputs else {
        unreachable!()
      };
      fields.insert(
        "source".to_string(),
        ValueExpression::Literal {
          value: json!(
            "await new Promise(resolve => setTimeout(resolve, 5000)); return { done: true };"
          ),
        },
      );
    }
  }
  workflow.validate_structure().unwrap();
  workflow
}

fn start_known_run(
  store: &mut DurableEventStore,
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
) {
  store.register_definition(workflow, HASH).unwrap();
  store
    .append_payload(
      run_id,
      format!("event_start_{run_id}"),
      Utc::now(),
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: workflow.workflow_id.clone(),
        definition_hash: HASH.to_string(),
        trigger_id: Some("start".to_string()),
        trigger_handler: Some("trigger.manual".to_string()),
        trigger_occurrence_id: Some(format!("occ_{run_id}")),
        ingress: None,
        trigger: Map::new(),
      }),
    )
    .unwrap();
}

#[tokio::test]
async fn active_script_is_signalled_and_run_finalizes_cancelled() {
  let Some(host) = host_options() else {
    return;
  };
  let workflow = lifecycle_workflow();
  let database = std::env::temp_dir().join(format!(
    "woml-lec6-live-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let run_id = "run_lec6_live";
  let mut control = DurableEventStore::open(database.clone()).unwrap();
  start_known_run(&mut control, &workflow, run_id);

  let runtime_database = database.clone();
  let runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_database,
      run_id,
      RuntimeExecutionOptions::new(host, 10_000),
    )
    .await
  });

  tokio::time::timeout(std::time::Duration::from_secs(3), async {
    loop {
      if control
        .projection(run_id)
        .unwrap()
        .latest_attempt("prepare")
        .is_some_and(|attempt| {
          matches!(
            attempt.status,
            woml_engine::projection::AttemptStatus::Started
          )
        })
      {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("the long-running script starts");

  control
    .request_run_cancellation(run_id, "cancel_live", Utc::now())
    .unwrap();
  let error = tokio::time::timeout(std::time::Duration::from_secs(3), runtime)
    .await
    .expect("cancellation settles promptly")
    .unwrap()
    .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));

  let projection = control.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  assert_eq!(
    projection.business_outcome,
    Some(BusinessOutcome::Cancelled)
  );
  assert!(!projection.context.steps.contains_key("finish"));
  assert!(matches!(
    &projection.latest_attempt("prepare").unwrap().status,
    woml_engine::projection::AttemptStatus::Failed { failure }
      if failure.kind == AttemptFailureKind::InvocationCancelled
  ));
  let events = control.events(run_id).unwrap();
  let cancellation_sequence = events
    .iter()
    .find(|event| matches!(event.payload, RunEventPayload::RunCancellationRequested(_)))
    .unwrap()
    .sequence;
  let outcome_sequence = events
    .iter()
    .find(|event| matches!(event.payload, RunEventPayload::RunOutcomeDecided(_)))
    .unwrap()
    .sequence;
  assert!(cancellation_sequence < outcome_sequence);
  let hook_events = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::LifecycleHookRequested(data) => Some(data.event),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(
    hook_events,
    vec![
      LifecycleEventName::RunCancel,
      LifecycleEventName::StepComplete,
      LifecycleEventName::RunComplete,
    ]
  );
  let _ = std::fs::remove_file(database);
}

#[tokio::test]
async fn active_pre_outcome_lifecycle_action_settles_before_on_cancel() {
  let Some(host) = host_options() else {
    return;
  };
  let workflow = active_lifecycle_workflow();
  let database = std::env::temp_dir().join(format!(
    "woml-lec6-lifecycle-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let run_id = "run_lec6_lifecycle";
  let mut control = DurableEventStore::open(database.clone()).unwrap();
  start_known_run(&mut control, &workflow, run_id);
  let runtime_database = database.clone();
  let runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_database,
      run_id,
      RuntimeExecutionOptions::new(host, 10_000),
    )
    .await
  });

  tokio::time::timeout(std::time::Duration::from_secs(3), async {
    loop {
      if control
        .projection(run_id)
        .unwrap()
        .lifecycle_hooks
        .values()
        .any(|hook| {
          hook.event == LifecycleEventName::RunStart
            && hook
              .actions
              .values()
              .any(|action| action.status == woml_engine::LifecycleActionStatus::Started)
        })
      {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("the run-start action begins");
  control
    .request_run_cancellation(run_id, "cancel_lifecycle", Utc::now())
    .unwrap();
  let error = tokio::time::timeout(std::time::Duration::from_secs(3), runtime)
    .await
    .expect("lifecycle cancellation settles promptly")
    .unwrap()
    .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  let events = control.events(run_id).unwrap();
  let action_failure = events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::LifecycleActionFailed(data)
        if data.failure.code == "WOML_LIFECYCLE_ACTION_CANCELLED" =>
      {
        Some(event.sequence)
      }
      _ => None,
    })
    .expect("the active lifecycle action is durably cancelled");
  let on_cancel_start = events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::LifecycleActionAttemptStarted(data)
        if data.action_id.contains("run_cancel") =>
      {
        Some(event.sequence)
      }
      _ => None,
    })
    .expect("on-cancel executes");
  assert!(action_failure < on_cancel_start);
  assert_eq!(
    control.projection(run_id).unwrap().status,
    RunStatus::Cancelled
  );
  let _ = std::fs::remove_file(database);
}

#[tokio::test]
async fn waiting_approval_cancels_and_invalidates_its_credential() {
  let Some(host) = host_options() else {
    return;
  };
  let workflow = approval_workflow();
  let database = std::env::temp_dir().join(format!(
    "woml-lec6-approval-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let waiting = execute_workflow_durable_outcome(
    workflow,
    HASH.to_string(),
    Map::from_iter([("articleId".to_string(), json!("article-1"))]),
    RuntimeExecutionOptions::new(host.clone(), 2_000),
    database.clone(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Waiting {
    run_id, approval, ..
  } = waiting
  else {
    panic!("workflow should wait for approval")
  };
  let mut store = DurableEventStore::open(database.clone()).unwrap();
  store
    .verify_approval_token(&approval.token, Utc::now())
    .unwrap();
  store
    .request_run_cancellation(&run_id, "cancel_approval", Utc::now())
    .unwrap();
  assert!(matches!(
    store.verify_approval_token(&approval.token, Utc::now()),
    Err(DurableStoreError::ApprovalExpired)
  ));

  let error = resume_workflow_durable_any_outcome(
    database.clone(),
    &run_id,
    RuntimeExecutionOptions::new(host, 2_000),
  )
  .await
  .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  assert_eq!(
    store.projection(&run_id).unwrap().status,
    RunStatus::Cancelled
  );
  let _ = std::fs::remove_file(database);
}

#[tokio::test]
async fn cancellation_stops_a_durable_retry_before_the_next_attempt() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = lifecycle_free_workflow();
  let first = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "a")
    .unwrap();
  first.retry_policy = Some(RetryPolicy {
    max_attempts: 3,
    backoff: BackoffPolicy::Fixed { delay_ms: 5_000 },
  });
  let ValueExpression::Object { fields } = &mut first.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!("throw new Error('retry me');"),
    },
  );
  workflow.validate_structure().unwrap();

  let database = std::env::temp_dir().join(format!(
    "woml-lec6-retry-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let run_id = "run_lec6_retry";
  let mut control = DurableEventStore::open(database.clone()).unwrap();
  start_known_run(&mut control, &workflow, run_id);
  let runtime_database = database.clone();
  let runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_database,
      run_id,
      RuntimeExecutionOptions::new(host, 2_000),
    )
    .await
  });

  tokio::time::timeout(std::time::Duration::from_secs(3), async {
    loop {
      if control
        .projection(run_id)
        .unwrap()
        .pending_retries
        .contains_key("a")
      {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("the retry is scheduled");
  control
    .request_run_cancellation(run_id, "cancel_retry", Utc::now())
    .unwrap();
  let error = tokio::time::timeout(std::time::Duration::from_secs(3), runtime)
    .await
    .expect("the retry wait observes cancellation")
    .unwrap()
    .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  let projection = control.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  assert_eq!(
    projection
      .attempts
      .iter()
      .filter(|attempt| attempt.identity.node_id == "a")
      .count(),
    1
  );
  let _ = std::fs::remove_file(database);
}

#[tokio::test]
async fn cancellation_signals_all_active_parallel_children() {
  let Some(host) = host_options() else {
    return;
  };
  let workflow = parallel_workflow();
  let database = std::env::temp_dir().join(format!(
    "woml-lec6-parallel-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let run_id = "run_lec6_parallel";
  let mut control = DurableEventStore::open(database.clone()).unwrap();
  start_known_run(&mut control, &workflow, run_id);
  let runtime_database = database.clone();
  let runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_database,
      run_id,
      RuntimeExecutionOptions::new(host, 10_000),
    )
    .await
  });

  tokio::time::timeout(std::time::Duration::from_secs(3), async {
    loop {
      let projection = control.projection(run_id).unwrap();
      let active_children = ["loadWeather", "loadSoil"]
        .into_iter()
        .filter(|node_id| {
          projection.latest_attempt(node_id).is_some_and(|attempt| {
            matches!(
              attempt.status,
              woml_engine::projection::AttemptStatus::Started
            )
          })
        })
        .count();
      if active_children == 2 {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("both parallel children start");
  control
    .request_run_cancellation(run_id, "cancel_parallel", Utc::now())
    .unwrap();
  let error = tokio::time::timeout(std::time::Duration::from_secs(3), runtime)
    .await
    .expect("parallel cancellation settles promptly")
    .unwrap()
    .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  let projection = control.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  for node_id in ["loadWeather", "loadSoil"] {
    assert!(matches!(
      &projection.latest_attempt(node_id).unwrap().status,
      woml_engine::projection::AttemptStatus::Failed { failure }
        if failure.kind == AttemptFailureKind::InvocationCancelled
    ));
  }
  assert!(matches!(
    projection.parallel_groups.get("fieldData").unwrap().status,
    woml_engine::ParallelGroupStatus::Completed { .. }
  ));
  assert!(!projection.context.steps.contains_key("buildReport"));
  let _ = std::fs::remove_file(database);
}

#[tokio::test]
async fn restart_finishes_a_requested_cancellation_without_replaying_the_step() {
  let workflow = lifecycle_free_workflow();
  let database = std::env::temp_dir().join(format!(
    "woml-lec6-recovery-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let run_id = "run_lec6_recovery";
  let mut store = DurableEventStore::open(database.clone()).unwrap();
  start_known_run(&mut store, &workflow, run_id);
  store
    .append_payload(
      run_id,
      "event_attempt_started",
      Utc::now(),
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "a".to_string(),
        attempt: 1,
        invocation_id: "inv_interrupted_by_cancel".to_string(),
        handler: "runtime.script".to_string(),
        idempotency_key: Some(woml_engine::step_effect_idempotency_key(run_id, HASH, "a")),
      }),
    )
    .unwrap();
  let step_key = woml_engine::step_effect_idempotency_key(run_id, HASH, "a");
  store
    .append_payload(
      run_id,
      "event_operation_started",
      Utc::now(),
      RunEventPayload::OperationStarted(OperationStartedData {
        node_id: "a".to_string(),
        attempt_number: 1,
        invocation_id: "inv_interrupted_by_cancel".to_string(),
        call_id: "call_active_during_cancel".to_string(),
        operation_key: woml_engine::derive_operation_key(&step_key, "services.http.request#1"),
        capability: "services.http".to_string(),
        operation: "request".to_string(),
        execution_mode: OperationExecutionMode::Managed,
        metadata: Map::new(),
      }),
    )
    .unwrap();
  store
    .request_run_cancellation(run_id, "cancel_recovery", Utc::now())
    .unwrap();
  drop(store);

  let dummy_host = ScriptHostProcessOptions::new(
    PathBuf::from("bun-not-needed"),
    PathBuf::from("script-host-not-needed.ts"),
  );
  let error = resume_workflow_durable_any_outcome(
    database.clone(),
    run_id,
    RuntimeExecutionOptions::new(dummy_host, 2_000),
  )
  .await
  .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));
  let store = DurableEventStore::open(database.clone()).unwrap();
  let projection = store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  assert!(matches!(
    &projection.latest_attempt("a").unwrap().status,
    woml_engine::projection::AttemptStatus::Failed { failure }
      if failure.kind == AttemptFailureKind::InvocationCancelled
  ));
  assert!(matches!(
    projection.operations.values().next().unwrap().status,
    OperationStatus::Failed { ref failure, .. }
      if failure.kind == CapabilityFailureKind::Ambiguous && failure.ambiguous
  ));
  assert!(!projection.context.steps.contains_key("a"));
  let _ = std::fs::remove_file(database);
}
