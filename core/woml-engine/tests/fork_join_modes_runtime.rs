use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::{CompiledWorkflowEdge, EdgeCondition, ValueExpression};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, RunEventPayload, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
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

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-fork-modes-{}.sqlite", Uuid::new_v4())))
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

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: String) {
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
      value: Value::String(source),
    },
  );
}

fn retain_finish_visibility(workflow: &mut CompiledWorkflowDefinition, step_ids: &[&str]) {
  let visibility = workflow
    .graph
    .context_visibility
    .as_mut()
    .unwrap()
    .iter_mut()
    .find(|visibility| visibility.node_id == "finish")
    .unwrap();
  visibility.step_ids = step_ids.iter().map(|id| (*id).to_string()).collect();
}

fn event_position(
  events: &[woml_engine::RunEvent],
  predicate: impl Fn(&RunEventPayload) -> bool,
) -> usize {
  events
    .iter()
    .position(|event| predicate(&event.payload))
    .unwrap()
}

async fn execute_selected(
  instagram_delay_ms: u64,
  facebook_delay_ms: u64,
) -> woml_engine::WorkflowExecutionResult {
  let host = host_options().expect("Bun script host is available");
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  workflow.graph.forks.as_mut().unwrap()[0].joined_branch_ids = vec!["instagram".to_string()];
  workflow
    .graph
    .edges
    .retain(|edge| edge.id != "distribution:join:facebook");
  retain_finish_visibility(&mut workflow, &["prepare", "publishInstagram"]);
  set_script(
    &mut workflow,
    "publishInstagram",
    format!(
      "await new Promise(resolve => setTimeout(resolve, {instagram_delay_ms})); return {{ platform: 'instagram', sawFacebook: context.steps.publishFacebook !== undefined }};"
    ),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    format!(
      "await new Promise(resolve => setTimeout(resolve, {facebook_delay_ms})); return {{ platform: 'facebook', sawInstagram: context.steps.publishInstagram !== undefined }};"
    ),
  );
  set_script(
    &mut workflow,
    "finish",
    "await new Promise(resolve => setTimeout(resolve, 5)); return { instagram: context.steps.publishInstagram, sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  workflow.validate_for_durable_execution().unwrap();
  execute_workflow_durable(
    workflow,
    format!("sha256:{instagram_delay_ms:032x}{facebook_delay_ms:032x}"),
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap()
}

#[tokio::test]
async fn selected_join_releases_for_named_branches_and_never_leaks_unjoined_output() {
  if host_options().is_none() {
    return;
  }
  let slow_unjoined = execute_selected(15, 180).await;
  let fast_unjoined = execute_selected(180, 15).await;
  let expected = json!({
    "instagram": { "platform": "instagram", "sawFacebook": false },
    "sawFacebook": false
  });
  assert_eq!(slow_unjoined.result, expected);
  assert_eq!(fast_unjoined.result, expected);

  let slow_join = event_position(&slow_unjoined.events, |payload| {
    matches!(payload, RunEventPayload::ForkJoinSettled(_))
  });
  let slow_finish = event_position(
    &slow_unjoined.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish"),
  );
  let slow_facebook = event_position(
    &slow_unjoined.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishFacebook"),
  );
  assert!(slow_join < slow_finish);
  assert!(slow_finish < slow_facebook);

  let fast_facebook = event_position(
    &fast_unjoined.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishFacebook"),
  );
  let fast_join = event_position(&fast_unjoined.events, |payload| {
    matches!(payload, RunEventPayload::ForkJoinSettled(_))
  });
  assert!(fast_facebook < fast_join);

  for execution in [&slow_unjoined, &fast_unjoined] {
    let last_branch = execution
      .events
      .iter()
      .rposition(|event| matches!(event.payload, RunEventPayload::ForkBranchSettled(_)))
      .unwrap();
    let outcome = event_position(&execution.events, |payload| {
      matches!(payload, RunEventPayload::RunOutcomeDecided(_))
    });
    assert!(last_branch < outcome);
  }
}

#[tokio::test]
async fn non_blocking_join_releases_immediately_but_final_success_waits_for_owned_branches() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  workflow.graph.forks.as_mut().unwrap()[0]
    .joined_branch_ids
    .clear();
  workflow.graph.edges.retain(|edge| {
    edge.id != "distribution:join:instagram" && edge.id != "distribution:join:facebook"
  });
  workflow.graph.edges.push(CompiledWorkflowEdge {
    id: "distribution:join:none".to_string(),
    from: "__woml_fork__distribution__open".to_string(),
    to: "__woml_fork__distribution__join".to_string(),
    condition: EdgeCondition::Always,
    branch_id: None,
    parallel_id: None,
    approval_id: None,
  });
  retain_finish_visibility(&mut workflow, &["prepare"]);
  set_script(
    &mut workflow,
    "publishInstagram",
    "await new Promise(resolve => setTimeout(resolve, 140)); return { platform: 'instagram', sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    "await new Promise(resolve => setTimeout(resolve, 160)); return { platform: 'facebook', sawInstagram: context.steps.publishInstagram !== undefined };".to_string(),
  );
  set_script(
    &mut workflow,
    "finish",
    "await new Promise(resolve => setTimeout(resolve, 5)); return { accepted: context.steps.prepare.enabled, sawInstagram: context.steps.publishInstagram !== undefined, sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  workflow.validate_for_durable_execution().unwrap();

  let execution = execute_workflow_durable(
    workflow,
    "sha256:6262626262626262626262626262626262626262626262626262626262626262".to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  assert_eq!(
    execution.result,
    json!({ "accepted": true, "sawInstagram": false, "sawFacebook": false })
  );

  let join = event_position(&execution.events, |payload| {
    matches!(payload, RunEventPayload::ForkJoinSettled(_))
  });
  let first_attempt = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "publishInstagram" || data.node_id == "publishFacebook" || data.node_id == "finish"),
  );
  let finish = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish"),
  );
  let instagram = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishInstagram"),
  );
  let facebook = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishFacebook"),
  );
  let outcome = event_position(&execution.events, |payload| {
    matches!(payload, RunEventPayload::RunOutcomeDecided(_))
  });
  assert!(join < first_attempt);
  assert!(finish < instagram);
  assert!(finish < facebook);
  assert!(instagram < outcome);
  assert!(facebook < outcome);
}
