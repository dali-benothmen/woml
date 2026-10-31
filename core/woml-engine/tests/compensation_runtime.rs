use std::path::{Path, PathBuf};

mod corrective_services;

use rusqlite::Connection;
use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::{
  execute_admitted_trigger_run_durable, execute_workflow_durable, resume_workflow_durable,
  AttemptFailure, AttemptFailureKind, CompiledCorrectionIdentity, CompiledWorkflowDefinition,
  CorrectionAttemptIdentityData, CorrectionRecoveryActionV1, CorrectionRecoveryRequestV1,
  CorrectiveActionKind, DurableDagEngine, DurableEventStore, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, SagaCorrectionReason, SagaCorrectionStartedData,
  SagaIdentityData, ScriptHostProcessOptions, StepAttemptFailedData, StepAttemptSucceededData,
  TriggerAdmissionRequest, CORRECTION_RECOVERY_REQUEST_PROFILE,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/compensation-and-cleanup/model.v19.reviewed.json");
const HASH: &str = "sha256:1919191919191919191919191919191919191919191919191919191919191919";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).expect("reviewed Model v19 fixture")
}

fn compile_reviewed_woml() -> Option<CompiledWorkflowDefinition> {
  compile_woml_fixture("model.reviewed.woml")
}

fn compile_woml_fixture(name: &str) -> Option<CompiledWorkflowDefinition> {
  runtime_options()?;
  let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
  let fixture = repository
    .join("woml/tests/fixtures/compensation-and-cleanup")
    .join(name);
  let output = std::process::Command::new("bun")
    .current_dir(&repository)
    .env("WOML_FIXTURE", fixture)
    .args([
      "-e",
      "import {readFileSync} from 'node:fs'; import {parseWoml,compileWoml} from './woml/src/index.ts'; process.stdout.write(JSON.stringify(compileWoml(parseWoml(readFileSync(process.env.WOML_FIXTURE,'utf8')))));",
    ])
    .output()
    .ok()?;
  output
    .status
    .success()
    .then(|| serde_json::from_slice(&output.stdout).ok())
    .flatten()
}

fn two_effect_failure_model() -> CompiledWorkflowDefinition {
  let mut workflow = model();
  let mut verification = workflow
    .node("deployApplication")
    .expect("deployment node")
    .clone();
  verification.id = "verifyDeployment".to_string();
  let woml_engine::model::ValueExpression::Object { fields } = &mut verification.inputs else {
    panic!("verification inputs")
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: Value::String("throw new Error('deployment rejected');".to_string()),
    },
  );
  workflow.graph.nodes.push(verification);
  let commit_edge = workflow
    .graph
    .edges
    .iter_mut()
    .find(|edge| edge.from == "deployApplication")
    .expect("saga commit edge");
  commit_edge.id = "verifyDeployment-to-saga-commit".to_string();
  commit_edge.from = "verifyDeployment".to_string();
  workflow
    .graph
    .edges
    .push(woml_engine::model::CompiledWorkflowEdge {
      id: "deployApplication-to-verifyDeployment".to_string(),
      from: "deployApplication".to_string(),
      to: "verifyDeployment".to_string(),
      condition: woml_engine::model::EdgeCondition::Always,
      branch_id: None,
      parallel_id: None,
      approval_id: None,
    });
  workflow
    .graph
    .context_visibility
    .as_mut()
    .expect("Model v19 visibility")
    .push(woml_engine::model::CompiledContextVisibility {
      node_id: "verifyDeployment".to_string(),
      step_ids: vec![
        "prepare".to_string(),
        "createNetwork".to_string(),
        "deployApplication".to_string(),
      ],
    });
  let saga = workflow
    .graph
    .sagas
    .as_mut()
    .and_then(|sagas| sagas.first_mut())
    .expect("reviewed saga");
  saga.body_terminal_node_id = "verifyDeployment".to_string();
  let mut deployment_compensation = saga.compensations[0].clone();
  deployment_compensation.action_id = "provision:compensate:deployApplication".to_string();
  deployment_compensation.for_step_id = Some("deployApplication".to_string());
  deployment_compensation.identity = CompiledCorrectionIdentity::Static {
    action_id: deployment_compensation.action_id.clone(),
  };
  deployment_compensation.node.id = deployment_compensation.action_id.clone();
  deployment_compensation.visible_step_ids = vec![
    "prepare".to_string(),
    "createNetwork".to_string(),
    "deployApplication".to_string(),
  ];
  let woml_engine::model::ValueExpression::Object { fields } =
    &mut deployment_compensation.node.inputs
  else {
    panic!("compensation inputs")
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: Value::String(
        "return { removed: context.steps.deployApplication.deployed };".to_string(),
      ),
    },
  );
  saga.compensations.push(deployment_compensation);
  workflow
}

fn correction_retry_model() -> CompiledWorkflowDefinition {
  let mut workflow = two_effect_failure_model();
  let saga = workflow
    .graph
    .sagas
    .as_mut()
    .and_then(|sagas| sagas.first_mut())
    .expect("reviewed saga");
  let compensation = saga
    .compensations
    .iter_mut()
    .find(|action| action.for_step_id.as_deref() == Some("deployApplication"))
    .expect("deployment compensation");
  let woml_engine::model::ValueExpression::Object { fields } = &mut compensation.node.inputs else {
    panic!("compensation inputs")
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: Value::String(
        "if (context.compensation.forStepId !== 'deployApplication') throw new Error('binding missing'); if (attempt.number === 1) throw new Error('retry correction'); return { corrected: true };".to_string(),
      ),
    },
  );
  let cleanup = saga.cleanup.first_mut().expect("mandatory cleanup");
  let woml_engine::model::ValueExpression::Object { fields } = &mut cleanup.node.inputs else {
    panic!("cleanup inputs")
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: Value::String(
        "if (context.cleanup.compensationStatus !== 'completed') throw new Error('cleanup status missing'); return { cleaned: true };".to_string(),
      ),
    },
  );
  workflow
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-compensation-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn runtime_options() -> Option<RuntimeExecutionOptions> {
  runtime_options_with_host("woml-cli/src/script-host.ts")
}

fn runtime_options_with_host(relative_host: &str) -> Option<RuntimeExecutionOptions> {
  std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(())?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../..")
    .join(relative_host);
  host.exists().then(|| {
    RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new(PathBuf::from("bun"), host),
      5_000,
    )
    .with_shared_script_hosts()
  })
}

#[tokio::test]
async fn successful_saga_runs_cleanup_without_compensation_and_continues() {
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("success");
  let result = execute_workflow_durable(
    model(),
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect("successful saga");
  assert_eq!(result.result, serde_json::json!({"provisioned": true}));

  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&result.run_id).unwrap();
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::SagaCorrectionStarted(data)
      if data.reason == SagaCorrectionReason::Success
  )));
  assert!(!events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CorrectionAttemptStarted(data)
      if data.action_id == "provision:compensate:createNetwork"
  )));
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CorrectionAttemptSucceeded(data)
      if data.action_id == "provision:cleanup:revokeCredentials"
  )));
  assert!(events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::SagaCorrectionCompleted(_))));
  let inspection = store.inspect_correction_v1(&result.run_id).unwrap();
  assert_eq!(inspection.phase, "completed");
  assert_eq!(inspection.original_outcome, "succeeded");
  assert_eq!(inspection.sagas[0].completed_cleanup, 1);
  assert!(inspection.unresolved_actions.is_empty());
}

#[tokio::test]
async fn real_woml_text_compiles_to_model_v19_and_executes_through_rust() {
  let Some(workflow) = compile_reviewed_woml() else {
    return;
  };
  let Some(options) = runtime_options() else {
    return;
  };
  assert_eq!(workflow.schema_version, 19);
  let database = TemporaryDatabase::new("real-woml");
  let result = execute_workflow_durable(
    workflow,
    "sha256:3939393939393939393939393939393939393939393939393939393939393939".to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect("compiled WOML saga executes");
  assert_eq!(result.result, serde_json::json!({"provisioned": true}));
}

#[tokio::test]
async fn failed_saga_compensates_reverse_completion_then_cleans_up_and_keeps_primary_failure() {
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("failure");
  let workflow = two_effect_failure_model();
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect_err("forward failure remains primary");
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected durable run failure, received {error:?}")
  };
  assert_eq!(details.node_id.as_deref(), Some("verifyDeployment"));
  assert!(details.message.contains("deployment rejected"));

  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&details.events[0].run_id).unwrap();
  let corrective_order = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::CorrectionAttemptStarted(data) => Some(data.action_id.as_str()),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(
    corrective_order,
    vec![
      "provision:compensate:deployApplication",
      "provision:compensate:createNetwork",
      "provision:cleanup:revokeCredentials"
    ]
  );
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CompensationRegistered(data)
      if data.for_step_id == "createNetwork"
  )));
  assert!(events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::SagaCorrectionCompleted(_))));
  let inspection = store
    .inspect_correction_v1(&details.events[0].run_id)
    .unwrap();
  assert_eq!(inspection.phase, "completed");
  assert_eq!(inspection.original_outcome, "failed");
  assert_eq!(inspection.sagas[0].registered_compensations, 2);
  assert_eq!(inspection.sagas[0].completed_compensations, 2);
  assert_eq!(inspection.sagas[0].completed_cleanup, 1);
  drop(store);

  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute("DELETE FROM woml_correction_attempts", [])
    .unwrap();
  connection
    .execute("DELETE FROM woml_correction_actions", [])
    .unwrap();
  connection
    .execute("DELETE FROM woml_saga_scopes", [])
    .unwrap();
  drop(connection);
  drop(DurableEventStore::open(database.path()).unwrap());
  let connection = Connection::open(database.path()).unwrap();
  let rebuilt_actions: i64 = connection
    .query_row("SELECT COUNT(*) FROM woml_correction_actions", [], |row| {
      row.get(0)
    })
    .unwrap();
  assert_eq!(rebuilt_actions, 3);
}

#[tokio::test]
async fn corrective_attempts_use_v11_bindings_and_durable_retry_identity() {
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("correction-retry");
  let error = execute_workflow_durable(
    correction_retry_model(),
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect_err("the original forward failure remains primary");
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected durable run failure, received {error:?}")
  };
  let events = DurableEventStore::open(database.path())
    .unwrap()
    .events(&details.events[0].run_id)
    .unwrap();
  let attempts = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::CorrectionAttemptStarted(data)
        if data.action_id == "provision:compensate:deployApplication" =>
      {
        Some(data.attempt)
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(attempts, vec![1, 2]);
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CorrectionRetryScheduled(data)
      if data.action_id == "provision:compensate:deployApplication"
        && data.attempt == 1
  )));
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CorrectionAttemptSucceeded(data)
      if data.action_id == "provision:cleanup:revokeCredentials"
  )));
}

#[tokio::test]
async fn cancellation_compensates_completed_work_and_runs_cleanup_before_settlement() {
  let Some(options) =
    runtime_options_with_host("woml-cli/tests/fixtures/nonresponsive-cancel-script-host.ts")
  else {
    return;
  };
  let workflow = model();
  workflow.validate_structure().unwrap();

  let database = TemporaryDatabase::new("cancel-correction");
  let mut control = DurableEventStore::open(database.path()).unwrap();
  control.register_definition(&workflow, HASH).unwrap();
  let run_id = control
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "manual:cancel-correction".to_string(),
      payload: Map::new(),
      received_at: chrono::Utc::now(),
    })
    .unwrap()
    .run_id;

  let runtime_database = database.path().to_path_buf();
  let runtime_run_id = run_id.clone();
  let mut runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(runtime_database, &runtime_run_id, options).await
  });
  tokio::time::timeout(std::time::Duration::from_secs(10), async {
    loop {
      if control
        .projection(&run_id)
        .unwrap()
        .latest_attempt("deployApplication")
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
  .expect("the saga deployment starts");
  control
    .request_run_cancellation(&run_id, "cancel-saga", chrono::Utc::now())
    .unwrap();

  // The production interruption grace remains two seconds. This wider test
  // watchdog also covers starting a replacement Bun host, which is
  // substantially slower on loaded Windows CI runners.
  let joined = match tokio::time::timeout(std::time::Duration::from_secs(30), &mut runtime).await {
    Ok(joined) => joined,
    Err(_) => {
      let projection = control.projection(&run_id).unwrap();
      let recent_events = control
        .events(&run_id)
        .unwrap()
        .into_iter()
        .rev()
        .take(12)
        .map(|event| format!("{}:{:?}", event.sequence, event.payload))
        .collect::<Vec<_>>();
      runtime.abort();
      panic!(
        "saga cancellation did not settle within its CI watchdog; status={:?}; recent_events={recent_events:#?}",
        projection.status
      );
    }
  };
  let error = joined.unwrap().unwrap_err();
  assert!(
    matches!(error, RuntimeExecutionError::RunCancelled(_)),
    "unexpected cancellation result: {error:?}"
  );
  drop(control);
  let control = DurableEventStore::open(database.path()).unwrap();
  let projection = control.projection(&run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  let events = control.events(&run_id).unwrap();
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::SagaCorrectionStarted(data)
      if data.reason == SagaCorrectionReason::Cancel
  )));
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CorrectionAttemptSucceeded(data)
      if data.action_id == "provision:compensate:createNetwork"
  )));
  let cleanup = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload,
        RunEventPayload::CorrectionAttemptSucceeded(data)
          if data.action_id == "provision:cleanup:revokeCredentials"
      )
    })
    .expect("cleanup succeeds during cancellation")
    .sequence;
  let terminal = events
    .iter()
    .find(|event| {
      matches!(
        event.payload,
        RunEventPayload::RunOutcomeDecided(woml_engine::RunOutcomeDecidedData::Cancelled { .. })
      )
    })
    .expect("run cancellation is durable")
    .sequence;
  assert!(cleanup < terminal);
}

#[tokio::test]
async fn externally_resolved_restart_continues_cleanup_without_replaying_the_effect() {
  let Some(options) = runtime_options() else {
    return;
  };
  let workflow = model();
  let database = TemporaryDatabase::new("external-resolution-resume");
  let now = chrono::Utc::now();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&workflow, HASH).unwrap();
  let run_id = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "manual:external-resolution-resume".to_string(),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap()
    .run_id;
  let claim = store
    .claim_policy_run(
      &run_id,
      "crashed-runtime",
      now,
      std::time::Duration::from_secs(30),
    )
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &run_id).unwrap();
  engine
    .start_step_attempt(&run_id, "prepare", 1, "inv-prepare", now)
    .unwrap();
  engine
    .append_payload(
      "evt-prepare-success",
      &run_id,
      now,
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: "prepare".to_string(),
        attempt: 1,
        invocation_id: "inv-prepare".to_string(),
        output: serde_json::json!({"pullRequest": 42}),
      }),
    )
    .unwrap();
  engine
    .append_payload(
      "evt-saga-start",
      &run_id,
      now,
      RunEventPayload::SagaStarted(SagaIdentityData {
        saga_id: "provision".to_string(),
      }),
    )
    .unwrap();
  engine
    .start_step_attempt(&run_id, "createNetwork", 1, "inv-network", now)
    .unwrap();
  engine
    .record_step_success_with_compensation(
      &run_id,
      StepAttemptSucceededData {
        node_id: "createNetwork".to_string(),
        attempt: 1,
        invocation_id: "inv-network".to_string(),
        output: serde_json::json!({"networkId": "network-42"}),
      },
      "provision",
      "provision:compensate:createNetwork",
    )
    .unwrap();
  engine
    .start_step_attempt(&run_id, "deployApplication", 1, "inv-deploy", now)
    .unwrap();
  engine
    .record_step_attempt_failure(
      &run_id,
      now,
      StepAttemptFailedData {
        node_id: "deployApplication".to_string(),
        attempt: 1,
        invocation_id: "inv-deploy".to_string(),
        failure: AttemptFailure {
          kind: AttemptFailureKind::ScriptThrew,
          code: "WOML_SCRIPT_THROWN".to_string(),
          message: "deployment failed".to_string(),
          ..AttemptFailure::legacy_defaults()
        },
      },
    )
    .unwrap();
  engine
    .append_payload(
      "evt-correction-start",
      &run_id,
      now,
      RunEventPayload::SagaCorrectionStarted(SagaCorrectionStartedData {
        saga_id: "provision".to_string(),
        reason: SagaCorrectionReason::Failure,
      }),
    )
    .unwrap();
  engine
    .append_payload(
      "evt-correction-attempt",
      &run_id,
      now,
      RunEventPayload::CorrectionAttemptStarted(CorrectionAttemptIdentityData {
        saga_id: "provision".to_string(),
        action_id: "provision:compensate:createNetwork".to_string(),
        action_kind: CorrectiveActionKind::Compensation,
        attempt: 1,
      }),
    )
    .unwrap();
  drop(engine);

  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.recover_interrupted_runs().unwrap();
  store
    .recover_correction_v1(
      CorrectionRecoveryRequestV1 {
        profile: CORRECTION_RECOVERY_REQUEST_PROFILE.to_string(),
        run_id: run_id.clone(),
        action: CorrectionRecoveryActionV1::ExternallyResolve,
        action_id: "provision:compensate:createNetwork".to_string(),
        note: Some("resource verified absent in the provider console".to_string()),
      },
      chrono::Utc::now(),
    )
    .unwrap();
  assert!(store
    .release_policy_claim(&run_id, "crashed-runtime", &claim.claim_id)
    .unwrap());
  drop(store);

  let error = resume_workflow_durable(database.path().to_path_buf(), &run_id, options)
    .await
    .expect_err("the original deployment failure remains primary");
  assert!(matches!(error, RuntimeExecutionError::RunFailed(_)));
  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::CorrectionAttemptStarted(data)
          if data.action_id == "provision:compensate:createNetwork"
      ))
      .count(),
    1,
    "the ambiguous effect must never be replayed"
  );
  assert!(events.iter().any(|event| matches!(
    &event.payload,
    RunEventPayload::CorrectionAttemptSucceeded(data)
      if data.action_id == "provision:cleanup:revokeCredentials"
  )));
  assert!(events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::SagaCorrectionCompleted(_))));
  assert_eq!(store.projection(&run_id).unwrap().status, RunStatus::Failed);
}

#[tokio::test]
async fn saga_composes_with_selected_choices_parallel_completion_and_joined_forks() {
  let Some(workflow) = compile_woml_fixture("control-flow.reviewed.woml") else {
    return;
  };
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("control-flow");
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect_err("the final forward step deliberately fails");
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected durable run failure, received {error:?}")
  };
  assert_eq!(details.node_id.as_deref(), Some("failDelivery"));

  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&details.events[0].run_id).unwrap();
  let registered = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::CompensationRegistered(data) => Some(data.for_step_id.as_str()),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert!(registered.contains(&"chosenEffect"));
  assert!(registered.contains(&"switchedEffect"));
  assert!(registered.contains(&"slowEffect"));
  assert!(registered.contains(&"fastEffect"));
  assert!(registered.contains(&"firstDestination"));
  assert!(registered.contains(&"secondDestination"));
  assert!(!registered.contains(&"unselectedEffect"));
  assert!(!registered.contains(&"defaultEffect"));

  let starts = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::CorrectionAttemptStarted(data) => Some(data.action_id.as_str()),
      _ => None,
    })
    .collect::<Vec<_>>();
  let compensations = starts
    .iter()
    .copied()
    .filter(|action_id| action_id.contains(":compensate:"))
    .collect::<Vec<_>>();
  let expected = registered
    .iter()
    .rev()
    .map(|step_id| format!("delivery:compensate:{step_id}"))
    .collect::<Vec<_>>();
  assert_eq!(
    compensations,
    expected.iter().map(String::as_str).collect::<Vec<_>>()
  );
  assert_eq!(starts.last(), Some(&"delivery:cleanup:cleanupDelivery"));
}

#[tokio::test]
async fn saga_registers_and_compensates_each_for_each_iteration_independently() {
  let Some(workflow) = compile_woml_fixture("for-each.reviewed.woml") else {
    return;
  };
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("for-each");
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect_err("the final forward step deliberately fails");
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected durable run failure, received {error:?}")
  };

  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&details.events[0].run_id).unwrap();
  let registered = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::CompensationRegistered(data) => Some(data.action_id.clone()),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(registered.len(), 3);
  assert_eq!(
    registered
      .iter()
      .collect::<std::collections::HashSet<_>>()
      .len(),
    3
  );
  assert!(registered
    .iter()
    .all(|action_id| action_id.starts_with("provisionItems:compensate:createItem:")));

  let compensated = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::CorrectionAttemptStarted(data)
        if data.action_id.contains(":compensate:") =>
      {
        Some(data.action_id.clone())
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(
    compensated,
    registered.into_iter().rev().collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn parallel_saga_failure_settles_siblings_before_correction_and_run_failure() {
  let Some(workflow) = compile_woml_fixture("parallel-failure.reviewed.woml") else {
    return;
  };
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("parallel-failure");
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect_err("one parallel child deliberately fails");
  let RuntimeExecutionError::ParallelFailed(details) = error else {
    panic!("expected parallel failure, received {error:?}")
  };
  let events = &details.events;
  let group_sequence = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload,
        RunEventPayload::ParallelGroupCompleted(data)
          if data.parallel_id == "destinations"
            && data.outcome == woml_engine::ParallelGroupOutcome::Failed
      )
    })
    .map(|event| event.sequence)
    .expect("parallel group settled");
  let correction_sequence = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload,
        RunEventPayload::CorrectionAttemptStarted(data)
          if data.action_id == "publish:compensate:published"
      )
    })
    .map(|event| event.sequence)
    .expect("successful sibling was compensated");
  let run_failure_sequence = events
    .iter()
    .find(|event| matches!(event.payload, RunEventPayload::RunFailed(_)))
    .map(|event| event.sequence)
    .expect("run failure was recorded");
  assert!(group_sequence < correction_sequence);
  assert!(correction_sequence < run_failure_sequence);
}

#[tokio::test]
async fn joined_fork_saga_failure_settles_all_branches_before_correction() {
  let Some(workflow) = compile_woml_fixture("fork-failure.reviewed.woml") else {
    return;
  };
  let Some(options) = runtime_options() else {
    return;
  };
  let database = TemporaryDatabase::new("fork-failure");
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .expect_err("one joined branch deliberately fails");
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected durable run failure, received {error:?}")
  };
  let events = &details.events;
  let latest_branch_sequence = events
    .iter()
    .filter(|event| {
      matches!(
        &event.payload,
        RunEventPayload::ForkBranchSettled(data) if data.fork_id == "destinations"
      )
    })
    .map(|event| event.sequence)
    .max()
    .expect("both fork branches settled");
  let correction_sequence = events
    .iter()
    .find(|event| {
      matches!(
        &event.payload,
        RunEventPayload::CorrectionAttemptStarted(data)
          if data.action_id == "distribute:compensate:published"
      )
    })
    .map(|event| event.sequence)
    .expect("successful branch was compensated");
  let run_failure_sequence = events
    .iter()
    .find(|event| matches!(event.payload, RunEventPayload::RunFailed(_)))
    .map(|event| event.sequence)
    .expect("run failure was recorded");
  assert!(latest_branch_sequence < correction_sequence);
  assert!(correction_sequence < run_failure_sequence);
}
