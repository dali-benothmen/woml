//! Bounded safe metadata, identifier/digest checks, encoded-size accounting, and result-envelope validation.

use crate::capability::CAPABILITY_CALL_CONTRACT;
use crate::capability::CAPABILITY_CALL_CONTRACT_VERSION;
use serde_json::Map;
use serde_json::Value;

pub fn validate_safe_metadata(metadata: &Map<String, Value>) -> Result<(), String> {
  const FORBIDDEN: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "token",
    "secret",
    "password",
    "body",
    "query",
    "url",
    "headers",
    "input",
    "output",
    "data",
    "value",
  ];
  if metadata.len() > 32 {
    return Err("Safe metadata may contain at most 32 fields.".to_string());
  }
  for (key, value) in metadata {
    let valid_key = key.len() <= 64
      && key
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
      && key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
      && !FORBIDDEN.contains(&key.to_ascii_lowercase().as_str());
    if !valid_key
      || !matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
      )
    {
      return Err(
        "Capability metadata must be bounded, scalar, and free of sensitive field names."
          .to_string(),
      );
    }
  }
  Ok(())
}

pub(in crate::capability) fn valid_name(value: &str) -> bool {
  value.len() <= 128
    && value.split(['.', '_', '-']).all(|segment| {
      let mut chars = segment.chars();
      matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

pub(in crate::capability) fn valid_digest(value: &str) -> bool {
  value.strip_prefix("sha256:").is_some_and(|digest| {
    digest.len() == 64
      && digest
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
  })
}

pub(in crate::capability) fn serialized_bytes(value: &Value) -> u64 {
  serde_json::to_vec(value)
    .map(|bytes| bytes.len() as u64)
    .unwrap_or(u64::MAX)
}

pub(in crate::capability) fn validate_result_base(
  contract: &str,
  contract_version: u32,
  message_type: &str,
  invocation_id: &str,
  call_id: &str,
  duration_ms: f64,
) -> Result<(), String> {
  if contract != CAPABILITY_CALL_CONTRACT
    || contract_version != CAPABILITY_CALL_CONTRACT_VERSION
    || message_type != "result"
    || invocation_id.is_empty()
    || invocation_id.len() > 256
    || call_id.is_empty()
    || call_id.len() > 256
    || !duration_ms.is_finite()
    || duration_ms < 0.0
  {
    return Err(
      "Capability result does not match the Capability Call v1 base contract.".to_string(),
    );
  }
  Ok(())
}
