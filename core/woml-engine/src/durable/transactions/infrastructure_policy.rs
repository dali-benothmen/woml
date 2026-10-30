//! Immutable admitted policy binding and verified evidence reads. Stored authority cannot widen the original effective policy.

use crate::durable::ensure_run_exists;
use crate::durable::DurableStoreError;
use crate::durable::InfrastructurePolicyApprovalEvidenceV1;
use crate::durable::InfrastructurePolicyArtifactEvidenceV1;
use crate::durable::InfrastructurePolicyEvidenceSnapshotV1;
use crate::durable::InfrastructurePolicyLockEvidenceV1;
use crate::durable::InfrastructurePolicyRunBindingV1;
use crate::durable::{parse_timestamp, DurableEventStore};
use crate::InfrastructurePolicyBundleV1;
use crate::InfrastructurePolicySignatureStatusV1;
use crate::InfrastructurePolicyTrustV1;
use crate::PreparedInfrastructurePolicyV1;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;

impl DurableEventStore {
  pub fn bind_infrastructure_policy(
    &mut self,
    run_id: &str,
    policy: &PreparedInfrastructurePolicyV1,
    admitted_at: DateTime<Utc>,
  ) -> Result<InfrastructurePolicyRunBindingV1, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let signature_status = match policy.signature_status {
      InfrastructurePolicySignatureStatusV1::Unsigned => "unsigned",
      InfrastructurePolicySignatureStatusV1::Verified => "verified",
    };
    transaction.execute(
      "INSERT OR IGNORE INTO woml_infrastructure_policy_bundles(
         policy_digest, policy_id, policy_version, environment, canonical_payload,
         signature_status, signing_key_id, created_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
      params![
        policy.digest,
        policy.bundle.id,
        policy.bundle.version,
        policy.bundle.environment,
        policy.canonical_payload,
        signature_status,
        policy.signing_key_id,
        admitted_at.to_rfc3339(),
      ],
    )?;
    let stored: (String, String, String, Vec<u8>, String, Option<String>) = transaction.query_row(
      "SELECT policy_id, policy_version, environment, canonical_payload,
              signature_status, signing_key_id
       FROM woml_infrastructure_policy_bundles WHERE policy_digest = ?1",
      [&policy.digest],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
        ))
      },
    )?;
    if stored.0 != policy.bundle.id
      || stored.1 != policy.bundle.version
      || stored.2 != policy.bundle.environment
      || stored.3 != policy.canonical_payload
      || stored.4 != signature_status
      || stored.5 != policy.signing_key_id
    {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: the stored policy digest is bound to different bytes or identity."
          .to_string(),
      ));
    }
    transaction.execute(
      "INSERT OR IGNORE INTO woml_run_infrastructure_policies(run_id, policy_digest, admitted_at)
       VALUES (?1, ?2, ?3)",
      params![run_id, policy.digest, admitted_at.to_rfc3339()],
    )?;
    let (bound_digest, bound_at): (String, String) = transaction.query_row(
      "SELECT policy_digest, admitted_at FROM woml_run_infrastructure_policies WHERE run_id = ?1",
      [run_id],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if bound_digest != policy.digest {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: the run is already pinned to a different infrastructure policy."
          .to_string(),
      ));
    }
    transaction.commit()?;
    Ok(InfrastructurePolicyRunBindingV1 {
      run_id: run_id.to_string(),
      policy_id: policy.bundle.id.clone(),
      policy_version: policy.bundle.version.clone(),
      policy_digest: policy.digest.clone(),
      canonical_bundle_reference: format!("policy:{}", policy.digest),
      signature_status: policy.signature_status,
      signing_key_id: policy.signing_key_id.clone(),
      admitted_at: parse_timestamp(&bound_at)?,
    })
  }

  pub fn infrastructure_policy_binding(
    &self,
    run_id: &str,
  ) -> Result<Option<InfrastructurePolicyRunBindingV1>, DurableStoreError> {
    let row = self
      .connection
      .query_row(
        "SELECT bundles.policy_id, bundles.policy_version, bindings.policy_digest,
                bundles.signature_status, bundles.signing_key_id, bindings.admitted_at
         FROM woml_run_infrastructure_policies AS bindings
         JOIN woml_infrastructure_policy_bundles AS bundles
           ON bundles.policy_digest = bindings.policy_digest
         WHERE bindings.run_id = ?1",
        [run_id],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
          ))
        },
      )
      .optional()?;
    let Some((policy_id, policy_version, digest, status, signing_key_id, admitted_at)) = row else {
      return Ok(None);
    };
    let signature_status = match status.as_str() {
      "unsigned" => InfrastructurePolicySignatureStatusV1::Unsigned,
      "verified" => InfrastructurePolicySignatureStatusV1::Verified,
      _ => {
        return Err(DurableStoreError::Contract(
          "WOML_POLICY_INVALID: admitted policy signature status is invalid.".to_string(),
        ))
      }
    };
    Ok(Some(InfrastructurePolicyRunBindingV1 {
      run_id: run_id.to_string(),
      policy_id,
      policy_version,
      canonical_bundle_reference: format!("policy:{digest}"),
      policy_digest: digest,
      signature_status,
      signing_key_id,
      admitted_at: parse_timestamp(&admitted_at)?,
    }))
  }

  pub fn infrastructure_policy_for_run(
    &self,
    run_id: &str,
  ) -> Result<Option<PreparedInfrastructurePolicyV1>, DurableStoreError> {
    let bound_digest = self
      .connection
      .query_row(
        "SELECT policy_digest FROM woml_run_infrastructure_policies WHERE run_id = ?1",
        [run_id],
        |row| row.get::<_, String>(0),
      )
      .optional()?;
    let Some(bound_digest) = bound_digest else {
      return Ok(None);
    };
    let stored = self
      .connection
      .query_row(
        "SELECT policy_id, policy_version, environment, policy_digest,
                canonical_payload, signature_status, signing_key_id
         FROM woml_infrastructure_policy_bundles WHERE policy_digest = ?1",
        [&bound_digest],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
          ))
        },
      )
      .optional()?;
    let Some((policy_id, policy_version, environment, digest, canonical_payload, status, key_id)) =
      stored
    else {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_UNAVAILABLE: admitted canonical policy bytes are missing.".to_string(),
      ));
    };
    if digest != bound_digest {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: admitted policy binding does not match stored bytes.".to_string(),
      ));
    }
    let bundle: InfrastructurePolicyBundleV1 =
      serde_json::from_slice(&canonical_payload).map_err(|_| {
        DurableStoreError::Contract(
          "WOML_POLICY_INVALID: admitted policy bytes are malformed.".to_string(),
        )
      })?;
    let mut prepared =
      crate::prepare_infrastructure_policy(bundle, &InfrastructurePolicyTrustV1::default())
        .map_err(|_| {
          DurableStoreError::Contract(
            "WOML_POLICY_INVALID: admitted policy bytes failed canonical validation.".to_string(),
          )
        })?;
    if prepared.digest != digest
      || prepared.canonical_payload != canonical_payload
      || prepared.bundle.id != policy_id
      || prepared.bundle.version != policy_version
      || prepared.bundle.environment != environment
    {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: admitted policy identity does not match its canonical bytes."
          .to_string(),
      ));
    }
    prepared.signature_status = match status.as_str() {
      "unsigned" if key_id.is_none() => InfrastructurePolicySignatureStatusV1::Unsigned,
      "verified" if key_id.is_some() => InfrastructurePolicySignatureStatusV1::Verified,
      _ => {
        return Err(DurableStoreError::Contract(
          "WOML_POLICY_INVALID: admitted policy signature status is contradictory.".to_string(),
        ))
      }
    };
    prepared.signing_key_id = key_id;
    Ok(Some(prepared))
  }

  pub(crate) fn infrastructure_policy_evidence_snapshot(
    &self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<InfrastructurePolicyEvidenceSnapshotV1, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let mut approvals_statement = self.connection.prepare(
      "SELECT reviews.request_id, reviews.review_digest, decisions.principal_id,
              decisions.roles_json, decisions.decided_at
       FROM woml_approval_reviews AS reviews
       JOIN woml_approval_reviewer_decisions AS decisions
         ON decisions.run_id = reviews.run_id AND decisions.request_id = reviews.request_id
       WHERE reviews.run_id = ?1 AND reviews.status = 'approved'
         AND decisions.decision = 'approved'
       ORDER BY decisions.decided_at, decisions.principal_id",
    )?;
    let approvals = approvals_statement
      .query_map([run_id], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
        ))
      })?
      .map(|row| {
        let (request_id, review_digest, principal_id, roles_json, decided_at) = row?;
        Ok(InfrastructurePolicyApprovalEvidenceV1 {
          request_id,
          review_digest,
          principal_id,
          roles: serde_json::from_str::<Vec<String>>(&roles_json)?
            .into_iter()
            .collect(),
          decided_at: parse_timestamp(&decided_at)?,
        })
      })
      .collect::<Result<Vec<_>, DurableStoreError>>()?;

    let mut artifacts_statement = self.connection.prepare(
      "SELECT pins.artifact_id, pins.artifact_sha256, reviews.review_digest
       FROM woml_approval_artifact_pins AS pins
       JOIN woml_approval_reviews AS reviews
         ON reviews.run_id = pins.run_id AND reviews.request_id = pins.request_id
       WHERE reviews.run_id = ?1 AND reviews.status = 'approved'
       ORDER BY pins.artifact_id, reviews.review_digest",
    )?;
    let artifacts = artifacts_statement
      .query_map([run_id], |row| {
        Ok(InfrastructurePolicyArtifactEvidenceV1 {
          artifact_id: row.get(0)?,
          sha256: row.get(1)?,
          review_digest: row.get(2)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;

    let mut locks_statement = self.connection.prepare(
      "SELECT key, owner_lease_id, CAST(owner_fencing_token AS TEXT), acquired_at, expires_at
       FROM woml_lock_resources
       WHERE owner_run_id = ?1 AND owner_lease_id IS NOT NULL
         AND owner_fencing_token IS NOT NULL AND acquired_at IS NOT NULL
         AND expires_at IS NOT NULL AND expires_at > ?2
       ORDER BY key",
    )?;
    let locks = locks_statement
      .query_map(params![run_id, now.to_rfc3339()], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
        ))
      })?
      .map(|row| {
        let (key, lease_id, fencing_token, acquired_at, expires_at) = row?;
        Ok(InfrastructurePolicyLockEvidenceV1 {
          key,
          lease_id,
          fencing_token,
          acquired_at: parse_timestamp(&acquired_at)?,
          expires_at: parse_timestamp(&expires_at)?,
        })
      })
      .collect::<Result<Vec<_>, DurableStoreError>>()?;

    Ok(InfrastructurePolicyEvidenceSnapshotV1 {
      approvals,
      artifacts,
      locks,
    })
  }
}
