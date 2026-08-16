use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};
use woml_engine::{
  event::is_notification_delivery_id,
  run_event_schema_version_for_model, CompiledWorkflowDefinition, DurableEventStore,
  ProviderMessageIdentity, RunEvent, TriggerAdmissionRequest, COMPILED_MODEL_SCHEMA_VERSION_V15,
  RUN_EVENT_SCHEMA_VERSION_V14,
};

#[test]
fn durable_notification_identity_accepts_telegram_chat_destinations() {
  assert!(is_notification_delivery_id(
    "review:notify:0:chat:0",
    "review"
  ));
  assert!(is_notification_delivery_id(
    "review:notify:2:channel:3",
    "review"
  ));
  assert!(!is_notification_delivery_id(
    "review:notify:00:chat:0",
    "review"
  ));
}

const MODEL_V14: &str =
  include_str!("../../../woml/tests/fixtures/reusable-definitions/model-v14.reviewed.json");
const APPROVED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-slack-approved.events.v5.json");
const EVENT_V14_SCHEMA: &str = include_str!("../../../docs/schemas/run-event.v14.schema.json");
const TELEGRAM_DEFINITION_HASH: &str =
  "sha256:1515151515151515151515151515151515151515151515151515151515151515";

fn telegram_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(MODEL_V14).expect("Model v14 fixture");
  value["schemaVersion"] = json!(COMPILED_MODEL_SCHEMA_VERSION_V15);
  value["triggers"] = json!([{
    "id": "agentMessage",
    "handler": "trigger.telegram",
    "config": {
      "kind": "object",
      "fields": {
        "events": {
          "kind": "array",
          "items": [{ "kind": "literal", "value": "message" }]
        },
        "botToken": {
          "kind": "secretReference",
          "name": "TELEGRAM_BOT_TOKEN"
        }
      }
    }
  }]);
  value["communication"] = json!({
    "profileVersion": 1,
    "providers": [{
      "provider": "telegram",
      "triggerIds": ["agentMessage"],
      "notificationDeliveryIds": [],
      "messaging": true,
      "credentialNames": ["TELEGRAM_BOT_TOKEN"]
    }]
  });

  CompiledWorkflowDefinition::from_json(&serde_json::to_string(&value).expect("Model v15 JSON"))
    .expect("Model v15 Telegram contract")
}

#[test]
fn model_v15_accepts_the_frozen_telegram_trigger_and_requirement() {
  let model = telegram_model();
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V15);
  assert_eq!(
    run_event_schema_version_for_model(model.schema_version),
    RUN_EVENT_SCHEMA_VERSION_V14
  );
}

#[test]
fn model_v15_durably_admits_a_telegram_message() {
  let model = telegram_model();
  let mut store = DurableEventStore::open_in_memory().expect("durable store");
  store
    .register_definition(&model, TELEGRAM_DEFINITION_HASH)
    .expect("register Telegram definition");
  let outcome = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: model.workflow_id.clone(),
      definition_hash: TELEGRAM_DEFINITION_HASH.to_string(),
      trigger_id: "agentMessage".to_string(),
      trigger_handler: "trigger.telegram".to_string(),
      source_identity: "telegram:987654321:41:telegram-agent:agentMessage".to_string(),
      payload: Map::from_iter([
        ("provider".to_string(), json!("telegram")),
        ("event".to_string(), json!("message")),
        ("text".to_string(), json!("hello")),
        ("senderId".to_string(), json!("111222333")),
        ("senderName".to_string(), json!("Dali")),
        ("conversationId".to_string(), json!("111222333")),
        ("conversationType".to_string(), json!("direct")),
        ("messageId".to_string(), json!("7")),
        ("providerData".to_string(), json!({ "botId": "987654321" })),
      ]),
      received_at: Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap(),
    })
    .expect("durable Telegram admission");
  assert!(!outcome.duplicate);
  assert_eq!(store.events(&outcome.run_id).unwrap().len(), 1);
}

#[test]
fn event_v14_persists_provider_neutral_telegram_message_identity() {
  let mut events: Vec<Value> = serde_json::from_str(APPROVED_EVENTS).expect("Event v5 fixture");
  let event = events
    .iter_mut()
    .find(|event| event["type"] == "notification_delivery_succeeded")
    .expect("notification success event");
  event["eventSchemaVersion"] = json!(RUN_EVENT_SCHEMA_VERSION_V14);
  event["data"]["providerMessage"] = json!({
    "provider": "telegram",
    "accountId": "987654321",
    "conversationId": "-1001234567890",
    "messageId": "42"
  });

  let event: RunEvent = serde_json::from_value(event.clone()).expect("Telegram event shape");
  event.validate().expect("Event v14 Telegram identity");
  let encoded = serde_json::to_value(event).expect("encoded Event v14");
  let schema: Value = serde_json::from_str(EVENT_V14_SCHEMA).expect("Event v14 schema JSON");
  jsonschema::draft202012::meta::validate(&schema).expect("Event v14 schema is valid");
  let validator = jsonschema::draft202012::options()
    .should_validate_formats(true)
    .build(&schema)
    .expect("Event v14 validator");
  assert!(
    validator.is_valid(&encoded),
    "invalid Event v14: {:?}",
    validator.iter_errors(&encoded).collect::<Vec<_>>()
  );
  let identity: ProviderMessageIdentity =
    serde_json::from_value(encoded["data"]["providerMessage"].clone())
      .expect("provider-neutral identity");
  assert!(matches!(
    identity,
    ProviderMessageIdentity::Communication(message)
      if message.provider == "telegram"
        && message.conversation_id == "-1001234567890"
        && message.message_id == "42"
  ));
}
