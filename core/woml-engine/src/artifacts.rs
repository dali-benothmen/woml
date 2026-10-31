//! Stable artifacts facade. Private modules retain existing authority, identities, limits, and lifecycle ownership.

// The frozen cross-capability failure envelope remains unboxed.
#![allow(clippy::result_large_err)]

mod authorization;
mod backend;
mod backup;
mod contracts;
mod failures;
mod filesystem;
mod handlers;
mod identity;
mod inspection;
mod maintenance;
mod materialization;
mod output;
mod policy;
mod schema;
mod store;
#[cfg(test)]
mod tests;
mod transfer;
mod validation;

pub(in crate::artifacts) use authorization::validate_reference;
pub(in crate::artifacts) use backend::{
  ArtifactBackend, ArtifactStage, LocalArtifactBackend, S3ArtifactBackend, VerifiedArtifactBlob,
  STREAM_BUFFER_BYTES,
};
pub use backup::{export_artifact_backup, restore_artifact_backup, verify_artifact_backup};
pub use contracts::{
  ArtifactBackupInventoryV1, ArtifactExportV1, ArtifactInspectionV1, ArtifactMaintenanceV1,
  ArtifactObservationV1, ArtifactProvenanceV1, ArtifactReferenceV1, ArtifactRunInspectionV1,
  ARTIFACTS_CONTRACT, ARTIFACTS_CONTRACT_VERSION, ARTIFACT_POLICY_CONTRACT,
  ARTIFACT_POLICY_CONTRACT_VERSION,
};
pub(in crate::artifacts) use contracts::{
  ArtifactReferenceRequestV1, ArtifactRequestV1, ARTIFACT_REFERENCE_CONTRACT,
  ARTIFACT_REFERENCE_CONTRACT_VERSION,
};
pub(in crate::artifacts) use failures::{
  artifact_access_denied, artifact_cancelled, artifact_corrupt, artifact_expired, artifact_failure,
  artifact_interrupted, artifact_not_found, artifact_path_invalid, artifact_quarantined,
  artifact_timed_out, artifact_too_large, backend_unavailable, materialization_unavailable,
  operation_identity_conflict, quota_exceeded, retention_invalid, source_changed, source_denied,
  source_type_unsupported,
};
#[cfg(unix)]
pub(in crate::artifacts) use filesystem::harden_file_permissions;
#[cfg(not(unix))]
pub(in crate::artifacts) use filesystem::harden_file_permissions;
#[cfg(windows)]
pub(in crate::artifacts) use filesystem::is_windows_reparse;
#[cfg(not(windows))]
pub(in crate::artifacts) use filesystem::is_windows_reparse;
#[cfg(unix)]
pub(in crate::artifacts) use filesystem::sync_directory;
#[cfg(not(unix))]
pub(in crate::artifacts) use filesystem::sync_directory;
pub(in crate::artifacts) use filesystem::{
  canonical_directory, cleanup_directory_contents, directory_bytes, ensure_private_directory,
  validate_authored_path,
};
pub(in crate::artifacts) use handlers::ArtifactReadOperation;
pub use handlers::{ManagedArtifactPutHandler, ManagedArtifactReadHandler};
pub(in crate::artifacts) use identity::{canonical_digest, new_artifact_id, timestamp, CROCKFORD};
pub use inspection::{export_artifact, inspect_artifact, inspect_run_artifacts, observe_artifacts};
pub use maintenance::maintain_artifacts;
pub(in crate::artifacts) use maintenance::ARTIFACT_ORPHAN_SCAN_BATCH;
pub(in crate::artifacts) use materialization::{stream_materialization, verify_materialized_file};
pub(crate) use output::OperationalArtifactWriter;
pub(crate) use policy::PreparedArtifactPolicy;
#[cfg(test)]
use policy::S3Credentials;
pub use policy::{
  ArtifactBackendPolicy, ArtifactLimits, ArtifactMaterializationPolicy,
  ArtifactOperationalOutputPolicy, ArtifactPolicyError, ArtifactPolicyModes, ArtifactPolicyV1,
  ArtifactRetentionPolicy, ArtifactRuntimeMode, ArtifactSecretScanningPolicy, KnownSecretAction,
};
pub(in crate::artifacts) use policy::{
  PreparedArtifactBackend, PreparedS3Backend, S3ServerSideEncryption, MAX_ARTIFACT_BYTES,
};
pub(in crate::artifacts) use schema::{ensure_store_identity, initialize_schema, validate_schema};
pub(in crate::artifacts) use store::ArtifactStore;
pub use store::ManagedArtifactStore;
pub(in crate::artifacts) use transfer::{
  enforce_quotas, read_settlement, validate_settlement_identity,
};
pub(in crate::artifacts) use validation::{
  contains_bytes, parse_put, parse_reference_request, validate_artifact_id, validate_content_type,
  validate_name,
};
