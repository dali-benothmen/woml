use std::path::{Path, PathBuf};

use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::model::{
  CompiledWorkflowEdge, CompiledWorkflowGraph, CompiledWorkflowNode, EdgeCondition, ValueExpression,
};
use woml_engine::{
  execute_workflow, execute_workflow_durable, recover_durable_runs, BranchFailure,
  BranchFailureSite, CompiledWorkflowDefinition, RuntimeExecutionError, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const BRANCH_HASH: &str = "sha256:6a9b3aa53e81ae0e95414f80df0192de5ff11489e9b65b1254b69b71a496155a";
const MODIFIED_HASH: &str =
  "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn branch_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(BRANCH_MODEL).unwrap()
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

fn set_script_source(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let node = workflow.node(node_id).unwrap().clone();
  let index = workflow
    .graph
    .nodes
    .iter()
    .position(|candidate| candidate.id == node.id)
    .unwrap();
  let ValueExpression::Object { fields } = &mut workflow.graph.nodes[index].inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
}

fn runtime_options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 1_000)
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-b4-{label}-{}.sqlite",
        Uuid::new_v4().simple()
      )),
    }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

#[tokio::test]
async fn reviewed_branch_executes_only_the_true_route_and_publishes_one_result() {
  let Some(host) = host_options() else {
    return;
  };
  let result = execute_workflow(
    branch_model(),
    BRANCH_HASH.to_string(),
    Map::new(),
    runtime_options(host),
  )
  .await
  .unwrap();

  assert_eq!(
    result.result,
    json!({ "message": "Final status: reviewed" })
  );
  assert_eq!(
    result.execution_order,
    [
      "checkContent",
      "reviewContent",
      "decision",
      "publishDecision"
    ]
  );
  assert!(result.context.steps.contains_key("reviewContent"));
  assert!(!result.context.steps.contains_key("acceptContent"));
  assert_eq!(
    result.context.steps.get("decision"),
    Some(&json!({ "status": "reviewed", "accepted": true }))
  );
  assert!(result
    .events
    .iter()
    .all(|event| event.event_schema_version == 2));
  assert_eq!(
    result
      .events
      .iter()
      .map(|event| match &event.payload {
        woml_engine::RunEventPayload::RunStarted(_) => "run_started",
        woml_engine::RunEventPayload::StepAttemptStarted(_) => "step_attempt_started",
        woml_engine::RunEventPayload::StepAttemptSucceeded(_) => "step_attempt_succeeded",
        woml_engine::RunEventPayload::StepAttemptFailed(_) => "step_attempt_failed",
        woml_engine::RunEventPayload::BranchSelected(_) => "branch_selected",
        woml_engine::RunEventPayload::ParallelGroupStarted(_) => "parallel_group_started",
        woml_engine::RunEventPayload::ParallelGroupCompleted(_) => "parallel_group_completed",
        woml_engine::RunEventPayload::ApprovalRequested(_) => "approval_requested",
        woml_engine::RunEventPayload::ApprovalResolved(_) => "approval_resolved",
        woml_engine::RunEventPayload::RunSucceeded(_) => "run_succeeded",
        woml_engine::RunEventPayload::RunFailed(_) => "run_failed",
        _ => "notification_event",
      })
      .collect::<Vec<_>>(),
    [
      "run_started",
      "step_attempt_started",
      "step_attempt_succeeded",
      "branch_selected",
      "step_attempt_started",
      "step_attempt_succeeded",
      "step_attempt_started",
      "step_attempt_succeeded",
      "step_attempt_started",
      "step_attempt_succeeded",
      "run_succeeded",
    ]
  );
}

#[tokio::test]
async fn false_condition_executes_only_otherwise() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = branch_model();
  set_script_source(
    &mut workflow,
    "checkContent",
    "return { needsReview: false };",
  );
  let result = execute_workflow(
    workflow,
    MODIFIED_HASH.to_string(),
    Map::new(),
    runtime_options(host),
  )
  .await
  .unwrap();

  assert_eq!(
    result.result,
    json!({ "message": "Final status: accepted-automatically" })
  );
  assert_eq!(
    result.execution_order,
    [
      "checkContent",
      "acceptContent",
      "decision",
      "publishDecision"
    ]
  );
  assert!(!result.context.steps.contains_key("reviewContent"));
  assert!(result.context.steps.contains_key("acceptContent"));
}

#[tokio::test]
async fn first_true_when_wins_even_when_a_later_condition_is_also_true() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = branch_model();
  let mut second: CompiledWorkflowNode = workflow.node("acceptContent").unwrap().clone();
  second.id = "secondReview".to_string();
  set_node_script_source(&mut second, "return { status: 'second', accepted: true };");
  let decision_index = workflow
    .graph
    .nodes
    .iter()
    .position(|node| node.id == "decision")
    .unwrap();
  workflow.graph.nodes.insert(decision_index, second);
  workflow.graph.edges.insert(
    2,
    CompiledWorkflowEdge {
      id: "decision:when:1".to_string(),
      from: "__woml_branch__decision__select".to_string(),
      to: "secondReview".to_string(),
      condition: EdgeCondition::Boolean {
        value: ValueExpression::ContextReference {
          path: vec![
            "steps".to_string(),
            "checkContent".to_string(),
            "needsReview".to_string(),
          ],
        },
      },
      branch_id: Some("decision".to_string()),
      parallel_id: None,
      approval_id: None,
    },
  );
  workflow.graph.edges.insert(
    5,
    CompiledWorkflowEdge {
      id: "secondReview-to-decision".to_string(),
      from: "secondReview".to_string(),
      to: "decision".to_string(),
      condition: EdgeCondition::Always,
      branch_id: None,
      parallel_id: None,
      approval_id: None,
    },
  );
  let decision = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "decision")
    .unwrap();
  let ValueExpression::Object { fields } = &mut decision.inputs else {
    panic!("expected result map");
  };
  fields.insert(
    "decision:when:1".to_string(),
    ValueExpression::ContextReference {
      path: vec!["steps".to_string(), "secondReview".to_string()],
    },
  );
  workflow.validate_for_execution().unwrap();

  let result = execute_workflow(
    workflow,
    MODIFIED_HASH.to_string(),
    Map::new(),
    runtime_options(host),
  )
  .await
  .unwrap();
  assert!(result.context.steps.contains_key("reviewContent"));
  assert!(!result.context.steps.contains_key("secondReview"));
  assert!(!result.context.steps.contains_key("acceptContent"));
  assert!(result.events.iter().any(|event| matches!(
    &event.payload,
    woml_engine::RunEventPayload::BranchSelected(data)
      if data.arm_id == "decision:when:0"
  )));
}

fn set_node_script_source(node: &mut CompiledWorkflowNode, source: &str) {
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
}

fn script_node(id: &str, source: &str) -> CompiledWorkflowNode {
  let mut fields = std::collections::BTreeMap::new();
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: json!(source),
    },
  );
  CompiledWorkflowNode {
    id: id.to_string(),
    handler: "runtime.script".to_string(),
    inputs: ValueExpression::Object { fields },
    timeout_ms: None,
    retry_policy: None,
    script_runtime: None,
    metadata: None,
  }
}

fn engine_node(
  id: &str,
  handler: &str,
  fields: std::collections::BTreeMap<String, ValueExpression>,
) -> CompiledWorkflowNode {
  CompiledWorkflowNode {
    id: id.to_string(),
    handler: handler.to_string(),
    inputs: ValueExpression::Object { fields },
    timeout_ms: None,
    retry_policy: None,
    script_runtime: None,
    metadata: None,
  }
}

fn reference(path: &[&str]) -> ValueExpression {
  ValueExpression::ContextReference {
    path: path.iter().map(|part| (*part).to_string()).collect(),
  }
}

fn edge(id: &str, from: &str, to: &str) -> CompiledWorkflowEdge {
  CompiledWorkflowEdge {
    id: id.to_string(),
    from: from.to_string(),
    to: to.to_string(),
    condition: EdgeCondition::Always,
    branch_id: None,
    parallel_id: None,
    approval_id: None,
  }
}

fn nested_branch_model() -> CompiledWorkflowDefinition {
  let mut workflow = branch_model();
  let mut inner_results = std::collections::BTreeMap::new();
  inner_results.insert("inner:when:0".to_string(), reference(&["steps", "inside"]));
  inner_results.insert(
    "inner:otherwise".to_string(),
    reference(&["steps", "innerFallback"]),
  );
  let mut outer_results = std::collections::BTreeMap::new();
  outer_results.insert("outer:when:0".to_string(), reference(&["steps", "inner"]));
  outer_results.insert(
    "outer:otherwise".to_string(),
    reference(&["steps", "outerFallback"]),
  );
  workflow.workflow_id = "nested-branch".to_string();
  workflow.graph = CompiledWorkflowGraph {
    entry_node_ids: vec!["ready".to_string()],
    nodes: vec![
      script_node("ready", "return true;"),
      engine_node(
        "__woml_branch__outer__select",
        "engine.branch-select",
        Default::default(),
      ),
      engine_node(
        "__woml_branch__inner__select",
        "engine.branch-select",
        Default::default(),
      ),
      script_node("inside", "return { ok: true };"),
      script_node("innerFallback", "return { ok: false };"),
      engine_node("inner", "engine.branch-result", inner_results),
      script_node("outerFallback", "return { ok: false };"),
      engine_node("outer", "engine.branch-result", outer_results),
    ],
    edges: vec![
      edge(
        "ready-to-__woml_branch__outer__select",
        "ready",
        "__woml_branch__outer__select",
      ),
      CompiledWorkflowEdge {
        id: "outer:when:0".to_string(),
        from: "__woml_branch__outer__select".to_string(),
        to: "__woml_branch__inner__select".to_string(),
        condition: EdgeCondition::Boolean {
          value: reference(&["steps", "ready"]),
        },
        branch_id: Some("outer".to_string()),
        parallel_id: None,
        approval_id: None,
      },
      CompiledWorkflowEdge {
        id: "outer:otherwise".to_string(),
        from: "__woml_branch__outer__select".to_string(),
        to: "outerFallback".to_string(),
        condition: EdgeCondition::Always,
        branch_id: Some("outer".to_string()),
        parallel_id: None,
        approval_id: None,
      },
      CompiledWorkflowEdge {
        id: "inner:when:0".to_string(),
        from: "__woml_branch__inner__select".to_string(),
        to: "inside".to_string(),
        condition: EdgeCondition::Boolean {
          value: reference(&["steps", "ready"]),
        },
        branch_id: Some("inner".to_string()),
        parallel_id: None,
        approval_id: None,
      },
      CompiledWorkflowEdge {
        id: "inner:otherwise".to_string(),
        from: "__woml_branch__inner__select".to_string(),
        to: "innerFallback".to_string(),
        condition: EdgeCondition::Always,
        branch_id: Some("inner".to_string()),
        parallel_id: None,
        approval_id: None,
      },
      edge("inside-to-inner", "inside", "inner"),
      edge("innerFallback-to-inner", "innerFallback", "inner"),
      edge("inner-to-outer", "inner", "outer"),
      edge("outerFallback-to-outer", "outerFallback", "outer"),
    ],
    forks: None,
    choices: None,
    context_visibility: None,
    settlement: None,
    for_each: None,
  };
  workflow.validate_for_execution().unwrap();
  workflow
}

#[tokio::test]
async fn nested_selected_branches_compose_without_running_any_fallback() {
  let Some(host) = host_options() else {
    return;
  };
  let result = execute_workflow(
    nested_branch_model(),
    MODIFIED_HASH.to_string(),
    Map::new(),
    runtime_options(host),
  )
  .await
  .unwrap();
  assert_eq!(result.result, json!({ "ok": true }));
  assert_eq!(
    result.execution_order,
    ["ready", "inside", "inner", "outer"]
  );
  assert!(!result.context.steps.contains_key("innerFallback"));
  assert!(!result.context.steps.contains_key("outerFallback"));
  let selections = result
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      woml_engine::RunEventPayload::BranchSelected(data) => {
        Some((data.branch_id.as_str(), data.arm_id.as_str()))
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(
    selections,
    [("outer", "outer:when:0"), ("inner", "inner:when:0")]
  );
}

#[tokio::test]
async fn non_boolean_and_missing_condition_values_fail_as_branch_errors() {
  let Some(host) = host_options() else {
    return;
  };
  for (source, expected_code) in [
    (
      "return { needsReview: 'yes' };",
      "WOML_BRANCH_TEST_NOT_BOOLEAN",
    ),
    ("return {};", "WOML_REFERENCE_NOT_AVAILABLE"),
  ] {
    let mut workflow = branch_model();
    set_script_source(&mut workflow, "checkContent", source);
    let error = execute_workflow(
      workflow,
      MODIFIED_HASH.to_string(),
      Map::new(),
      runtime_options(host.clone()),
    )
    .await
    .unwrap_err();
    let RuntimeExecutionError::BranchFailed(details) = error else {
      panic!("expected branch-scoped runtime failure");
    };
    assert_eq!(details.code, expected_code);
    assert_eq!(details.branch_id, "decision");
    assert_eq!(details.arm_id.as_deref(), Some("decision:when:0"));
    assert_eq!(
      details.path.as_deref(),
      Some(
        ["steps", "checkContent", "needsReview"]
          .map(str::to_string)
          .as_slice()
      )
    );
    assert_eq!(details.site, BranchFailureSite::Test);
    assert!(matches!(
      details.events.last().map(|event| &event.payload),
      Some(woml_engine::RunEventPayload::RunFailed(_))
    ));
    assert!(!details.events.iter().any(|event| matches!(
      event.payload,
      woml_engine::RunEventPayload::BranchSelected(_)
    )));
    match details.failure {
      BranchFailure::BranchTestNotBoolean { .. } | BranchFailure::ReferenceNotAvailable { .. } => {}
      BranchFailure::BranchSelectionInvalid { .. } => {
        panic!("unexpected selection failure")
      }
    }
  }
}

#[tokio::test]
async fn missing_selected_result_keeps_result_site_and_reference_details() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = branch_model();
  let result_node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "decision")
    .unwrap();
  let ValueExpression::Object { fields } = &mut result_node.inputs else {
    panic!("expected branch result inputs");
  };
  fields.insert(
    "decision:when:0".to_string(),
    reference(&["steps", "reviewContent", "missing"]),
  );

  let error = execute_workflow(
    workflow,
    MODIFIED_HASH.to_string(),
    Map::new(),
    runtime_options(host),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::BranchFailed(details) = error else {
    panic!("expected branch-scoped runtime failure");
  };
  assert_eq!(details.code, "WOML_REFERENCE_NOT_AVAILABLE");
  assert_eq!(details.branch_id, "decision");
  assert_eq!(details.arm_id.as_deref(), Some("decision:when:0"));
  assert_eq!(
    details.path,
    Some(
      ["steps", "reviewContent", "missing"]
        .map(str::to_string)
        .to_vec()
    )
  );
  assert_eq!(details.site, BranchFailureSite::Result);
}

#[tokio::test]
async fn selected_script_failures_remain_attempt_scoped_and_keep_the_selection() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow = branch_model();
  set_script_source(
    &mut workflow,
    "reviewContent",
    "throw new Error('selected route failed');",
  );
  let error = execute_workflow(
    workflow,
    MODIFIED_HASH.to_string(),
    Map::new(),
    runtime_options(host),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected attempt-scoped run failure");
  };
  assert_eq!(details.code, "WOML_SCRIPT_THROWN");
  assert!(details.events.iter().any(|event| matches!(
    &event.payload,
    woml_engine::RunEventPayload::BranchSelected(data)
      if data.arm_id == "decision:when:0"
  )));
  assert!(matches!(
    details.events.last().map(|event| &event.payload),
    Some(woml_engine::RunEventPayload::RunFailed(
      woml_engine::RunFailedData::V2(woml_engine::RunFailedDataV2::Attempt { .. })
    ))
  ));
}

#[tokio::test]
async fn durable_execution_reopens_with_selection_and_atomic_branch_result() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("complete");
  let result = execute_workflow_durable(
    branch_model(),
    BRANCH_HASH.to_string(),
    Map::new(),
    runtime_options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  assert_eq!(
    result.result,
    json!({ "message": "Final status: reviewed" })
  );
  let decision_events = result
    .events
    .iter()
    .filter(|event| match &event.payload {
      woml_engine::RunEventPayload::StepAttemptStarted(data) => data.node_id == "decision",
      woml_engine::RunEventPayload::StepAttemptSucceeded(data) => data.node_id == "decision",
      _ => false,
    })
    .collect::<Vec<_>>();
  assert_eq!(decision_events.len(), 2);
  assert_eq!(decision_events[1].sequence, decision_events[0].sequence + 1);

  let recovery = recover_durable_runs(database.path().to_path_buf()).unwrap();
  assert_eq!(recovery.inspected_runs, 1);
  assert_eq!(recovery.recovered_runs, 0);
  assert_eq!(recovery.resumable_runs, 0);
}
