use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::{
  CompiledContextVisibility, CompiledWorkflowEdge, EdgeCondition, ValueExpression,
};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, ForkBranchOutcome, ForkJoinOutcome,
  RunEventPayload, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");
const DEFINITION_HASH: &str =
  "sha256:5151515151515151515151515151515151515151515151515151515151515151";

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

fn options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 3_000)
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

fn add_second_step(
  workflow: &mut CompiledWorkflowDefinition,
  first_id: &str,
  second_id: &str,
  terminal_id: &str,
  source: &str,
) {
  let mut second = workflow
    .node(first_id)
    .expect("first branch step exists")
    .clone();
  second.id = second_id.to_string();
  let ValueExpression::Object { fields } = &mut second.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source.to_string()),
    },
  );
  let terminal_index = workflow
    .graph
    .nodes
    .iter()
    .position(|node| node.id == terminal_id)
    .unwrap();
  workflow.graph.nodes.insert(terminal_index, second);
  workflow
    .graph
    .edges
    .retain(|edge| !(edge.from == first_id && edge.to == terminal_id));
  workflow.graph.edges.push(CompiledWorkflowEdge {
    id: format!("{first_id}-to-{second_id}"),
    from: first_id.to_string(),
    to: second_id.to_string(),
    condition: EdgeCondition::Always,
    branch_id: None,
    parallel_id: None,
    approval_id: None,
  });
  workflow.graph.edges.push(CompiledWorkflowEdge {
    id: format!("{second_id}-to-{terminal_id}"),
    from: second_id.to_string(),
    to: terminal_id.to_string(),
    condition: EdgeCondition::Always,
    branch_id: None,
    parallel_id: None,
    approval_id: None,
  });
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-fork-runtime-{}.sqlite", Uuid::new_v4())))
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

fn event_position(
  events: &[woml_engine::RunEvent],
  predicate: impl Fn(&RunEventPayload) -> bool,
) -> usize {
  events
    .iter()
    .position(|event| predicate(&event.payload))
    .unwrap()
}

#[tokio::test]
async fn joined_branches_overlap_settle_durably_and_feed_the_main_route() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  set_script(
    &mut workflow,
    "publishInstagram",
    "await new Promise(resolve => setTimeout(resolve, 160)); return { platform: 'instagram', sawFacebook: context.steps.publishFacebook !== undefined };",
  );
  add_second_step(
    &mut workflow,
    "publishInstagram",
    "verifyInstagram",
    "__woml_fork__distribution__instagram__terminal",
    "return { verified: context.steps.publishInstagram.platform };",
  );
  add_second_step(
    &mut workflow,
    "publishFacebook",
    "verifyFacebook",
    "__woml_fork__distribution__facebook__terminal",
    "return { verified: context.steps.publishFacebook.platform };",
  );
  workflow.graph.context_visibility = Some(vec![
    CompiledContextVisibility {
      node_id: "prepare".to_string(),
      step_ids: vec![],
    },
    CompiledContextVisibility {
      node_id: "publishInstagram".to_string(),
      step_ids: vec!["prepare".to_string()],
    },
    CompiledContextVisibility {
      node_id: "verifyInstagram".to_string(),
      step_ids: vec!["prepare".to_string(), "publishInstagram".to_string()],
    },
    CompiledContextVisibility {
      node_id: "publishFacebook".to_string(),
      step_ids: vec!["prepare".to_string()],
    },
    CompiledContextVisibility {
      node_id: "verifyFacebook".to_string(),
      step_ids: vec!["prepare".to_string(), "publishFacebook".to_string()],
    },
    CompiledContextVisibility {
      node_id: "finish".to_string(),
      step_ids: vec![
        "prepare".to_string(),
        "publishInstagram".to_string(),
        "verifyInstagram".to_string(),
        "publishFacebook".to_string(),
        "verifyFacebook".to_string(),
      ],
    },
  ]);
  set_script(
    &mut workflow,
    "finish",
    "return { instagram: context.steps.publishInstagram, facebook: context.steps.publishFacebook, verified: [context.steps.verifyInstagram.verified, context.steps.verifyFacebook.verified] };",
  );
  workflow.validate_for_durable_execution().unwrap();
  set_script(
    &mut workflow,
    "publishFacebook",
    "await new Promise(resolve => setTimeout(resolve, 10)); return { platform: 'facebook', sawInstagram: context.steps.publishInstagram !== undefined };",
  );

  let result = execute_workflow_durable(
    workflow,
    DEFINITION_HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(
    result.result,
    json!({
      "instagram": { "platform": "instagram", "sawFacebook": false },
      "facebook": { "platform": "facebook", "sawInstagram": false },
      "verified": ["instagram", "facebook"]
    })
  );
  assert_eq!(result.execution_order[0], "prepare");
  assert!(
    result
      .execution_order
      .iter()
      .position(|node| node == "publishFacebook")
      .unwrap()
      < result
        .execution_order
        .iter()
        .position(|node| node == "publishInstagram")
        .unwrap()
  );
  assert!(
    result
      .execution_order
      .iter()
      .position(|node| node == "publishFacebook")
      .unwrap()
      < result
        .execution_order
        .iter()
        .position(|node| node == "verifyFacebook")
        .unwrap()
  );

  let instagram_start = event_position(
    &result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "publishInstagram"),
  );
  let facebook_start = event_position(
    &result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "publishFacebook"),
  );
  let first_branch_terminal = event_position(
    &result.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishInstagram" || data.node_id == "publishFacebook"),
  );
  assert!(instagram_start < first_branch_terminal);
  assert!(facebook_start < first_branch_terminal);

  let branch_settlements = result
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::ForkBranchSettled(data) => Some(data),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(branch_settlements.len(), 2);
  assert!(branch_settlements
    .iter()
    .all(|settlement| settlement.outcome == ForkBranchOutcome::Succeeded));
  let join = result
    .events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::ForkJoinSettled(data) => Some(data),
      _ => None,
    })
    .unwrap();
  assert_eq!(join.outcome, ForkJoinOutcome::Succeeded);
  assert!(result
    .events
    .iter()
    .all(|event| event.event_schema_version == 12));

  let reopened = woml_engine::DurableEventStore::open(database.path()).unwrap();
  let projection = reopened.projection(&result.run_id).unwrap();
  assert_eq!(
    projection.forks["distribution"].join_status,
    woml_engine::ForkJoinStatus::Succeeded
  );
  assert_eq!(projection.result, Some(result.result));
}
