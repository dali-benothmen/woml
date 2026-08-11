use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, RunStatus, RuntimeExecutionOptions,
  ScriptHostProcessOptions, TriggerProgress, WebhookDefinitionRegistration, WomlWebhookServer,
  WomlWebhookServerConfig, WorkflowCallProgress,
};

const PARENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/request-risk.compiled.v8.json");
const CHILD_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/calculate-risk.compiled.v10.json");
const PARENT_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CHILD_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-wc3-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
  }
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

fn model(source: &str, replacement: Option<&str>) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(source).unwrap();
  if let Some(replacement) = replacement {
    value["graph"]["nodes"][0]["inputs"]["fields"]["source"]["value"] = json!(replacement);
  }
  serde_json::from_value(value).unwrap()
}

async fn run_case(
  label: &str,
  parent_source: &str,
  child_source: &str,
) -> Option<(
  TemporaryDatabase,
  String,
  WomlWebhookServer,
  Arc<Mutex<Vec<WorkflowCallProgress>>>,
)> {
  let host = host_options()?;
  let database = TemporaryDatabase::new(label);
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let captured = Arc::clone(&progress);
  let call_progress = Arc::new(Mutex::new(Vec::<WorkflowCallProgress>::new()));
  let captured_call_progress = Arc::clone(&call_progress);
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![
      WebhookDefinitionRegistration::new(model(PARENT_MODEL, Some(parent_source)), PARENT_HASH),
      WebhookDefinitionRegistration::new(model(CHILD_MODEL, Some(child_source)), CHILD_HASH),
    ],
    startup_manual_triggers: BTreeMap::from([("request-risk".to_string(), "start".to_string())]),
    execution: RuntimeExecutionOptions::new(host, 2_000).with_workflow_call_progress_reporter(
      Arc::new(move |message| captured_call_progress.lock().unwrap().push(message)),
    ),
    progress_reporter: Some(Arc::new(move |message| {
      captured.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();

  let deadline = Instant::now() + Duration::from_secs(10);
  let parent_run_id = loop {
    let accepted = progress.lock().unwrap().iter().find_map(|message| {
      if let TriggerProgress::OccurrenceAccepted {
        workflow_id,
        run_id,
        ..
      } = message
      {
        (workflow_id == "request-risk").then(|| run_id.clone())
      } else {
        None
      }
    });
    if let Some(run_id) = accepted {
      break run_id;
    }
    assert!(Instant::now() < deadline, "parent was not admitted");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  };
  Some((database, parent_run_id, server, call_progress))
}

async fn wait_for_terminal(database: &TemporaryDatabase, run_id: &str) -> RunStatus {
  let deadline = Instant::now() + Duration::from_secs(10);
  loop {
    let projection = DurableEventStore::open(database.path())
      .unwrap()
      .projection(run_id)
      .unwrap();
    if matches!(projection.status, RunStatus::Succeeded | RunStatus::Failed) {
      return projection.status;
    }
    assert!(Instant::now() < deadline, "run did not become terminal");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

#[actix_web::test]
async fn same_runtime_call_executes_child_with_payload_and_returns_object_result() {
  let Some((database, parent_run_id, server, call_progress)) = run_case(
    "object",
    "const risk = await services.workflows.call('calculate-risk', { customerId: 'customer-42' }); return { score: risk.score };",
    "return { score: context.payload.customerId === 'customer-42' ? 90 : 20 };",
  )
  .await
  else {
    return;
  };
  assert_eq!(
    wait_for_terminal(&database, &parent_run_id).await,
    RunStatus::Succeeded
  );
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store.projection(&parent_run_id).unwrap().result,
    Some(json!({ "score": 90 }))
  );
  let connection = rusqlite::Connection::open(database.path()).unwrap();
  let (child_run_id, state): (String, String) = connection
    .query_row(
      "SELECT child_run_id, state FROM woml_workflow_calls",
      [],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap();
  assert_eq!(state, "succeeded");
  let child = store.projection(&child_run_id).unwrap();
  assert_eq!(child.context.trigger["customerId"], "customer-42");
  assert_eq!(child.result, Some(json!({ "score": 90 })));
  let progress = call_progress.lock().unwrap();
  assert!(progress.iter().any(|message| matches!(
    message,
    WorkflowCallProgress::CallAdmitted {
      parent_run_id: progress_parent,
      child_run_id: progress_child,
      ..
    } if progress_parent == &parent_run_id && progress_child == &child_run_id
  )));
  assert!(progress.iter().any(|message| matches!(
    message,
    WorkflowCallProgress::ChildTerminal {
      child_run_id: progress_child,
      status: "succeeded",
      ..
    } if progress_child == &child_run_id
  )));
  drop(progress);
  server.stop().await;
}

#[actix_web::test]
async fn workflow_start_returns_after_admission_while_the_child_keeps_running() {
  let Some((database, parent_run_id, server, _)) = run_case(
    "start",
    "const started = await services.workflows.start('calculate-risk', { customerId: 'customer-42' }); return { childRunId: started.runId };",
    "await new Promise(resolve => setTimeout(resolve, 750)); return { score: context.payload.customerId === 'customer-42' ? 90 : 20 };",
  )
  .await
  else {
    return;
  };
  assert_eq!(
    wait_for_terminal(&database, &parent_run_id).await,
    RunStatus::Succeeded
  );
  let store = DurableEventStore::open(database.path()).unwrap();
  let parent = store.projection(&parent_run_id).unwrap();
  let child_run_id = parent
    .result
    .as_ref()
    .and_then(|result| result.get("childRunId"))
    .and_then(Value::as_str)
    .unwrap()
    .to_string();
  assert_eq!(store.projection(&child_run_id).unwrap().status, RunStatus::Running);
  assert_eq!(
    wait_for_terminal(&database, &child_run_id).await,
    RunStatus::Succeeded
  );
  assert_eq!(
    DurableEventStore::open(database.path())
      .unwrap()
      .projection(&child_run_id)
      .unwrap()
      .result,
    Some(json!({ "score": 90 }))
  );
  server.stop().await;
}

#[actix_web::test]
async fn scalar_and_null_child_results_remain_intentional_json_values() {
  for (label, child_source, expected) in [
    ("scalar", "return 42;", json!(42)),
    ("null", "return null;", Value::Null),
  ] {
    let Some((database, parent_run_id, server, _)) = run_case(
      label,
      "return await services.workflows.call('calculate-risk', {});",
      child_source,
    )
    .await
    else {
      return;
    };
    assert_eq!(
      wait_for_terminal(&database, &parent_run_id).await,
      RunStatus::Succeeded
    );
    assert_eq!(
      DurableEventStore::open(database.path())
        .unwrap()
        .projection(&parent_run_id)
        .unwrap()
        .result,
      Some(expected)
    );
    server.stop().await;
  }
}

#[actix_web::test]
async fn child_failure_missing_result_and_wait_timeout_are_catchable_service_failures() {
  for (label, parent_source, child_source, expected_code) in [
    (
      "failure",
      "return await services.workflows.call('calculate-risk', {});",
      "throw new Error('child failed');",
      "WOML_WORKFLOW_CALL_FAILED",
    ),
    (
      "missing",
      "return await services.workflows.call('calculate-risk', {});",
      "return undefined;",
      "WOML_WORKFLOW_RESULT_MISSING",
    ),
    (
      "timeout",
      "return await services.workflows.call('calculate-risk', {}, { timeout: '10ms' });",
      "await new Promise(resolve => setTimeout(resolve, 100)); return { late: true };",
      "WOML_WORKFLOW_CALL_TIMED_OUT",
    ),
  ] {
    let Some((database, parent_run_id, server, _)) =
      run_case(label, parent_source, child_source).await
    else {
      return;
    };
    assert_eq!(
      wait_for_terminal(&database, &parent_run_id).await,
      RunStatus::Failed
    );
    let projection = DurableEventStore::open(database.path())
      .unwrap()
      .projection(&parent_run_id)
      .unwrap();
    assert!(
      format!("{:?}", projection.failure).contains(expected_code),
      "expected {expected_code}, got {:?}",
      projection.failure
    );
    server.stop().await;
  }
}
