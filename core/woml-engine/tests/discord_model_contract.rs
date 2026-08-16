use serde_json::{json, Value};
use woml_engine::{
  run_event_schema_version_for_model, CompiledWorkflowDefinition,
  COMPILED_MODEL_SCHEMA_VERSION_V15, RUN_EVENT_SCHEMA_VERSION_V14,
};

const MODEL_V14: &str =
  include_str!("../../../woml/tests/fixtures/reusable-definitions/model-v14.reviewed.json");

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
