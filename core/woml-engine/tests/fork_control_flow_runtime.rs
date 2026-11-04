use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::model::{BackoffPolicy, RetryPolicy};
use woml_engine::{
  execute_admitted_trigger_run_durable, execute_workflow_durable, project_run_presentation_v1,
  run_presentation_from_store_v1, AttemptFailureKind, BusinessOutcome, CompiledWorkflowDefinition,
  DurableEventStore, ForkBranchOutcome, OperationStatus, PresentationStepKind,
  PresentationStepStatus, RunEventPayload, RunStatus, RuntimeExecutionError,
  RuntimeExecutionOptions, ScriptHostProcessOptions, TriggerAdmissionRequest,
};

const CONTROL_CHOICE: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/control-choice.compiled.v13.json");
const FORK_JOIN_ALL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");

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

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source.to_string()),
    },
  );
}

#[tokio::test]
async fn a_branch_retry_settles_before_the_fork_join() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(FORK_JOIN_ALL).unwrap();
  let facebook = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "publishFacebook")
    .unwrap();
  facebook.retry_policy = Some(RetryPolicy {
    max_attempts: 2,
    backoff: BackoffPolicy::Fixed { delay_ms: 1 },
  });
  let ValueExpression::Object { fields } = &mut facebook.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(
        "if (attempt.number === 1) throw new Error('temporary'); return { platform: 'facebook' };"
          .to_string(),
      ),
    },
  );
  workflow.validate_for_durable_execution().unwrap();
  let hash = format!("sha256:{:064x}", 77);
  let result = execute_workflow_durable(
    workflow.clone(),
    hash.clone(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert!(result.events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepRetryScheduled(data) if data.node_id == "publishFacebook" && data.next_attempt == 2)
  }));
  assert_eq!(
    result.context.steps["publishFacebook"]["platform"],
    "facebook"
  );
  let presentation = project_run_presentation_v1(&workflow, &hash, &result.events).unwrap();
  let fork = presentation
    .steps
    .iter()
    .find(|step| step.kind == PresentationStepKind::Fork)
    .unwrap();
  assert_eq!(fork.status, PresentationStepStatus::Succeeded);
  assert!(fork
    .detail
    .as_deref()
    .is_some_and(|detail| detail.contains("join all") && detail.contains("continuation released")));
  assert_eq!(
    presentation
      .steps
      .iter()
      .filter(|step| step.kind == PresentationStepKind::Branch)
      .count(),
    2
  );
  assert_eq!(presentation.summary.total, 7);
}

#[tokio::test]
async fn cancellation_closes_active_and_queued_fork_work_before_the_run() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(FORK_JOIN_ALL).unwrap();
  set_script(
    &mut workflow,
    "publishInstagram",
    "await services.cache.set('fork-cancel-ready-instagram', true, { name: 'ready' }); await new Promise(() => {}); return { platform: 'instagram' };",
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    "await services.cache.set('fork-cancel-ready-facebook', true, { name: 'ready' }); await new Promise(() => {}); return { platform: 'facebook' };",
  );
  workflow.validate_for_durable_execution().unwrap();
  let hash = "sha256:8888888888888888888888888888888888888888888888888888888888888888";
  let run_id = {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store.register_definition(&workflow, hash).unwrap();
    store
      .admit_trigger_occurrence(TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id.clone(),
        definition_hash: hash.to_string(),
        trigger_id: "start".to_string(),
        trigger_handler: "trigger.manual".to_string(),
        source_identity: format!("fork-cancel:{}", Uuid::new_v4()),
        payload: Map::new(),
        received_at: Utc::now(),
      })
      .unwrap()
      .run_id
  };
  let runtime_database = database.path().to_path_buf();
  let runtime_run_id = run_id.clone();
  let mut runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_database,
      &runtime_run_id,
      RuntimeExecutionOptions::new(host, 10_000),
    )
    .await
  });
  let mut control = DurableEventStore::open(database.path()).unwrap();
  // A durable attempt start precedes dispatch and cold Worker initialization.
  // Require a receipt from each exact invocation before measuring cancellation
  // of active work; script timers must not provide an alternative completion.
  let ready = tokio::time::timeout(std::time::Duration::from_secs(10), async {
    loop {
      let projection = control.projection(&run_id).unwrap();
      let active_workers = ["publishInstagram", "publishFacebook"]
        .into_iter()
        .filter(|node_id| {
          projection.latest_attempt(node_id).is_some_and(|attempt| {
            matches!(
              attempt.status,
              woml_engine::projection::AttemptStatus::Started
            ) && projection.operations.values().any(|operation| {
              operation.node_id == *node_id
                && operation.identity.invocation_id == attempt.identity.invocation_id
                && operation.attempt_number == attempt.identity.attempt
                && operation.capability == "cache"
                && operation.operation == "set"
                && matches!(operation.status, OperationStatus::Succeeded { .. })
            })
          })
        })
        .count();
      if projection.forks.contains_key("distribution") && active_workers == 2 {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await;
  if ready.is_err() {
    let projection = control.projection(&run_id).unwrap();
    runtime.abort();
    let _ = runtime.await;
    panic!("both fork Workers must acknowledge readiness: {projection:#?}");
  }
  control
    .request_run_cancellation(&run_id, "cancel-fork", Utc::now())
    .unwrap();
  // Preserve the original responsiveness deadline; only setup has a separate
  // watchdog. Keep the handle so a failure cannot detach the live runtime.
  let settled = tokio::time::timeout(std::time::Duration::from_secs(3), &mut runtime).await;
  if settled.is_err() {
    let projection = control.projection(&run_id).unwrap();
    runtime.abort();
    let _ = runtime.await;
    panic!("fork cancellation did not settle within three seconds: {projection:#?}");
  }
  let error = settled.unwrap().unwrap().unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));

  let projection = control.projection(&run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  assert_eq!(
    projection.business_outcome,
    Some(BusinessOutcome::Cancelled)
  );
  for node_id in ["publishInstagram", "publishFacebook"] {
    assert!(matches!(
      &projection.latest_attempt(node_id).unwrap().status,
      woml_engine::projection::AttemptStatus::Failed { failure }
        if failure.kind == AttemptFailureKind::InvocationCancelled
    ));
  }
  assert!(projection.latest_attempt("finish").is_none());
  assert!(!projection.context.steps.contains_key("finish"));
  let fork = &projection.forks["distribution"];
  assert_eq!(fork.branches.len(), 2);
  assert!(fork
    .branches
    .values()
    .all(|branch| branch.outcome == Some(ForkBranchOutcome::Cancelled)));
  let events = control.events(&run_id).unwrap();
  let run_outcome_sequence = events
    .iter()
    .find(|event| matches!(event.payload, RunEventPayload::RunOutcomeDecided(_)))
    .expect("the cancelled run must have a durable outcome")
    .sequence;
  let branch_settlements = events
    .iter()
    .filter(|event| {
      matches!(&event.payload, RunEventPayload::ForkBranchSettled(data)
        if data.fork_id == "distribution" && data.outcome == ForkBranchOutcome::Cancelled)
    })
    .collect::<Vec<_>>();
  assert_eq!(branch_settlements.len(), 2);
  assert!(branch_settlements
    .iter()
    .all(|event| event.sequence < run_outcome_sequence));
  let presentation = run_presentation_from_store_v1(&control, &run_id).unwrap();
  assert_eq!(
    presentation
      .steps
      .iter()
      .find(|step| step.kind == PresentationStepKind::Fork)
      .unwrap()
      .status,
    PresentationStepStatus::Cancelled
  );
  assert!(presentation.steps.iter().any(|step| {
    step.kind == PresentationStepKind::Branch && step.status == PresentationStepStatus::Cancelled
  }));
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-fork-control-flow-{}.sqlite", Uuid::new_v4())))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
  }
}

fn choice_model(condition: bool) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(CONTROL_CHOICE).unwrap();
  let ready = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "ready")
    .unwrap();
  let ValueExpression::Object { fields } = &mut ready.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(format!("return {condition};")),
    },
  );
  workflow
}

#[tokio::test]
async fn control_only_choice_selects_one_route_and_reaches_workflow_settlement() {
  let Some(host) = host_options() else {
    return;
  };
  for (condition, selected_step) in [(true, "yes"), (false, "no")] {
    let database = TemporaryDatabase::new();
    let workflow = choice_model(condition);
    workflow.validate_for_durable_execution().unwrap();
    let hash = format!("sha256:{:064x}", u8::from(condition) + 10);
    let result = execute_workflow_durable(
      workflow.clone(),
      hash.clone(),
      Map::new(),
      RuntimeExecutionOptions::new(host.clone(), 3_000),
      database.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert!(result.context.steps.contains_key(selected_step));
    assert!(result.events.iter().any(|event| {
      matches!(&event.payload, RunEventPayload::ChoiceSelected(data) if data.arm_id.contains(if condition { ":when:" } else { ":otherwise" }))
    }));
    assert_eq!(result.terminal_node_id, "__woml_workflow__settlement");
    let presentation = project_run_presentation_v1(&workflow, &hash, &result.events).unwrap();
    let choice = presentation
      .steps
      .iter()
      .find(|step| step.kind == PresentationStepKind::Choose)
      .unwrap();
    assert_eq!(choice.id, "choose-1");
    assert_eq!(choice.status, PresentationStepStatus::Succeeded);
    assert_eq!(
      choice.detail.as_deref(),
      Some(if condition {
        "Selected condition 1."
      } else {
        "Selected otherwise."
      })
    );
    assert!(!serde_json::to_string(&presentation)
      .unwrap()
      .contains("__woml_choice__"));
  }
}
