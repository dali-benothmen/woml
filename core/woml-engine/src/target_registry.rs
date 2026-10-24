//! Frozen Target Registry v1 parsing and canonical identity.
//!
//! The CLI owns filesystem and certificate-reference preflight. The Rust core
//! independently validates the public registry contract before later remote
//! dispatch phases are allowed to consume it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const TARGET_REGISTRY_PROFILE: &str = "woml.target-registry/v1";
pub const TARGET_REGISTRY_VERSION: u32 = 1;
pub const TARGET_REGISTRY_MAX_BYTES: usize = 1024 * 1024;
pub const TARGET_REGISTRY_MAX_TARGETS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetRegistryEntryV1 {
  pub id: String,
  pub kind: String,
  pub allowed_worker_identities: Vec<String>,
  pub required_capabilities: Vec<String>,
  pub minimum_worker_version: String,
  pub maximum_in_flight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetRegistryV1 {
  pub profile: String,
  pub version: u32,
  pub targets: Vec<TargetRegistryEntryV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTargetRegistryV1 {
  pub registry: TargetRegistryV1,
  pub canonical_payload: Vec<u8>,
  pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TargetRegistryError {
  #[error("the target registry exceeds its size limit")]
  TooLarge,
  #[error("the target registry is incompatible: {0}")]
  Incompatible(String),
  #[error("the target registry is invalid: {0}")]
  Invalid(String),
}

impl TargetRegistryError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::TooLarge | Self::Invalid(_) => "WOML_TARGET_INVALID",
      Self::Incompatible(_) => "WOML_TARGET_INCOMPATIBLE",
    }
  }
}

pub fn parse_target_registry(
  source: &[u8],
) -> Result<PreparedTargetRegistryV1, TargetRegistryError> {
  if source.len() > TARGET_REGISTRY_MAX_BYTES {
    return Err(TargetRegistryError::TooLarge);
  }
  let registry: TargetRegistryV1 = serde_json::from_slice(source).map_err(|_| {
    TargetRegistryError::Invalid(
      "the document has missing, duplicate, unknown, or malformed fields".to_string(),
    )
  })?;
  prepare_target_registry(registry)
}

pub fn prepare_target_registry(
  mut registry: TargetRegistryV1,
) -> Result<PreparedTargetRegistryV1, TargetRegistryError> {
  if registry.profile != TARGET_REGISTRY_PROFILE || registry.version != TARGET_REGISTRY_VERSION {
    return Err(TargetRegistryError::Incompatible(
      "the document is not Target Registry v1".to_string(),
    ));
  }
  if registry.targets.len() > TARGET_REGISTRY_MAX_TARGETS {
    return Err(invalid("more than 256 targets are configured"));
  }
  let mut target_ids = HashSet::new();
  for target in &mut registry.targets {
    validate_identifier(&target.id, false, "target id")?;
    if target.id == "local" {
      return Err(invalid("the built-in local target cannot be redefined"));
    }
    if !target_ids.insert(target.id.clone()) {
      return Err(invalid("a target id is duplicated"));
    }
    if target.kind != "worker-pool" {
      return Err(TargetRegistryError::Incompatible(
        "Target Registry v1 supports only worker-pool targets".to_string(),
      ));
    }
    normalize_set(
      &mut target.allowed_worker_identities,
      64,
      false,
      "allowed worker identities",
    )?;
    normalize_set(
      &mut target.required_capabilities,
      128,
      true,
      "required capabilities",
    )?;
    if !valid_semver(&target.minimum_worker_version) {
      return Err(invalid("minimumWorkerVersion is invalid"));
    }
    if !(1..=64).contains(&target.maximum_in_flight) {
      return Err(invalid("maximumInFlight must be from 1 through 64"));
    }
  }
  registry
    .targets
    .sort_by(|left, right| left.id.cmp(&right.id));
  let canonical_payload = canonical_json(&registry)
    .map_err(|_| invalid("the canonical registry payload could not be encoded"))?;
  let digest = format!("sha256:{:x}", Sha256::digest(&canonical_payload));
  Ok(PreparedTargetRegistryV1 {
    registry,
    canonical_payload,
    digest,
  })
}

fn normalize_set(
  values: &mut [String],
  maximum: usize,
  capability: bool,
  label: &str,
) -> Result<(), TargetRegistryError> {
  if values.is_empty() || values.len() > maximum {
    return Err(invalid(&format!("{label} exceed their bounds")));
  }
  for value in values.iter() {
    validate_identifier(value, capability, label)?;
  }
  values.sort();
  if values.windows(2).any(|pair| pair[0] == pair[1]) {
    return Err(invalid(&format!("{label} contain a duplicate")));
  }
  Ok(())
}

fn validate_identifier(
  value: &str,
  capability: bool,
  label: &str,
) -> Result<(), TargetRegistryError> {
  let bytes = value.as_bytes();
  let separators: &[u8] = if capability { b".-" } else { b"-" };
  let valid = !bytes.is_empty()
    && bytes.len() <= 128
    && bytes[0].is_ascii_lowercase()
    && bytes
      .iter()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || separators.contains(byte))
    && !separators.contains(bytes.last().expect("non-empty"))
    && !bytes
      .windows(2)
      .any(|pair| separators.contains(&pair[0]) && separators.contains(&pair[1]));
  if valid {
    Ok(())
  } else {
    Err(invalid(&format!("{label} contains an invalid identifier")))
  }
}

fn valid_semver(value: &str) -> bool {
  if value.is_empty() || value.len() > 64 || !value.is_ascii() {
    return false;
  }
  let core = value.split_once('-').map_or(value, |(core, suffix)| {
    if suffix.is_empty()
      || !suffix
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    {
      return "";
    }
    core
  });
  let parts: Vec<_> = core.split('.').collect();
  parts.len() == 3
    && parts
      .iter()
      .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn invalid(message: &str) -> TargetRegistryError {
  TargetRegistryError::Invalid(message.to_string())
}
