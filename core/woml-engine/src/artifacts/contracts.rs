//! Frozen artifact references, requests, inspection/backup records, and public constants. A reference is not a backend URL or an access grant.

use serde::Deserialize;
use serde::Serialize;

pub const ARTIFACTS_CONTRACT: &str = "woml.artifacts";

pub const ARTIFACTS_CONTRACT_VERSION: u32 = 1;

pub const ARTIFACT_POLICY_CONTRACT: &str = "woml.artifact-policy";

pub const ARTIFACT_POLICY_CONTRACT_VERSION: u32 = 1;

pub(in crate::artifacts) const ARTIFACT_REFERENCE_CONTRACT: &str = "woml.artifact-reference";

pub(in crate::artifacts) const ARTIFACT_REFERENCE_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactReferenceV1 {
  pub contract: String,
  pub contract_version: u32,
  pub id: String,
  pub name: String,
  pub size: u64,
  pub sha256: String,
  pub content_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactMaintenanceV1 {
  pub profile: String,
  pub kind: String,
  pub completed_at: String,
  pub eligible_artifacts: u64,
  pub eligible_blobs: u64,
  pub estimated_bytes: u64,
  pub deleted_artifacts: u64,
  pub deleted_blobs: u64,
  pub deleted_bytes: u64,
  pub scrubbed_blobs: u64,
  pub corrupt_blobs: u64,
  pub collected_orphans: u64,
  pub live_artifacts: u64,
  pub live_blobs: u64,
  pub live_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactBackupInventoryV1 {
  pub profile: String,
  pub directory: String,
  pub blob_count: u64,
  pub total_bytes: u64,
  pub inventory_digest: String,
  pub verified: bool,
}

/// Body-free metadata returned to an authenticated local operator. This is
/// deliberately richer than Artifact Reference v1 but never contains a
/// backend location, materialized path, signed URL, credential, or body data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactInspectionV1 {
  pub profile: String,
  pub artifact: ArtifactReferenceV1,
  pub digest_prefix: String,
  pub status: String,
  pub created_at: String,
  pub expires_at: String,
  pub retention_ms: u64,
  pub provenance: ArtifactProvenanceV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactProvenanceV1 {
  pub workflow_id: String,
  pub run_id: String,
  pub node_id: String,
  pub attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRunInspectionV1 {
  pub profile: String,
  pub run_id: String,
  pub artifacts: Vec<ArtifactInspectionV1>,
  pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactExportV1 {
  pub profile: String,
  pub audit_id: String,
  pub artifact_id: String,
  pub name: String,
  pub size: u64,
  pub digest_prefix: String,
  pub exported_at: String,
  pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactObservationV1 {
  pub profile: String,
  pub backend: String,
  pub backend_healthy: bool,
  pub transfers: u64,
  pub transferred_bytes: u64,
  pub transfer_duration_ms: u64,
  pub failed_transfers: u64,
  pub corrupt_artifacts: u64,
  pub orphan_blobs: u64,
  pub live_artifacts: u64,
  pub live_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::artifacts) struct ArtifactRequestV1 {
  pub(in crate::artifacts) contract: String,
  pub(in crate::artifacts) contract_version: u32,
  pub(in crate::artifacts) kind: String,
  pub(in crate::artifacts) operation: String,
  pub(in crate::artifacts) input: ArtifactPutInputV1,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::artifacts) struct ArtifactPutInputV1 {
  pub(in crate::artifacts) source_path: String,
  pub(in crate::artifacts) name: String,
  pub(in crate::artifacts) content_type: String,
  pub(in crate::artifacts) retention_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::artifacts) struct ArtifactReferenceRequestV1 {
  pub(in crate::artifacts) contract: String,
  pub(in crate::artifacts) contract_version: u32,
  pub(in crate::artifacts) kind: String,
  pub(in crate::artifacts) operation: String,
  pub(in crate::artifacts) input: ArtifactReferenceInputV1,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::artifacts) struct ArtifactReferenceInputV1 {
  pub(in crate::artifacts) artifact: ArtifactReferenceV1,
}
