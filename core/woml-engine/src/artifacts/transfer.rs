//! Stable transfer receipts, quotas, staging, publication, and logical settlement. Byte publication and SQLite commit are not a distributed transaction.

use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::artifact_interrupted;
use crate::artifacts::backend_unavailable;
use crate::artifacts::canonical_digest;
use crate::artifacts::new_artifact_id;
use crate::artifacts::operation_identity_conflict;
use crate::artifacts::quota_exceeded;
use crate::artifacts::retention_invalid;
use crate::artifacts::timestamp;
use crate::artifacts::validate_schema;
use crate::artifacts::ArtifactLimits;
use crate::artifacts::ArtifactReferenceV1;
use crate::artifacts::ArtifactRequestV1;
use crate::artifacts::ArtifactStore;
use crate::artifacts::ARTIFACTS_CONTRACT;
use crate::artifacts::ARTIFACTS_CONTRACT_VERSION;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT_VERSION;
use crate::CapabilityCallRequest;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use chrono::SecondsFormat;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::json;

#[derive(Debug, Clone)]
pub(in crate::artifacts) struct StoredSettlement {
  pub(in crate::artifacts) status: String,
  pub(in crate::artifacts) input_digest: String,
  pub(in crate::artifacts) artifact: Option<ArtifactReferenceV1>,
}

pub(in crate::artifacts) fn read_settlement(
  connection: &Connection,
  operation_key: &str,
) -> Result<Option<StoredSettlement>, CapabilityFailure> {
  let row: Option<(String, String, Option<String>)> = connection
    .query_row(
      "SELECT status, input_digest, artifact_id FROM woml_artifact_transfers
       WHERE operation_key = ?1",
      [operation_key],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .map_err(|_| backend_unavailable())?;
  let Some((status, input_digest, artifact_id)) = row else {
    return Ok(None);
  };
  let artifact = artifact_id
    .map(|id| {
      connection
        .query_row(
          "SELECT artifact_id, name, size, sha256, content_type
           FROM woml_artifacts WHERE artifact_id = ?1 AND status = 'available'",
          [id],
          |row| {
            Ok(ArtifactReferenceV1 {
              contract: ARTIFACT_REFERENCE_CONTRACT.to_string(),
              contract_version: ARTIFACT_REFERENCE_CONTRACT_VERSION,
              id: row.get(0)?,
              name: row.get(1)?,
              size: row.get(2)?,
              sha256: row.get(3)?,
              content_type: row.get(4)?,
            })
          },
        )
        .map_err(|_| artifact_corrupt())
    })
    .transpose()?;
  Ok(Some(StoredSettlement {
    status,
    input_digest,
    artifact,
  }))
}

pub(in crate::artifacts) fn validate_settlement_identity(
  settlement: &StoredSettlement,
  input_digest: &str,
) -> Result<(), CapabilityFailure> {
  if settlement.input_digest != input_digest {
    return Err(operation_identity_conflict());
  }
  Ok(())
}

pub(in crate::artifacts) fn enforce_quotas(
  connection: &Connection,
  run_id: &str,
  digest: &str,
  size: u64,
  limits: &ArtifactLimits,
) -> Result<(), CapabilityFailure> {
  let run_bytes: u64 = connection
    .query_row(
      "SELECT COALESCE(SUM(a.size), 0)
       FROM woml_artifacts a
       JOIN woml_artifact_provenance p ON p.artifact_id = a.artifact_id
       WHERE p.run_id = ?1 AND a.status = 'available'",
      [run_id],
      |row| row.get(0),
    )
    .map_err(|_| backend_unavailable())?;
  if run_bytes.saturating_add(size) > limits.max_run_bytes {
    return Err(quota_exceeded());
  }
  let existing_size: Option<u64> = connection
    .query_row(
      "SELECT size FROM woml_artifact_blobs WHERE sha256 = ?1",
      [digest],
      |row| row.get(0),
    )
    .optional()
    .map_err(|_| backend_unavailable())?;
  if let Some(existing_size) = existing_size {
    if existing_size != size {
      return Err(artifact_corrupt());
    }
  } else {
    let store_bytes: u64 = connection
      .query_row(
        "SELECT COALESCE(SUM(size), 0) FROM woml_artifact_blobs",
        [],
        |row| row.get(0),
      )
      .map_err(|_| backend_unavailable())?;
    if store_bytes.saturating_add(size) > limits.max_store_bytes {
      return Err(quota_exceeded());
    }
  }
  Ok(())
}

impl ArtifactStore {
  pub(in crate::artifacts) fn put(
    &self,
    request: &CapabilityCallRequest,
    workflow_id: &str,
    parsed: &ArtifactRequestV1,
    cancellation: &CapabilityCancellationToken,
    known_secrets: &[Vec<u8>],
  ) -> Result<ArtifactReferenceV1, CapabilityFailure> {
    let input_digest = canonical_digest(b"woml.artifact-input\0v1\0", &request.input)?;
    let started_at = timestamp();
    let mut connection = self.connection()?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| backend_unavailable())?;
    validate_schema(&transaction)?;
    if let Some(settlement) = read_settlement(&transaction, &request.identity.operation_key)? {
      validate_settlement_identity(&settlement, &input_digest)?;
      transaction.commit().map_err(|_| backend_unavailable())?;
      return match settlement.status.as_str() {
        "succeeded" => {
          let artifact = settlement.artifact.ok_or_else(artifact_corrupt)?;
          let (_, digest, size) =
            self.read_source(&parsed.input.source_path, cancellation, known_secrets, None)?;
          if digest != artifact.sha256 || size != artifact.size {
            Err(operation_identity_conflict())
          } else {
            Ok(artifact)
          }
        }
        _ => Err(artifact_interrupted()),
      };
    }
    transaction
      .execute(
        "INSERT INTO woml_artifact_transfers(
           operation_key, run_id, node_id, attempt_number, operation, input_digest,
           status, started_at
         ) VALUES (?1, ?2, ?3, ?4, 'put', ?5, 'started', ?6)",
        params![
          request.identity.operation_key,
          request.run_id,
          request.node_id,
          request.attempt_number,
          input_digest,
          started_at,
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction.commit().map_err(|_| backend_unavailable())?;

    let outcome = self.put_started(request, workflow_id, parsed, cancellation, known_secrets);
    if let Err(failure) = &outcome {
      self.settle_failure(&request.identity.operation_key, failure)?;
    }
    outcome
  }

  pub(in crate::artifacts) fn put_started(
    &self,
    request: &CapabilityCallRequest,
    workflow_id: &str,
    parsed: &ArtifactRequestV1,
    cancellation: &CapabilityCancellationToken,
    known_secrets: &[Vec<u8>],
  ) -> Result<ArtifactReferenceV1, CapabilityFailure> {
    let mut stage = self.backend.begin_stage()?;
    let (source_identity, sha256, size) = self.read_source(
      &parsed.input.source_path,
      cancellation,
      known_secrets,
      stage.file.as_mut(),
    )?;
    let _ = source_identity;
    if cancellation.is_cancelled() {
      return Err(artifact_cancelled());
    }
    let artifact = ArtifactReferenceV1 {
      contract: ARTIFACT_REFERENCE_CONTRACT.to_string(),
      contract_version: ARTIFACT_REFERENCE_CONTRACT_VERSION,
      id: new_artifact_id()?,
      name: parsed.input.name.clone(),
      size,
      sha256: sha256.clone(),
      content_type: parsed.input.content_type.clone(),
    };
    let created_at = timestamp();
    let expires_at = Utc::now()
      .checked_add_signed(chrono::Duration::milliseconds(
        parsed.input.retention_ms as i64,
      ))
      .ok_or_else(retention_invalid)?
      .to_rfc3339_opts(SecondsFormat::Millis, true);
    let result = json!({
      "contract": ARTIFACTS_CONTRACT,
      "contractVersion": ARTIFACTS_CONTRACT_VERSION,
      "kind": "result",
      "operation": "put",
      "data": serde_json::to_value(&artifact).map_err(|_| artifact_corrupt())?
    });
    let result_digest = canonical_digest(b"woml.artifact-result\0v1\0", &result)?;

    let mut connection = self.connection()?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| backend_unavailable())?;
    validate_schema(&transaction)?;
    enforce_quotas(
      &transaction,
      &request.run_id,
      &sha256,
      size,
      &self.policy.limits,
    )?;
    self.backend.publish(stage, &sha256, size, cancellation)?;
    transaction
      .execute(
        "INSERT OR IGNORE INTO woml_artifact_blobs(sha256, size, created_at)
         VALUES (?1, ?2, ?3)",
        params![sha256, size, created_at],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "INSERT INTO woml_artifacts(
           artifact_id, name, size, sha256, content_type, status, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'available', ?6)",
        params![
          artifact.id,
          artifact.name,
          artifact.size,
          artifact.sha256,
          artifact.content_type,
          created_at,
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "INSERT INTO woml_artifact_provenance(
           artifact_id, workflow_id, run_id, node_id, attempt_number, operation_key
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
          artifact.id,
          workflow_id,
          request.run_id,
          request.node_id,
          request.attempt_number,
          request.identity.operation_key,
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "INSERT INTO woml_artifact_access(artifact_id, run_id, granted_at)
         VALUES (?1, ?2, ?3)",
        params![artifact.id, request.run_id, created_at],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "INSERT INTO woml_artifact_retention(
           artifact_id, requested_ms, effective_ms, expires_at
         ) VALUES (?1, ?2, ?2, ?3)",
        params![artifact.id, parsed.input.retention_ms, expires_at],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "UPDATE woml_artifact_transfers
         SET status = 'succeeded', artifact_id = ?2, content_digest = ?3,
             size = ?4, result_digest = ?5, settled_at = ?6
         WHERE operation_key = ?1 AND status = 'started'",
        params![
          request.identity.operation_key,
          artifact.id,
          format!("sha256:{sha256}"),
          size,
          result_digest,
          timestamp(),
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction.commit().map_err(|_| backend_unavailable())?;
    Ok(artifact)
  }

  pub(in crate::artifacts) fn settle_failure(
    &self,
    operation_key: &str,
    failure: &CapabilityFailure,
  ) -> Result<(), CapabilityFailure> {
    let status = if failure.kind == CapabilityFailureKind::Cancelled {
      "cancelled"
    } else if failure.code == "WOML_ARTIFACT_INTERRUPTED" || failure.ambiguous {
      "interrupted"
    } else {
      "failed"
    };
    let connection = self.connection()?;
    connection
      .execute(
        "UPDATE woml_artifact_transfers
         SET status = ?2, error_code = ?3, ambiguous = ?4, settled_at = ?5
         WHERE operation_key = ?1 AND status = 'started'",
        params![
          operation_key,
          status,
          failure.code,
          i64::from(failure.ambiguous),
          timestamp(),
        ],
      )
      .map_err(|_| backend_unavailable())?;
    Ok(())
  }
}
