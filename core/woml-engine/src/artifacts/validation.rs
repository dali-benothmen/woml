//! Strict artifact put/reference requests, names/content types, bounded identifiers, and exact-byte secret matching.

use crate::artifacts::artifact_access_denied;
use crate::artifacts::artifact_failure;
use crate::artifacts::artifact_path_invalid;
use crate::artifacts::retention_invalid;
use crate::artifacts::validate_authored_path;
use crate::artifacts::validate_reference;
use crate::artifacts::ArtifactReferenceRequestV1;
use crate::artifacts::ArtifactRequestV1;
use crate::artifacts::PreparedArtifactPolicy;
use crate::artifacts::ARTIFACTS_CONTRACT;
use crate::artifacts::ARTIFACTS_CONTRACT_VERSION;
use crate::artifacts::CROCKFORD;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use serde_json::Value;

pub(in crate::artifacts) fn validate_artifact_id(
  artifact_id: &str,
) -> Result<(), CapabilityFailure> {
  let encoded = artifact_id.strip_prefix("artifact_").unwrap_or_default();
  if encoded.len() != 26 || !encoded.bytes().all(|byte| CROCKFORD.contains(&byte)) {
    return Err(artifact_access_denied());
  }
  Ok(())
}

pub(in crate::artifacts) fn parse_put(
  value: &Value,
  policy: &PreparedArtifactPolicy,
) -> Result<ArtifactRequestV1, CapabilityFailure> {
  let request: ArtifactRequestV1 =
    serde_json::from_value(value.clone()).map_err(|_| artifact_path_invalid())?;
  if request.contract != ARTIFACTS_CONTRACT
    || request.contract_version != ARTIFACTS_CONTRACT_VERSION
    || request.kind != "request"
    || request.operation != "put"
  {
    return Err(artifact_path_invalid());
  }
  validate_authored_path(&request.input.source_path)?;
  validate_name(&request.input.name)?;
  validate_content_type(&request.input.content_type)?;
  if request.input.retention_ms < policy.retention.minimum_ms
    || request.input.retention_ms > policy.retention.maximum_ms
  {
    return Err(retention_invalid());
  }
  Ok(request)
}

pub(in crate::artifacts) fn parse_reference_request(
  value: &Value,
  expected_operation: &str,
) -> Result<ArtifactReferenceRequestV1, CapabilityFailure> {
  let request: ArtifactReferenceRequestV1 =
    serde_json::from_value(value.clone()).map_err(|_| artifact_access_denied())?;
  if request.contract != ARTIFACTS_CONTRACT
    || request.contract_version != ARTIFACTS_CONTRACT_VERSION
    || request.kind != "request"
    || request.operation != expected_operation
  {
    return Err(artifact_access_denied());
  }
  validate_reference(&request.input.artifact)?;
  Ok(request)
}

pub(in crate::artifacts) fn validate_name(value: &str) -> Result<(), CapabilityFailure> {
  if value.is_empty() || value.len() > 256 || value.chars().any(|character| character.is_control())
  {
    return Err(artifact_failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_ARTIFACT_PATH_INVALID",
      "The artifact name is invalid.",
      false,
      false,
    ));
  }
  Ok(())
}

pub(in crate::artifacts) fn validate_content_type(value: &str) -> Result<(), CapabilityFailure> {
  let mut parts = value.split('/');
  let valid_part = |part: &str| {
    !part.is_empty()
      && part.len() <= 127
      && part.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
          || matches!(
            byte,
            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
          )
      })
  };
  if value.len() > 255
    || !parts.next().is_some_and(valid_part)
    || !parts.next().is_some_and(valid_part)
    || parts.next().is_some()
  {
    return Err(artifact_failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_ARTIFACT_PATH_INVALID",
      "The artifact content type is invalid.",
      false,
      false,
    ));
  }
  Ok(())
}

pub(in crate::artifacts) fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
  !needle.is_empty()
    && haystack
      .windows(needle.len())
      .any(|window| window == needle)
}
