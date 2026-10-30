//! Private remote worker journal artifacts.

use crate::event::is_definition_hash;
use crate::remote_protocol::RemoteArtifactGrantV1;
use crate::remote_protocol::RemoteArtifactOperationV1;
use crate::remote_protocol::RemoteDispatchPackageV1;
use crate::remote_worker_journal::sha256;
use crate::remote_worker_journal::valid_bound_id;
use crate::remote_worker_journal::PreparedRemoteDispatchV1;
use crate::remote_worker_journal::RemoteWorkerError;
use chrono::DateTime;
use chrono::Utc;
use serde_json_canonicalizer::to_vec as canonical_json;
use std::collections::BTreeSet;

/// Verifies a scoped artifact transfer at the worker boundary. Coordinators
/// perform the same checks before issuing a package; workers repeat them so a
/// compromised or stale transport cannot widen a grant.
#[allow(clippy::too_many_arguments)]
pub fn verify_remote_artifact_transfer(
  prepared: &PreparedRemoteDispatchV1,
  grant: &RemoteArtifactGrantV1,
  worker_id: &str,
  operation: RemoteArtifactOperationV1,
  bytes: &[u8],
  maximum_bytes: u64,
  now: DateTime<Utc>,
) -> Result<String, RemoteWorkerError> {
  let digest = sha256(bytes);
  if prepared.worker_id != worker_id
    || !prepared.package.artifact_grants.contains(grant)
    || grant.operation != operation
    || grant.expires_at <= now
    || bytes.is_empty()
    || maximum_bytes == 0
    || bytes.len() as u64 > maximum_bytes
    || digest != grant.digest
  {
    return Err(RemoteWorkerError::ArtifactIntegrity);
  }
  Ok(digest)
}

pub(in crate::remote_worker_journal) fn validate_artifact_grants(
  grants: &[RemoteArtifactGrantV1],
  package: &RemoteDispatchPackageV1,
  now: DateTime<Utc>,
) -> Result<(), RemoteWorkerError> {
  if grants.len() > 128 {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  let mut ids = BTreeSet::new();
  for grant in grants {
    if !ids.insert((grant.artifact_id.as_str(), grant.operation))
      || grant.expires_at <= now
      || grant.expires_at > package.deadline
      || !valid_bound_id(&grant.artifact_id)
      || !is_definition_hash(&grant.digest)
      || matches!(grant.operation, RemoteArtifactOperationV1::Write) && grant.artifact_id == "*"
    {
      return Err(RemoteWorkerError::InvalidPackage);
    }
  }
  Ok(())
}

pub(in crate::remote_worker_journal) fn artifact_grant_digest(
  grant: &RemoteArtifactGrantV1,
) -> Result<String, RemoteWorkerError> {
  canonical_json(grant)
    .map(|bytes| sha256(&bytes))
    .map_err(|_| RemoteWorkerError::InvalidPackage)
}
