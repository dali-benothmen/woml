use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, DurableStoreError, RunStatus,
  RuntimeExecutionOptions, ScriptHostProcessOptions, TriggerProgress,
  WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer, WomlWebhookServerConfig,
  WorkflowRoutingWakeup, WorkflowTargetRegistry, WORKFLOW_ROUTING_CONTRACT,
  WORKFLOW_ROUTING_CONTRACT_VERSION, WORKFLOW_ROUTING_WAKE_PATH,
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
      "woml-wc5-{label}-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    for suffix in ["", "-wal", "-shm", ".workflow-routing-v1.key"] {
      let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
    }
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

fn model(source: &str, replacement: &str) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(source).unwrap();
  value["graph"]["nodes"][0]["inputs"]["fields"]["source"]["value"] = json!(replacement);
  serde_json::from_value(value).unwrap()
}

fn config(
  database: &TemporaryDatabase,
  registrations: Vec<WebhookDefinitionRegistration>,
  startup_manual_triggers: BTreeMap<String, String>,
  host: ScriptHostProcessOptions,
  progress_reporter: Option<Arc<Mutex<Vec<TriggerProgress>>>>,
) -> WomlWebhookServerConfig {
  WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations,
    startup_manual_triggers,
    execution: RuntimeExecutionOptions::new(host, 2_000),
    progress_reporter: progress_reporter
      .map(|messages| Arc::new(move |message| messages.lock().unwrap().push(message)) as _),
  }
}

async fn parent_run_id(progress: &Arc<Mutex<Vec<TriggerProgress>>>) -> String {
  let deadline = Instant::now() + Duration::from_secs(10);
  loop {
    if let Some(run_id) = progress.lock().unwrap().iter().find_map(|message| {
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
    }) {
      return run_id;
    }
    assert!(Instant::now() < deadline, "parent was not admitted");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

async fn wait_for_terminal(database: &TemporaryDatabase, run_id: &str) -> RunStatus {
  let deadline = Instant::now() + Duration::from_secs(10);
  loop {
    let status = DurableEventStore::open(database.path())
      .unwrap()
      .projection(run_id)
      .unwrap()
      .status;
    if matches!(status, RunStatus::Succeeded | RunStatus::Failed) {
      return status;
    }
    assert!(Instant::now() < deadline, "run did not become terminal");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

#[actix_web::test]
async fn separate_runtime_registries_route_over_authenticated_loopback() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("cross-runtime");
  let child = WomlWebhookServer::start(config(
    &database,
    vec![WebhookDefinitionRegistration::new(
      model(
        CHILD_MODEL,
        "return { score: context.trigger.customerId === 'customer-42' ? 90 : 20 };",
      ),
      CHILD_HASH,
    )],
    BTreeMap::new(),
    host.clone(),
    None,
  ))
  .await
  .unwrap();

  let route = DurableEventStore::open(database.path())
    .unwrap()
    .workflow_runtime_route("calculate-risk", Utc::now())
    .unwrap()
    .unwrap();
  let unauthorized = reqwest::Client::new()
    .post(format!("{}{}", route.endpoint, WORKFLOW_ROUTING_WAKE_PATH))
    .bearer_auth("not-the-runtime-credential")
    .json(&WorkflowRoutingWakeup {
      contract: WORKFLOW_ROUTING_CONTRACT.to_string(),
      contract_version: WORKFLOW_ROUTING_CONTRACT_VERSION,
      kind: "wakeup".to_string(),
      runtime_id: route.runtime_id,
      child_run_id: "run_not_admitted".to_string(),
      call_key: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        .to_string(),
    })
    .send()
    .await
    .unwrap();
  assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

  let progress = Arc::new(Mutex::new(Vec::new()));
  let parent = WomlWebhookServer::start(config(
    &database,
    vec![WebhookDefinitionRegistration::new(
      model(
        PARENT_MODEL,
        "const risk = await services.workflows.call('calculate-risk', { customerId: 'customer-42' }); return { score: risk.score };",
      ),
      PARENT_HASH,
    )],
    BTreeMap::from([("request-risk".to_string(), "start".to_string())]),
    host,
    Some(Arc::clone(&progress)),
  ))
  .await
  .unwrap();

  let parent_run_id = parent_run_id(&progress).await;
  assert_eq!(
    wait_for_terminal(&database, &parent_run_id).await,
    RunStatus::Succeeded
  );
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store.projection(&parent_run_id).unwrap().result,
    Some(json!({ "score": 90 }))
  );

  parent.stop().await;
  child.stop().await;
}

#[actix_web::test]
async fn duplicate_live_owner_is_rejected_and_graceful_stop_releases_the_id() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("duplicate-owner");
  let registration =
    || WebhookDefinitionRegistration::new(model(CHILD_MODEL, "return { score: 1 };"), CHILD_HASH);
  let first = WomlWebhookServer::start(config(
    &database,
    vec![registration()],
    BTreeMap::new(),
    host.clone(),
    None,
  ))
  .await
  .unwrap();
  let duplicate = WomlWebhookServer::start(config(
    &database,
    vec![registration()],
    BTreeMap::new(),
    host.clone(),
    None,
  ))
  .await;
  assert!(matches!(
    duplicate,
    Err(WebhookRuntimeError::DurableStore(
      DurableStoreError::WorkflowRuntimeDuplicateOwner(ref id)
    )) if id == "calculate-risk"
  ));

  first.stop().await;
  let replacement = WomlWebhookServer::start(config(
    &database,
    vec![registration()],
    BTreeMap::new(),
    host,
    None,
  ))
  .await
  .unwrap();
  replacement.stop().await;
}

#[actix_web::test]
async fn pending_scan_recovers_a_lost_loopback_wakeup() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("lost-wakeup");
  let child = WomlWebhookServer::start(config(
    &database,
    vec![WebhookDefinitionRegistration::new(
      model(CHILD_MODEL, "return { score: 73 };"),
      CHILD_HASH,
    )],
    BTreeMap::new(),
    host.clone(),
    None,
  ))
  .await
  .unwrap();
  rusqlite::Connection::open(database.path())
    .unwrap()
    .execute(
      "UPDATE woml_workflow_runtime_routes SET endpoint = 'http://127.0.0.1:1'
       WHERE workflow_id = 'calculate-risk'",
      [],
    )
    .unwrap();

  let progress = Arc::new(Mutex::new(Vec::new()));
  let parent = WomlWebhookServer::start(config(
    &database,
    vec![WebhookDefinitionRegistration::new(
      model(
        PARENT_MODEL,
        "return await services.workflows.call('calculate-risk', {});",
      ),
      PARENT_HASH,
    )],
    BTreeMap::from([("request-risk".to_string(), "start".to_string())]),
    host,
    Some(Arc::clone(&progress)),
  ))
  .await
  .unwrap();

  let parent_run_id = parent_run_id(&progress).await;
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
    Some(json!({ "score": 73 }))
  );

  parent.stop().await;
  child.stop().await;
}

#[test]
fn expired_lease_can_be_replaced_but_a_live_lease_cannot() {
  let database = TemporaryDatabase::new("lease-expiry");
  let workflow = model(CHILD_MODEL, "return null;");
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&workflow, CHILD_HASH).unwrap();
  let first = WorkflowTargetRegistry::new("runtime_first").unwrap();
  let first_target = first.register(&workflow, CHILD_HASH).unwrap();
  let now = Utc::now();
  store
    .register_workflow_runtime_routes(
      "runtime_first",
      &[first_target],
      "http://127.0.0.1:43127",
      "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
      now,
      now + ChronoDuration::seconds(1),
    )
    .unwrap();

  let second = WorkflowTargetRegistry::new("runtime_second").unwrap();
  let second_target = second.register(&workflow, CHILD_HASH).unwrap();
  assert!(matches!(
    store.register_workflow_runtime_routes(
      "runtime_second",
      std::slice::from_ref(&second_target),
      "http://127.0.0.1:43128",
      "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
      now,
      now + ChronoDuration::seconds(10),
    ),
    Err(DurableStoreError::WorkflowRuntimeDuplicateOwner(ref id))
      if id == "calculate-risk"
  ));
  let after_expiry = now + ChronoDuration::seconds(2);
  assert!(store
    .workflow_runtime_route("calculate-risk", after_expiry)
    .unwrap()
    .is_none());
  store
    .register_workflow_runtime_routes(
      "runtime_second",
      &[second_target],
      "http://127.0.0.1:43128",
      "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
      after_expiry,
      after_expiry + ChronoDuration::seconds(10),
    )
    .unwrap();
  assert_eq!(
    store
      .workflow_runtime_route("calculate-risk", after_expiry)
      .unwrap()
      .unwrap()
      .runtime_id,
    "runtime_second"
  );
}
