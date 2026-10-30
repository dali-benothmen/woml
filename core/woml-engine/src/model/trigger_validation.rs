//! model trigger validation.

use super::{valid_secret_name, CompiledTrigger, ValueExpression};
use std::collections::BTreeMap;
use std::collections::HashSet;

pub(super) fn collect_custom_provider_ids(expression: &ValueExpression, result: &mut Vec<String>) {
  match expression {
    ValueExpression::Object { fields } => {
      if literal_string(fields.get("provider")) == Some("custom") {
        if let Some(provider_id) = literal_string(fields.get("providerId")) {
          result.push(provider_id.to_string());
        }
      }
      for value in fields.values() {
        collect_custom_provider_ids(value, result);
      }
    }
    ValueExpression::Array { items } => {
      for value in items {
        collect_custom_provider_ids(value, result);
      }
    }
    _ => {}
  }
}

pub(super) fn object_fields(
  expression: &ValueExpression,
) -> Option<&BTreeMap<String, ValueExpression>> {
  match expression {
    ValueExpression::Object { fields } => Some(fields),
    _ => None,
  }
}

pub(super) fn literal_string(expression: Option<&ValueExpression>) -> Option<&str> {
  match expression {
    Some(ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  }
}

fn exact_fields(fields: &BTreeMap<String, ValueExpression>, names: &[&str]) -> bool {
  fields.len() == names.len() && names.iter().all(|name| fields.contains_key(*name))
}

fn valid_schema_literal(expression: Option<&ValueExpression>) -> bool {
  matches!(expression, Some(ValueExpression::Literal { value }) if value.is_object())
}

fn valid_webhook_path(path: &str) -> bool {
  path.len() <= 2048
    && path.starts_with('/')
    && path != "/_woml"
    && !path.starts_with("/_woml/")
    && (path == "/"
      || path[1..].split('/').all(|segment| {
        !segment.is_empty()
          && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-'))
      }))
}

fn valid_webhook_authentication(expression: Option<&ValueExpression>) -> bool {
  let Some(fields) = expression.and_then(object_fields) else {
    return false;
  };
  match literal_string(fields.get("kind")) {
    Some("none") => exact_fields(fields, &["kind"]),
    Some("bearer") => {
      exact_fields(fields, &["kind", "secret"])
        && matches!(fields.get("secret"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
    }
    _ => false,
  }
}

fn valid_webhook_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let keys_are_valid = exact_fields(fields, &["path", "method", "authentication"])
    || exact_fields(fields, &["path", "method", "authentication", "schema"]);
  keys_are_valid
    && literal_string(fields.get("path")).is_some_and(valid_webhook_path)
    && literal_string(fields.get("method")) == Some("POST")
    && valid_webhook_authentication(fields.get("authentication"))
    && (!fields.contains_key("schema") || valid_schema_literal(fields.get("schema")))
}

fn literal_string_array(expression: Option<&ValueExpression>) -> Option<Vec<&str>> {
  let Some(ValueExpression::Array { items }) = expression else {
    return None;
  };
  items
    .iter()
    .map(|item| match item {
      ValueExpression::Literal { value } => value.as_str(),
      _ => None,
    })
    .collect()
}

fn unique_non_empty(values: &[&str]) -> bool {
  let mut seen = HashSet::new();
  values
    .iter()
    .all(|value| !value.is_empty() && seen.insert(*value))
}

fn valid_slack_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let Some(events) = literal_string_array(fields.get("events")) else {
    return false;
  };
  let Some(channels) = literal_string_array(fields.get("channels")) else {
    return false;
  };
  exact_fields(fields, &["events", "channels", "botToken", "appToken"])
    && (1..=2).contains(&events.len())
    && unique_non_empty(&events)
    && events
      .iter()
      .all(|event| matches!(*event, "app-mention" | "direct-message"))
    && unique_non_empty(&channels)
    && matches!(fields.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
    && matches!(fields.get("appToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
}

fn valid_telegram_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let Some(events) = literal_string_array(fields.get("events")) else {
    return false;
  };
  exact_fields(fields, &["events", "botToken"])
    && events == ["message"]
    && matches!(fields.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
}

pub(super) fn valid_discord_snowflake(value: &str) -> bool {
  (17..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_discord_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let Some(events) = literal_string_array(fields.get("events")) else {
    return false;
  };
  let channels = fields
    .get("channels")
    .map(|value| literal_string_array(Some(value)));
  if channels == Some(None) {
    return false;
  }
  let channels = channels.flatten();
  let exact = exact_fields(fields, &["events", "botToken"])
    || exact_fields(fields, &["events", "channels", "botToken"]);
  exact
    && (1..=2).contains(&events.len())
    && unique_non_empty(&events)
    && events
      .iter()
      .all(|event| matches!(*event, "app-mention" | "direct-message"))
    && channels.is_none_or(|channels| {
      !channels.is_empty()
        && unique_non_empty(&channels)
        && channels
          .iter()
          .all(|channel| valid_discord_snowflake(channel))
    })
    && matches!(fields.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
}

pub(super) fn valid_whatsapp_phone_number_id(value: &str) -> bool {
  (6..=20).contains(&value.len())
    && !value.starts_with('0')
    && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn valid_whatsapp_recipient(value: &str) -> bool {
  (8..=16).contains(&value.len())
    && !value.starts_with('0')
    && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn valid_whatsapp_template(value: &str) -> bool {
  !value.is_empty()
    && value
      .bytes()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(super) fn valid_whatsapp_language(value: &str) -> bool {
  let bytes = value.as_bytes();
  matches!(bytes.len(), 2 | 3) && bytes.iter().all(|byte| byte.is_ascii_lowercase())
    || matches!(bytes.len(), 5 | 6)
      && bytes[..bytes.len() - 3]
        .iter()
        .all(|byte| byte.is_ascii_lowercase())
      && bytes[bytes.len() - 3] == b'_'
      && bytes[bytes.len() - 2..]
        .iter()
        .all(|byte| byte.is_ascii_uppercase())
}

fn valid_whatsapp_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let Some(events) = literal_string_array(fields.get("events")) else {
    return false;
  };
  exact_fields(
    fields,
    &["events", "phoneNumberId", "verifyToken", "appSecret"],
  ) && events == ["message"]
    && literal_string(fields.get("phoneNumberId")).is_some_and(valid_whatsapp_phone_number_id)
    && matches!(fields.get("verifyToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
    && matches!(fields.get("appSecret"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
}

fn valid_on_missed(expression: Option<&ValueExpression>) -> bool {
  matches!(literal_string(expression), Some("skip" | "run-once"))
}

fn valid_schedule_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  exact_fields(fields, &["cron", "timezone", "onMissed"])
    && literal_string(fields.get("cron")).is_some_and(|value| !value.is_empty())
    && literal_string(fields.get("timezone")).is_some_and(|value| !value.is_empty())
    && valid_on_missed(fields.get("onMissed"))
}

fn valid_interval_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  exact_fields(fields, &["everyMs", "onMissed"])
    && matches!(fields.get("everyMs"), Some(ValueExpression::Literal { value }) if value.as_u64().is_some_and(|value| (1_000..=2_592_000_000).contains(&value)))
    && valid_on_missed(fields.get("onMissed"))
}

fn valid_event_name(value: &str) -> bool {
  if value.len() > 256
    || !value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_lowercase())
  {
    return false;
  }
  let mut has_separator = false;
  let mut previous_separator = false;
  for byte in value.bytes() {
    let separator = matches!(byte, b'.' | b'_' | b'-');
    if separator {
      if previous_separator {
        return false;
      }
      has_separator = true;
    } else if !byte.is_ascii_lowercase() && !byte.is_ascii_digit() {
      return false;
    }
    previous_separator = separator;
  }
  has_separator && !previous_separator
}

fn valid_event_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  // Model v7 event definitions were persisted before publisher authentication
  // became an authored frontend requirement. Keep those immutable definitions
  // readable for recovery; the active ingress runtime separately requires the
  // symbolic secret on newly registered event triggers.
  let keys_are_valid = exact_fields(fields, &["name"])
    || exact_fields(fields, &["name", "schema"])
    || exact_fields(fields, &["name", "secret"])
    || exact_fields(fields, &["name", "schema", "secret"]);
  keys_are_valid
    && literal_string(fields.get("name")).is_some_and(valid_event_name)
    && (!fields.contains_key("secret")
      || matches!(fields.get("secret"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name)))
    && (!fields.contains_key("schema") || valid_schema_literal(fields.get("schema")))
}

pub(super) fn valid_model_v7_trigger(trigger: &CompiledTrigger) -> bool {
  match trigger.handler.as_str() {
    "trigger.manual" => {
      matches!(&trigger.config, ValueExpression::Object { fields } if fields.is_empty())
    }
    "trigger.webhook" => valid_webhook_trigger(&trigger.config),
    "trigger.slack" => valid_slack_trigger(&trigger.config),
    "trigger.telegram" => valid_telegram_trigger(&trigger.config),
    "trigger.discord" => valid_discord_trigger(&trigger.config),
    "trigger.whatsapp" => valid_whatsapp_trigger(&trigger.config),
    "trigger.schedule" => valid_schedule_trigger(&trigger.config),
    "trigger.interval" => valid_interval_trigger(&trigger.config),
    "trigger.event" => valid_event_trigger(&trigger.config),
    _ => false,
  }
}

pub(super) fn valid_slack_destination(value: &str) -> bool {
  if let Some(alias) = value.strip_prefix('#') {
    return !alias.is_empty()
      && alias.len() <= 80
      && alias.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase()
          || byte.is_ascii_digit()
          || (index > 0 && matches!(byte, b'_' | b'-'))
      });
  }
  let bytes = value.as_bytes();
  (9..=32).contains(&bytes.len())
    && matches!(bytes.first(), Some(b'C' | b'G'))
    && bytes[1..]
      .iter()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}
