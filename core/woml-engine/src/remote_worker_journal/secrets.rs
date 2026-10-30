//! Private remote worker journal secrets.

use crate::remote_worker_journal::RemoteWorkerError;
use serde_json::Value;
use std::collections::BTreeMap;

pub trait RemoteSecretProvider: Send + Sync {
  fn resolve(&self, name: &str) -> Result<String, RemoteWorkerError>;
}

#[derive(Debug, Default)]
pub struct EnvironmentRemoteSecretProvider;

impl RemoteSecretProvider for EnvironmentRemoteSecretProvider {
  fn resolve(&self, name: &str) -> Result<String, RemoteWorkerError> {
    std::env::var(name).map_err(|_| RemoteWorkerError::SecretUnavailable)
  }
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedRemoteSecretProvider {
  pub(in crate::remote_worker_journal) values: BTreeMap<String, String>,
}

impl ResolvedRemoteSecretProvider {
  pub fn new(values: BTreeMap<String, String>) -> Self {
    Self { values }
  }
}

impl RemoteSecretProvider for ResolvedRemoteSecretProvider {
  fn resolve(&self, name: &str) -> Result<String, RemoteWorkerError> {
    self
      .values
      .get(name)
      .cloned()
      .ok_or(RemoteWorkerError::SecretUnavailable)
  }
}

pub(in crate::remote_worker_journal) fn redact_json_value(
  value: &mut Value,
  sensitive_values: &[String],
) {
  match value {
    Value::String(text) => {
      let mut ordered = sensitive_values
        .iter()
        .filter(|secret| !secret.is_empty())
        .collect::<Vec<_>>();
      ordered.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
      for secret in ordered {
        *text = text.replace(secret, "[redacted]");
      }
    }
    Value::Array(values) => {
      for value in values {
        redact_json_value(value, sensitive_values);
      }
    }
    Value::Object(fields) => {
      for value in fields.values_mut() {
        redact_json_value(value, sensitive_values);
      }
    }
    Value::Null | Value::Bool(_) | Value::Number(_) => {}
  }
}

pub(in crate::remote_worker_journal) fn valid_secret_name(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 128
    && value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_uppercase())
    && value
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}
