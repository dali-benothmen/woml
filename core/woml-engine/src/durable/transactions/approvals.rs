//! Approval credentials, atomic request/outbox creation, and human/timeout settlement. Decisions retain their exact request binding and terminal race.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::ApprovalDecisionOutcome;
use crate::durable::ApprovalDecisionOutcomeStatus;
use crate::durable::ApprovalTimeoutSettlement;
use crate::durable::ApprovalTimeoutSettlementStatus;
use crate::durable::ApprovalTokenBinding;
use crate::durable::DurableStoreError;
use crate::durable::IssuedApprovalToken;
use crate::durable::{
  approval_credential_expiry, parse_approval_token, parse_stored_timestamp, random_hex,
  DurableEventStore,
};
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::event::ApprovalDecision;
use crate::event::ApprovalDecisionSource;
use crate::event::ApprovalFailure;
use crate::event::ApprovalRequestedData;
use crate::event::ApprovalResolution;
use crate::event::ApprovalResolvedData;
use crate::event::ApprovalTimeoutPolicy;
use crate::event::NotificationDeliveryRequestedData;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV4;
use crate::fold_events;
use crate::projection::ApprovalRequestStatus;
use crate::projection::NotificationDeliveryStatus;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use sha2::Digest;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

impl DurableEventStore {
  pub fn issue_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Cannot issue a credential for an unknown approval request.".to_string(),
        )
      })?;
    if request.request_id != request_id
      || !matches!(request.status, ApprovalRequestStatus::Waiting)
      || projection.status != RunStatus::Waiting
    {
      return Err(DurableStoreError::Contract(
        "Approval credentials may be issued only for the matching unresolved request.".to_string(),
      ));
    }
    let credential_expires_at = approval_credential_expiry(issued_at, request.expires_at)?;

    let token_id = random_hex(16)?;
    let secret = random_hex(32)?;
    let secret_hash = Sha256::digest(secret.as_bytes());
    self.connection.execute(
      "INSERT INTO woml_approval_tokens(
         token_id, secret_hash, request_id, run_id, approval_id,
         issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        token_id,
        secret_hash.as_slice(),
        request_id,
        run_id,
        approval_id,
        issued_at.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    Ok(IssuedApprovalToken {
      token: format!("apr_{token_id}.{secret}"),
      token_id,
      request_id: request_id.to_string(),
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      issued_at,
      credential_expires_at,
    })
  }

  pub fn request_approval_atomically(
    &mut self,
    run_id: &str,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    request: ApprovalRequestedData,
  ) -> Result<(RunProjection, IssuedApprovalToken), DurableStoreError> {
    let token_id = random_hex(16)?;
    let secret = random_hex(32)?;
    let secret_hash = Sha256::digest(secret.as_bytes());
    let credential_expires_at = approval_credential_expiry(occurred_at, request.expires_at)?;

    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "An approval cannot be requested before run_started.".to_string(),
        )
      })?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let notification_definitions = workflow
      .approval(&request.approval_id)
      .map(|approval| approval.notifications)
      .unwrap_or_default();
    let payload = RunEventPayload::ApprovalRequested(request.clone());
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id.into(),
      occurred_at,
      event_schema_version,
      payload,
    )?;
    for notification in notification_definitions {
      let payload =
        RunEventPayload::NotificationDeliveryRequested(NotificationDeliveryRequestedData {
          approval_id: request.approval_id.clone(),
          request_id: request.request_id.clone(),
          delivery_id: notification.delivery_id,
          provider: notification.provider,
          destination: notification.destination,
        });
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        occurred_at,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let folded_request = projection
      .approval_requests
      .get(&request.approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "The approval request did not enter the durable waiting projection.".to_string(),
        )
      })?;
    if folded_request.request_id != request.request_id
      || !matches!(folded_request.status, ApprovalRequestStatus::Waiting)
      || projection.status != RunStatus::Waiting
    {
      return Err(DurableStoreError::Contract(
        "The approval request did not produce the matching waiting state.".to_string(),
      ));
    }

    transaction.execute(
      "INSERT INTO woml_approval_tokens(
         token_id, secret_hash, request_id, run_id, approval_id,
         issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        token_id,
        secret_hash.as_slice(),
        request.request_id,
        run_id,
        request.approval_id,
        occurred_at.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    transaction.commit()?;

    Ok((
      projection,
      IssuedApprovalToken {
        token: format!("apr_{token_id}.{secret}"),
        token_id,
        request_id: request.request_id,
        run_id: run_id.to_string(),
        approval_id: request.approval_id,
        issued_at: occurred_at,
        credential_expires_at,
      },
    ))
  }

  pub fn resolve_human_approval(
    &mut self,
    token: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    let (token_id, secret) = parse_approval_token(token)?;
    let candidate_hash = Sha256::digest(secret.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(Vec<u8>, String, String, String, String)> = transaction
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id, credential_expires_at
         FROM woml_approval_tokens WHERE token_id = ?1",
        [token_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, credential_expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    if stored_hash.len() != 32
      || candidate_hash
        .as_slice()
        .ct_eq(stored_hash.as_slice())
        .unwrap_u8()
        != 1
    {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    let credential_expires_at = parse_stored_timestamp(&credential_expires_at)?;

    let mut events = load_events(&transaction, &run_id)?;
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::Cancelling | RunStatus::Finalizing
    ) || projection.business_outcome == Some(crate::event::BusinessOutcome::Cancelled)
    {
      return Err(DurableStoreError::ApprovalExpired);
    }
    let request = projection
      .approval_requests
      .get(&approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Approval credential references an unknown durable request.".to_string(),
        )
      })?;
    if request.request_id != request_id {
      return Err(DurableStoreError::Contract(
        "Approval credential does not match its durable request.".to_string(),
      ));
    }
    match &request.status {
      ApprovalRequestStatus::Resolved {
        resolution:
          ApprovalResolution::Decision {
            decision: existing,
            source: ApprovalDecisionSource::Human,
          },
        resolved_at,
      } if *existing == decision => {
        return Ok(ApprovalDecisionOutcome {
          status: ApprovalDecisionOutcomeStatus::AlreadyResolved,
          run_id,
          approval_id,
          request_id,
          decision,
          source: ApprovalDecisionSource::Human,
          decided_at: *resolved_at,
        });
      }
      ApprovalRequestStatus::Resolved {
        resolution:
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          },
        ..
      } => return Err(DurableStoreError::ApprovalDecisionConflict),
      ApprovalRequestStatus::Resolved { .. } => {
        return Err(DurableStoreError::ApprovalExpired);
      }
      ApprovalRequestStatus::Waiting => {}
    }
    if request.expires_at.is_some_and(|deadline| now >= deadline) {
      return Err(DurableStoreError::ApprovalExpired);
    }
    if now >= credential_expires_at {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }

    let mut payloads = vec![RunEventPayload::ApprovalResolved(
      ApprovalResolvedData::basic(
        approval_id.clone(),
        request_id.clone(),
        ApprovalResolution::Decision {
          decision,
          source: ApprovalDecisionSource::Human,
        },
      ),
    )];
    let notification_resolution = match decision {
      ApprovalDecision::Approved => crate::event::NotificationResolution::Approved,
      ApprovalDecision::Rejected => crate::event::NotificationResolution::Rejected,
    };
    for delivery in projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        delivery.approval_id == approval_id
          && delivery.request_id == request_id
          && matches!(
            delivery.provider.as_str(),
            "slack" | "telegram" | "discord" | "whatsapp"
          )
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      })
    {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivery.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution: notification_resolution,
        },
      ));
    }
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("Approval resolution requires run_started.".to_string())
      })?;
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        &run_id,
        generated_event_id(),
        now,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(ApprovalDecisionOutcome {
      status: ApprovalDecisionOutcomeStatus::Accepted,
      run_id,
      approval_id,
      request_id,
      decision,
      source: ApprovalDecisionSource::Human,
      decided_at: now,
    })
  }

  pub fn settle_approval_timeout(
    &mut self,
    run_id: &str,
    approval_id: &str,
    now: DateTime<Utc>,
  ) -> Result<ApprovalTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Timeout settlement references an unknown approval request.".to_string(),
        )
      })?;
    if let ApprovalRequestStatus::Resolved {
      resolution,
      resolved_at,
    } = &request.status
    {
      return Ok(ApprovalTimeoutSettlement {
        status: ApprovalTimeoutSettlementStatus::AlreadyResolved,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: Some(resolution.clone()),
        settled_at: Some(*resolved_at),
      });
    }
    let deadline = request
      .expires_at
      .ok_or(DurableStoreError::ApprovalHasNoDeadline)?;
    if now < deadline {
      return Ok(ApprovalTimeoutSettlement {
        status: ApprovalTimeoutSettlementStatus::NotDue,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: None,
      });
    }
    let resolution = match request.on_timeout {
      ApprovalTimeoutPolicy::Reject => ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        source: ApprovalDecisionSource::Timeout,
      },
      ApprovalTimeoutPolicy::Fail => ApprovalResolution::TimeoutFailure,
    };
    let mut payloads = vec![RunEventPayload::ApprovalResolved(
      ApprovalResolvedData::basic(
        approval_id.to_string(),
        request.request_id.clone(),
        resolution.clone(),
      ),
    )];
    let notification_resolution = match &resolution {
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        ..
      } => crate::event::NotificationResolution::Rejected,
      ApprovalResolution::TimeoutFailure => crate::event::NotificationResolution::TimeoutFailed,
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Approved,
        ..
      } => crate::event::NotificationResolution::Approved,
    };
    for delivery in projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        delivery.approval_id == approval_id
          && delivery.request_id == request.request_id
          && matches!(
            delivery.provider.as_str(),
            "slack" | "telegram" | "discord" | "whatsapp"
          )
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      })
    {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.to_string(),
          request_id: request.request_id.clone(),
          delivery_id: delivery.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution: notification_resolution,
        },
      ));
    }
    if request.on_timeout == ApprovalTimeoutPolicy::Fail {
      payloads.push(RunEventPayload::RunFailed(RunFailedData::V4(
        RunFailedDataV4::Approval {
          approval_id: approval_id.to_string(),
          request_id: request.request_id.clone(),
          failure: ApprovalFailure {
            kind: "approval_timeout".to_string(),
            code: "WOML_APPROVAL_TIMEOUT".to_string(),
            message: format!("Approval {approval_id:?} reached its deadline."),
          },
        },
      )));
    }
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("Approval timeout requires run_started.".to_string())
      })?;
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    fold_events(&events)?;
    transaction.commit()?;
    Ok(ApprovalTimeoutSettlement {
      status: ApprovalTimeoutSettlementStatus::Settled,
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: Some(resolution),
      settled_at: Some(now),
    })
  }

  pub fn verify_approval_token(
    &self,
    token: &str,
    now: DateTime<Utc>,
  ) -> Result<ApprovalTokenBinding, DurableStoreError> {
    let (token_id, secret) = parse_approval_token(token)?;
    let row: Option<(Vec<u8>, String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id,
                issued_at, credential_expires_at
         FROM woml_approval_tokens WHERE token_id = ?1",
        [token_id],
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
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, issued_at, expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    let candidate_hash = Sha256::digest(secret.as_bytes());
    if stored_hash.len() != 32
      || candidate_hash
        .as_slice()
        .ct_eq(stored_hash.as_slice())
        .unwrap_u8()
        != 1
    {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    let issued_at = parse_stored_timestamp(&issued_at)?;
    let credential_expires_at = parse_stored_timestamp(&expires_at)?;
    if now >= credential_expires_at {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }
    let projection = self.projection(&run_id)?;
    let request_is_waiting =
      projection
        .approval_requests
        .get(&approval_id)
        .is_some_and(|request| {
          request.request_id == request_id
            && matches!(request.status, ApprovalRequestStatus::Waiting)
        });
    if projection.status != RunStatus::Waiting || !request_is_waiting {
      return Err(DurableStoreError::ApprovalExpired);
    }
    Ok(ApprovalTokenBinding {
      token_id: token_id.to_string(),
      request_id,
      run_id,
      approval_id,
      issued_at,
      credential_expires_at,
    })
  }

  pub fn reissue_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableStoreError> {
    self.issue_approval_token(run_id, approval_id, request_id, issued_at)
  }

  pub fn approval_token_count_for_request(
    &self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
  ) -> Result<usize, DurableStoreError> {
    let count: i64 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_approval_tokens
       WHERE run_id = ?1 AND approval_id = ?2 AND request_id = ?3",
      params![run_id, approval_id, request_id],
      |row| row.get(0),
    )?;
    usize::try_from(count)
      .map_err(|_| DurableStoreError::Contract("Approval credential count is invalid.".to_string()))
  }
}
