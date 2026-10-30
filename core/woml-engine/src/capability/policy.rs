//! Trusted infrastructure-policy execution descriptors and evidence. No input metadata may manufacture permission.

use crate::infrastructure_policy_runtime::InfrastructurePolicyEvidenceAuthorityV1;
use crate::infrastructure_policy_runtime::InfrastructurePolicyEvidenceKindV1;
use crate::infrastructure_policy_runtime::InfrastructurePolicyEvidenceReferenceV1;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::InfrastructurePolicyExecutionModeV1;
use chrono::Utc;
use sha2::Digest;
use sha2::Sha256;
use std::sync::Arc;

pub(in crate::capability) fn infrastructure_policy_execution_mode(
  projection: &crate::RunProjection,
  node_id: &str,
  attempt: u32,
) -> InfrastructurePolicyExecutionModeV1 {
  for saga in projection.sagas.values() {
    if let Some(action) = saga.actions.get(node_id) {
      if action.active_attempt == Some(attempt) {
        if saga.phase == crate::projection::SagaPhase::ManualRecovery {
          return InfrastructurePolicyExecutionModeV1::Recovery;
        }
        return match action.action_kind {
          crate::CorrectiveActionKind::Compensation => {
            InfrastructurePolicyExecutionModeV1::Compensation
          }
          crate::CorrectiveActionKind::Cleanup => InfrastructurePolicyExecutionModeV1::Cleanup,
        };
      }
    }
  }
  InfrastructurePolicyExecutionModeV1::Normal
}

pub(in crate::capability) fn infrastructure_policy_evidence(
  store: &DurableEventStore,
  artifact_store: Option<&Arc<crate::ManagedArtifactStore>>,
  run_id: &str,
  now: chrono::DateTime<Utc>,
) -> Result<(Option<String>, Vec<InfrastructurePolicyEvidenceReferenceV1>), DurableStoreError> {
  let snapshot = store.infrastructure_policy_evidence_snapshot(run_id, now)?;
  let authority = InfrastructurePolicyEvidenceAuthorityV1::engine();
  let mut review_digests = std::collections::BTreeSet::new();
  let mut evidence = Vec::new();
  for approval in snapshot.approvals {
    review_digests.insert(approval.review_digest.clone());
    let identity = format!(
      "{}\0{}\0{}",
      approval.request_id, approval.review_digest, approval.principal_id
    );
    if let Ok(reference) = authority.issue(
      InfrastructurePolicyEvidenceKindV1::Approval,
      format!("approval_{:x}", Sha256::digest(identity.as_bytes())),
      "woml.rich-approval-authority/v1".to_string(),
      approval.decided_at,
      None,
      run_id.to_string(),
      None,
      None,
      Some(approval.review_digest),
      Some(approval.principal_id),
      approval.roles,
      true,
      None,
    ) {
      evidence.push(reference);
    }
  }
  if let Some(artifact_store) = artifact_store {
    for artifact in snapshot.artifacts {
      if artifact_store
        .verify_policy_evidence(run_id, &artifact.artifact_id, &artifact.sha256)
        .is_err()
      {
        continue;
      }
      review_digests.insert(artifact.review_digest.clone());
      if let Ok(reference) = authority.issue(
        InfrastructurePolicyEvidenceKindV1::Artifact,
        artifact.artifact_id,
        "woml.artifact-store/v1".to_string(),
        now,
        None,
        run_id.to_string(),
        None,
        None,
        Some(artifact.review_digest),
        None,
        std::collections::BTreeSet::new(),
        true,
        None,
      ) {
        evidence.push(reference);
      }
    }
  }
  for lock in snapshot.locks {
    let identity = format!("{}\0{}\0{}", lock.key, lock.lease_id, lock.fencing_token);
    if let Ok(reference) = authority.issue(
      InfrastructurePolicyEvidenceKindV1::Lock,
      format!("lock_{:x}", Sha256::digest(identity.as_bytes())),
      "woml.lock-authority/v1".to_string(),
      lock.acquired_at,
      Some(lock.expires_at),
      run_id.to_string(),
      None,
      None,
      None,
      None,
      std::collections::BTreeSet::new(),
      true,
      Some(lock.key),
    ) {
      evidence.push(reference);
    }
  }
  let review_digest = if review_digests.len() == 1 {
    review_digests.into_iter().next()
  } else {
    None
  };
  Ok((review_digest, evidence))
}
