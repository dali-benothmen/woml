use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::model::{BackoffPolicy, RetryPolicy};
use woml_engine::{
  execute_admitted_trigger_run_durable, execute_workflow_durable, BusinessOutcome,
  CompiledWorkflowDefinition, DurableEventStore, ForkBranchOutcome, RunEventPayload, RunStatus,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
  TriggerAdmissionRequest,
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
  let result = execute_workflow_durable(
    workflow,
    format!("sha256:{:064x}", 77),
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
    "await new Promise(resolve => setTimeout(resolve, 5000)); return { platform: 'instagram' };",
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    "await new Promise(resolve => setTimeout(resolve, 5000)); return { platform: 'facebook' };",
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
  let runtime = tokio::spawn(async move {
    execute_admitted_trigger_run_durable(
      runtime_database,
      &runtime_run_id,
      RuntimeExecutionOptions::new(host, 10_000),
    )
    .await
  });
  let mut control = DurableEventStore::open(database.path()).unwrap();
  tokio::time::timeout(std::time::Duration::from_secs(3), async {
    loop {
      let projection = control.projection(&run_id).unwrap();
      if projection.forks.contains_key("distribution")
        && projection.active_attempt_node_ids().len() >= 2
      {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("fork branches start");
  control
    .request_run_cancellation(&run_id, "cancel-fork", Utc::now())
    .unwrap();
  let error = tokio::time::timeout(std::time::Duration::from_secs(3), runtime)
    .await
    .expect("fork cancellation settles")
    .unwrap()
    .unwrap_err();
  assert!(matches!(error, RuntimeExecutionError::RunCancelled(_)));

  let projection = control.projection(&run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Cancelled);
  assert_eq!(
    projection.business_outcome,
    Some(BusinessOutcome::Cancelled)
  );
  let fork = &projection.forks["distribution"];
  assert_eq!(fork.branches.len(), 2);
  assert!(fork
    .branches
    .values()
    .all(|branch| branch.outcome == Some(ForkBranchOutcome::Cancelled)));
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
    let result = execute_workflow_durable(
      workflow,
      format!("sha256:{:064x}", u8::from(condition) + 10),
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
  }
}
