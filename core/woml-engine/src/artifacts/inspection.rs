//! Bounded operator inspection, explicit verified export, observation, and destination-free durable audit records.

use crate::artifacts::artifact_access_denied;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::artifact_failure;
use crate::artifacts::artifact_not_found;
use crate::artifacts::backend_unavailable;
use crate::artifacts::harden_file_permissions;
use crate::artifacts::initialize_schema;
use crate::artifacts::materialization_unavailable;
use crate::artifacts::stream_materialization;
use crate::artifacts::timestamp;
use crate::artifacts::validate_artifact_id;
use crate::artifacts::validate_schema;
use crate::artifacts::ArtifactExportV1;
use crate::artifacts::ArtifactInspectionV1;
use crate::artifacts::ArtifactObservationV1;
use crate::artifacts::ArtifactPolicyV1;
use crate::artifacts::ArtifactProvenanceV1;
use crate::artifacts::ArtifactReferenceV1;
use crate::artifacts::ArtifactRunInspectionV1;
use crate::artifacts::ArtifactRuntimeMode;
use crate::artifacts::ArtifactStore;
use crate::artifacts::PreparedArtifactBackend;
use crate::artifacts::PreparedArtifactPolicy;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT_VERSION;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::DurableEventStore;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

const OPERATOR_ARTIFACT_INSPECTION_LIMIT: usize = 100;

pub fn inspect_artifact(
  state_path: impl AsRef<Path>,
  artifact_id: &str,
) -> Result<ArtifactInspectionV1, CapabilityFailure> {
  validate_artifact_id(artifact_id)?;
  DurableEventStore::open(state_path.as_ref()).map_err(|_| backend_unavailable())?;
  let connection = Connection::open_with_flags(
    state_path.as_ref(),
    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
  )
  .map_err(|_| backend_unavailable())?;
  initialize_schema(&connection)?;
  validate_schema(&connection)?;
  let inspection =
    artifact_inspection_row(&connection, artifact_id)?.ok_or_else(artifact_not_found)?;
  record_operator_audit(&connection, "inspect", artifact_id)?;
  Ok(inspection)
}

pub fn inspect_run_artifacts(
  state_path: impl AsRef<Path>,
  run_id: &str,
) -> Result<ArtifactRunInspectionV1, CapabilityFailure> {
  if run_id.is_empty() || run_id.len() > 320 {
    return Err(artifact_access_denied());
  }
  DurableEventStore::open(state_path.as_ref()).map_err(|_| backend_unavailable())?;
  let connection = Connection::open_with_flags(
    state_path.as_ref(),
    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
  )
  .map_err(|_| backend_unavailable())?;
  initialize_schema(&connection)?;
  validate_schema(&connection)?;
  let mut statement = connection
    .prepare(
      "SELECT artifact_id FROM woml_artifact_provenance
       WHERE run_id = ?1 ORDER BY artifact_id LIMIT ?2",
    )
    .map_err(|_| backend_unavailable())?;
  let artifact_ids = statement
    .query_map(
      params![run_id, (OPERATOR_ARTIFACT_INSPECTION_LIMIT + 1) as u64],
      |row| row.get::<_, String>(0),
    )
    .map_err(|_| backend_unavailable())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| backend_unavailable())?;
  let truncated = artifact_ids.len() > OPERATOR_ARTIFACT_INSPECTION_LIMIT;
  let artifacts = artifact_ids
    .into_iter()
    .take(OPERATOR_ARTIFACT_INSPECTION_LIMIT)
    .map(|artifact_id| {
      artifact_inspection_row(&connection, &artifact_id)?.ok_or_else(artifact_corrupt)
    })
    .collect::<Result<Vec<_>, _>>()?;
  record_operator_audit(&connection, "inspect_run", run_id)?;
  Ok(ArtifactRunInspectionV1 {
    profile: "woml.artifact-run-inspection/v1".to_string(),
    run_id: run_id.to_string(),
    artifacts,
    truncated,
  })
}

pub fn export_artifact(
  state_path: impl AsRef<Path>,
  destination: impl AsRef<Path>,
  artifact_id: &str,
  policy: ArtifactPolicyV1,
  mode: ArtifactRuntimeMode,
  resolved_secrets: &BTreeMap<String, String>,
) -> Result<ArtifactExportV1, CapabilityFailure> {
  validate_artifact_id(artifact_id)?;
  let policy = Arc::new(
    PreparedArtifactPolicy::prepare(policy, mode, resolved_secrets).map_err(|error| {
      artifact_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_ARTIFACT_POLICY_INVALID",
        &error.to_string(),
        false,
        false,
      )
    })?,
  );
  let store = ArtifactStore::open_with_startup_cleanup(state_path.as_ref(), policy, false)?;
  let connection = store.connection()?;
  let inspection =
    artifact_inspection_row(&connection, artifact_id)?.ok_or_else(artifact_not_found)?;
  if inspection.status != "available" {
    return Err(artifact_access_denied());
  }
  let verified = store
    .backend
    .verify(&inspection.artifact.sha256, inspection.artifact.size)?;
  let destination = destination.as_ref();
  let parent = destination
    .parent()
    .ok_or_else(materialization_unavailable)?;
  let parent = fs::canonicalize(parent).map_err(|_| materialization_unavailable())?;
  let file_name = destination
    .file_name()
    .and_then(|value| value.to_str())
    .filter(|value| !value.is_empty() && *value != "." && *value != "..")
    .ok_or_else(materialization_unavailable)?;
  let final_path = parent.join(file_name);
  if fs::symlink_metadata(&final_path).is_ok() {
    return Err(artifact_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_ARTIFACT_EXPORT_EXISTS",
      "The export destination already exists; choose a new path.",
      false,
      false,
    ));
  }
  stream_materialization(
    &verified.path,
    &final_path,
    &inspection.artifact.sha256,
    inspection.artifact.size,
    &CapabilityCancellationToken::default(),
  )?;
  harden_file_permissions(&final_path)?;
  let audit_id = record_operator_audit(&connection, "export", artifact_id)?;
  Ok(ArtifactExportV1 {
    profile: "woml.artifact-export/v1".to_string(),
    audit_id,
    artifact_id: inspection.artifact.id,
    name: inspection.artifact.name,
    size: inspection.artifact.size,
    digest_prefix: inspection.digest_prefix,
    exported_at: timestamp(),
    verified: true,
  })
}

pub fn observe_artifacts(
  state_path: impl AsRef<Path>,
  policy: ArtifactPolicyV1,
  mode: ArtifactRuntimeMode,
  resolved_secrets: &BTreeMap<String, String>,
) -> Result<ArtifactObservationV1, CapabilityFailure> {
  let policy = Arc::new(
    PreparedArtifactPolicy::prepare(policy, mode, resolved_secrets).map_err(|error| {
      artifact_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_ARTIFACT_POLICY_INVALID",
        &error.to_string(),
        false,
        false,
      )
    })?,
  );
  let backend_name = match &policy.backend {
    PreparedArtifactBackend::Local { .. } => "local",
    PreparedArtifactBackend::S3(_) => "s3",
  };
  let store = ArtifactStore::open_with_startup_cleanup(state_path.as_ref(), policy, false)?;
  let connection = store.connection()?;
  let (transfers, transferred_bytes, transfer_duration_ms, failed_transfers): (u64, u64, u64, u64) = connection
    .query_row(
      "SELECT COUNT(*),
              COALESCE(SUM(CASE WHEN status = 'succeeded' THEN size ELSE 0 END), 0),
              COALESCE(CAST(SUM(CASE WHEN settled_at IS NOT NULL
                THEN (julianday(settled_at) - julianday(started_at)) * 86400000
                ELSE 0 END) AS INTEGER), 0),
              COALESCE(SUM(CASE WHEN status IN ('failed','cancelled','interrupted') THEN 1 ELSE 0 END), 0)
       FROM woml_artifact_transfers",
      [],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .map_err(|_| backend_unavailable())?;
  let (corrupt_artifacts, live_artifacts, live_bytes): (u64, u64, u64) = connection
    .query_row(
      "SELECT
         COALESCE(SUM(CASE WHEN status = 'corrupt' THEN 1 ELSE 0 END), 0),
         COALESCE(SUM(CASE WHEN status = 'available' THEN 1 ELSE 0 END), 0),
         COALESCE(SUM(CASE WHEN status = 'available' THEN size ELSE 0 END), 0)
       FROM woml_artifacts",
      [],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .map_err(|_| backend_unavailable())?;
  let orphan_blobs = connection
    .query_row(
      "SELECT COUNT(*) FROM woml_artifact_blobs AS blobs
       WHERE NOT EXISTS (SELECT 1 FROM woml_artifacts AS artifacts WHERE artifacts.sha256 = blobs.sha256)",
      [],
      |row| row.get::<_, u64>(0),
    )
    .map_err(|_| backend_unavailable())?;
  let backend_healthy = store.backend.inventory(1).is_ok();
  Ok(ArtifactObservationV1 {
    profile: "woml.artifact-observation/v1".to_string(),
    backend: backend_name.to_string(),
    backend_healthy,
    transfers,
    transferred_bytes,
    transfer_duration_ms,
    failed_transfers,
    corrupt_artifacts,
    orphan_blobs,
    live_artifacts,
    live_bytes,
  })
}

fn artifact_inspection_row(
  connection: &Connection,
  artifact_id: &str,
) -> Result<Option<ArtifactInspectionV1>, CapabilityFailure> {
  connection
    .query_row(
      "SELECT a.artifact_id, a.name, a.size, a.sha256, a.content_type,
              a.status, a.created_at, retention.expires_at, retention.effective_ms,
              provenance.workflow_id, provenance.run_id, provenance.node_id,
              provenance.attempt_number
       FROM woml_artifacts AS a
       JOIN woml_artifact_retention AS retention ON retention.artifact_id = a.artifact_id
       JOIN woml_artifact_provenance AS provenance ON provenance.artifact_id = a.artifact_id
       WHERE a.artifact_id = ?1",
      [artifact_id],
      |row| {
        let digest: String = row.get(3)?;
        Ok(ArtifactInspectionV1 {
          profile: "woml.artifact-inspection/v1".to_string(),
          artifact: ArtifactReferenceV1 {
            contract: ARTIFACT_REFERENCE_CONTRACT.to_string(),
            contract_version: ARTIFACT_REFERENCE_CONTRACT_VERSION,
            id: row.get(0)?,
            name: row.get(1)?,
            size: row.get(2)?,
            sha256: digest.clone(),
            content_type: row.get(4)?,
          },
          digest_prefix: digest.chars().take(12).collect(),
          status: row.get(5)?,
          created_at: row.get(6)?,
          expires_at: row.get(7)?,
          retention_ms: row.get(8)?,
          provenance: ArtifactProvenanceV1 {
            workflow_id: row.get(9)?,
            run_id: row.get(10)?,
            node_id: row.get(11)?,
            attempt: row.get(12)?,
          },
        })
      },
    )
    .optional()
    .map_err(|_| backend_unavailable())
}

fn record_operator_audit(
  connection: &Connection,
  operation: &str,
  subject_id: &str,
) -> Result<String, CapabilityFailure> {
  let audit_id = format!("audit_{}", uuid::Uuid::new_v4().simple());
  connection
    .execute(
      "INSERT INTO woml_artifact_operator_audit(audit_id, operation, subject_id, occurred_at)
       VALUES (?1, ?2, ?3, ?4)",
      params![audit_id, operation, subject_id, timestamp()],
    )
    .map_err(|_| backend_unavailable())?;
  Ok(audit_id)
}
