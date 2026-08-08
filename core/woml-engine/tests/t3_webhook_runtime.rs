use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::ValueExpression;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, RunEventPayload, RunStatus,
  RuntimeExecutionOptions, ScriptHostProcessOptions, TriggerAdmissionRequest,
  WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer, WomlWebhookServerConfig,
  WEBHOOK_MAX_BODY_BYTES,
};

const WEBHOOK_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-webhook.compiled.v7.json");
const WEBHOOK_HASH: &str =
  "sha256:4b4899d13cefc7ed88033d24c898549a4eb8862bebf4a73ed1c26f0af99bd082";
const WEBHOOK_PATH: &str = "/webhooks/orders";
const BEARER_TOKEN: &str = "t3-super-secret-token";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(WEBHOOK_MODEL).unwrap()
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
    execution: RuntimeExecutionOptions::new(host, 2_000),
  })
  .await
  .unwrap()
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
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
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
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
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
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000),
  })
  .await;
  assert!(matches!(
    invalid_schema,
    Err(WebhookRuntimeError::InvalidSchema { .. })
  ));
  assert_eq!(run_count(&database), 0);
}
