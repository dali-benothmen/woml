use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::{
  execute_workflow_durable, run_presentation_from_store_v1, CompiledWorkflowDefinition,
  DurableEventStore, PresentationStepKind, PresentationStepStatus, RunEventPayload,
  RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const REVIEWED_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/for-each/model.v16.reviewed.json");

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

fn sequential_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  value["graph"]["forEach"][0]["concurrency"] = json!(1);
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

fn concurrent_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  value["graph"]["forEach"][0]["concurrency"] = json!(2);
  value["graph"]["forEach"][0]["body"]["nodes"][0]["inputs"]["fields"]["source"]["value"] = json!(
    r#"
      await new Promise(resolve => setTimeout(resolve, context.item.delay));
      return { value: context.item.value, index: context.iteration.index };
    "#
  );
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

fn failing_concurrent_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  value["graph"]["forEach"][0]["concurrency"] = json!(2);
  value["graph"]["forEach"][0]["body"]["nodes"][0]["inputs"]["fields"]["source"]["value"] = json!(
    r#"
        if (context.item.fail) {
          throw new Error('item failed');
        }
        await new Promise(resolve => setTimeout(resolve, context.item.delay));
        return { value: context.item.value, index: context.iteration.index };
      "#
  );
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

fn timeout_concurrent_model() -> CompiledWorkflowDefinition {
  let mut workflow = concurrent_model();
  // Leave enough startup headroom when the full test binary launches several
  // Bun hosts concurrently; the test specifically targets an already-open
  // loop rather than a deadline reached before its first iteration.
  workflow.runtime_policy.as_mut().unwrap().timeout_ms = Some(3_000);
  workflow.validate_for_durable_execution().unwrap();
  workflow
}

fn inner_parallel_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(REVIEWED_MODEL).unwrap();
  value["graph"]["forEach"][0]["concurrency"] = json!(1);
  let runtime = value["graph"]["forEach"][0]["body"]["nodes"][0]["scriptRuntime"].clone();
  value["graph"]["forEach"][0]["body"] = json!({
    "entryNodeIds": ["__woml_parallel__inspect__start"],
    "nodes": [
      {
        "id": "__woml_parallel__inspect__start",
        "handler": "engine.parallel-start",
        "inputs": {
          "kind": "object",
          "fields": {
            "concurrency": { "kind": "literal", "value": 2 },
            "onError": { "kind": "literal", "value": "wait-all" }
          }
        }
      },
      {
        "id": "slowChild",
        "handler": "runtime.script",
        "inputs": {
          "kind": "object",
          "fields": {
            "source": {
              "kind": "literal",
              "value": "await new Promise(resolve => setTimeout(resolve, 800)); return { child: 'slow' };"
            }
          }
        },
        "scriptRuntime": runtime
      },
      {
        "id": "fastChild",
        "handler": "runtime.script",
        "inputs": {
          "kind": "object",
          "fields": {
            "source": {
              "kind": "literal",
              "value": "await new Promise(resolve => setTimeout(resolve, 5)); return { child: 'fast' };"
            }
          }
        },
        "scriptRuntime": runtime
      },
      {
        "id": "thirdChild",
        "handler": "runtime.script",
        "inputs": {
          "kind": "object",
          "fields": {
            "source": {
              "kind": "literal",
              "value": "await new Promise(resolve => setTimeout(resolve, 5)); return { child: 'third' };"
            }
          }
        },
        "scriptRuntime": runtime
      },
      {
        "id": "inspect",
        "handler": "engine.parallel-join",
        "inputs": { "kind": "object", "fields": {} }
      }
    ],
    "edges": [
      {
        "id": "inspect:child:0",
        "from": "__woml_parallel__inspect__start",
        "to": "slowChild",
        "condition": { "kind": "always" },
        "parallelId": "inspect"
      },
      {
        "id": "inspect:child:1",
        "from": "__woml_parallel__inspect__start",
        "to": "fastChild",
        "condition": { "kind": "always" },
        "parallelId": "inspect"
      },
      {
        "id": "inspect:join:0",
        "from": "slowChild",
        "to": "inspect",
        "condition": { "kind": "always" },
        "parallelId": "inspect"
      },
      {
        "id": "inspect:child:2",
        "from": "__woml_parallel__inspect__start",
        "to": "thirdChild",
        "condition": { "kind": "always" },
        "parallelId": "inspect"
      },
      {
        "id": "inspect:join:1",
        "from": "fastChild",
        "to": "inspect",
        "condition": { "kind": "always" },
        "parallelId": "inspect"
      },
      {
        "id": "inspect:join:2",
        "from": "thirdChild",
        "to": "inspect",
        "condition": { "kind": "always" },
        "parallelId": "inspect"
      }
    ],
    "choices": [],
    "contextVisibility": [
      { "nodeId": "slowChild", "stepIds": [] },
      { "nodeId": "fastChild", "stepIds": [] },
      { "nodeId": "thirdChild", "stepIds": [] }
    ],
    "terminalNodeId": "inspect"
  });
  value["graph"]["forEach"][0]["result"] = json!({
    "kind": "contextReference",
    "path": ["steps", "slowChild"]
  });
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

#[tokio::test]
async fn sequential_iterations_publish_ordered_results_for_later_steps() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = sequential_model();
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), json!(["alpha", "beta", "gamma"]));

  let result = execute_workflow_durable(
    model,
    "sha256:1616161616161616161616161616161616161616161616161616161616161616".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result, json!({ "processed": 3 }));
  assert_eq!(result.context.steps["organize"]["total"], 3);
  assert_eq!(result.context.steps["organize"]["succeeded"], 3);
  assert_eq!(
    result.context.steps["organize"]["results"],
    json!([
      { "value": "alpha", "index": 0 },
      { "value": "beta", "index": 1 },
      { "value": "gamma", "index": 2 }
    ])
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationSucceeded(_)))
      .count(),
    3
  );
  assert!(result.events.iter().all(|event| {
    !matches!(
      event.payload,
      RunEventPayload::ForEachIterationStarted(_) | RunEventPayload::ForEachIterationSucceeded(_)
    ) || event.iteration.is_some()
  }));
  let reopened = DurableEventStore::open(database.path()).unwrap();
  let projection = reopened.projection(&result.run_id).unwrap();
  assert_eq!(
    projection.context.steps["organize"],
    result.context.steps["organize"]
  );

  let presentation = run_presentation_from_store_v1(&reopened, &result.run_id).unwrap();
  let loop_step = presentation
    .steps
    .iter()
    .find(|step| step.id == "organize")
    .unwrap();
  assert_eq!(loop_step.kind, PresentationStepKind::ForEach);
  assert_eq!(loop_step.status, PresentationStepStatus::Succeeded);
  assert_eq!(
    loop_step.detail.as_deref(),
    Some("3 items · 3 succeeded · concurrency 1")
  );
  let loop_summary = loop_step.for_each.as_ref().unwrap();
  assert_eq!(loop_summary.total, 3);
  assert_eq!(loop_summary.succeeded, 3);
  assert_eq!(loop_summary.active, 0);
  assert_eq!(loop_summary.pending, 0);
  assert_eq!(loop_summary.iterations.len(), 3);

  let inspection = reopened.inspect_run_v6(&result.run_id).unwrap();
  assert_eq!(inspection.profile, "woml.run-inspection/v6");
  assert_eq!(inspection.for_each.counts.succeeded, 1);
  assert_eq!(inspection.for_each.items[0].for_each_id, "organize");
  assert_eq!(inspection.for_each.items[0].succeeded, 3);
  let inspection_json = serde_json::to_string(&inspection).unwrap();
  assert!(!inspection_json.contains("alpha"));
  assert!(!inspection_json.contains("beta"));
  assert!(!inspection_json.contains("gamma"));
}

#[tokio::test]
async fn an_empty_items_array_settles_without_an_iteration() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = sequential_model();
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), json!([]));

  let result = execute_workflow_durable(
    model,
    "sha256:2626262626262626262626262626262626262626262626262626262626262626".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result, json!({ "processed": 0 }));
  assert_eq!(
    result.context.steps["organize"],
    json!({ "total": 0, "succeeded": 0 })
  );
  assert!(!result
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::ForEachIterationStarted(_))));
}

#[tokio::test]
async fn bounded_iterations_complete_out_of_order_and_aggregate_in_input_order() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = concurrent_model();
  let mut trigger = Map::new();
  trigger.insert(
    "items".to_string(),
    json!([
      { "value": "slow-first", "delay": 800 },
      { "value": "fast-second", "delay": 5 },
      { "value": "third", "delay": 5 }
    ]),
  );

  let result = execute_workflow_durable(
    model,
    "sha256:3636363636363636363636363636363636363636363636363636363636363636".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(
    result.context.steps["organize"]["results"],
    json!([
      { "value": "slow-first", "index": 0 },
      { "value": "fast-second", "index": 1 },
      { "value": "third", "index": 2 }
    ])
  );

  let completion_order = result
    .events
    .iter()
    .filter_map(|event| {
      matches!(event.payload, RunEventPayload::ForEachIterationSucceeded(_))
        .then(|| event.iteration.as_ref().unwrap().index)
    })
    .collect::<Vec<_>>();
  assert!(
    completion_order.iter().position(|index| *index == 1)
      < completion_order.iter().position(|index| *index == 0),
    "the faster second item should settle before the slower first item: {completion_order:?}"
  );
  let mut completed_indexes = completion_order.clone();
  completed_indexes.sort_unstable();
  assert_eq!(completed_indexes, vec![0, 1, 2]);

  let mut active = 0_i32;
  let mut peak = 0_i32;
  for event in &result.events {
    match event.payload {
      RunEventPayload::ForEachIterationStarted(_) => {
        active += 1;
        peak = peak.max(active);
      }
      RunEventPayload::ForEachIterationSucceeded(_)
      | RunEventPayload::ForEachIterationFailed(_) => active -= 1,
      _ => {}
    }
  }
  assert_eq!(peak, 2);
  assert_eq!(active, 0);
}

#[tokio::test]
async fn many_concurrent_iterations_keep_unique_dynamic_identity_and_input_order() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let mut model = concurrent_model();
  model.graph.for_each.as_mut().unwrap()[0].concurrency = 8;
  model.validate_for_durable_execution().unwrap();
  let items = (0..16)
    .map(|index| json!({ "value": index, "delay": (index * 7) % 4 }))
    .collect::<Vec<_>>();
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), Value::Array(items));

  let result = execute_workflow_durable(
    model,
    "sha256:3737373737373737373737373737373737373737373737373737373737373737".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 5_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  let results = result.context.steps["organize"]["results"]
    .as_array()
    .unwrap();
  assert_eq!(results.len(), 16);
  assert!(results
    .iter()
    .enumerate()
    .all(|(index, value)| { value["value"] == json!(index) && value["index"] == json!(index) }));

  let identities = result
    .events
    .iter()
    .filter_map(|event| {
      matches!(event.payload, RunEventPayload::ForEachIterationStarted(_)).then(|| {
        let iteration = event.iteration.as_ref().unwrap();
        (iteration.for_each_id.clone(), iteration.index)
      })
    })
    .collect::<std::collections::BTreeSet<_>>();
  assert_eq!(identities.len(), 16);
  assert!(identities
    .iter()
    .enumerate()
    .all(|(index, (for_each_id, item_index))| {
      for_each_id == "organize" && *item_index as usize == index
    }));
}

#[tokio::test]
async fn inner_parallel_children_use_their_own_concurrency_limit() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = inner_parallel_model();
  let mut trigger = Map::new();
  trigger.insert("items".to_string(), json!(["only-item"]));

  let result = execute_workflow_durable(
    model,
    "sha256:4646464646464646464646464646464646464646464646464646464646464646".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  let child_completion_order = result
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptSucceeded(data)
        if data.node_id == "slowChild"
          || data.node_id == "fastChild"
          || data.node_id == "thirdChild" =>
      {
        Some(data.node_id.as_str())
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  assert!(
    child_completion_order
      .iter()
      .position(|node_id| *node_id == "fastChild")
      < child_completion_order
        .iter()
        .position(|node_id| *node_id == "slowChild")
  );
  let mut active_children = 0_i32;
  let mut peak_children = 0_i32;
  for event in &result.events {
    match &event.payload {
      RunEventPayload::StepAttemptStarted(data)
        if data.node_id == "slowChild"
          || data.node_id == "fastChild"
          || data.node_id == "thirdChild" =>
      {
        active_children += 1;
        peak_children = peak_children.max(active_children);
      }
      RunEventPayload::StepAttemptSucceeded(data)
        if data.node_id == "slowChild"
          || data.node_id == "fastChild"
          || data.node_id == "thirdChild" =>
      {
        active_children -= 1;
      }
      _ => {}
    }
  }
  assert_eq!(peak_children, 2);
  assert_eq!(active_children, 0);
  assert_eq!(
    result.context.steps["organize"]["results"],
    json!([{ "child": "slow" }])
  );
}

#[tokio::test]
async fn failure_settles_active_and_pending_iterations_before_the_run_fails() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = failing_concurrent_model();
  let mut trigger = Map::new();
  trigger.insert(
    "items".to_string(),
    json!([
      { "value": "fails", "fail": true, "delay": 0 },
      { "value": "active", "fail": false, "delay": 800 },
      { "value": "pending-1", "fail": false, "delay": 5 },
      { "value": "pending-2", "fail": false, "delay": 5 }
    ]),
  );

  let error = execute_workflow_durable(
    model,
    "sha256:5656565656565656565656565656565656565656565656565656565656565656".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let woml_engine::RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected a durable run failure");
  };
  let events = details.events;
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationFailed(_)))
      .count(),
    2
  );
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationSkipped(_)))
      .count(),
    2
  );
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachFailed(_)))
      .count(),
    1
  );
  let run_id = events[0].run_id.clone();
  let reopened = DurableEventStore::open(database.path()).unwrap();
  let projection = reopened.projection(&run_id).unwrap();
  assert_eq!(projection.status, woml_engine::RunStatus::Failed);
  let presentation = run_presentation_from_store_v1(&reopened, &run_id).unwrap();
  let loop_step = presentation
    .steps
    .iter()
    .find(|step| step.id == "organize")
    .unwrap();
  assert_eq!(loop_step.kind, PresentationStepKind::ForEach);
  assert_eq!(loop_step.status, PresentationStepStatus::Failed);
  assert!(loop_step.detail.as_deref().unwrap().contains("Item 1 of 4"));
  assert!(loop_step.detail.as_deref().unwrap().contains("index 0"));
  assert!(loop_step
    .detail
    .as_deref()
    .unwrap()
    .contains("step \"normalize\""));
  let loop_summary = loop_step.for_each.as_ref().unwrap();
  assert_eq!(loop_summary.total, 4);
  assert_eq!(loop_summary.failed, 2);
  assert_eq!(loop_summary.skipped, 2);
  let inspection = reopened.inspect_run_v6(&run_id).unwrap();
  assert_eq!(inspection.for_each.counts.failed, 1);
  assert_eq!(inspection.for_each.items[0].failed_index, Some(0));
  assert_eq!(
    inspection.for_each.items[0].failed_node_id.as_deref(),
    Some("normalize")
  );
}

#[tokio::test]
async fn workflow_timeout_settles_the_open_loop_before_deciding_the_run_outcome() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let model = timeout_concurrent_model();
  let mut trigger = Map::new();
  trigger.insert(
    "items".to_string(),
    json!([
      { "value": "active-1", "delay": 10_000 },
      { "value": "active-2", "delay": 10_000 },
      { "value": "pending", "delay": 5 }
    ]),
  );

  let error = execute_workflow_durable(
    model,
    "sha256:5757575757575757575757575757575757575757575757575757575757575757".to_string(),
    trigger,
    RuntimeExecutionOptions::new(host, 12_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let details = match error {
    woml_engine::RuntimeExecutionError::RunFailed(details) => details,
    other => panic!("expected a timeout run failure, received {other:?}"),
  };
  assert_eq!(details.code, "WOML_WORKFLOW_TIMED_OUT");
  let loop_settled = details
    .events
    .iter()
    .position(|event| matches!(event.payload, RunEventPayload::ForEachFailed(_)))
    .unwrap();
  let outcome_decided = details
    .events
    .iter()
    .position(|event| matches!(event.payload, RunEventPayload::RunOutcomeDecided(_)))
    .unwrap();
  assert!(loop_settled < outcome_decided);
  assert_eq!(
    details
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::ForEachIterationSkipped(_)))
      .count(),
    1
  );
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!("woml-for-each-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    Self(directory.join("state.sqlite"))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    if let Some(directory) = self.0.parent() {
      let _ = std::fs::remove_dir_all(directory);
    }
  }
}
