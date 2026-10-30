//! Explicit bounded operational-output artifact rollover with redacted-byte staging, durable ordering, and final publication.

use crate::artifacts::artifact_interrupted;
use crate::artifacts::artifact_quarantined;
use crate::artifacts::backend_unavailable;
use crate::artifacts::canonical_digest;
use crate::artifacts::contains_bytes;
use crate::artifacts::enforce_quotas;
use crate::artifacts::new_artifact_id;
use crate::artifacts::quota_exceeded;
use crate::artifacts::read_settlement;
use crate::artifacts::retention_invalid;
use crate::artifacts::timestamp;
use crate::artifacts::validate_schema;
use crate::artifacts::validate_settlement_identity;
use crate::artifacts::ArtifactReferenceV1;
use crate::artifacts::ArtifactStage;
use crate::artifacts::ArtifactStore;
use crate::artifacts::ManagedArtifactPutHandler;
use crate::artifacts::ARTIFACTS_CONTRACT;
use crate::artifacts::ARTIFACTS_CONTRACT_VERSION;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT_VERSION;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use chrono::SecondsFormat;
use chrono::Utc;
use rusqlite::params;
use rusqlite::TransactionBehavior;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use std::io::Write;
use std::sync::Arc;

pub(crate) struct OperationalArtifactWriter {
  pub(in crate::artifacts) store: Arc<ArtifactStore>,
  pub(in crate::artifacts) stage: Option<ArtifactStage>,
  pub(in crate::artifacts) digest: Sha256,
  pub(in crate::artifacts) size: u64,
  pub(in crate::artifacts) max_bytes: u64,
  pub(in crate::artifacts) retention_ms: u64,
  pub(in crate::artifacts) workflow_id: String,
  pub(in crate::artifacts) run_id: String,
  pub(in crate::artifacts) node_id: String,
  pub(in crate::artifacts) attempt: u32,
  pub(in crate::artifacts) invocation_id: String,
  pub(in crate::artifacts) known_secrets: Arc<Vec<Vec<u8>>>,
  pub(in crate::artifacts) unavailable: bool,
}

impl std::fmt::Debug for OperationalArtifactWriter {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("OperationalArtifactWriter")
      .field("run_id", &self.run_id)
      .field("node_id", &self.node_id)
      .field("attempt", &self.attempt)
      .field("size", &self.size)
      .field("unavailable", &self.unavailable)
      .finish_non_exhaustive()
  }
}

impl OperationalArtifactWriter {
  pub(crate) fn append_redacted(
    &mut self,
    source: &str,
    text: &str,
  ) -> Result<(), CapabilityFailure> {
    if self.unavailable {
      return Ok(());
    }
    let mut encoded = serde_json::to_vec(&json!({
      "source": source,
      "text": text,
    }))
    .map_err(|_| backend_unavailable())?;
    encoded.push(b'\n');
    if self.size.saturating_add(encoded.len() as u64) > self.max_bytes {
      self.unavailable = true;
      self.stage.take();
      return Err(quota_exceeded());
    }
    if self
      .known_secrets
      .iter()
      .any(|secret| contains_bytes(&encoded, secret))
    {
      self.unavailable = true;
      self.stage.take();
      return Err(artifact_quarantined());
    }
    let stage = self.stage.as_mut().ok_or_else(backend_unavailable)?;
    stage
      .file
      .as_mut()
      .ok_or_else(backend_unavailable)?
      .write_all(&encoded)
      .map_err(|_| backend_unavailable())?;
    self.digest.update(&encoded);
    self.size = self.size.saturating_add(encoded.len() as u64);
    Ok(())
  }

  pub(crate) fn finish(
    mut self,
    publish: bool,
  ) -> Result<Option<ArtifactReferenceV1>, CapabilityFailure> {
    if !publish || self.unavailable || self.size == 0 {
      return Ok(None);
    }
    let stage = self.stage.take().ok_or_else(backend_unavailable)?;
    let sha256 = hex::encode(self.digest.finalize());
    let operation_key = format!("operational-output:{}", self.invocation_id);
    let input_digest = format!("sha256:{sha256}");
    let started_at = timestamp();
    let mut connection = self.store.connection()?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| backend_unavailable())?;
    validate_schema(&transaction)?;
    if let Some(settlement) = read_settlement(&transaction, &operation_key)? {
      validate_settlement_identity(&settlement, &input_digest)?;
      transaction.commit().map_err(|_| backend_unavailable())?;
      return match settlement.status.as_str() {
        "succeeded" => Ok(settlement.artifact),
        _ => Err(artifact_interrupted()),
      };
    }
    enforce_quotas(
      &transaction,
      &self.run_id,
      &sha256,
      self.size,
      &self.store.policy.limits,
    )?;
    transaction
      .execute(
        "INSERT INTO woml_artifact_transfers(
           operation_key, run_id, node_id, attempt_number, operation, input_digest,
           status, started_at
         ) VALUES (?1, ?2, ?3, ?4, 'put', ?5, 'started', ?6)",
        params![
          operation_key,
          self.run_id,
          self.node_id,
          self.attempt,
          input_digest,
          started_at,
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction.commit().map_err(|_| backend_unavailable())?;

    let artifact = ArtifactReferenceV1 {
      contract: ARTIFACT_REFERENCE_CONTRACT.to_string(),
      contract_version: ARTIFACT_REFERENCE_CONTRACT_VERSION,
      id: new_artifact_id()?,
      name: format!(
        "{}-attempt-{}-operational-output.ndjson",
        self.node_id, self.attempt
      ),
      size: self.size,
      sha256: sha256.clone(),
      content_type: "application/x-ndjson".to_string(),
    };
    let created_at = timestamp();
    let expires_at = Utc::now()
      .checked_add_signed(chrono::Duration::milliseconds(self.retention_ms as i64))
      .ok_or_else(retention_invalid)?
      .to_rfc3339_opts(SecondsFormat::Millis, true);
    let result = json!({
      "contract": ARTIFACTS_CONTRACT,
      "contractVersion": ARTIFACTS_CONTRACT_VERSION,
      "kind": "result",
      "operation": "put",
      "data": &artifact,
    });
    let result_digest = canonical_digest(b"woml.artifact-result\0v1\0", &result)?;
    let cancellation = CapabilityCancellationToken::default();
    let mut connection = self.store.connection()?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| backend_unavailable())?;
    validate_schema(&transaction)?;
    enforce_quotas(
      &transaction,
      &self.run_id,
      &sha256,
      self.size,
      &self.store.policy.limits,
    )?;
    self
      .store
      .backend
      .publish(stage, &sha256, self.size, &cancellation)?;
    transaction
      .execute(
        "INSERT OR IGNORE INTO woml_artifact_blobs(sha256, size, created_at)
         VALUES (?1, ?2, ?3)",
        params![sha256, self.size, created_at],
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
          self.workflow_id,
          self.run_id,
          self.node_id,
          self.attempt,
          operation_key,
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "INSERT INTO woml_artifact_access(artifact_id, run_id, granted_at)
         VALUES (?1, ?2, ?3)",
        params![artifact.id, self.run_id, created_at],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "INSERT INTO woml_artifact_retention(
           artifact_id, requested_ms, effective_ms, expires_at
         ) VALUES (?1, ?2, ?2, ?3)",
        params![artifact.id, self.retention_ms, expires_at],
      )
      .map_err(|_| backend_unavailable())?;
    transaction
      .execute(
        "UPDATE woml_artifact_transfers
         SET status = 'succeeded', artifact_id = ?2, content_digest = ?3,
             size = ?4, result_digest = ?5, settled_at = ?6
         WHERE operation_key = ?1 AND status = 'started'",
        params![
          operation_key,
          artifact.id,
          format!("sha256:{sha256}"),
          self.size,
          result_digest,
          timestamp(),
        ],
      )
      .map_err(|_| backend_unavailable())?;
    transaction.commit().map_err(|_| backend_unavailable())?;
    Ok(Some(artifact))
  }
}

impl ManagedArtifactPutHandler {
  pub(crate) fn begin_operational_output(
    &self,
    workflow_id: &str,
    run_id: &str,
    node_id: &str,
    attempt: u32,
    invocation_id: &str,
  ) -> Result<Option<OperationalArtifactWriter>, CapabilityFailure> {
    let Some(policy) = self.policy.operational_output.as_ref() else {
      return Ok(None);
    };
    let store = self.store.configured()?;
    let stage = store.backend.begin_stage()?;
    Ok(Some(OperationalArtifactWriter {
      store,
      stage: Some(stage),
      digest: Sha256::new(),
      size: 0,
      max_bytes: self.policy.limits.max_artifact_bytes,
      retention_ms: policy.retention_ms,
      workflow_id: workflow_id.to_string(),
      run_id: run_id.to_string(),
      node_id: node_id.to_string(),
      attempt,
      invocation_id: invocation_id.to_string(),
      known_secrets: Arc::clone(&self.known_secrets),
      unavailable: false,
    }))
  }
}
