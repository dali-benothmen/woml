//! Worker Configuration v1 parsing and safe file-reference resolution.

use std::{
  collections::BTreeSet,
  fs,
  path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WORKER_CONFIGURATION_PROFILE: &str = "woml.worker/v1";
pub const WORKER_CONFIGURATION_VERSION: u32 = 1;
pub const WORKER_CONFIGURATION_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerTlsReferencesV1 {
  pub certificate_ref: String,
  pub private_key_ref: String,
  pub ca_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerSecretProviderV1 {
  Environment,
  File,
  External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerConfigurationV1 {
  pub profile: String,
  pub version: u32,
  pub worker_id: String,
  pub coordinator_url: String,
  pub target_ids: Vec<String>,
  pub capacity: u32,
  pub tls: WorkerTlsReferencesV1,
  pub process_policy: String,
  pub secret_provider: WorkerSecretProviderV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWorkerConfigurationV1 {
  pub configuration: WorkerConfigurationV1,
  pub certificate_path: PathBuf,
  pub private_key_path: PathBuf,
  pub ca_path: PathBuf,
  pub process_policy_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteWorkerConfigurationError {
  #[error("the worker configuration exceeds its size limit")]
  TooLarge,
  #[error("the worker configuration is incompatible")]
  Incompatible,
  #[error("the worker configuration is invalid: {0}")]
  Invalid(String),
  #[error("the worker configuration references unavailable operator material")]
  Unavailable,
}

impl RemoteWorkerConfigurationError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::TooLarge | Self::Invalid(_) => "WOML_REMOTE_PROTOCOL_INVALID",
      Self::Incompatible => "WOML_TARGET_INCOMPATIBLE",
      Self::Unavailable => "WOML_TARGET_UNAVAILABLE",
    }
  }
}

pub fn parse_worker_configuration(
  source: &[u8],
) -> Result<WorkerConfigurationV1, RemoteWorkerConfigurationError> {
  if source.len() > WORKER_CONFIGURATION_MAX_BYTES {
    return Err(RemoteWorkerConfigurationError::TooLarge);
  }
  let configuration: WorkerConfigurationV1 = serde_json::from_slice(source)
    .map_err(|_| invalid("JSON has malformed, duplicate, missing, or unknown fields"))?;
  if configuration.profile != WORKER_CONFIGURATION_PROFILE
    || configuration.version != WORKER_CONFIGURATION_VERSION
  {
    return Err(RemoteWorkerConfigurationError::Incompatible);
  }
  if !valid_id(&configuration.worker_id)
    || configuration.target_ids.is_empty()
    || configuration.target_ids.len() > 32
    || configuration.capacity == 0
    || configuration.capacity > 64
    || configuration.process_policy.is_empty()
    || configuration.process_policy.len() > 512
  {
    return Err(invalid(
      "worker identity, targets, capacity, or process policy is invalid",
    ));
  }
  let mut targets = BTreeSet::new();
  for target in &configuration.target_ids {
    if !valid_id(target) || !targets.insert(target) {
      return Err(invalid("targetIds contain an invalid or duplicate target"));
    }
  }
  if !valid_https_origin(&configuration.coordinator_url) {
    return Err(invalid(
      "coordinatorUrl must be an HTTPS origin without credentials, path, query, or fragment",
    ));
  }
  for reference in [
    &configuration.tls.certificate_ref,
    &configuration.tls.private_key_ref,
    &configuration.tls.ca_ref,
  ] {
    if !reference.starts_with("file:")
      || reference.len() <= 5
      || reference.len() > 1024
      || reference.contains(['\r', '\n'])
    {
      return Err(invalid("TLS references must be bounded file: references"));
    }
  }
  Ok(configuration)
}

pub fn prepare_worker_configuration(
  source: &[u8],
  configuration_path: &Path,
) -> Result<PreparedWorkerConfigurationV1, RemoteWorkerConfigurationError> {
  let configuration = parse_worker_configuration(source)?;
  let root = configuration_path
    .canonicalize()
    .map_err(|_| RemoteWorkerConfigurationError::Unavailable)?
    .parent()
    .ok_or(RemoteWorkerConfigurationError::Unavailable)?
    .to_path_buf();
  let certificate_path = resolve_file_reference(&root, &configuration.tls.certificate_ref, false)?;
  let private_key_path = resolve_file_reference(&root, &configuration.tls.private_key_ref, true)?;
  let ca_path = resolve_file_reference(&root, &configuration.tls.ca_ref, false)?;
  let process_policy_path = resolve_path(&root, &configuration.process_policy, false)?;
  Ok(PreparedWorkerConfigurationV1 {
    configuration,
    certificate_path,
    private_key_path,
    ca_path,
    process_policy_path,
  })
}

fn resolve_file_reference(
  root: &Path,
  reference: &str,
  private: bool,
) -> Result<PathBuf, RemoteWorkerConfigurationError> {
  resolve_path(root, &reference[5..], private)
}

fn resolve_path(
  root: &Path,
  reference: &str,
  private: bool,
) -> Result<PathBuf, RemoteWorkerConfigurationError> {
  #[cfg(not(unix))]
  let _ = private;
  let unresolved = Path::new(reference);
  let candidate = if unresolved.is_absolute() {
    unresolved.to_path_buf()
  } else {
    root.join(unresolved)
  };
  let metadata =
    fs::symlink_metadata(&candidate).map_err(|_| RemoteWorkerConfigurationError::Unavailable)?;
  if !metadata.is_file()
    || metadata.file_type().is_symlink()
    || metadata.len() == 0
    || metadata.len() > 1024 * 1024
  {
    return Err(invalid(
      "referenced operator material must be a bounded regular non-symlink file",
    ));
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    if mode & 0o022 != 0 || (private && mode & 0o077 != 0) {
      return Err(invalid(
        "referenced operator material has unsafe permissions",
      ));
    }
  }
  candidate
    .canonicalize()
    .map_err(|_| RemoteWorkerConfigurationError::Unavailable)
}

fn valid_https_origin(value: &str) -> bool {
  let Some(authority) = value.strip_prefix("https://") else {
    return false;
  };
  if authority.is_empty()
    || authority.len() > 500
    || authority.contains(['/', '?', '#', '@', '\r', '\n'])
    || authority.chars().any(char::is_whitespace)
  {
    return false;
  }
  let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
    let Some((host, remainder)) = bracketed.split_once(']') else {
      return false;
    };
    if remainder.is_empty() {
      (host, None)
    } else if let Some(port) = remainder.strip_prefix(':') {
      (host, Some(port))
    } else {
      return false;
    }
  } else if let Some((host, port)) = authority.rsplit_once(':') {
    if host.contains(':') {
      return false;
    }
    (host, Some(port))
  } else {
    (authority, None)
  };
  !host.is_empty()
    && port.is_none_or(|port| {
      !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
    })
}

fn valid_id(value: &str) -> bool {
  let bytes = value.as_bytes();
  !bytes.is_empty()
    && bytes.len() <= 128
    && bytes[0].is_ascii_lowercase()
    && bytes
      .iter()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    && bytes.last() != Some(&b'-')
    && !bytes.windows(2).any(|pair| pair == b"--")
}

fn invalid(message: &str) -> RemoteWorkerConfigurationError {
  RemoteWorkerConfigurationError::Invalid(message.to_string())
}
