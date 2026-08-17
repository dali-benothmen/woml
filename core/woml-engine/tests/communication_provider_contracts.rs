use jsonschema::Registry;
use serde_json::Value;

const PAYLOAD_SCHEMA: &str =
  include_str!("../../../docs/schemas/communication-trigger-payload.v1.schema.json");
const TRIGGER_HOST_SCHEMA: &str =
  include_str!("../../../docs/schemas/communication-trigger-host.v1.schema.json");
const NOTIFICATION_SCHEMA: &str =
  include_str!("../../../docs/schemas/communication-notification-adapter.v1.schema.json");
const MESSAGING_SCHEMA: &str =
  include_str!("../../../docs/schemas/communication-messaging.v1.schema.json");
const MODEL_SCHEMA: &str =
  include_str!("../../../docs/schemas/communication-provider-model.v1.schema.json");
const EVENT_SCHEMA: &str =
  include_str!("../../../docs/schemas/communication-provider-run-event.v1.schema.json");
const PAYLOAD_FIXTURES: &str =
  include_str!("../../../woml/tests/fixtures/communication-providers/payloads.v1.json");
const CONTRACT_FIXTURES: &str =
  include_str!("../../../woml/tests/fixtures/communication-providers/contracts.v1.json");

fn parse(source: &str) -> Value {
  serde_json::from_str(source).expect("artifact must be valid JSON")
}

fn validator(schema: &Value, resources: &[(&str, Value)]) -> jsonschema::Validator {
  for (_, resource) in resources {
    jsonschema::draft202012::meta::validate(resource)
      .expect("resource must be a valid Draft 2020-12 schema");
  }
  jsonschema::draft202012::meta::validate(schema)
    .expect("contract must be a valid Draft 2020-12 schema");
  let mut registry = Registry::new();
  for (uri, resource) in resources {
    registry = registry
      .add(*uri, resource)
      .expect("schema resource must register");
  }
  let registry = registry.prepare().expect("registry must prepare");
  jsonschema::draft202012::options()
    .with_registry(&registry)
    .should_validate_formats(true)
    .build(schema)
    .expect("schema must compile")
}

#[test]
fn rust_independently_validates_the_communication_provider_fixtures() {
  let payload_schema = parse(PAYLOAD_SCHEMA);
  let payload_validator = validator(&payload_schema, &[]);
  for payload in parse(PAYLOAD_FIXTURES).as_array().unwrap() {
    assert!(
      payload_validator.is_valid(payload),
      "invalid reviewed payload: {:?}",
      payload_validator.iter_errors(payload).collect::<Vec<_>>()
    );
  }

  let payload_id = "https://woml.dev/schemas/communication-trigger-payload/v1";
  let trigger_validator = validator(&parse(TRIGGER_HOST_SCHEMA), &[(payload_id, payload_schema)]);
  let notification_validator = validator(&parse(NOTIFICATION_SCHEMA), &[]);
  let messaging_validator = validator(&parse(MESSAGING_SCHEMA), &[]);
  let model_validator = validator(&parse(MODEL_SCHEMA), &[]);
  let event_validator = validator(&parse(EVENT_SCHEMA), &[]);
  let fixtures = parse(CONTRACT_FIXTURES);

  for value in fixtures["triggerHost"].as_array().unwrap() {
    assert!(trigger_validator.is_valid(value));
  }
  for value in fixtures["notifications"].as_array().unwrap() {
    assert!(notification_validator.is_valid(value));
  }
  for value in fixtures["messaging"].as_array().unwrap() {
    assert!(messaging_validator.is_valid(value));
  }
  for value in fixtures["modelFragments"].as_array().unwrap() {
    assert!(model_validator.is_valid(value));
  }
  for value in fixtures["events"].as_array().unwrap() {
    assert!(event_validator.is_valid(value));
  }
}

#[test]
fn rust_rejects_credentials_and_raw_provider_envelopes_in_context_payload() {
  let payload_validator = validator(&parse(PAYLOAD_SCHEMA), &[]);
  let mut payload = parse(PAYLOAD_FIXTURES)[1].clone();
  payload["token"] = Value::String("synthetic-secret".to_owned());
  assert!(!payload_validator.is_valid(&payload));

  let mut payload = parse(PAYLOAD_FIXTURES)[1].clone();
  payload["providerData"]["raw"] = Value::String("synthetic-envelope".to_owned());
  assert!(!payload_validator.is_valid(&payload));
}
