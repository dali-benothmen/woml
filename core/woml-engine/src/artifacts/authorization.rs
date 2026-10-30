//! Same-run grants, status/expiry, full reference equality, and authorized head verification before backend content can be used.

use crate::artifacts::artifact_access_denied;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::artifact_expired;
use crate::artifacts::artifact_not_found;
use crate::artifacts::artifact_quarantined;
use crate::artifacts::backend_unavailable;
use crate::artifacts::validate_content_type;
use crate::artifacts::validate_name;
use crate::artifacts::validate_schema;
use crate::artifacts::ArtifactReferenceRequestV1;
use crate::artifacts::ArtifactReferenceV1;
use crate::artifacts::ArtifactStore;
use crate::artifacts::ARTIFACTS_CONTRACT;
use crate::artifacts::ARTIFACTS_CONTRACT_VERSION;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT_VERSION;
use crate::artifacts::CROCKFORD;
use crate::artifacts::MAX_ARTIFACT_BYTES;
use crate::CapabilityCallRequest;
use crate::CapabilityFailure;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use serde_json::json;
use serde_json::Value;

#[derive(Debug, Clone)]
pub(in crate::artifacts) struct AuthorizedArtifact {
  pub(in crate::artifacts) reference: ArtifactReferenceV1,
  pub(in crate::artifacts) created_at: String,
  pub(in crate::artifacts) expires_at: String,
}

pub(in crate::artifacts) type StoredArtifactRow =
  (String, String, u64, String, String, String, String, String);

pub(in crate::artifacts) fn validate_reference(
  reference: &ArtifactReferenceV1,
) -> Result<(), CapabilityFailure> {
  let encoded_id = reference.id.strip_prefix("artifact_");
  if reference.contract != ARTIFACT_REFERENCE_CONTRACT
    || reference.contract_version != ARTIFACT_REFERENCE_CONTRACT_VERSION
    || encoded_id
      .is_none_or(|id| id.len() != 26 || !id.bytes().all(|byte| CROCKFORD.contains(&byte)))
    || reference.sha256.len() != 64
    || !reference
      .sha256
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    || reference.size > MAX_ARTIFACT_BYTES
  {
    return Err(artifact_access_denied());
  }
  validate_name(&reference.name).map_err(|_| artifact_access_denied())?;
  validate_content_type(&reference.content_type).map_err(|_| artifact_access_denied())
}

impl ArtifactStore {
  pub(in crate::artifacts) fn head(
    &self,
    request: &CapabilityCallRequest,
    parsed: &ArtifactReferenceRequestV1,
  ) -> Result<Value, CapabilityFailure> {
    let artifact = self.authorize_reference(&request.run_id, &parsed.input.artifact)?;
    self
      .backend
      .verify(&artifact.reference.sha256, artifact.reference.size)?;
    Ok(json!({
      "contract": ARTIFACTS_CONTRACT,
      "contractVersion": ARTIFACTS_CONTRACT_VERSION,
      "kind": "result",
      "operation": "head",
      "data": {
        "artifact": artifact.reference,
        "createdAt": artifact.created_at,
        "expiresAt": artifact.expires_at,
        "integrity": "verified"
      }
    }))
  }

  pub(in crate::artifacts) fn authorize_reference(
    &self,
    run_id: &str,
    supplied: &ArtifactReferenceV1,
  ) -> Result<AuthorizedArtifact, CapabilityFailure> {
    validate_reference(supplied)?;
    let connection = self.connection()?;
    validate_schema(&connection)?;
    let stored: Option<StoredArtifactRow> = connection
      .query_row(
        "SELECT a.artifact_id, a.name, a.size, a.sha256, a.content_type,
                a.status, a.created_at, r.expires_at
         FROM woml_artifacts a
         JOIN woml_artifact_retention r ON r.artifact_id = a.artifact_id
         WHERE a.artifact_id = ?1",
        [&supplied.id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
          ))
        },
      )
      .optional()
      .map_err(|_| backend_unavailable())?;
    let Some((id, name, size, sha256, content_type, status, created_at, expires_at)) = stored
    else {
      return Err(artifact_not_found());
    };
    let authorized: bool = connection
      .query_row(
        "SELECT EXISTS(
           SELECT 1 FROM woml_artifact_access
           WHERE artifact_id = ?1 AND run_id = ?2 AND revoked_at IS NULL
         )",
        params![id, run_id],
        |row| row.get(0),
      )
      .map_err(|_| backend_unavailable())?;
    if !authorized {
      return Err(artifact_access_denied());
    }
    match status.as_str() {
      "available" => {}
      "expired" => return Err(artifact_expired()),
      "quarantined" => return Err(artifact_quarantined()),
      "corrupt" => return Err(artifact_corrupt()),
      "deleted" => return Err(artifact_not_found()),
      _ => return Err(artifact_corrupt()),
    }
    let expires =
      chrono::DateTime::parse_from_rfc3339(&expires_at).map_err(|_| artifact_corrupt())?;
    if expires <= Utc::now() {
      return Err(artifact_expired());
    }
    let reference = ArtifactReferenceV1 {
      contract: ARTIFACT_REFERENCE_CONTRACT.to_string(),
      contract_version: ARTIFACT_REFERENCE_CONTRACT_VERSION,
      id,
      name,
      size,
      sha256,
      content_type,
    };
    if &reference != supplied {
      return Err(artifact_access_denied());
    }
    Ok(AuthorizedArtifact {
      reference,
      created_at,
      expires_at,
    })
  }
}
