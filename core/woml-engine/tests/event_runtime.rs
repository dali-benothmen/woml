use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::future::join_all;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, RunStatus, RuntimeExecutionOptions,
  ScriptHostProcessOptions, TriggerAdmissionRequest, WebhookDefinitionRegistration,
  WomlWebhookServer, WomlWebhookServerConfig,
};

const EVENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-event.compiled.v7.json");
const CONTROL_TOKEN: &str = "t12-control-token-never-persisted";

fn event_model(workflow_id: &str, require_warehouse: bool) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(EVENT_MODEL).unwrap();
  value["workflowId"] = json!(workflow_id);
  if require_warehouse {
    value["triggers"][0]["config"]["fields"]["schema"]["value"]["required"] =
      json!(["orderId", "warehouseId"]);
    value["triggers"][0]["config"]["fields"]["schema"]["value"]["properties"]["warehouseId"] =
      json!({ "type": "string" });
  }
  serde_json::from_value(value).unwrap()
}

#[test]
fn historical_model_v7_events_without_ingress_secrets_remain_recoverable() {
  let mut value: Value = serde_json::from_str(EVENT_MODEL).unwrap();
  value["triggers"][0]["config"]["fields"]
    .as_object_mut()
    .unwrap()
    .remove("secret");
  let workflow: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  workflow.validate_for_durable_execution().unwrap();
}

fn registration(
  workflow_id: &str,
  hash_character: char,
  require_warehouse: bool,
) -> WebhookDefinitionRegistration {
  WebhookDefinitionRegistration::new(
    event_model(workflow_id, require_warehouse),
    format!("sha256:{}", hash_character.to_string().repeat(64)),
  )
  .with_secret("EVENT_CONTROL_TOKEN", CONTROL_TOKEN)
}

fn definition_hash(character: char) -> String {
  format!("sha256:{}", character.to_string().repeat(64))
}

fn source_identity(event_id: &str, workflow_id: &str, trigger_id: &str) -> String {
  let mut hasher = Sha256::new();
  hasher.update(event_id.as_bytes());
  hasher.update([0]);
  hasher.update(workflow_id.as_bytes());
  hasher.update([0]);
  hasher.update(trigger_id.as_bytes());
  format!("event:v1:sha256:{}", hex::encode(hasher.finalize()))
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

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-t12-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(format!("{}-wal", self.path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.path.display()));
  }
}

struct HttpResult {
  status: u16,
  body: Value,
}

async fn request(
  address: SocketAddr,
  method: &str,
  path: &str,
  token: &str,
  event_id: Option<&str>,
  body: &[u8],
) -> HttpResult {
  let method = method.to_string();
  let path = path.to_string();
  let token = token.to_string();
  let event_id = event_id.map(str::to_string);
  let body = body.to_vec();
  actix_web::rt::task::spawn_blocking(move || {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
      .set_read_timeout(Some(Duration::from_secs(10)))
      .unwrap();
    let mut head = format!(
      "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAuthorization: Bearer {token}\r\n",
      body.len()
    );
    if let Some(event_id) = event_id {
      head.push_str(&format!("Event-ID: {event_id}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response
      .windows(4)
      .position(|window| window == b"\r\n\r\n")
      .unwrap();
    let headers = String::from_utf8_lossy(&response[..split]);
    let status = headers
      .lines()
      .next()
      .unwrap()
      .split_whitespace()
      .nth(1)
      .unwrap()
      .parse()
      .unwrap();
    HttpResult {
      status,
      body: serde_json::from_slice(&response[split + 4..]).unwrap(),
    }
  })
  .await
  .unwrap()
}

async fn start(
  database: &TemporaryDatabase,
  host: ScriptHostProcessOptions,
  registrations: Vec<WebhookDefinitionRegistration>,
) -> WomlWebhookServer {
  WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations,
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(host, 2_000),
    progress_reporter: None,
  })
  .await
  .unwrap()
}

fn run_count(database: &TemporaryDatabase) -> i64 {
  Connection::open(database.path())
    .unwrap()
    .query_row("SELECT COUNT(*) FROM woml_runs", [], |row| row.get(0))
    .unwrap_or(0)
}

async fn wait_for_terminal(database: &TemporaryDatabase, run_id: &str) {
  let deadline = Instant::now() + Duration::from_secs(10);
  loop {
    let status = DurableEventStore::open(database.path())
      .unwrap()
      .projection(run_id)
      .unwrap()
      .status;
    if matches!(status, RunStatus::Succeeded | RunStatus::Failed) {
      return;
    }
    assert!(
      Instant::now() < deadline,
      "event run did not become terminal"
    );
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

#[actix_web::test]
async fn subscribers_to_one_event_must_resolve_the_same_authentication_secret() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("event-secret-conflict");
  let result = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![
      registration("send-confirmation", '1', false),
      registration("update-inventory", '2', false)
        .with_secret("EVENT_CONTROL_TOKEN", "different-token"),
    ],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(host, 2_000),
    progress_reporter: None,
  })
  .await;
  let error = result
    .err()
    .expect("conflicting event secrets must fail startup");
  assert!(error
    .to_string()
    .contains("all subscribers to event \"order.created\" must resolve to the same secret"));
}

#[actix_web::test]
async fn one_publication_fans_out_durably_and_restart_returns_original_runs() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("fanout");
  let registrations = || {
    vec![
      registration("send-confirmation", 'a', false),
      registration("update-inventory", 'b', false),
    ]
  };
  let server = start(&database, host.clone(), registrations()).await;
  let first = request(
    server.local_address(),
    "POST",
    "/_woml/events/order.created",
    CONTROL_TOKEN,
    Some("order-42-created"),
    br#"{"orderId":"order-42"}"#,
  )
  .await;
  assert_eq!(first.status, 200);
  assert_eq!(first.body["status"], "accepted");
  assert_eq!(first.body["deliveries"].as_array().unwrap().len(), 2);
  assert_eq!(
    first.body["deliveries"][0]["workflowId"],
    "send-confirmation"
  );
  assert_eq!(
    first.body["deliveries"][1]["workflowId"],
    "update-inventory"
  );
  let run_ids = first.body["deliveries"]
    .as_array()
    .unwrap()
    .iter()
    .map(|delivery| delivery["runId"].as_str().unwrap().to_string())
    .collect::<Vec<_>>();
  for run_id in &run_ids {
    wait_for_terminal(&database, run_id).await;
  }
  assert_eq!(run_count(&database), 2);
  server.stop().await;

  let restarted = start(&database, host, registrations()).await;
  let duplicate = request(
    restarted.local_address(),
    "POST",
    "/_woml/events/order.created",
    CONTROL_TOKEN,
    Some("order-42-created"),
    br#"{"orderId":"order-42"}"#,
  )
  .await;
  assert_eq!(duplicate.status, 200);
  assert_eq!(duplicate.body["status"], "accepted");
  for (index, delivery) in duplicate.body["deliveries"]
    .as_array()
    .unwrap()
    .iter()
    .enumerate()
  {
    assert_eq!(delivery["duplicate"], true);
    assert_eq!(delivery["runId"], run_ids[index]);
  }
  assert_eq!(run_count(&database), 2);

  let conflict = request(
    restarted.local_address(),
    "POST",
    "/_woml/events/order.created",
    CONTROL_TOKEN,
    Some("order-42-created"),
    br#"{"orderId":"changed"}"#,
  )
  .await;
  assert_eq!(conflict.status, 200);
  assert_eq!(conflict.body["status"], "rejected");
  assert!(conflict.body["deliveries"]
    .as_array()
    .unwrap()
    .iter()
    .all(|delivery| delivery["code"] == "WOML_TRIGGER_IDEMPOTENCY_CONFLICT"));
  restarted.stop().await;

  let bytes = std::fs::read(database.path()).unwrap();
  assert!(!bytes
    .windows(CONTROL_TOKEN.len())
    .any(|window| window == CONTROL_TOKEN.as_bytes()));
}

#[actix_web::test]
async fn subscribers_validate_independently_and_request_failures_create_no_extra_runs() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("partial");
  let server = start(
    &database,
    host,
    vec![
      registration("send-confirmation", 'c', false),
      registration("strict-inventory", 'd', true),
    ],
  )
  .await;
  let partial = request(
    server.local_address(),
    "POST",
    "/_woml/events/order.created",
    CONTROL_TOKEN,
    Some("partial-1"),
    br#"{"orderId":"order-43"}"#,
  )
  .await;
  assert_eq!(partial.status, 200);
  assert_eq!(partial.body["status"], "partial");
  assert_eq!(partial.body["deliveries"][0]["status"], "accepted");
  assert_eq!(partial.body["deliveries"][1]["status"], "rejected");
  assert_eq!(
    partial.body["deliveries"][1]["code"],
    "WOML_TRIGGER_SCHEMA_INVALID"
  );
  assert_eq!(run_count(&database), 1);

  let unauthorized = request(
    server.local_address(),
    "POST",
    "/_woml/events/order.created",
    "wrong-token",
    Some("unauthorized-1"),
    br#"{"orderId":"order-44"}"#,
  )
  .await;
  assert_eq!(unauthorized.status, 401);
  assert_eq!(
    unauthorized.body["error"]["code"],
    "WOML_EVENT_UNAUTHORIZED"
  );

  let missing = request(
    server.local_address(),
    "POST",
    "/_woml/events/customer.created",
    CONTROL_TOKEN,
    Some("missing-1"),
    br#"{"customerId":"customer-1"}"#,
  )
  .await;
  assert_eq!(missing.status, 404);
  assert_eq!(missing.body["error"]["code"], "WOML_EVENT_NOT_FOUND");

  let invalid_id = request(
    server.local_address(),
    "POST",
    "/_woml/events/order.created",
    CONTROL_TOKEN,
    Some("invalid id"),
    br#"{"orderId":"order-44"}"#,
  )
  .await;
  assert_eq!(invalid_id.status, 400);
  assert_eq!(invalid_id.body["error"]["code"], "WOML_EVENT_ID_INVALID");
  assert_eq!(run_count(&database), 1);
  server.stop().await;
}

#[actix_web::test]
async fn retry_after_a_mid_fanout_crash_completes_only_the_missing_subscriber() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("mid-fanout-recovery");
  let first_model = event_model("send-confirmation", false);
  let second_model = event_model("update-inventory", false);
  let first_hash = definition_hash('e');
  let second_hash = definition_hash('f');
  let event_id = "crash-recovery-1";
  {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&first_model, &first_hash)
      .unwrap();
    store
      .register_definition(&second_model, &second_hash)
      .unwrap();
    store
      .admit_trigger_occurrence(TriggerAdmissionRequest {
        workflow_id: "send-confirmation".to_string(),
        definition_hash: first_hash.clone(),
        trigger_id: "orderCreated".to_string(),
        trigger_handler: "trigger.event".to_string(),
        source_identity: source_identity(event_id, "send-confirmation", "orderCreated"),
        payload: Map::from_iter([("orderId".to_string(), json!("order-45"))]),
        received_at: Utc::now(),
      })
      .unwrap();
  }
  assert_eq!(run_count(&database), 1);

  let server = start(
    &database,
    host,
    vec![
      WebhookDefinitionRegistration::new(first_model, first_hash)
        .with_secret("EVENT_CONTROL_TOKEN", CONTROL_TOKEN),
      WebhookDefinitionRegistration::new(second_model, second_hash)
        .with_secret("EVENT_CONTROL_TOKEN", CONTROL_TOKEN),
    ],
  )
  .await;
  let resumed = request(
    server.local_address(),
    "POST",
    "/_woml/events/order.created",
    CONTROL_TOKEN,
    Some(event_id),
    br#"{"orderId":"order-45"}"#,
  )
  .await;
  assert_eq!(resumed.status, 200);
  assert_eq!(resumed.body["status"], "accepted");
  assert_eq!(resumed.body["deliveries"][0]["duplicate"], true);
  assert_eq!(resumed.body["deliveries"][1]["duplicate"], false);
  assert_eq!(run_count(&database), 2);
  server.stop().await;
}

#[actix_web::test]
async fn concurrent_identical_publications_converge_on_one_run_per_subscriber() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("concurrent");
  let server = start(
    &database,
    host,
    vec![
      registration("send-confirmation", '1', false),
      registration("update-inventory", '2', false),
    ],
  )
  .await;
  let address = server.local_address();
  let responses = join_all((0..8).map(|_| {
    request(
      address,
      "POST",
      "/_woml/events/order.created",
      CONTROL_TOKEN,
      Some("concurrent-1"),
      br#"{"orderId":"order-46"}"#,
    )
  }))
  .await;
  let mut run_ids = BTreeSet::new();
  let mut new_deliveries = 0;
  for response in responses {
    assert_eq!(response.status, 200);
    assert_eq!(response.body["status"], "accepted");
    for delivery in response.body["deliveries"].as_array().unwrap() {
      run_ids.insert(delivery["runId"].as_str().unwrap().to_string());
      if delivery["duplicate"] == false {
        new_deliveries += 1;
      }
    }
  }
  assert_eq!(new_deliveries, 2);
  assert_eq!(run_ids.len(), 2);
  assert_eq!(run_count(&database), 2);
  server.stop().await;
}
