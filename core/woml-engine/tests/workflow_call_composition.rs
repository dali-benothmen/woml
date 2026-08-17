use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, RunStatus, RuntimeExecutionOptions,
  ScriptHostProcessOptions, TriggerProgress, WebhookDefinitionRegistration, WomlWebhookServer,
  WomlWebhookServerConfig, WorkflowCallProgress,
};

const CALLER_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/request-risk.compiled.v8.json");
const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const PARALLEL_MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const RETRY_MODEL: &str = include_str!("../../../woml/tests/fixtures/retry.compiled.v6.json");
const EVENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-event.compiled.v7.json");
const TOP_HASH: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const MIDDLE_HASH: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const BRANCH_HASH: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const PARALLEL_HASH: &str =
  "sha256:4444444444444444444444444444444444444444444444444444444444444444";
const RETRY_HASH: &str = "sha256:5555555555555555555555555555555555555555555555555555555555555555";
const EVENT_HASH: &str = "sha256:6666666666666666666666666666666666666666666666666666666666666666";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-wc6-{}.sqlite", Uuid::new_v4().simple())))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    for suffix in [
      "",
      "-wal",
      "-shm",
      ".workflow-routing-v1.key",
      ".application.sqlite",
    ] {
      let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
    }
  }
}

struct LocalJsonServer {
  address: SocketAddr,
  stop: Arc<AtomicBool>,
  thread: Option<thread::JoinHandle<()>>,
}

impl LocalJsonServer {
  fn start() -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
      while !thread_stop.load(Ordering::Relaxed) {
        match listener.accept() {
          Ok((stream, _)) => {
            thread::spawn(move || respond(stream));
          }
          Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            thread::sleep(Duration::from_millis(1));
          }
          Err(_) => break,
        }
      }
    });
    Self {
      address,
      stop,
      thread: Some(handle),
    }
  }

  fn url(&self) -> String {
    format!("http://{}/data", self.address)
  }
}

impl Drop for LocalJsonServer {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(self.address);
    if let Some(handle) = self.thread.take() {
      let _ = handle.join();
    }
  }
}

fn respond(mut stream: TcpStream) {
  let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
  let mut request = [0_u8; 16_384];
  let _ = stream.read(&mut request);
  let body = br#"{"customer":"Dali"}"#;
  let response = format!(
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  );
  if stream.write_all(response.as_bytes()).is_ok() {
    let _ = stream.write_all(body);
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

fn caller_model(workflow_id: &str, source: &str) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(CALLER_MODEL).unwrap();
  value["workflowId"] = json!(workflow_id);
  value["graph"]["nodes"][0]["inputs"]["fields"]["source"]["value"] = json!(source);
  serde_json::from_value(value).unwrap()
}

fn fixture_model(source: &str) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(source).unwrap();
  value["schemaVersion"] = json!(7);
  serde_json::from_value(value).unwrap()
}

fn retry_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(RETRY_MODEL).unwrap();
  value["schemaVersion"] = json!(7);
  value["graph"]["nodes"][1]["retryPolicy"]["backoff"]["initialDelayMs"] = json!(1);
  value["graph"]["nodes"][1]["retryPolicy"]["backoff"]["maximumDelayMs"] = json!(1);
  value["graph"]["nodes"][1]["inputs"]["fields"]["source"]["value"] = json!(
    "if (attempt.number < 2) throw new Error('temporary'); return { message: `Hello ${context.steps.prepare.name}` };"
  );
  serde_json::from_value(value).unwrap()
}

fn config(
  database: &TemporaryDatabase,
  registrations: Vec<WebhookDefinitionRegistration>,
  startup_manual_triggers: BTreeMap<String, String>,
  host: ScriptHostProcessOptions,
  trigger_progress: Option<Arc<Mutex<Vec<TriggerProgress>>>>,
  call_progress: Arc<Mutex<Vec<WorkflowCallProgress>>>,
) -> WomlWebhookServerConfig {
  WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations,
    startup_manual_triggers,
    execution: RuntimeExecutionOptions::new(host, 15_000).with_workflow_call_progress_reporter(
      Arc::new(move |message| call_progress.lock().unwrap().push(message)),
    ),
    progress_reporter: trigger_progress
      .map(|messages| Arc::new(move |message| messages.lock().unwrap().push(message)) as _),
  }
}

async fn top_run_id(progress: &Arc<Mutex<Vec<TriggerProgress>>>) -> String {
  let deadline = Instant::now() + Duration::from_secs(30);
  loop {
    if let Some(run_id) = progress.lock().unwrap().iter().find_map(|message| {
      if let TriggerProgress::OccurrenceAccepted {
        workflow_id,
        run_id,
        ..
      } = message
      {
        (workflow_id == "top-call").then(|| run_id.clone())
      } else {
        None
      }
    }) {
      return run_id;
    }
    assert!(Instant::now() < deadline, "top workflow was not admitted");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

async fn wait_for_terminal(database: &TemporaryDatabase, run_id: &str) -> RunStatus {
  let deadline = Instant::now() + Duration::from_secs(30);
  loop {
    let status = DurableEventStore::open(database.path())
      .unwrap()
      .projection(run_id)
      .unwrap()
      .status;
    if matches!(status, RunStatus::Succeeded | RunStatus::Failed) {
      return status;
    }
    assert!(
      Instant::now() < deadline,
      "workflow did not become terminal"
    );
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

#[actix_web::test]
async fn nested_cross_process_calls_compose_with_branch_and_parallel_targets() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new();
  let http = LocalJsonServer::start();
  let call_progress = Arc::new(Mutex::new(Vec::new()));
  let application_database =
    serde_json::to_string(&format!("{}.application.sqlite", database.path().display())).unwrap();
  let raw_service_url = http.url();
  let service_url = serde_json::to_string(&raw_service_url).unwrap();
  let middle_source = format!(
    r#"
      const db = services.db({{ driver: "sqlite", connection: {application_database} }});
      await db.execute({{
        text: "CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value TEXT NOT NULL)"
      }}, {{ name: "create-records" }});
      const [nativeResponse, managedResponse, databaseWrite, object, cached] = await Promise.all([
        fetch({service_url}),
        services.http.request({{ url: {service_url} }}),
        db.execute({{
          text: "INSERT INTO records (id, value) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET value = excluded.value",
          values: ["record-42", "ready"]
        }}, {{ name: "save-record" }}),
        services.storage.put({{
          key: "wc6/record.json",
          value: {{ id: "record-42" }},
          overwrite: true
        }}, {{ name: "store-record" }}),
        services.cache.set("wc6:record", {{ id: "record-42" }}, {{ ttl: "5m", name: "cache-record" }})
      ]);
      const nativeData = await nativeResponse.json();
      const [databaseRead, stored, cachedValue] = await Promise.all([
        db.query({{ text: "SELECT id, value FROM records WHERE id = ?", values: ["record-42"] }}),
        services.storage.get({{ key: object.key, responseType: "json", ifVersion: object.version }}),
        services.cache.get("wc6:record")
      ]);
      const emitted = await services.events.emit("order.created", {{ orderId: "wc6-order" }});
      const review = await services.workflows.call('review-content', {{}});
      const field = await services.workflows.call('field-report', {{}});
      const retry = await services.workflows.call('retry-demo', {{ name: 'Dali' }});
      return {{
        branchMessage: review.message,
        summary: field.summary,
        greeting: retry.message,
        nativeFetch: nativeResponse.status === 200 && nativeData.customer === "Dali",
        managedHttp: managedResponse.status === 200 && managedResponse.data.customer === "Dali",
        database: databaseWrite.rowsAffected >= 1 && databaseRead.rows[0].id === "record-42",
        storage: stored.data.id === "record-42",
        cache: cached.stored && cachedValue.hit,
        events: emitted.acceptedCount === 1
      }};
    "#
  );
  let middle = caller_model("middle-call", &middle_source);
  let target_runtime = WomlWebhookServer::start(config(
    &database,
    vec![
      WebhookDefinitionRegistration::new(middle, MIDDLE_HASH),
      WebhookDefinitionRegistration::new(fixture_model(BRANCH_MODEL), BRANCH_HASH),
      WebhookDefinitionRegistration::new(fixture_model(PARALLEL_MODEL), PARALLEL_HASH),
      WebhookDefinitionRegistration::new(retry_model(), RETRY_HASH),
      WebhookDefinitionRegistration::new(fixture_model(EVENT_MODEL), EVENT_HASH)
        .with_secret("EVENT_CONTROL_TOKEN", "wc6-internal-event-token"),
    ],
    BTreeMap::new(),
    host.clone(),
    None,
    Arc::clone(&call_progress),
  ))
  .await
  .unwrap();

  let trigger_progress = Arc::new(Mutex::new(Vec::new()));
  let top = caller_model(
    "top-call",
    "const result = await services.workflows.call('middle-call', {}); return result;",
  );
  let caller_runtime = WomlWebhookServer::start(config(
    &database,
    vec![WebhookDefinitionRegistration::new(top, TOP_HASH)],
    BTreeMap::from([("top-call".to_string(), "start".to_string())]),
    host,
    Some(Arc::clone(&trigger_progress)),
    Arc::clone(&call_progress),
  ))
  .await
  .unwrap();

  let top_run_id = top_run_id(&trigger_progress).await;
  let store = DurableEventStore::open(database.path()).unwrap();
  let status = wait_for_terminal(&database, &top_run_id).await;
  let top_projection = store.projection(&top_run_id).unwrap();
  assert_eq!(status, RunStatus::Succeeded, "{top_projection:#?}");
  assert_eq!(
    top_projection.result,
    Some(json!({
      "branchMessage": "Final status: reviewed",
      "summary": "Weather 22°C, soil 41%",
      "greeting": "Hello Dali",
      "nativeFetch": true,
      "managedHttp": true,
      "database": true,
      "storage": true,
      "cache": true,
      "events": true
    }))
  );
  let top_relations = store.workflow_call_relations_for_run(&top_run_id).unwrap();
  assert_eq!(top_relations.child_calls.len(), 1);
  let middle_run_id = top_relations.child_calls[0].child_run_id.clone();
  let middle_relations = store
    .workflow_call_relations_for_run(&middle_run_id)
    .unwrap();
  assert_eq!(
    middle_relations.parent_call.unwrap().parent_run_id,
    top_run_id
  );
  assert_eq!(middle_relations.child_calls.len(), 3);
  assert!(middle_relations
    .child_calls
    .iter()
    .all(|call| call.depth == 2));

  let messages = call_progress.lock().unwrap();
  assert_eq!(
    messages
      .iter()
      .filter(|message| matches!(message, WorkflowCallProgress::CallAdmitted { .. }))
      .count(),
    4
  );
  let encoded_progress = serde_json::to_string(&*messages).unwrap();
  for private in [
    "wc6-order",
    "record-42",
    "wc6:record",
    raw_service_url.as_str(),
    application_database.as_str(),
  ] {
    assert!(!encoded_progress.contains(private));
  }
  drop(messages);

  caller_runtime.stop().await;
  target_runtime.stop().await;
}
