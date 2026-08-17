use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, ForkBranchOutcome, ForkJoinOutcome,
  RunEventPayload, RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
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
    Self(std::env::temp_dir().join(format!("woml-fork-failure-{}.sqlite", Uuid::new_v4())))
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

fn failed_events(error: RuntimeExecutionError) -> Vec<woml_engine::RunEvent> {
  match error {
    RuntimeExecutionError::RunFailed(details) => details.events,
    other => panic!("expected a truthful failed run, got {other:?}"),
  }
}

#[tokio::test]
async fn joined_failure_closes_the_barrier_attempts_siblings_and_never_runs_main() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  set_script(
    &mut workflow,
    "publishInstagram",
    "await new Promise(resolve => setTimeout(resolve, 10)); throw new Error('instagram rejected');"
      .to_string(),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    "await new Promise(resolve => setTimeout(resolve, 90)); return { platform: 'facebook' };"
      .to_string(),
  );
  workflow.validate_for_durable_execution().unwrap();

  let error = execute_workflow_durable(
    workflow,
    "sha256:7171717171717171717171717171717171717171717171717171717171717171".to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let initial_events = failed_events(error);
  let run_id = initial_events.first().unwrap().run_id.clone();
  let events = woml_engine::DurableEventStore::open(database.path())
    .unwrap()
    .events(&run_id)
    .unwrap();
  let instagram = events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::ForkBranchSettled(data) if data.branch_id == "instagram" => Some(data),
      _ => None,
    })
    .unwrap();
  assert_eq!(instagram.outcome, ForkBranchOutcome::Failed);
  let facebook = events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::ForkBranchSettled(data) if data.branch_id == "facebook" => Some(data),
      _ => None,
    })
    .unwrap();
  assert_eq!(facebook.outcome, ForkBranchOutcome::Succeeded);
  let join = events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::ForkJoinSettled(data) => Some(data),
      _ => None,
    })
    .unwrap();
  assert_eq!(join.outcome, ForkJoinOutcome::Failed);
  assert_eq!(join.blocking_branch_id.as_deref(), Some("instagram"));
  assert!(!events.iter().any(|event| {
    matches!(&event.payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "finish")
  }));
  assert!(matches!(
    events.last().map(|event| &event.payload),
    Some(RunEventPayload::RunFinalized(_))
  ));
}

async fn selected_failure_events(facebook_delay_ms: u64) -> Vec<woml_engine::RunEvent> {
  let host = host_options().unwrap();
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  workflow.graph.forks.as_mut().unwrap()[0].joined_branch_ids = vec!["instagram".to_string()];
  workflow
    .graph
    .edges
    .retain(|edge| edge.id != "distribution:join:facebook");
  workflow
    .graph
    .context_visibility
    .as_mut()
    .unwrap()
    .iter_mut()
    .find(|visibility| visibility.node_id == "finish")
    .unwrap()
    .step_ids
    .retain(|step_id| step_id != "publishFacebook");
  set_script(
    &mut workflow,
    "publishInstagram",
    "await new Promise(resolve => setTimeout(resolve, 40)); return { platform: 'instagram' };"
      .to_string(),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    format!(
      "await new Promise(resolve => setTimeout(resolve, {facebook_delay_ms})); throw new Error('analytics transport failed');"
    ),
  );
  set_script(
    &mut workflow,
    "finish",
    "await new Promise(resolve => setTimeout(resolve, 10)); return { main: 'recorded', sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  workflow.validate_for_durable_execution().unwrap();
  failed_events(
    execute_workflow_durable(
      workflow,
      format!("sha256:{facebook_delay_ms:064x}"),
      Map::new(),
      RuntimeExecutionOptions::new(host, 3_000),
      database.path().to_path_buf(),
    )
    .await
    .unwrap_err(),
  )
}

#[tokio::test]
async fn unjoined_failure_never_blocks_selected_join_and_partial_main_is_not_public_success() {
  if host_options().is_none() {
    return;
  }
  let early = selected_failure_events(5).await;
  let late = selected_failure_events(150).await;
  for events in [&early, &late] {
    let join = events
      .iter()
      .find_map(|event| match &event.payload {
        RunEventPayload::ForkJoinSettled(data) => Some(data),
        _ => None,
      })
      .unwrap();
    assert_eq!(join.outcome, ForkJoinOutcome::Succeeded);
    assert!(events.iter().any(|event| {
      matches!(&event.payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish")
    }));
    assert!(!events.iter().any(|event| {
      matches!(
        event.payload,
        RunEventPayload::RunOutcomeDecided(woml_engine::RunOutcomeDecidedData::Succeeded { .. })
      )
    }));
  }

  let early_failure = early
    .iter()
    .position(|event| matches!(&event.payload, RunEventPayload::StepAttemptFailed(data) if data.node_id == "publishFacebook"))
    .unwrap();
  let early_join = early
    .iter()
    .position(|event| matches!(event.payload, RunEventPayload::ForkJoinSettled(_)))
    .unwrap();
  assert!(early_failure < early_join);

  let late_finish = late
    .iter()
    .position(|event| matches!(&event.payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish"))
    .unwrap();
  let late_failure = late
    .iter()
    .position(|event| matches!(&event.payload, RunEventPayload::StepAttemptFailed(data) if data.node_id == "publishFacebook"))
    .unwrap();
  assert!(late_finish < late_failure);
}

#[tokio::test]
async fn workflow_failure_and_complete_hooks_wait_for_every_owned_branch() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let mut workflow = model();
  set_script(
    &mut workflow,
    "publishInstagram",
    "throw new Error('joined route failed');".to_string(),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    "await new Promise(resolve => setTimeout(resolve, 80)); return { platform: 'facebook' };"
      .to_string(),
  );
  workflow.lifecycle = Some(
    serde_json::from_value(json!({
      "profileVersion": 1,
      "hooks": [
        {
          "hookId": "lifecycle:run_failure",
          "event": "run_failure",
          "actions": [{
            "actionId": "lifecycle:run_failure:action:0",
            "handler": "runtime.lifecycle-script",
            "inputs": { "kind": "object", "fields": {
              "source": { "kind": "literal", "value": "return null;" }
            }},
            "scriptRuntime": {
              "bindingVersion": 2,
              "bindings": ["context", "lifecycle", "attempt", "services", "secrets"],
              "requiredSecrets": []
            }
          }]
        },
        {
          "hookId": "lifecycle:run_complete",
          "event": "run_complete",
          "actions": [{
            "actionId": "lifecycle:run_complete:action:0",
            "handler": "runtime.lifecycle-script",
            "inputs": { "kind": "object", "fields": {
              "source": { "kind": "literal", "value": "return null;" }
            }},
            "scriptRuntime": {
              "bindingVersion": 2,
              "bindings": ["context", "lifecycle", "attempt", "services", "secrets"],
              "requiredSecrets": []
            }
          }]
        }
      ]
    }))
    .unwrap(),
  );
  workflow.validate_for_durable_execution().unwrap();
  let events = failed_events(
    execute_workflow_durable(
      workflow,
      "sha256:9999999999999999999999999999999999999999999999999999999999999999".to_string(),
      Map::new(),
      RuntimeExecutionOptions::new(host, 3_000),
      database.path().to_path_buf(),
    )
    .await
    .unwrap_err(),
  );
  let last_branch = events
    .iter()
    .filter(|event| matches!(event.payload, RunEventPayload::ForkBranchSettled(_)))
    .map(|event| event.sequence)
    .max()
    .unwrap();
  let hook_requests = events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::LifecycleHookRequested(data) => Some((data.event, event.sequence)),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(
    hook_requests
      .iter()
      .map(|(event, _)| *event)
      .collect::<Vec<_>>(),
    vec![woml_engine::LifecycleEventName::RunFailure]
  );
  assert!(hook_requests[0].1 > last_branch);
}
