//! Bound review evidence, reviewer votes, quorum, terminal settlement, and artifact pins. All related changes retain one original caller-owned transaction.

use crate::durable::append_to_history_scoped_incremental;
use crate::durable::cache_projection_state;
use crate::durable::definition_for_run;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::projection_state_for_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::RichApprovalDecisionAdmission;
use crate::durable::RichApprovalDecisionOutcome;
use crate::durable::RichApprovalDecisionStatus;
use crate::durable::RichApprovalIdentityDecision;
use crate::durable::RichQuorumState;
use crate::durable::RichReviewRow;
use crate::durable::{approval_decision_text, DurableEventStore};
use crate::event::is_definition_hash;
use crate::event::ApprovalDecision;
use crate::event::ApprovalFailure;
use crate::event::ApprovalInvalidatedData;
use crate::event::ApprovalQuorumProgressedData;
use crate::event::ApprovalResolvedData;
use crate::event::ApprovalReviewRequestedData;
use crate::event::ApprovalReviewerDecisionRecordedData;
use crate::event::RichApprovalResolutionReason;
use crate::event::RunFailedData;
use crate::fold_events;
use crate::projection::ProjectionFoldState;
use crate::verify_bound_rich_approval_review;
use crate::BoundRichApprovalReview;
use crate::ReviewerDirectoryAuthorityV1;
use crate::RunEventPayload;
use crate::RunProjection;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use serde_json_canonicalizer::to_vec as canonical_json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;

pub(in crate::durable) fn rich_review_row(
  connection: &Connection,
  run_id: &str,
  request_id: &str,
) -> Result<Option<RichReviewRow>, DurableStoreError> {
  let row: Option<(String, String, String, i64, String, String, String)> = connection
    .query_row(
      "SELECT approval_id, request_id, review_digest, required_approvals, required_roles_json,
              directory_version, status
       FROM woml_approval_reviews WHERE run_id = ?1 AND request_id = ?2",
      params![run_id, request_id],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
        ))
      },
    )
    .optional()?;
  row
    .map(
      |(
        approval_id,
        request_id,
        review_digest,
        required,
        required_roles_json,
        directory_version,
        status,
      )| {
        Ok(RichReviewRow {
          approval_id,
          request_id,
          review_digest,
          required: u32::try_from(required).map_err(|_| {
            DurableStoreError::Contract(
              "Stored rich approval quorum is outside the frozen range.".to_string(),
            )
          })?,
          required_roles: serde_json::from_str(&required_roles_json)?,
          directory_version,
          status,
        })
      },
    )
    .transpose()
}

fn rich_quorum_state(
  connection: &Connection,
  review: &RichReviewRow,
) -> Result<RichQuorumState, DurableStoreError> {
  let mut statement = connection.prepare(
    "SELECT decision, roles_json FROM woml_approval_reviewer_decisions
     WHERE request_id = ?1 ORDER BY principal_id",
  )?;
  let rows = statement
    .query_map([&review.request_id], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?
    .collect::<Result<Vec<_>, _>>()?;
  let mut received = 0u32;
  let mut represented = BTreeMap::<String, ()>::new();
  for (decision, roles_json) in rows {
    if decision != "approved" {
      continue;
    }
    received = received.saturating_add(1);
    let roles: Vec<String> = serde_json::from_str(&roles_json)?;
    for role in roles {
      if review.required_roles.contains(&role) {
        represented.insert(role, ());
      }
    }
  }
  Ok(RichQuorumState {
    received,
    represented_roles: represented.into_keys().collect(),
  })
}

fn settle_rich_review_row(
  transaction: &Transaction<'_>,
  run_id: &str,
  request_id: &str,
  status: &str,
  settled_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  let changed = transaction.execute(
    "UPDATE woml_approval_reviews SET status = ?3, resolved_at = ?4
     WHERE run_id = ?1 AND request_id = ?2 AND status = 'pending'",
    params![run_id, request_id, status, settled_at.to_rfc3339()],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::ApprovalDecisionConflict);
  }
  transaction.execute(
    "UPDATE woml_approval_artifact_pins SET released_at = ?3
     WHERE run_id = ?1 AND request_id = ?2 AND released_at IS NULL",
    params![run_id, request_id, settled_at.to_rfc3339()],
  )?;
  Ok(())
}

fn validate_rich_decision_admission(
  admission: &RichApprovalDecisionAdmission,
) -> Result<(), DurableStoreError> {
  let valid_text = |value: &str, max: usize| {
    !value.is_empty()
      && value.len() <= max
      && !value.chars().any(|character| character.is_control())
  };
  if !valid_text(&admission.run_id, 320)
    || !admission.request_id.starts_with("aprreq_")
    || !is_definition_hash(&admission.review_digest)
    || !valid_text(&admission.principal_id, 128)
    || !valid_text(&admission.provider, 64)
    || !valid_text(&admission.provider_account_id, 256)
    || !valid_text(&admission.provider_actor_id, 256)
    || admission.roles.len() > crate::RICH_APPROVAL_MAX_ROLES
    || !admission.roles.windows(2).all(|pair| pair[0] < pair[1])
  {
    return Err(DurableStoreError::Contract(
      "The rich approval decision admission is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn append_rich_approval_payloads(
  transaction: &Transaction<'_>,
  projection_cache: &RefCell<HashMap<String, ProjectionFoldState>>,
  run_id: &str,
  occurred_at: DateTime<Utc>,
  payloads: Vec<RunEventPayload>,
) -> Result<RunProjection, DurableStoreError> {
  let mut events = load_events(transaction, run_id)?;
  let mut projection_state = projection_state_for_events(projection_cache, run_id, &events)?;
  let version = events
    .first()
    .map(|event| event.event_schema_version)
    .ok_or_else(|| DurableStoreError::Contract("A rich approval requires a started run.".into()))?;
  if !matches!(
    version,
    crate::RUN_EVENT_SCHEMA_VERSION_V19 | crate::RUN_EVENT_SCHEMA_VERSION_V20
  ) {
    return Err(DurableStoreError::Contract(
      "Rich approval persistence requires Event v19 or later.".to_string(),
    ));
  }
  for payload in payloads {
    append_to_history_scoped_incremental(
      transaction,
      &mut events,
      &mut projection_state,
      run_id,
      generated_event_id(),
      occurred_at,
      version,
      None,
      payload,
      true,
    )?;
  }
  Ok(projection_state.projection().clone())
}

impl DurableEventStore {
  /// Persists one immutable review snapshot and its artifact pins before any
  /// decision capability may be exposed. Model v20 execution is enabled only
  /// in RPA5; this method is the RPA2/RPA3 storage authority used by that
  /// future execution path.
  pub fn create_rich_approval_request(
    &mut self,
    binding: &BoundRichApprovalReview,
    created_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    verify_bound_rich_approval_review(binding)
      .map_err(|_| DurableStoreError::RichApprovalBindingMismatch)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run_binding = run_binding_in_transaction(&transaction, &binding.run_id)?;
    let workflow = definition_for_run(&transaction, &binding.run_id)?;
    let workflow_version = workflow
      .metadata
      .as_ref()
      .and_then(|metadata| metadata.version.as_deref())
      .unwrap_or("0.0.0");
    let approval = workflow.approval(&binding.approval_id).ok_or_else(|| {
      DurableStoreError::Contract("The rich approval is not in Model v20.".into())
    })?;
    if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20
      || approval.review.is_none()
      || approval.required_approvals != Some(binding.required_approvals)
      || approval.required_roles != binding.required_roles
      || run_binding.definition_hash != binding.definition_hash
      || workflow.workflow_id != binding.workflow_id
      || workflow_version != binding.workflow_version
    {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    let review_json = String::from_utf8(canonical_json(&binding.review)?).map_err(|_| {
      DurableStoreError::Contract("The immutable review is not valid UTF-8 JSON.".to_string())
    })?;
    let required_roles_json = serde_json::to_string(&binding.required_roles)?;
    let inserted = transaction.execute(
      "INSERT OR IGNORE INTO woml_approval_reviews(
         run_id, approval_id, request_id, definition_hash, workflow_version,
         review_json, review_digest, required_approvals, required_roles_json,
         directory_version, status, created_at, resolved_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11, NULL)",
      params![
        binding.run_id,
        binding.approval_id,
        binding.request_id,
        binding.definition_hash,
        binding.workflow_version,
        review_json,
        binding.review_digest,
        i64::from(binding.required_approvals),
        required_roles_json,
        binding.directory_version,
        created_at.to_rfc3339(),
      ],
    )?;
    if inserted == 0 {
      let existing: Option<(String, String)> = transaction
        .query_row(
          "SELECT request_id, review_digest FROM woml_approval_reviews
           WHERE run_id = ?1 AND approval_id = ?2",
          params![binding.run_id, binding.approval_id],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
      if existing.as_ref() == Some(&(binding.request_id.clone(), binding.review_digest.clone())) {
        return Ok(fold_events(&load_events(&transaction, &binding.run_id)?)?);
      }
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    for artifact in &binding.artifacts {
      transaction.execute(
        "INSERT INTO woml_approval_artifact_pins(
           run_id, request_id, artifact_id, artifact_sha256, released_at
         ) VALUES (?1, ?2, ?3, ?4, NULL)",
        params![
          binding.run_id,
          binding.request_id,
          artifact.id,
          artifact.sha256
        ],
      )?;
    }

    let mut events = load_events(&transaction, &binding.run_id)?;
    let mut projection_state =
      projection_state_for_events(&self.projection_cache, &binding.run_id, &events)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("A rich approval requires a started run.".into())
      })?;
    if !matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V19 | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      return Err(DurableStoreError::Contract(
        "A rich approval request requires Event v19 or later.".to_string(),
      ));
    }
    append_to_history_scoped_incremental(
      &transaction,
      &mut events,
      &mut projection_state,
      &binding.run_id,
      generated_event_id(),
      created_at,
      event_schema_version,
      None,
      RunEventPayload::ApprovalReviewRequested(ApprovalReviewRequestedData {
        approval_id: binding.approval_id.clone(),
        request_id: binding.request_id.clone(),
        review_digest: binding.review_digest.clone(),
        directory_version: binding.directory_version.clone(),
        required: binding.required_approvals,
        required_roles: binding.required_roles.clone(),
      }),
      true,
    )?;
    let projection = projection_state.projection().clone();
    transaction.commit()?;
    cache_projection_state(&self.projection_cache, &binding.run_id, projection_state);
    Ok(projection)
  }

  pub fn record_rich_approval_decision(
    &mut self,
    admission: RichApprovalDecisionAdmission,
  ) -> Result<RichApprovalDecisionOutcome, DurableStoreError> {
    validate_rich_decision_admission(&admission)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let review = rich_review_row(&transaction, &admission.run_id, &admission.request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    if review.review_digest != admission.review_digest {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    let current = rich_quorum_state(&transaction, &review)?;
    if review.status != "pending" {
      return Ok(RichApprovalDecisionOutcome {
        status: RichApprovalDecisionStatus::AlreadyResolved,
        required: review.required,
        received: current.received,
        represented_roles: current.represented_roles,
      });
    }
    let existing: Option<String> = transaction
      .query_row(
        "SELECT decision FROM woml_approval_reviewer_decisions
         WHERE run_id = ?1 AND request_id = ?2 AND principal_id = ?3",
        params![
          admission.run_id,
          admission.request_id,
          admission.principal_id
        ],
        |row| row.get(0),
      )
      .optional()?;
    if let Some(existing) = existing {
      if existing != approval_decision_text(admission.decision) {
        return Err(DurableStoreError::RichApprovalReviewerConflict);
      }
      return Ok(RichApprovalDecisionOutcome {
        status: RichApprovalDecisionStatus::Idempotent,
        required: review.required,
        received: current.received,
        represented_roles: current.represented_roles,
      });
    }
    transaction.execute(
      "INSERT INTO woml_approval_reviewer_decisions(
         run_id, request_id, principal_id, decision, provider, provider_account_id,
         provider_actor_id, roles_json, decided_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
      params![
        admission.run_id,
        admission.request_id,
        admission.principal_id,
        approval_decision_text(admission.decision),
        admission.provider,
        admission.provider_account_id,
        admission.provider_actor_id,
        serde_json::to_string(&admission.roles)?,
        admission.decided_at.to_rfc3339(),
      ],
    )?;
    let quorum = rich_quorum_state(&transaction, &review)?;
    let terminal = if admission.decision == ApprovalDecision::Rejected {
      Some((
        "rejected",
        RichApprovalResolutionReason::Rejection,
        ApprovalDecision::Rejected,
      ))
    } else if quorum.received >= review.required
      && review
        .required_roles
        .iter()
        .all(|role| quorum.represented_roles.contains(role))
    {
      Some((
        "approved",
        RichApprovalResolutionReason::Quorum,
        ApprovalDecision::Approved,
      ))
    } else {
      None
    };
    if let Some((status, _, _)) = terminal {
      settle_rich_review_row(
        &transaction,
        &admission.run_id,
        &admission.request_id,
        status,
        admission.decided_at,
      )?;
    }

    let mut payloads = vec![RunEventPayload::ApprovalReviewerDecisionRecorded(
      ApprovalReviewerDecisionRecordedData {
        approval_id: review.approval_id.clone(),
        request_id: admission.request_id.clone(),
        review_digest: admission.review_digest.clone(),
        principal_id: admission.principal_id.clone(),
        decision: admission.decision,
      },
    )];
    if admission.decision == ApprovalDecision::Approved {
      payloads.push(RunEventPayload::ApprovalQuorumProgressed(
        ApprovalQuorumProgressedData {
          approval_id: review.approval_id.clone(),
          request_id: admission.request_id.clone(),
          review_digest: admission.review_digest.clone(),
          required: review.required,
          received: quorum.received,
          required_roles: review.required_roles.clone(),
          represented_roles: quorum.represented_roles.clone(),
        },
      ));
    }
    if let Some((_, reason, decision)) = terminal {
      payloads.push(RunEventPayload::ApprovalResolved(ApprovalResolvedData {
        approval_id: review.approval_id.clone(),
        request_id: admission.request_id.clone(),
        resolution: None,
        review_digest: Some(admission.review_digest.clone()),
        decision: Some(decision),
        reason: Some(reason),
        required: Some(review.required),
        received: Some(quorum.received),
        required_roles: Some(review.required_roles.clone()),
        represented_roles: Some(quorum.represented_roles.clone()),
      }));
    }
    append_rich_approval_payloads(
      &transaction,
      &self.projection_cache,
      &admission.run_id,
      admission.decided_at,
      payloads,
    )?;
    transaction.commit()?;
    self.projection_cache.borrow_mut().remove(&admission.run_id);
    Ok(RichApprovalDecisionOutcome {
      status: match terminal {
        Some(("approved", _, _)) => RichApprovalDecisionStatus::Approved,
        Some(_) => RichApprovalDecisionStatus::Rejected,
        None => RichApprovalDecisionStatus::Pending,
      },
      required: review.required,
      received: quorum.received,
      represented_roles: quorum.represented_roles,
    })
  }

  /// Resolves an authenticated provider identity through the operator-owned
  /// directory version pinned by the immutable review, then converges on the
  /// same atomic quorum transaction used by every rich-approval ingress path.
  /// Principal IDs and roles are never accepted from callback input.
  pub fn record_rich_approval_identity_decision(
    &mut self,
    authority: &ReviewerDirectoryAuthorityV1,
    decision: RichApprovalIdentityDecision,
  ) -> Result<RichApprovalDecisionOutcome, DurableStoreError> {
    let review = rich_review_row(&self.connection, &decision.run_id, &decision.request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    if review.review_digest != decision.review_digest {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    authority.validate_requirements(
      &review.directory_version,
      review.required,
      &review.required_roles,
    )?;
    let reviewer = authority.resolve(
      &review.directory_version,
      &decision.identity,
      &review.required_roles,
    )?;
    self.record_rich_approval_decision(RichApprovalDecisionAdmission {
      run_id: decision.run_id,
      request_id: decision.request_id,
      review_digest: decision.review_digest,
      principal_id: reviewer.principal_id,
      decision: decision.decision,
      provider: reviewer.provider.as_str().to_string(),
      provider_account_id: reviewer.account_id,
      provider_actor_id: reviewer.actor_id,
      roles: reviewer.roles,
      decided_at: decision.decided_at,
    })
  }

  pub fn settle_rich_approval_without_vote(
    &mut self,
    run_id: &str,
    request_id: &str,
    review_digest: &str,
    reason: RichApprovalResolutionReason,
    settled_at: DateTime<Utc>,
  ) -> Result<RichApprovalDecisionOutcome, DurableStoreError> {
    if matches!(
      reason,
      RichApprovalResolutionReason::Quorum | RichApprovalResolutionReason::Rejection
    ) {
      return Err(DurableStoreError::Contract(
        "Human rich approval outcomes require a reviewer decision.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let review = rich_review_row(&transaction, run_id, request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    if review.review_digest != review_digest {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    let quorum = rich_quorum_state(&transaction, &review)?;
    if review.status != "pending" {
      return Ok(RichApprovalDecisionOutcome {
        status: RichApprovalDecisionStatus::AlreadyResolved,
        required: review.required,
        received: quorum.received,
        represented_roles: quorum.represented_roles,
      });
    }
    let status = match reason {
      RichApprovalResolutionReason::Timeout => "expired",
      RichApprovalResolutionReason::Cancellation => "cancelled",
      RichApprovalResolutionReason::ArtifactInvalidated => "invalidated",
      _ => unreachable!(),
    };
    settle_rich_review_row(&transaction, run_id, request_id, status, settled_at)?;
    let mut payloads = Vec::new();
    if reason == RichApprovalResolutionReason::ArtifactInvalidated {
      payloads.push(RunEventPayload::ApprovalInvalidated(
        ApprovalInvalidatedData {
          approval_id: review.approval_id.clone(),
          request_id: request_id.to_string(),
          review_digest: review_digest.to_string(),
          reason,
        },
      ));
    }
    payloads.push(RunEventPayload::ApprovalResolved(ApprovalResolvedData {
      approval_id: review.approval_id.clone(),
      request_id: request_id.to_string(),
      resolution: None,
      review_digest: Some(review_digest.to_string()),
      decision: Some(ApprovalDecision::Rejected),
      reason: Some(reason),
      required: Some(review.required),
      received: Some(quorum.received),
      required_roles: Some(review.required_roles),
      represented_roles: Some(quorum.represented_roles.clone()),
    }));
    if reason == RichApprovalResolutionReason::Timeout {
      let workflow = definition_for_run(&transaction, run_id)?;
      let approval = workflow.approval(&review.approval_id).ok_or_else(|| {
        DurableStoreError::Contract("A rich approval timeout has no compiled descriptor.".into())
      })?;
      if approval.on_timeout == "fail" {
        payloads.push(RunEventPayload::RunFailed(RunFailedData::V4(
          crate::RunFailedDataV4::Approval {
            approval_id: review.approval_id.clone(),
            request_id: request_id.to_string(),
            failure: ApprovalFailure {
              kind: "approval_timeout".to_string(),
              code: "WOML_APPROVAL_TIMEOUT".to_string(),
              message: format!("Approval {:?} reached its deadline.", review.approval_id),
            },
          },
        )));
      }
    }
    append_rich_approval_payloads(
      &transaction,
      &self.projection_cache,
      run_id,
      settled_at,
      payloads,
    )?;
    transaction.commit()?;
    self.projection_cache.borrow_mut().remove(run_id);
    Ok(RichApprovalDecisionOutcome {
      status: RichApprovalDecisionStatus::Rejected,
      required: review.required,
      received: quorum.received,
      represented_roles: quorum.represented_roles,
    })
  }
}
