use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use futures_util::future::join_all;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::{
  execute_admitted_trigger_run_durable, run_notification_provider_journey, ApprovalDecision,
  CompiledWorkflowDefinition, DurableEventStore, NotificationHostProcessOptions, RunEventPayload,
  RunStatus, RuntimeExecutionOptions, ScriptHostProcessOptions, TriggerAdmissionRequest,
  TriggerProgress, WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer,
  WomlWebhookServerConfig, WorkflowRuntimeOutcome, WEBHOOK_MAX_BODY_BYTES,
};

const WEBHOOK_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-webhook.compiled.v7.json");
const WEBHOOK_HASH: &str =
  "sha256:4b4899d13cefc7ed88033d24c898549a4eb8862bebf4a73ed1c26f0af99bd082";
const WEBHOOK_PATH: &str = "/webhooks/orders";
const BEARER_TOKEN: &str = "t3-super-secret-token";
const BRANCH_MODEL: &str = include_str!("../../../woml/tests/fixtures/branch.compiled.v2.json");
const PARALLEL_MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const RETRY_MODEL: &str = include_str!("../../../woml/tests/fixtures/retry.compiled.v6.json");
const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const NOTIFICATION_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/approval-slack.compiled.v5.json");

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(WEBHOOK_MODEL).unwrap()
}

fn production_model(source: &str, path: &str, trigger_id: &str) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(source).unwrap();
  let webhook: Value = serde_json::from_str(WEBHOOK_MODEL).unwrap();
  value["schemaVersion"] = json!(7);
  value["triggers"] = json!([webhook["triggers"][1].clone()]);
  value["triggers"][0]["id"] = json!(trigger_id);
  value["triggers"][0]["config"]["fields"]["path"]["value"] = json!(path);
  serde_json::from_value(value).unwrap()
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

fn placeholder_host() -> ScriptHostProcessOptions {
  ScriptHostProcessOptions::new("bun", "unused-script-host.ts")
}

fn notification_host_options() -> Option<NotificationHostProcessOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../woml-cli/tests/fixtures/fake-notification-provider-host.ts");
  host
    .exists()
    .then(|| NotificationHostProcessOptions::new(bun, host))
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-t3-{label}-{}.sqlite",
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

async fn start_server(
  database: &TemporaryDatabase,
  host: ScriptHostProcessOptions,
) -> WomlWebhookServer {
  WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(model(), WEBHOOK_HASH)
      .with_secret("ORDER_WEBHOOK_TOKEN", BEARER_TOKEN)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(host, 2_000),
    progress_reporter: None,
  })
  .await
  .unwrap()
}

#[actix_web::test]
async fn prepared_runtime_rejects_traffic_until_atomic_activation_opens_admission() {
  let database = TemporaryDatabase::new("pro2-readiness-gate");
  let mut server = WomlWebhookServer::prepare_with_external_ingress(
    WomlWebhookServerConfig {
      bind_address: "127.0.0.1:0".parse().unwrap(),
      database_path: database.path().to_path_buf(),
      registrations: vec![WebhookDefinitionRegistration::new(model(), WEBHOOK_HASH)
        .with_secret("ORDER_WEBHOOK_TOKEN", BEARER_TOKEN)],
      startup_manual_triggers: Default::default(),
      execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
      progress_reporter: None,
    },
    None,
  )
  .await
  .unwrap();
  let address = server.local_address();
  let authorization = format!("Bearer {BEARER_TOKEN}");

  let closed = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &[
      ("Authorization", authorization.as_str()),
      ("Content-Type", "application/json"),
    ],
    br#"{"orderId":"before-ready"}"#,
    None,
  )
  .await;
  assert_eq!(closed.status, 503);
  assert_eq!(closed.body["error"]["code"], "WOML_RUNTIME_NOT_READY");
  let store = DurableEventStore::open(database.path()).unwrap();
  assert!(store
    .recover_undispatched_trigger_runs()
    .unwrap()
    .is_empty());

  server.activate().await.unwrap();
  let opened = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &[
      ("Authorization", authorization.as_str()),
      ("Content-Type", "application/json"),
    ],
    br#"{"orderId":"after-ready"}"#,
    None,
  )
  .await;
  assert_eq!(opened.status, 202);
  server.stop_with_deadline(Duration::from_secs(3)).await;
}

struct HttpResult {
  status: u16,
  headers: String,
  body: Value,
}

async fn request(
  address: SocketAddr,
  method: &str,
  path: &str,
  headers: &[(&str, &str)],
  body: &[u8],
  declared_length: Option<usize>,
) -> HttpResult {
  let method = method.to_string();
  let path = path.to_string();
  let headers = headers
    .iter()
    .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
    .collect::<Vec<_>>();
  let body = body.to_vec();
  actix_web::rt::task::spawn_blocking(move || {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
      .set_read_timeout(Some(Duration::from_secs(10)))
      .unwrap();
    let mut head = format!(
      "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
      declared_length.unwrap_or(body.len())
    );
    for (name, value) in headers {
      head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    if !body.is_empty() {
      let _ = stream.write_all(&body);
    }
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response
      .windows(4)
      .position(|window| window == b"\r\n\r\n")
      .unwrap();
    let response_headers = String::from_utf8(response[..split].to_vec()).unwrap();
    let status = response_headers
      .lines()
      .next()
      .unwrap()
      .split_whitespace()
      .nth(1)
      .unwrap()
      .parse()
      .unwrap();
    let body = serde_json::from_slice(&response[split + 4..]).unwrap();
    HttpResult {
      status,
      headers: response_headers,
      body,
    }
  })
  .await
  .unwrap()
}

async fn raw_status(address: SocketAddr, bytes: Vec<u8>) -> u16 {
  actix_web::rt::task::spawn_blocking(move || {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
      .set_read_timeout(Some(Duration::from_secs(10)))
      .unwrap();
    stream.write_all(&bytes).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    String::from_utf8_lossy(&response)
      .lines()
      .next()
      .and_then(|line| line.split_whitespace().nth(1))
      .and_then(|status| status.parse().ok())
      .unwrap()
  })
  .await
  .unwrap()
}

fn standard_headers<'a>(token: &'a str, key: &'a str) -> [(&'a str, &'a str); 3] {
  [
    ("Authorization", token),
    ("Content-Type", "application/json"),
    ("Idempotency-Key", key),
  ]
}

fn run_count(database: &TemporaryDatabase) -> i64 {
  let connection = Connection::open(database.path()).unwrap();
  let exists: bool = connection
    .query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'woml_runs')",
      [],
      |row| row.get(0),
    )
    .unwrap();
  if exists {
    connection
      .query_row("SELECT COUNT(*) FROM woml_runs", [], |row| row.get(0))
      .unwrap()
  } else {
    0
  }
}

async fn wait_for_status(database: &TemporaryDatabase, run_id: &str, expected: RunStatus) {
  let deadline = Instant::now() + Duration::from_secs(10);
  loop {
    let store = DurableEventStore::open(database.path()).unwrap();
    let projection = store.projection(run_id).unwrap();
    if projection.status == expected {
      return;
    }
    assert!(Instant::now() < deadline, "run did not reach {expected:?}");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  }
}

fn registration(
  workflow: CompiledWorkflowDefinition,
  hash_character: char,
) -> WebhookDefinitionRegistration {
  WebhookDefinitionRegistration::new(
    workflow,
    format!("sha256:{}", hash_character.to_string().repeat(64)),
  )
  .with_secret("ORDER_WEBHOOK_TOKEN", BEARER_TOKEN)
}

#[actix_web::test]
async fn a_real_post_is_admitted_then_executes_through_the_durable_dag() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("success");
  let server = start_server(&database, host).await;
  let authorization = format!("Bearer {BEARER_TOKEN}");
  let headers = standard_headers(&authorization, "delivery-001");
  let response = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &headers,
    br#"{"orderId":"order-42"}"#,
    None,
  )
  .await;
  assert_eq!(response.status, 202);
  assert_eq!(
    response.body,
    json!({
      "runId": response.body["runId"],
      "status": "accepted",
      "duplicate": false
    })
  );
  let run_id = response.body["runId"].as_str().unwrap();
  wait_for_status(&database, run_id, RunStatus::Succeeded).await;

  let store = DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection(run_id).unwrap();
  assert_eq!(
    projection.context.trigger,
    Map::from_iter([("orderId".to_string(), json!("order-42"))])
  );
  assert_eq!(
    projection.context.steps["capture"],
    json!({ "orderId": "order-42" })
  );
  let persisted = serde_json::to_string(&store.events(run_id).unwrap()).unwrap();
  assert!(!persisted.contains(BEARER_TOKEN));
  assert!(!persisted.contains("delivery-001"));
  server.stop().await;
}

#[actix_web::test]
async fn selected_manual_trigger_fires_once_at_long_lived_runtime_startup() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("startup-manual");
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let captured = progress.clone();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(model(), WEBHOOK_HASH)
      .with_secret("ORDER_WEBHOOK_TOKEN", BEARER_TOKEN)],
    startup_manual_triggers: BTreeMap::from([(
      "webhook-trigger-contract".to_string(),
      "manualStart".to_string(),
    )]),
    execution: RuntimeExecutionOptions::new(host, 2_000),
    progress_reporter: Some(Arc::new(move |message| {
      captured.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();

  let deadline = Instant::now() + Duration::from_secs(10);
  let run_id = loop {
    let selected = progress.lock().unwrap().iter().find_map(|message| {
      if let TriggerProgress::OccurrenceAccepted {
        trigger_handler,
        run_id,
        duplicate,
        ..
      } = message
      {
        (trigger_handler == "trigger.manual" && !duplicate).then(|| run_id.clone())
      } else {
        None
      }
    });
    if let Some(run_id) = selected {
      break run_id;
    }
    assert!(Instant::now() < deadline, "manual trigger was not admitted");
    actix_web::rt::time::sleep(Duration::from_millis(10)).await;
  };
  wait_for_status(&database, &run_id, RunStatus::Succeeded).await;
  assert_eq!(run_count(&database), 1);
  let store = DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection(&run_id).unwrap();
  assert_eq!(projection.trigger_id.as_deref(), Some("manualStart"));
  assert_eq!(
    projection.trigger_handler.as_deref(),
    Some("trigger.manual")
  );
  assert!(projection.context.trigger.is_empty());
  server.stop().await;
}

#[actix_web::test]
async fn schema_auth_shape_content_type_method_and_route_rejections_create_no_run() {
  let database = TemporaryDatabase::new("rejections");
  let server = start_server(&database, placeholder_host()).await;
  let address = server.local_address();

  let missing_order = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &standard_headers(&format!("Bearer {BEARER_TOKEN}"), "schema-key"),
    br#"{}"#,
    None,
  )
  .await;
  assert_eq!(missing_order.status, 400);
  assert_eq!(
    missing_order.body,
    json!({
      "error": {
        "code": "WOML_TRIGGER_SCHEMA_INVALID",
        "message": "Webhook payload does not match the declared schema.",
        "issues": [{ "path": "/orderId", "message": "Required property is missing." }]
      }
    })
  );

  let unauthorized = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &standard_headers("Bearer wrong", "auth-key"),
    br#"{"orderId":"secret payload"}"#,
    None,
  )
  .await;
  assert_eq!(unauthorized.status, 401);
  assert_eq!(
    unauthorized.body["error"]["code"],
    "WOML_TRIGGER_UNAUTHORIZED"
  );
  assert!(!serde_json::to_string(&unauthorized.body)
    .unwrap()
    .contains("secret payload"));

  let non_object = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &standard_headers(&format!("Bearer {BEARER_TOKEN}"), "shape-key"),
    br#"[1,2,3]"#,
    None,
  )
  .await;
  assert_eq!(non_object.status, 400);
  assert_eq!(
    non_object.body["error"]["code"],
    "WOML_TRIGGER_PAYLOAD_INVALID"
  );

  let wrong_content_type = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &[
      ("Authorization", &format!("Bearer {BEARER_TOKEN}")),
      ("Content-Type", "text/plain"),
    ],
    br#"{"orderId":"order-42"}"#,
    None,
  )
  .await;
  assert_eq!(wrong_content_type.status, 400);

  let wrong_method = request(address, "PUT", WEBHOOK_PATH, &[], b"", None).await;
  assert_eq!(wrong_method.status, 405);
  assert!(wrong_method.headers.contains("allow: POST"));

  let missing_route = request(address, "POST", "/not-registered", &[], b"", None).await;
  assert_eq!(missing_route.status, 404);
  assert_eq!(run_count(&database), 0);
  server.stop().await;
}

#[actix_web::test]
async fn the_frozen_one_mib_limit_rejects_before_admission() {
  let database = TemporaryDatabase::new("size");
  let server = start_server(&database, placeholder_host()).await;
  let response = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &[
      ("Authorization", &format!("Bearer {BEARER_TOKEN}")),
      ("Content-Type", "application/json"),
    ],
    b"",
    Some(WEBHOOK_MAX_BODY_BYTES + 1),
  )
  .await;
  assert_eq!(response.status, 413);
  assert_eq!(
    response.body["error"]["code"],
    "WOML_TRIGGER_PAYLOAD_TOO_LARGE"
  );
  assert_eq!(run_count(&database), 0);
  server.stop().await;
}

#[actix_web::test]
async fn runtime_validation_uses_draft_2020_12_semantics() {
  let database = TemporaryDatabase::new("draft-2020-12");
  let mut workflow = model();
  let ValueExpression::Object { fields } = &mut workflow.triggers[1].config else {
    panic!("webhook config must be an object");
  };
  fields.insert(
    "schema".to_string(),
    ValueExpression::Literal {
      value: json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["orderId", "codes"],
        "properties": {
          "orderId": { "type": "string" },
          "codes": {
            "type": "array",
            "prefixItems": [{ "const": "v1" }],
            "items": true
          }
        },
        "additionalProperties": false
      }),
    },
  );
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(
      workflow,
      "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .with_secret("ORDER_WEBHOOK_TOKEN", BEARER_TOKEN)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
    progress_reporter: None,
  })
  .await
  .unwrap();
  let response = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &standard_headers(&format!("Bearer {BEARER_TOKEN}"), "draft-key"),
    br#"{"orderId":"order-42","codes":["legacy"]}"#,
    None,
  )
  .await;
  assert_eq!(response.status, 400);
  assert_eq!(
    response.body["error"]["code"],
    "WOML_TRIGGER_SCHEMA_INVALID"
  );
  assert_eq!(run_count(&database), 0);
  server.stop().await;
}

#[actix_web::test]
async fn duplicate_replay_returns_the_original_run_and_changed_payload_conflicts() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("duplicate");
  let server = start_server(&database, host).await;
  let authorization = format!("Bearer {BEARER_TOKEN}");
  let headers = standard_headers(&authorization, "stable-delivery");
  let first = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &headers,
    br#"{"orderId":"order-42"}"#,
    None,
  )
  .await;
  let duplicate = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &headers,
    br#"{"orderId":"order-42"}"#,
    None,
  )
  .await;
  assert_eq!(duplicate.status, 202);
  assert_eq!(duplicate.body["duplicate"], true);
  assert_eq!(duplicate.body["runId"], first.body["runId"]);

  let conflict = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &headers,
    br#"{"orderId":"changed"}"#,
    None,
  )
  .await;
  assert_eq!(conflict.status, 409);
  assert_eq!(
    conflict.body["error"]["code"],
    "WOML_TRIGGER_IDEMPOTENCY_CONFLICT"
  );
  assert_eq!(run_count(&database), 1);
  let run_id = first.body["runId"].as_str().unwrap();
  wait_for_status(&database, run_id, RunStatus::Succeeded).await;
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store
      .events(run_id)
      .unwrap()
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
      .count(),
    1
  );
  server.stop().await;
}

#[actix_web::test]
async fn startup_recovers_an_accepted_occurrence_that_was_never_dispatched() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("recovery");
  let accepted = {
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store.register_definition(&model(), WEBHOOK_HASH).unwrap();
    store
      .admit_trigger_occurrence(TriggerAdmissionRequest {
        workflow_id: "webhook-trigger-contract".to_string(),
        definition_hash: WEBHOOK_HASH.to_string(),
        trigger_id: "newOrder".to_string(),
        trigger_handler: "trigger.webhook".to_string(),
        source_identity: "accepted-before-crash".to_string(),
        payload: Map::from_iter([("orderId".to_string(), json!("order-42"))]),
        received_at: Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap(),
      })
      .unwrap()
  };
  assert_eq!(
    DurableEventStore::open(database.path())
      .unwrap()
      .events(&accepted.run_id)
      .unwrap()
      .len(),
    1
  );

  let server = start_server(&database, host).await;
  wait_for_status(&database, &accepted.run_id, RunStatus::Succeeded).await;
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(store.events(&accepted.run_id).unwrap().len(), 4);
  server.stop().await;
}

#[actix_web::test]
async fn missing_secrets_and_invalid_inline_schemas_fail_before_binding() {
  let database = TemporaryDatabase::new("preflight");
  let missing_secret = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(model(), WEBHOOK_HASH)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
    progress_reporter: None,
  })
  .await;
  assert!(matches!(
    missing_secret,
    Err(WebhookRuntimeError::SecretMissing(ref name)) if name == "ORDER_WEBHOOK_TOKEN"
  ));

  let mut invalid_schema = model();
  let ValueExpression::Object { fields } = &mut invalid_schema.triggers[1].config else {
    panic!("webhook config must be an object");
  };
  fields.insert(
    "schema".to_string(),
    ValueExpression::Literal {
      value: json!({ "type": "not-a-json-schema-type" }),
    },
  );
  let invalid_schema = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(
      invalid_schema,
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .with_secret("ORDER_WEBHOOK_TOKEN", BEARER_TOKEN)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
    progress_reporter: None,
  })
  .await;
  assert!(matches!(
    invalid_schema,
    Err(WebhookRuntimeError::InvalidSchema { .. })
  ));
  assert_eq!(run_count(&database), 0);
}

#[actix_web::test]
async fn concurrent_requests_and_database_contention_preserve_one_run_per_occurrence() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("concurrency");
  let server = start_server(&database, host).await;
  let address = server.local_address();
  let authorization = format!("Bearer {BEARER_TOKEN}");
  let responses = join_all((0..8).map(|index| {
    let authorization = authorization.clone();
    async move {
      let key = format!("concurrent-{index}");
      let order = format!(r#"{{"orderId":"order-{index}"}}"#);
      request(
        address,
        "POST",
        WEBHOOK_PATH,
        &standard_headers(&authorization, &key),
        order.as_bytes(),
        None,
      )
      .await
    }
  }))
  .await;
  assert!(responses.iter().all(|response| response.status == 202));
  let run_ids = responses
    .iter()
    .map(|response| response.body["runId"].as_str().unwrap().to_string())
    .collect::<std::collections::HashSet<_>>();
  assert_eq!(run_ids.len(), 8);
  for run_id in &run_ids {
    wait_for_status(&database, run_id, RunStatus::Succeeded).await;
  }

  let lock = Connection::open(database.path()).unwrap();
  lock.execute_batch("BEGIN IMMEDIATE").unwrap();
  let blocked_request = actix_web::rt::spawn({
    let authorization = authorization.clone();
    async move {
      request(
        address,
        "POST",
        WEBHOOK_PATH,
        &standard_headers(&authorization, "contention-key"),
        br#"{"orderId":"after-contention"}"#,
        None,
      )
      .await
    }
  });
  actix_web::rt::time::sleep(Duration::from_millis(100)).await;
  lock.execute_batch("COMMIT").unwrap();
  let response = blocked_request.await.unwrap();
  assert_eq!(
    response.status, 202,
    "contention admission response: {}",
    response.body
  );
  let run_id = response.body["runId"].as_str().unwrap();
  wait_for_status(&database, run_id, RunStatus::Succeeded).await;
  assert_eq!(run_count(&database), 9);
  server.stop().await;
}

#[actix_web::test]
async fn slow_clients_malformed_framing_and_streamed_oversize_bodies_do_not_block_admission() {
  let database = TemporaryDatabase::new("transport-hardening");
  let server = start_server(&database, placeholder_host()).await;
  let address = server.local_address();
  let (slow_ready_sender, slow_ready_receiver) = std::sync::mpsc::sync_channel(1);
  let slow_client = actix_web::rt::task::spawn_blocking(move || {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
      .write_all(
        format!(
          "POST {WEBHOOK_PATH} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{"
        )
        .as_bytes(),
      )
      .unwrap();
    slow_ready_sender.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(500));
  });
  actix_web::rt::task::spawn_blocking(move || slow_ready_receiver.recv().unwrap())
    .await
    .unwrap();

  let started = Instant::now();
  let authorization = format!("Bearer {BEARER_TOKEN}");
  let healthy = request(
    address,
    "POST",
    WEBHOOK_PATH,
    &standard_headers(&authorization, "healthy-while-slow"),
    br#"{"orderId":"healthy"}"#,
    None,
  )
  .await;
  assert_eq!(healthy.status, 202);
  assert!(started.elapsed() < Duration::from_millis(450));
  slow_client.await.unwrap();

  let malformed = raw_status(
    address,
    format!(
      "POST {WEBHOOK_PATH} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: nope\r\nConnection: close\r\n\r\n{{}}"
    )
    .into_bytes(),
  )
  .await;
  assert_eq!(malformed, 400);

  let chunk_length = WEBHOOK_MAX_BODY_BYTES + 1;
  let mut streamed = format!(
    "POST {WEBHOOK_PATH} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {BEARER_TOKEN}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{chunk_length:X}\r\n"
  )
  .into_bytes();
  streamed.extend(std::iter::repeat_n(b'x', chunk_length));
  streamed.extend_from_slice(b"\r\n0\r\n\r\n");
  assert_eq!(raw_status(address, streamed).await, 413);
  assert_eq!(run_count(&database), 1);
  server.stop().await;
}

#[actix_web::test]
async fn route_and_port_conflicts_fail_before_a_second_runtime_becomes_ready() {
  let route_database = TemporaryDatabase::new("route-conflict");
  let mut second_route_owner = model();
  second_route_owner.workflow_id = "other-orders".to_string();
  let duplicate_route = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: route_database.path().to_path_buf(),
    registrations: vec![
      registration(model(), '1'),
      registration(second_route_owner, '2'),
    ],
    startup_manual_triggers: BTreeMap::new(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
    progress_reporter: None,
  })
  .await;
  assert!(matches!(
    duplicate_route,
    Err(WebhookRuntimeError::RouteConflict(ref path)) if path == WEBHOOK_PATH
  ));
  assert_eq!(run_count(&route_database), 0);

  let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
  let occupied = reservation.local_addr().unwrap();
  let port_database = TemporaryDatabase::new("port-conflict");
  let port_conflict = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: occupied,
    database_path: port_database.path().to_path_buf(),
    registrations: vec![registration(model(), '3')],
    startup_manual_triggers: BTreeMap::new(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
    progress_reporter: None,
  })
  .await;
  assert!(matches!(
    port_conflict,
    Err(WebhookRuntimeError::Io(ref error))
      if error.kind() == std::io::ErrorKind::AddrInUse
  ));
}

#[actix_web::test]
async fn host_crashes_fail_closed_and_credentials_never_enter_state_or_progress() {
  let database = TemporaryDatabase::new("host-crash");
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let captured = progress.clone();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![registration(model(), '4')],
    startup_manual_triggers: BTreeMap::new(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
    progress_reporter: Some(Arc::new(move |message| {
      captured.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();
  let secret_key = "private-delivery-identity";
  let authorization = format!("Bearer {BEARER_TOKEN}");
  let response = request(
    server.local_address(),
    "POST",
    WEBHOOK_PATH,
    &standard_headers(&authorization, secret_key),
    br#"{"orderId":"safe-payload"}"#,
    None,
  )
  .await;
  let run_id = response.body["runId"].as_str().unwrap();
  wait_for_status(&database, run_id, RunStatus::Failed).await;
  server.stop().await;

  let progress_json = serde_json::to_string(&*progress.lock().unwrap()).unwrap();
  assert!(!progress_json.contains(BEARER_TOKEN));
  assert!(!progress_json.contains(secret_key));
  for suffix in ["", "-wal", "-shm"] {
    let path = format!("{}{suffix}", database.path().display());
    if let Ok(bytes) = std::fs::read(path) {
      assert!(!bytes
        .windows(BEARER_TOKEN.len())
        .any(|item| item == BEARER_TOKEN.as_bytes()));
      assert!(!bytes
        .windows(secret_key.len())
        .any(|item| item == secret_key.as_bytes()));
    }
  }
  if let Some(host) = host_options() {
    let restarted = start_server(&database, host).await;
    actix_web::rt::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
      DurableEventStore::open(database.path())
        .unwrap()
        .projection(run_id)
        .unwrap()
        .status,
      RunStatus::Failed
    );
    restarted.stop().await;
  }
  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(run_id).unwrap();
  assert_eq!(
    events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
      .count(),
    1
  );
}

#[actix_web::test]
async fn webhook_composes_with_retry_branch_parallel_approval_and_slack_delivery() {
  let (Some(host), Some(notification_host)) = (host_options(), notification_host_options()) else {
    return;
  };
  let database = TemporaryDatabase::new("composition");
  let workflows = [
    (production_model(RETRY_MODEL, "/retry", "retryHook"), '5'),
    (production_model(BRANCH_MODEL, "/branch", "branchHook"), '6'),
    (
      production_model(PARALLEL_MODEL, "/parallel", "parallelHook"),
      '7',
    ),
    (
      production_model(APPROVAL_MODEL, "/approval", "approvalHook"),
      '8',
    ),
    (
      production_model(NOTIFICATION_MODEL, "/slack", "slackHook"),
      '9',
    ),
  ];
  let registrations = workflows
    .iter()
    .map(|(workflow, hash)| registration(workflow.clone(), *hash))
    .collect();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations,
    startup_manual_triggers: BTreeMap::new(),
    execution: RuntimeExecutionOptions::new(host.clone(), 3_000),
    progress_reporter: None,
  })
  .await
  .unwrap();
  let address = server.local_address();
  let authorization = format!("Bearer {BEARER_TOKEN}");

  for (index, path) in ["/retry", "/branch", "/parallel"].iter().enumerate() {
    let key = format!("composition-{index}");
    let response = request(
      address,
      "POST",
      path,
      &standard_headers(&authorization, &key),
      br#"{"orderId":"composition"}"#,
      None,
    )
    .await;
    assert_eq!(response.status, 202);
    wait_for_status(
      &database,
      response.body["runId"].as_str().unwrap(),
      RunStatus::Succeeded,
    )
    .await;
  }

  let approval = request(
    address,
    "POST",
    "/approval",
    &standard_headers(&authorization, "approval-composition"),
    br#"{"orderId":"approval"}"#,
    None,
  )
  .await;
  wait_for_status(
    &database,
    approval.body["runId"].as_str().unwrap(),
    RunStatus::Waiting,
  )
  .await;

  let slack = request(
    address,
    "POST",
    "/slack",
    &standard_headers(&authorization, "slack-composition"),
    br#"{"orderId":"slack"}"#,
    None,
  )
  .await;
  let slack_run_id = slack.body["runId"].as_str().unwrap();
  wait_for_status(&database, slack_run_id, RunStatus::Waiting).await;
  let journey = run_notification_provider_journey(
    database.path(),
    slack_run_id,
    notification_host,
    Duration::from_secs(5),
  )
  .await
  .unwrap();
  assert_eq!(
    journey.decision.as_ref().map(|decision| decision.decision),
    Some(ApprovalDecision::Approved)
  );
  let resumed = execute_admitted_trigger_run_durable(
    database.path().to_path_buf(),
    slack_run_id,
    RuntimeExecutionOptions::new(host, 3_000),
  )
  .await
  .unwrap();
  assert!(matches!(resumed, WorkflowRuntimeOutcome::Succeeded { .. }));
  wait_for_status(&database, slack_run_id, RunStatus::Succeeded).await;
  server.stop().await;
}
