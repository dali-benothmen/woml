use serde_json::{json, Value};
use woml_engine::{
  run_event_schema_version_for_model, CompiledWorkflowDefinition,
  COMPILED_MODEL_SCHEMA_VERSION_V15, RUN_EVENT_SCHEMA_VERSION_V14,
};

const MODEL_V13: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");

fn whatsapp_model() -> Value {
  let mut value: Value = serde_json::from_str(MODEL_V13).expect("Model v13 fixture");
  value["schemaVersion"] = json!(COMPILED_MODEL_SCHEMA_VERSION_V15);
  value["triggers"] = json!([{
    "id": "customerMessage",
    "handler": "trigger.whatsapp",
    "config": {
      "kind": "object",
      "fields": {
        "events": {
          "kind": "array",
          "items": [{ "kind": "literal", "value": "message" }]
        },
        "phoneNumberId": { "kind": "literal", "value": "123456789012345" },
        "verifyToken": {
          "kind": "secretReference",
          "name": "WHATSAPP_VERIFY_TOKEN"
        },
        "appSecret": {
          "kind": "secretReference",
          "name": "WHATSAPP_APP_SECRET"
        }
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
  value
}

#[test]
fn model_v15_accepts_the_frozen_whatsapp_trigger_and_requirement() {
  let model = CompiledWorkflowDefinition::from_json(
    &serde_json::to_string(&whatsapp_model()).expect("Model v15 JSON"),
  )
  .expect("Model v15 WhatsApp contract");
  model
    .validate_structure()
    .expect("valid Model v15 WhatsApp authoring structure");
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V15);
  assert_eq!(
    run_event_schema_version_for_model(model.schema_version),
    RUN_EVENT_SCHEMA_VERSION_V14
  );
}

#[test]
fn model_v15_rejects_unreviewed_whatsapp_events_and_display_numbers() {
  for (field, invalid) in [
    (
      "events",
      json!({
        "kind": "array",
        "items": [{ "kind": "literal", "value": "status" }]
      }),
    ),
    (
      "phoneNumberId",
      json!({ "kind": "literal", "value": "+1 555 123 4567" }),
    ),
  ] {
    let mut value = whatsapp_model();
    value["triggers"][0]["config"]["fields"][field] = invalid;
    let model = CompiledWorkflowDefinition::from_json(
      &serde_json::to_string(&value).expect("invalid Model v15 JSON"),
    )
    .expect("invalid contract still deserializes");
    assert!(
      model.validate_structure().is_err(),
      "WhatsApp field {field} should have failed validation"
    );
  }
}

#[test]
fn model_v15_accepts_frozen_whatsapp_lifecycle_template_metadata() {
  let mut value = whatsapp_model();
  let delivery_id = "lifecycle:run_success:action:0:provider:0:recipient:0";
  value["lifecycle"] = json!({
    "profileVersion": 1,
    "hooks": [{
      "hookId": "lifecycle:run_success",
      "event": "run_success",
      "actions": [{
        "actionId": "lifecycle:run_success:action:0",
        "handler": "notification.informational",
        "inputs": {
          "kind": "object",
          "fields": {
            "deliveries": {
              "kind": "array",
              "items": [{
                "kind": "object",
                "fields": {
                  "deliveryId": { "kind": "literal", "value": delivery_id },
                  "provider": { "kind": "literal", "value": "whatsapp" },
                  "destination": { "kind": "literal", "value": "15551234567" },
                  "credentials": {
                    "kind": "object",
                    "fields": {
                      "accessToken": {
                        "kind": "secretReference",
                        "name": "WHATSAPP_ACCESS_TOKEN"
                      },
                      "phoneNumberId": {
                        "kind": "literal",
                        "value": "123456789012345"
                      }
                    }
                  },
                  "templateName": { "kind": "literal", "value": "workflow_completed" },
                  "language": { "kind": "literal", "value": "en_US" },
                  "message": {
                    "kind": "template",
                    "parts": [{ "kind": "text", "text": "Workflow completed." }]
                  }
                }
              }]
            }
          }
        }
      }]
    }]
  });
  value["communication"]["providers"][0]["notificationDeliveryIds"] = json!([delivery_id]);
  value["communication"]["providers"][0]["credentialNames"] = json!([
    "WHATSAPP_ACCESS_TOKEN",
    "WHATSAPP_APP_SECRET",
    "WHATSAPP_VERIFY_TOKEN"
  ]);

  let model =
    CompiledWorkflowDefinition::from_json(&serde_json::to_string(&value).expect("Model v15 JSON"))
      .expect("Model v15 WhatsApp lifecycle contract");
  model
    .validate_structure()
    .expect("valid WhatsApp lifecycle-template structure");
}
