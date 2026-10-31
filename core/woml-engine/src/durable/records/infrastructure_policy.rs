//! Durable records / infrastructure policy.

use crate::InfrastructurePolicySignatureStatusV1;
use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyRunBindingV1 {
  pub run_id: String,
  pub policy_id: String,
  pub policy_version: String,
  pub policy_digest: String,
  pub canonical_bundle_reference: String,
  pub signature_status: InfrastructurePolicySignatureStatusV1,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub signing_key_id: Option<String>,
  pub admitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InfrastructurePolicyApprovalEvidenceV1 {
  pub request_id: String,
  pub review_digest: String,
  pub principal_id: String,
  pub roles: BTreeSet<String>,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InfrastructurePolicyArtifactEvidenceV1 {
  pub artifact_id: String,
  pub sha256: String,
  pub review_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InfrastructurePolicyLockEvidenceV1 {
  pub key: String,
  pub lease_id: String,
  pub fencing_token: String,
  pub acquired_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct InfrastructurePolicyEvidenceSnapshotV1 {
  pub approvals: Vec<InfrastructurePolicyApprovalEvidenceV1>,
  pub artifacts: Vec<InfrastructurePolicyArtifactEvidenceV1>,
  pub locks: Vec<InfrastructurePolicyLockEvidenceV1>,
}
