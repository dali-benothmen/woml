use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};
use woml_engine::{
  run_event_schema_version_for_model, CompiledWorkflowDefinition, DurableEventStore, RunEvent,
  TriggerAdmissionRequest, COMPILED_MODEL_SCHEMA_VERSION_V15, RUN_EVENT_SCHEMA_VERSION_V14,
};

const DISCORD_DEFINITION_HASH: &str =
  "sha256:1616161616161616161616161616161616161616161616161616161616161616";

const MODEL_V14: &str =
  include_str!("../../../woml/tests/fixtures/reusable-definitions/model-v14.reviewed.json");
const APPROVED_EVENTS: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-slack-approved.events.v5.json");

fn discord_model() -> Value {
  let mut value: Value = serde_json::from_str(MODEL_V14).expect("Model v14 fixture");
  value["schemaVersion"] = json!(COMPILED_MODEL_SCHEMA_VERSION_V15);
  value["triggers"] = json!([{
    "id": "agentMessage",
    "handler": "trigger.discord",
    "config": {
      "kind": "object",
      "fields": {
        "events": {
          "kind": "array",
          "items": [
            { "kind": "literal", "value": "app-mention" },
            { "kind": "literal", "value": "direct-message" }
          ]
        },
        "channels": {
          "kind": "array",
          "items": [
            { "kind": "literal", "value": "200000000000000001" },
            { "kind": "literal", "value": "200000000000000002" }
          ]
        },
        "botToken": {
          "kind": "secretReference",
          "name": "DISCORD_BOT_TOKEN"
        }
      }
    }
  }]);
  value["communication"] = json!({
    "profileVersion": 1,
    "providers": [{
      "provider": "discord",
      "triggerIds": ["agentMessage"],
      "notificationDeliveryIds": [],
      "messaging": true,
      "credentialNames": ["DISCORD_BOT_TOKEN"]
    }]
  });
  value
}

#[test]
fn model_v15_accepts_the_frozen_discord_trigger_and_requirement() {
  let model = CompiledWorkflowDefinition::from_json(
    &serde_json::to_string(&discord_model()).expect("Model v15 JSON"),
  )
  .expect("Model v15 Discord contract");
  model
    .validate_structure()
    .expect("valid Model v15 Discord structure");
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V15);
  assert_eq!(
    run_event_schema_version_for_model(model.schema_version),
    RUN_EVENT_SCHEMA_VERSION_V14
  );
}

#[test]
fn model_v15_rejects_discord_channel_names_and_unreviewed_events() {
  for (field, invalid) in [
    (
      "channels",
      json!({
        "kind": "array",
        "items": [{ "kind": "literal", "value": "general" }]
      }),
    ),
    (
      "events",
      json!({
        "kind": "array",
        "items": [{ "kind": "literal", "value": "slash-command" }]
      }),
    ),
  ] {
    let mut value = discord_model();
    value["triggers"][0]["config"]["fields"][field] = invalid;
    let model = CompiledWorkflowDefinition::from_json(
      &serde_json::to_string(&value).expect("invalid Model v15 JSON"),
    )
    .expect("invalid contract still deserializes");
    assert!(
      model.validate_structure().is_err(),
      "Discord field {field} should have failed validation"
    );
  }
}

#[test]
fn model_v15_durably_admits_a_discord_mention_once() {
  let model = CompiledWorkflowDefinition::from_json(
    &serde_json::to_string(&discord_model()).expect("Model v15 JSON"),
  )
  .expect("Model v15 Discord contract");
  let mut store = DurableEventStore::open_in_memory().expect("durable store");
  store
    .register_definition(&model, DISCORD_DEFINITION_HASH)
    .expect("register Discord definition");
  let request = TriggerAdmissionRequest {
    workflow_id: model.workflow_id.clone(),
    definition_hash: DISCORD_DEFINITION_HASH.to_string(),
    trigger_id: "agentMessage".to_string(),
    trigger_handler: "trigger.discord".to_string(),
    source_identity: "discord:123456789012345678:456789012345678901:discord-agent:agentMessage"
      .to_string(),
    payload: Map::from_iter([
      ("provider".to_string(), json!("discord")),
      ("event".to_string(), json!("app-mention")),
      ("text".to_string(), json!("hello")),
      ("senderId".to_string(), json!("234567890123456789")),
      ("conversationId".to_string(), json!("345678901234567890")),
      ("conversationType".to_string(), json!("group")),
      ("messageId".to_string(), json!("456789012345678901")),
      (
        "providerData".to_string(),
        json!({ "botId": "123456789012345678" }),
      ),
    ]),
    received_at: Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap(),
  };
  let first = store
    .admit_trigger_occurrence(request.clone())
    .expect("first Discord admission");
  let duplicate = store
    .admit_trigger_occurrence(request)
    .expect("duplicate Discord admission");
  assert!(!first.duplicate);
  assert!(duplicate.duplicate);
  assert_eq!(first.run_id, duplicate.run_id);
}

#[test]
fn event_v14_accepts_a_bounded_discord_approval_actor() {
  let events: Vec<Value> = serde_json::from_str(APPROVED_EVENTS).expect("approval events");
  let mut value = events
    .into_iter()
    .find(|event| event["type"] == "notification_decision_accepted")
    .expect("notification decision event");
  value["eventSchemaVersion"] = json!(RUN_EVENT_SCHEMA_VERSION_V14);
  value["data"]["provider"] = json!("discord");
  value["data"]["providerActorId"] = json!("discord:234567890123456789");
  let event: RunEvent = serde_json::from_value(value).expect("Discord decision event");
  event
    .validate()
    .expect("valid Discord actor audit identity");
}
