//! Dispatch transitions on the existing durable store connection.
//!
//! Preserve each original transaction around identity checks, event append,
//! dispatch/output cursor updates and commit. A worker fact cannot forge a run.

use crate::remote_dispatch::derive_remote_dispatch_id;
use crate::remote_dispatch::digest_json;
use crate::remote_dispatch::parse_optional_time;
use crate::remote_dispatch::parse_time;
use crate::remote_dispatch::valid_digest;
use crate::remote_dispatch::valid_id;
use crate::remote_dispatch::validate_request_against_definition;
use crate::remote_dispatch::RemoteDispatchError;
use crate::remote_dispatch::RemoteDispatchInspectionV1;
use crate::remote_dispatch::RemoteDispatchRecordV1;
use crate::remote_dispatch::RemoteDispatchRequestV1;
use crate::remote_dispatch::RemoteDispatchResolutionV1;
use crate::remote_dispatch::RemoteDispatchStateV1;
use crate::remote_dispatch::RemoteOutputBindingV1;
use crate::remote_dispatch::REMOTE_DISPATCH_HEARTBEAT_MAX_BYTES;
use crate::remote_dispatch::REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT;
use crate::CompiledWorkflowDefinition;
use crate::DurableEventStore;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use serde_json::json;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

impl DurableEventStore {
  pub fn remote_dispatch_record(
    &self,
    dispatch_id: &str,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    load_dispatch(&self.connection, dispatch_id)?.ok_or(RemoteDispatchError::NotFound)
  }

  pub fn inspect_remote_dispatch(
    &self,
    dispatch_id: &str,
  ) -> Result<RemoteDispatchInspectionV1, RemoteDispatchError> {
    let dispatch =
      load_dispatch(&self.connection, dispatch_id)?.ok_or(RemoteDispatchError::NotFound)?;
    Ok(RemoteDispatchInspectionV1 {
      profile: "woml.remote-dispatch-inspection/v1",
      dispatch_id: dispatch.dispatch_id,
      run_id: dispatch.run_id,
      node_id: dispatch.node_id,
      attempt: dispatch.attempt,
      target: dispatch.target,
      worker_id: dispatch.worker_id,
      state: match dispatch.state {
        RemoteDispatchStateV1::Requested => "requested",
        RemoteDispatchStateV1::Offered => "offered",
        RemoteDispatchStateV1::Accepted => "accepted",
        RemoteDispatchStateV1::Started => "started",
        RemoteDispatchStateV1::CancelRequested => "cancel_requested",
        RemoteDispatchStateV1::Succeeded => "succeeded",
        RemoteDispatchStateV1::Failed => "failed",
        RemoteDispatchStateV1::Cancelled => "cancelled",
        RemoteDispatchStateV1::Ambiguous => "ambiguous",
        RemoteDispatchStateV1::Abandoned => "abandoned",
      },
      ambiguous: dispatch.state == RemoteDispatchStateV1::Ambiguous,
      last_heartbeat_at: dispatch.last_heartbeat_at,
      output_sequence: dispatch.last_heartbeat_sequence.unwrap_or(0),
      failure_code: (dispatch.state == RemoteDispatchStateV1::Failed)
        .then(|| "WOML_REMOTE_EXECUTION_FAILED".to_string()),
    })
  }

  pub fn request_remote_dispatch(
    &mut self,
    request: &RemoteDispatchRequestV1,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    let dispatch_id = derive_remote_dispatch_id(request)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_request_against_definition(&transaction, request)?;
    if let Some(existing) = load_dispatch(&transaction, &dispatch_id)? {
      if same_request(&existing, request) {
        transaction.commit()?;
        return Ok(existing);
      }
      return Err(RemoteDispatchError::Conflict);
    }
    let conflicting_dispatch: Option<String> = transaction
      .query_row(
        "SELECT dispatch_id FROM woml_remote_dispatches
         WHERE (run_id=?1 AND node_id=?2 AND iteration_identity=?3 AND attempt=?4)
            OR invocation_id=?5
         LIMIT 1",
        params![
          request.run_id,
          request.node_id,
          request.iteration_identity,
          request.attempt,
          request.invocation_id,
        ],
        |row| row.get(0),
      )
      .optional()?;
    if conflicting_dispatch.is_some() {
      return Err(RemoteDispatchError::Conflict);
    }
    transaction.execute(
      "INSERT INTO woml_remote_dispatches(
         dispatch_id, run_id, node_id, iteration_identity, attempt, invocation_id,
         target, target_digest, package_digest, state, created_at, updated_at
       ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'requested',?10,?10)",
      params![
        dispatch_id,
        request.run_id,
        request.node_id,
        request.iteration_identity,
        request.attempt,
        request.invocation_id,
        request.target,
        request.target_digest,
        request.package_digest,
        request.requested_at.to_rfc3339(),
      ],
    )?;
    audit(
      &transaction,
      &dispatch_id,
      "requested",
      "coordinator",
      &digest_json(&json!({"packageDigest": request.package_digest}))?,
      &json!({}),
      request.requested_at,
    )?;
    let record = load_dispatch(&transaction, &dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  pub fn authorize_remote_output(
    &self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
  ) -> Result<RemoteOutputBindingV1, RemoteDispatchError> {
    let dispatch =
      bound_dispatch_without_offer(&self.connection, dispatch_id, worker_id, session_id)?;
    if !matches!(
      dispatch.state,
      RemoteDispatchStateV1::Started
        | RemoteDispatchStateV1::CancelRequested
        | RemoteDispatchStateV1::Ambiguous
    ) {
      return Err(RemoteDispatchError::Conflict);
    }
    let (workflow_id, model_json) = self
      .connection
      .query_row(
        "SELECT runs.workflow_id, definitions.model_json
         FROM woml_runs AS runs
         JOIN woml_definitions AS definitions
           ON definitions.definition_hash = runs.definition_hash
         WHERE runs.run_id=?1",
        [&dispatch.run_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
      )
      .optional()?
      .ok_or(RemoteDispatchError::Corrupt)?;
    let workflow: CompiledWorkflowDefinition =
      serde_json::from_str(&model_json).map_err(|_| RemoteDispatchError::Corrupt)?;
    let corrective = workflow
      .graph
      .sagas
      .as_deref()
      .unwrap_or_default()
      .iter()
      .flat_map(|saga| saga.compensations.iter().chain(saga.cleanup.iter()))
      .any(|action| action.node.id == dispatch.node_id);
    let phase = if corrective {
      crate::OperationalExecutionPhaseV1::Corrective
    } else {
      crate::OperationalExecutionPhaseV1::Forward
    };
    Ok(RemoteOutputBindingV1 {
      workflow_id,
      run_id: dispatch.run_id,
      node_id: dispatch.node_id,
      attempt: dispatch.attempt,
      invocation_id: dispatch.invocation_id,
      phase,
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub fn offer_remote_dispatch(
    &mut self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
    offer_id: &str,
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    if !valid_id(worker_id) || !valid_id(session_id) || !valid_id(offer_id) || expires_at <= now {
      return Err(RemoteDispatchError::Invalid);
    }
    self.transition(
      dispatch_id,
      &[RemoteDispatchStateV1::Requested],
      RemoteDispatchStateV1::Offered,
      worker_id,
      session_id,
      offer_id,
      Some(expires_at),
      "offered",
      now,
      None,
    )
  }

  pub fn accept_remote_dispatch(
    &mut self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
    offer_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = bound_dispatch(&transaction, dispatch_id, worker_id, session_id, offer_id)?;
    if existing.state == RemoteDispatchStateV1::Accepted {
      transaction.commit()?;
      return Ok(existing);
    }
    if existing.state != RemoteDispatchStateV1::Offered
      || existing.offer_expires_at.is_none_or(|expiry| expiry <= now)
    {
      return Err(RemoteDispatchError::Conflict);
    }
    transaction.execute(
      "UPDATE woml_remote_dispatches SET state='accepted', accepted_at=?2, updated_at=?2 WHERE dispatch_id=?1",
      params![dispatch_id, now.to_rfc3339()],
    )?;
    audit(
      &transaction,
      dispatch_id,
      "accepted",
      worker_id,
      &digest_json(&json!({"offerId": offer_id}))?,
      &json!({}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  pub fn start_remote_dispatch(
    &mut self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    self.bound_state_transition(
      dispatch_id,
      worker_id,
      session_id,
      RemoteDispatchStateV1::Accepted,
      RemoteDispatchStateV1::Started,
      "started",
      now,
    )
  }

  pub fn heartbeat_remote_dispatch(
    &mut self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
    sequence: u64,
    state: &serde_json::Value,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    let bytes = canonical_json(state).map_err(|_| RemoteDispatchError::Invalid)?;
    if bytes.len() > REMOTE_DISPATCH_HEARTBEAT_MAX_BYTES {
      return Err(RemoteDispatchError::Invalid);
    }
    let heartbeat_digest = format!("sha256:{:x}", Sha256::digest(bytes));
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = bound_dispatch_without_offer(&transaction, dispatch_id, worker_id, session_id)?;
    if existing.state != RemoteDispatchStateV1::Started
      && existing.state != RemoteDispatchStateV1::CancelRequested
    {
      return Err(RemoteDispatchError::Conflict);
    }
    let previous: (Option<u64>, Option<String>) = transaction.query_row(
      "SELECT last_heartbeat_sequence, last_heartbeat_digest FROM woml_remote_dispatches WHERE dispatch_id=?1",
      [dispatch_id],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if let (Some(previous_sequence), Some(previous_digest)) = previous {
      if previous_sequence == sequence && previous_digest == heartbeat_digest {
        transaction.commit()?;
        return Ok(existing);
      }
      if sequence <= previous_sequence {
        return Err(RemoteDispatchError::Conflict);
      }
    }
    transaction.execute(
      "UPDATE woml_remote_dispatches SET last_heartbeat_at=?2, last_heartbeat_sequence=?3, last_heartbeat_digest=?4, updated_at=?2 WHERE dispatch_id=?1",
      params![dispatch_id, now.to_rfc3339(), sequence, heartbeat_digest],
    )?;
    audit(
      &transaction,
      dispatch_id,
      "heartbeat",
      worker_id,
      &digest_json(&json!({"sequence": sequence, "digest": heartbeat_digest}))?,
      &json!({"sequence": sequence}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  pub fn request_remote_dispatch_cancel(
    &mut self,
    dispatch_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing =
      load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::NotFound)?;
    if existing.state == RemoteDispatchStateV1::CancelRequested {
      transaction.commit()?;
      return Ok(existing);
    }
    if !matches!(
      existing.state,
      RemoteDispatchStateV1::Offered
        | RemoteDispatchStateV1::Accepted
        | RemoteDispatchStateV1::Started
    ) {
      return Err(RemoteDispatchError::Conflict);
    }
    transaction.execute("UPDATE woml_remote_dispatches SET state='cancel_requested', updated_at=?2 WHERE dispatch_id=?1", params![dispatch_id, now.to_rfc3339()])?;
    audit(
      &transaction,
      dispatch_id,
      "cancel_requested",
      "operator",
      &digest_json(&json!({"dispatchId": dispatch_id}))?,
      &json!({}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  pub fn terminal_remote_dispatch(
    &mut self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
    outcome: RemoteDispatchStateV1,
    terminal_digest: &str,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    if !matches!(
      outcome,
      RemoteDispatchStateV1::Succeeded
        | RemoteDispatchStateV1::Failed
        | RemoteDispatchStateV1::Cancelled
    ) || !valid_digest(terminal_digest)
    {
      return Err(RemoteDispatchError::Invalid);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = bound_dispatch_without_offer(&transaction, dispatch_id, worker_id, session_id)?;
    if existing.state == outcome && existing.terminal_digest.as_deref() == Some(terminal_digest) {
      transaction.commit()?;
      return Ok(existing);
    }
    let late_reconciliation = existing.state == RemoteDispatchStateV1::Ambiguous;
    if !matches!(
      existing.state,
      RemoteDispatchStateV1::Started
        | RemoteDispatchStateV1::CancelRequested
        | RemoteDispatchStateV1::Ambiguous
    ) {
      return Err(RemoteDispatchError::Conflict);
    }
    transaction.execute(
      "UPDATE woml_remote_dispatches SET state=?2, terminal_digest=?3, ambiguity_reason=NULL, updated_at=?4 WHERE dispatch_id=?1",
      params![dispatch_id, outcome.as_str(), terminal_digest, now.to_rfc3339()],
    )?;
    audit(
      &transaction,
      dispatch_id,
      outcome.as_str(),
      worker_id,
      terminal_digest,
      &json!({"lateReconciliation": late_reconciliation}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  pub fn recover_remote_dispatches(
    &self,
    limit: usize,
  ) -> Result<Vec<RemoteDispatchRecordV1>, RemoteDispatchError> {
    if limit == 0 || limit > REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT {
      return Err(RemoteDispatchError::Invalid);
    }
    let mut statement = self.connection.prepare(
      "SELECT dispatch_id FROM woml_remote_dispatches
       WHERE state IN ('accepted','started','cancel_requested','ambiguous')
       ORDER BY updated_at, dispatch_id LIMIT ?1",
    )?;
    let ids = statement
      .query_map([limit], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    ids
      .into_iter()
      .map(|id| load_dispatch(&self.connection, &id)?.ok_or(RemoteDispatchError::Corrupt))
      .collect()
  }

  pub fn mark_lost_remote_dispatches(
    &mut self,
    due_before: DateTime<Utc>,
    now: DateTime<Utc>,
    limit: usize,
  ) -> Result<Vec<RemoteDispatchRecordV1>, RemoteDispatchError> {
    if limit == 0 || limit > REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT {
      return Err(RemoteDispatchError::Invalid);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut statement = transaction.prepare(
      "SELECT dispatch_id, state FROM woml_remote_dispatches
       WHERE state IN ('offered','accepted','started','cancel_requested')
         AND COALESCE(last_heartbeat_at, accepted_at, offer_expires_at, updated_at) <= ?1
       ORDER BY updated_at, dispatch_id LIMIT ?2",
    )?;
    let due = statement
      .query_map(params![due_before.to_rfc3339(), limit], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut records = Vec::with_capacity(due.len());
    for (dispatch_id, state) in due {
      if state == "offered" {
        transaction.execute(
          "UPDATE woml_remote_dispatches SET state='requested', worker_id=NULL, session_id=NULL, offer_id=NULL, offer_expires_at=NULL, updated_at=?2 WHERE dispatch_id=?1",
          params![dispatch_id, now.to_rfc3339()],
        )?;
        audit(
          &transaction,
          &dispatch_id,
          "offer_expired",
          "coordinator",
          &digest_json(&json!({"dispatchId": dispatch_id}))?,
          &json!({}),
          now,
        )?;
      } else {
        transaction.execute(
          "UPDATE woml_remote_dispatches SET state='ambiguous', ambiguity_reason='worker_lost_after_acceptance', updated_at=?2 WHERE dispatch_id=?1",
          params![dispatch_id, now.to_rfc3339()],
        )?;
        audit(
          &transaction,
          &dispatch_id,
          "lost",
          "coordinator",
          &digest_json(&json!({"dispatchId": dispatch_id, "previousState": state}))?,
          &json!({"previousState": state}),
          now,
        )?;
      }
      records.push(load_dispatch(&transaction, &dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?);
    }
    transaction.commit()?;
    Ok(records)
  }

  pub fn reconcile_remote_dispatch(
    &mut self,
    dispatch_id: &str,
    resolution: RemoteDispatchResolutionV1,
    actor: &str,
    evidence_digest: &str,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    if !valid_id(actor) || !valid_digest(evidence_digest) {
      return Err(RemoteDispatchError::Invalid);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (state, action, terminal) = match resolution {
      RemoteDispatchResolutionV1::AuthorizePossibleDuplicateRetry => (
        RemoteDispatchStateV1::Abandoned,
        "authorized_possible_duplicate_retry",
        None,
      ),
      RemoteDispatchResolutionV1::RecordFailure => (
        RemoteDispatchStateV1::Failed,
        "reconciled_failed",
        Some(evidence_digest),
      ),
    };
    let existing =
      load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::NotFound)?;
    let prior_request_digest = transaction
      .query_row(
        "SELECT request_digest FROM woml_remote_dispatch_audit
         WHERE dispatch_id=?1 AND action=?2 AND actor=?3
         ORDER BY recorded_at, audit_id LIMIT 1",
        params![dispatch_id, action, actor],
        |row| row.get::<_, String>(0),
      )
      .optional()?;
    if let Some(prior_request_digest) = prior_request_digest {
      if prior_request_digest == evidence_digest {
        transaction.commit()?;
        return Ok(existing);
      }
      return Err(RemoteDispatchError::Conflict);
    }
    if existing.state != RemoteDispatchStateV1::Ambiguous {
      return Err(RemoteDispatchError::Conflict);
    }
    transaction.execute(
      "UPDATE woml_remote_dispatches SET state=?2, terminal_digest=?3, ambiguity_reason=NULL, updated_at=?4 WHERE dispatch_id=?1",
      params![dispatch_id, state.as_str(), terminal, now.to_rfc3339()],
    )?;
    audit(
      &transaction,
      dispatch_id,
      action,
      actor,
      evidence_digest,
      &json!({"resolution": action}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  #[allow(clippy::too_many_arguments)]
  pub(in crate::remote_dispatch) fn transition(
    &mut self,
    dispatch_id: &str,
    allowed: &[RemoteDispatchStateV1],
    state: RemoteDispatchStateV1,
    worker_id: &str,
    session_id: &str,
    offer_id: &str,
    expires_at: Option<DateTime<Utc>>,
    action: &str,
    now: DateTime<Utc>,
    terminal_digest: Option<&str>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing =
      load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::NotFound)?;
    if existing.state == state
      && existing.worker_id.as_deref() == Some(worker_id)
      && existing.session_id.as_deref() == Some(session_id)
      && existing.offer_id.as_deref() == Some(offer_id)
    {
      if existing.offer_expires_at == expires_at
        && existing.terminal_digest.as_deref() == terminal_digest
      {
        transaction.commit()?;
        return Ok(existing);
      }
      return Err(RemoteDispatchError::Conflict);
    }
    if !allowed.contains(&existing.state) {
      return if existing.state == RemoteDispatchStateV1::Ambiguous {
        Err(RemoteDispatchError::Ambiguous)
      } else {
        Err(RemoteDispatchError::Conflict)
      };
    }
    transaction.execute(
      "UPDATE woml_remote_dispatches SET state=?2, worker_id=?3, session_id=?4, offer_id=?5, offer_expires_at=?6, terminal_digest=?7, updated_at=?8 WHERE dispatch_id=?1",
      params![dispatch_id, state.as_str(), worker_id, session_id, offer_id, expires_at.map(|value| value.to_rfc3339()), terminal_digest, now.to_rfc3339()],
    )?;
    audit(
      &transaction,
      dispatch_id,
      action,
      worker_id,
      &digest_json(&json!({"sessionId": session_id, "offerId": offer_id}))?,
      &json!({}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }

  #[allow(clippy::too_many_arguments)]
  pub(in crate::remote_dispatch) fn bound_state_transition(
    &mut self,
    dispatch_id: &str,
    worker_id: &str,
    session_id: &str,
    expected: RemoteDispatchStateV1,
    state: RemoteDispatchStateV1,
    action: &str,
    now: DateTime<Utc>,
  ) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = bound_dispatch_without_offer(&transaction, dispatch_id, worker_id, session_id)?;
    if existing.state == state {
      transaction.commit()?;
      return Ok(existing);
    }
    if existing.state != expected {
      return if existing.state == RemoteDispatchStateV1::Ambiguous {
        Err(RemoteDispatchError::Ambiguous)
      } else {
        Err(RemoteDispatchError::Conflict)
      };
    }
    transaction.execute(
      "UPDATE woml_remote_dispatches SET state=?2, started_at=CASE WHEN ?2='started' THEN ?3 ELSE started_at END, updated_at=?3 WHERE dispatch_id=?1",
      params![dispatch_id, state.as_str(), now.to_rfc3339()],
    )?;
    audit(
      &transaction,
      dispatch_id,
      action,
      worker_id,
      &digest_json(&json!({"sessionId": session_id}))?,
      &json!({}),
      now,
    )?;
    let record = load_dispatch(&transaction, dispatch_id)?.ok_or(RemoteDispatchError::Corrupt)?;
    transaction.commit()?;
    Ok(record)
  }
}

fn same_request(record: &RemoteDispatchRecordV1, request: &RemoteDispatchRequestV1) -> bool {
  record.run_id == request.run_id
    && record.node_id == request.node_id
    && record.iteration_identity == request.iteration_identity
    && record.attempt == request.attempt
    && record.invocation_id == request.invocation_id
    && record.target == request.target
    && record.target_digest == request.target_digest
    && record.package_digest == request.package_digest
}

fn bound_dispatch(
  connection: &Connection,
  dispatch_id: &str,
  worker_id: &str,
  session_id: &str,
  offer_id: &str,
) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
  let record = bound_dispatch_without_offer(connection, dispatch_id, worker_id, session_id)?;
  if record.offer_id.as_deref() != Some(offer_id) {
    return Err(RemoteDispatchError::Conflict);
  }
  Ok(record)
}

fn bound_dispatch_without_offer(
  connection: &Connection,
  dispatch_id: &str,
  worker_id: &str,
  session_id: &str,
) -> Result<RemoteDispatchRecordV1, RemoteDispatchError> {
  let record = load_dispatch(connection, dispatch_id)?.ok_or(RemoteDispatchError::NotFound)?;
  if record.worker_id.as_deref() != Some(worker_id)
    || record.session_id.as_deref() != Some(session_id)
  {
    return Err(RemoteDispatchError::Conflict);
  }
  Ok(record)
}

fn load_dispatch(
  connection: &Connection,
  dispatch_id: &str,
) -> Result<Option<RemoteDispatchRecordV1>, RemoteDispatchError> {
  connection
    .query_row(
      "SELECT dispatch_id, run_id, node_id, iteration_identity, attempt, invocation_id,
              target, target_digest, package_digest, state, worker_id, session_id, offer_id,
              offer_expires_at, accepted_at, started_at, last_heartbeat_at,
              last_heartbeat_sequence, terminal_digest, ambiguity_reason, created_at, updated_at
       FROM woml_remote_dispatches WHERE dispatch_id=?1",
      [dispatch_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, u32>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, String>(6)?,
          row.get::<_, String>(7)?,
          row.get::<_, String>(8)?,
          row.get::<_, String>(9)?,
          row.get::<_, Option<String>>(10)?,
          row.get::<_, Option<String>>(11)?,
          row.get::<_, Option<String>>(12)?,
          row.get::<_, Option<String>>(13)?,
          row.get::<_, Option<String>>(14)?,
          row.get::<_, Option<String>>(15)?,
          row.get::<_, Option<String>>(16)?,
          row.get::<_, Option<u64>>(17)?,
          row.get::<_, Option<String>>(18)?,
          row.get::<_, Option<String>>(19)?,
          row.get::<_, String>(20)?,
          row.get::<_, String>(21)?,
        ))
      },
    )
    .optional()?
    .map(|row| {
      Ok(RemoteDispatchRecordV1 {
        dispatch_id: row.0,
        run_id: row.1,
        node_id: row.2,
        iteration_identity: row.3,
        attempt: row.4,
        invocation_id: row.5,
        target: row.6,
        target_digest: row.7,
        package_digest: row.8,
        state: RemoteDispatchStateV1::parse(&row.9)?,
        worker_id: row.10,
        session_id: row.11,
        offer_id: row.12,
        offer_expires_at: parse_optional_time(row.13)?,
        accepted_at: parse_optional_time(row.14)?,
        started_at: parse_optional_time(row.15)?,
        last_heartbeat_at: parse_optional_time(row.16)?,
        last_heartbeat_sequence: row.17,
        terminal_digest: row.18,
        ambiguity_reason: row.19,
        created_at: parse_time(&row.20)?,
        updated_at: parse_time(&row.21)?,
      })
    })
    .transpose()
}

fn audit(
  transaction: &Transaction<'_>,
  dispatch_id: &str,
  action: &str,
  actor: &str,
  request_digest: &str,
  details: &serde_json::Value,
  now: DateTime<Utc>,
) -> Result<(), RemoteDispatchError> {
  let details = canonical_json(details).map_err(|_| RemoteDispatchError::Invalid)?;
  if details.len() > 16 * 1024 {
    return Err(RemoteDispatchError::Invalid);
  }
  transaction.execute(
    "INSERT INTO woml_remote_dispatch_audit(audit_id, dispatch_id, action, actor, request_digest, details_json, recorded_at)
     VALUES (?1,?2,?3,?4,?5,?6,?7)",
    params![format!("audit_{}", Uuid::new_v4().simple()), dispatch_id, action, actor, request_digest, String::from_utf8(details).map_err(|_| RemoteDispatchError::Invalid)?, now.to_rfc3339()],
  )?;
  Ok(())
}
