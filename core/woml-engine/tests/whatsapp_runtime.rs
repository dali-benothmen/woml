use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, RuntimeExecutionOptions, ScriptHostProcessOptions,
  TriggerProgress, WebhookDefinitionRegistration, WomlWebhookServer, WomlWebhookServerConfig,
  COMPILED_MODEL_SCHEMA_VERSION_V15,
};

const BASE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");
const DEFINITION_HASH: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VERIFY_TOKEN: &str = "whatsapp-verify-token";
const APP_SECRET: &str = "whatsapp-test-app-secret";
const PHONE_NUMBER_ID: &str = "123456789012345";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-whatsapp-runtime-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
  }
}

fn model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(BASE_MODEL).expect("base model");
  value["schemaVersion"] = json!(COMPILED_MODEL_SCHEMA_VERSION_V15);
  value["triggers"] = json!([{
    "id": "customerMessage",
    "handler": "trigger.whatsapp",
    "config": {
      "kind": "object",
      "fields": {
        "events": { "kind": "array", "items": [{ "kind": "literal", "value": "message" }] },
        "phoneNumberId": { "kind": "literal", "value": PHONE_NUMBER_ID },
        "verifyToken": { "kind": "secretReference", "name": "WHATSAPP_VERIFY_TOKEN" },
        "appSecret": { "kind": "secretReference", "name": "WHATSAPP_APP_SECRET" }
      }
    }
  }]);
  value["communication"] = json!({
    "profileVersion": 1,
    "providers": [{
      "provider": "whatsapp",
      "triggerIds": ["customerMessage"],
      "notificationDeliveryIds": [],
      "messaging": false,
      "credentialNames": ["WHATSAPP_APP_SECRET", "WHATSAPP_VERIFY_TOKEN"]
    }]
  });
  serde_json::from_value(value).expect("WhatsApp model")
}

async fn request(address: SocketAddr, request: String) -> String {
  actix_web::rt::task::spawn_blocking(move || {
    let mut stream = TcpStream::connect(address).expect("connect callback");
    stream
      .set_read_timeout(Some(Duration::from_secs(5)))
      .expect("read timeout");
    stream.write_all(request.as_bytes()).expect("send request");
    stream.shutdown(Shutdown::Write).expect("finish request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
  })
  .await
  .expect("callback request task")
}

async fn signed_post(address: SocketAddr, body: &str, signature_secret: &str) -> String {
  let mut mac = Hmac::<Sha256>::new_from_slice(signature_secret.as_bytes()).expect("HMAC key");
  mac.update(body.as_bytes());
  let signature = hex::encode(mac.finalize().into_bytes());
  request(
    address,
    format!(
      "POST /callbacks/whatsapp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nX-Hub-Signature-256: sha256={signature}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
      body.len()
    ),
  )
  .await
}

#[actix_web::test]
async fn verifies_handshake_signature_and_durably_deduplicates_messages() {
  let database = TemporaryDatabase::new();
  let progress = Arc::new(Mutex::new(Vec::new()));
  let report = Arc::clone(&progress);
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.0.clone(),
    registrations: vec![WebhookDefinitionRegistration::new(model(), DEFINITION_HASH)
      .with_secret("WHATSAPP_VERIFY_TOKEN", VERIFY_TOKEN)
      .with_secret("WHATSAPP_APP_SECRET", APP_SECRET)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new("bun", "missing-whatsapp-test-host.ts"),
      1_000,
    ),
    progress_reporter: Some(Arc::new(move |event| {
      report.lock().unwrap().push(event);
    })),
  })
  .await
  .expect("WhatsApp runtime");
  let address = server.local_address();

  let handshake = request(
    address,
    format!(
      "GET /callbacks/whatsapp?hub.mode=subscribe&hub.verify_token={VERIFY_TOKEN}&hub.challenge=challenge-42 HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    ),
  )
  .await;
  assert!(handshake.starts_with("HTTP/1.1 200"));
  assert!(handshake.ends_with("challenge-42"));

  let body = json!({
    "object": "whatsapp_business_account",
    "entry": [{
      "changes": [{
        "field": "messages",
        "value": {
          "metadata": { "phone_number_id": PHONE_NUMBER_ID },
          "messages": [{
            "from": "15551234567",
            "id": "wamid.acp7-message-1",
            "timestamp": "1786880000",
            "type": "text",
            "text": { "body": "hello WOML" }
          }]
        }
      }]
    }]
  })
  .to_string();
  let accepted = signed_post(address, &body, APP_SECRET).await;
  assert!(accepted.starts_with("HTTP/1.1 200"), "{accepted}");
  assert!(accepted.ends_with("EVENT_RECEIVED"));
  let duplicate = signed_post(address, &body, APP_SECRET).await;
  assert!(duplicate.starts_with("HTTP/1.1 200"));
  let rejected = signed_post(address, &body, "wrong-secret").await;
  assert!(rejected.starts_with("HTTP/1.1 401"));

  let accepted_events = progress
    .lock()
    .unwrap()
    .iter()
    .filter_map(|event| match event {
      TriggerProgress::OccurrenceAccepted {
        duplicate,
        trigger_handler,
        ..
      } if trigger_handler == "trigger.whatsapp" => Some(*duplicate),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(accepted_events, vec![false, true]);
  let run_id = progress
    .lock()
    .unwrap()
    .iter()
    .find_map(|event| match event {
      TriggerProgress::OccurrenceAccepted {
        run_id,
        duplicate: false,
        trigger_handler,
        ..
      } if trigger_handler == "trigger.whatsapp" => Some(run_id.clone()),
      _ => None,
    })
    .expect("accepted WhatsApp run");
  let projection = DurableEventStore::open(&database.0)
    .and_then(|store| store.projection(&run_id))
    .expect("durable WhatsApp projection");
  assert_eq!(
    projection.context.trigger.get("provider"),
    Some(&json!("whatsapp"))
  );
  assert_eq!(
    projection.context.trigger.get("event"),
    Some(&json!("message"))
  );
  assert_eq!(
    projection.context.trigger.get("senderId"),
    Some(&json!("15551234567"))
  );
  assert_eq!(
    projection.context.trigger.get("conversationType"),
    Some(&json!("direct"))
  );
  assert_eq!(
    projection.context.trigger.get("text"),
    Some(&json!("hello WOML"))
  );
  assert!(projection.context.trigger.get("occurredAt").is_some());
  server.stop_with_deadline(Duration::from_millis(100)).await;
}

#[actix_web::test]
async fn rejects_adversarial_batches_and_accepts_bounded_unicode_without_leaking_secrets() {
  let database = TemporaryDatabase::new();
  let progress = Arc::new(Mutex::new(Vec::new()));
  let report = Arc::clone(&progress);
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.0.clone(),
    registrations: vec![WebhookDefinitionRegistration::new(model(), DEFINITION_HASH)
      .with_secret("WHATSAPP_VERIFY_TOKEN", VERIFY_TOKEN)
      .with_secret("WHATSAPP_APP_SECRET", APP_SECRET)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(
      ScriptHostProcessOptions::new("bun", "missing-whatsapp-test-host.ts"),
      1_000,
    ),
    progress_reporter: Some(Arc::new(move |event| {
      report.lock().unwrap().push(event);
    })),
  })
  .await
  .expect("WhatsApp runtime");
  let address = server.local_address();

  let malformed = signed_post(address, "{", APP_SECRET).await;
  assert!(malformed.starts_with("HTTP/1.1 400"), "{malformed}");
  assert!(!malformed.contains(APP_SECRET));
  assert!(!malformed.contains(VERIFY_TOKEN));

  let messages = (0..101)
    .map(|index| {
      json!({
        "from": "15551234567",
        "id": format!("wamid.batch-{index}"),
        "timestamp": "1786880000",
        "type": "text",
        "text": { "body": "bounded" }
      })
    })
    .collect::<Vec<_>>();
  let oversized_batch = json!({
    "object": "whatsapp_business_account",
    "entry": [{
      "changes": [{
        "field": "messages",
        "value": {
          "metadata": { "phone_number_id": PHONE_NUMBER_ID },
          "messages": messages
        }
      }]
    }]
  })
  .to_string();
  let rejected = signed_post(address, &oversized_batch, APP_SECRET).await;
  assert!(rejected.starts_with("HTTP/1.1 413"), "{rejected}");
  assert!(rejected.contains("WOML_WHATSAPP_BATCH_TOO_LARGE"));

  let unicode = json!({
    "object": "whatsapp_business_account",
    "entry": [{
      "changes": [{
        "field": "messages",
        "value": {
          "metadata": { "phone_number_id": PHONE_NUMBER_ID },
          "messages": [{
            "from": "15551234567",
            "id": "wamid.unicode-1",
            "timestamp": "1786880000",
            "type": "text",
            "text": { "body": "مرحبا 👋 WOML" }
          }]
        }
      }]
    }]
  })
  .to_string();
  let accepted = signed_post(address, &unicode, APP_SECRET).await;
  assert!(accepted.starts_with("HTTP/1.1 200"), "{accepted}");
  assert!(progress.lock().unwrap().iter().any(|event| matches!(
    event,
    TriggerProgress::OccurrenceAccepted {
      trigger_handler,
      ..
    } if trigger_handler == "trigger.whatsapp"
  )));

  server.stop_with_deadline(Duration::from_millis(100)).await;
}
