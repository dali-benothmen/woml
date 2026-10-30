//! Durable delivery intents, scoped capabilities, attempts, and send outcomes. Uncertain sends must not become automatic safe retries.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::NotificationDeliveryWork;
use crate::durable::NotificationDispatchReport;
use crate::durable::NotificationProviderAdapter;
use crate::durable::NotificationProviderDeliveryResult;
use crate::durable::{approval_credential_expiry, random_hex, DurableEventStore};
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::event::NotificationDeliveryAttemptStartedData;
use crate::event::NotificationDeliveryFailedData;
use crate::event::NotificationDeliverySucceededData;
use crate::event::NotificationRunFailure;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV5;
use crate::fold_events;
use crate::projection::ApprovalRequestStatus;
use crate::projection::NotificationDeliveryStatus;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use crate::RUN_EVENT_SCHEMA_VERSION_V5;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::TransactionBehavior;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

fn notification_idempotency_key(run_id: &str, request_id: &str, delivery_id: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.notification.delivery.v1\0");
  digest.update(run_id.as_bytes());
  digest.update(b"\0");
  digest.update(request_id.as_bytes());
  digest.update(b"\0");
  digest.update(delivery_id.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn next_notification_attempt(
  status: &NotificationDeliveryStatus,
  now: DateTime<Utc>,
) -> Result<Option<u32>, DurableStoreError> {
  match status {
    NotificationDeliveryStatus::Requested => Ok(Some(1)),
    NotificationDeliveryStatus::Failed {
      attempt,
      final_: false,
      failure,
      failed_at,
      ..
    } if failure.retryable && *attempt < 3 => {
      let scheduled_delay = if *attempt == 1 { 1_000 } else { 5_000 };
      let delay_ms = scheduled_delay.max(failure.retry_after_ms.unwrap_or(0));
      let delay = i64::try_from(delay_ms)
        .map(chrono::Duration::milliseconds)
        .map_err(|_| {
          DurableStoreError::Contract("Notification retry delay is invalid.".to_string())
        })?;
      Ok((now >= *failed_at + delay).then_some(*attempt + 1))
    }
    _ => Ok(None),
  }
}

pub(in crate::durable) fn notification_request_all_failed(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  approval_id: &str,
  request_id: &str,
) -> bool {
  let Some(approval) = workflow.approval(approval_id) else {
    return false;
  };
  !approval.notifications.is_empty()
    && approval.notifications.iter().all(|definition| {
      projection
        .notification_deliveries
        .get(&definition.delivery_id)
        .is_some_and(|delivery| {
          delivery.request_id == request_id
            && matches!(
              delivery.status,
              NotificationDeliveryStatus::Failed { final_: true, .. }
            )
        })
    })
}

impl DurableEventStore {
  pub fn begin_notification_delivery(
    &mut self,
    run_id: &str,
    delivery_id: &str,
    now: DateTime<Utc>,
  ) -> Result<NotificationDeliveryWork, DurableStoreError> {
    // Telegram callback_data is limited to 64 bytes. A 64-bit random lookup
    // identity plus a 128-bit secret keeps the full capability (including the
    // approve/reject prefix) within that limit; authority still rests on the
    // secret hash stored by Rust.
    let capability_id = random_hex(8)?;
    let capability_secret = random_hex(16)?;
    let capability_hash = Sha256::digest(capability_secret.as_bytes());
    let attempt_id = format!("nattempt_{}", Uuid::new_v4().simple());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let delivery = projection
      .notification_deliveries
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(format!("Unknown notification delivery {delivery_id:?}."))
      })?;
    let request = projection
      .approval_requests
      .get(&delivery.approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification delivery has no approval request.".to_string())
      })?;
    if projection.status != RunStatus::Waiting
      || !matches!(request.status, ApprovalRequestStatus::Waiting)
    {
      return Err(DurableStoreError::Contract(
        "Notification delivery may start only while its approval is waiting.".to_string(),
      ));
    }
    let attempt = next_notification_attempt(&delivery.status, now)?.ok_or_else(|| {
      DurableStoreError::Contract(
        "Notification delivery is not ready for another attempt.".to_string(),
      )
    })?;
    let approval = workflow.approval(&delivery.approval_id).ok_or_else(|| {
      DurableStoreError::Contract(
        "Notification delivery references an unknown compiled approval.".to_string(),
      )
    })?;
    let definition = approval
      .notifications
      .iter()
      .find(|item| item.delivery_id == delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification delivery is absent from its compiled approval.".to_string(),
        )
      })?;
    let idempotency_key = notification_idempotency_key(run_id, &delivery.request_id, delivery_id);
    let payload =
      RunEventPayload::NotificationDeliveryAttemptStarted(NotificationDeliveryAttemptStartedData {
        approval_id: delivery.approval_id.clone(),
        request_id: delivery.request_id.clone(),
        delivery_id: delivery_id.to_string(),
        attempt,
        attempt_id: attempt_id.clone(),
        idempotency_key: idempotency_key.clone(),
      });
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Notification delivery requires run_started.".to_string())
    })?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      payload,
    )?;
    let credential_expires_at = approval_credential_expiry(now, request.expires_at)?;
    transaction.execute(
      "INSERT INTO woml_notification_capabilities(
         capability_id, secret_hash, attempt_id, request_id, run_id, approval_id,
         delivery_id, issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
      params![
        capability_id,
        capability_hash.as_slice(),
        attempt_id,
        delivery.request_id,
        run_id,
        delivery.approval_id,
        delivery_id,
        now.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(NotificationDeliveryWork {
      run_id: run_id.to_string(),
      approval_id: delivery.approval_id.clone(),
      request_id: delivery.request_id.clone(),
      delivery_id: delivery_id.to_string(),
      provider: definition.provider.clone(),
      destination: definition.destination.clone(),
      credentials: definition.credentials.clone(),
      provider_id: definition.provider_id.clone(),
      message: definition.message.clone(),
      template_name: definition.template_name.clone(),
      language: definition.language.clone(),
      attempt,
      attempt_id,
      idempotency_key,
      decision_capability: format!("ncap_{capability_id}.{capability_secret}"),
    })
  }

  pub fn complete_notification_delivery(
    &mut self,
    work: &NotificationDeliveryWork,
    result: NotificationProviderDeliveryResult,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, &work.run_id)?;
    let workflow = definition_for_run(&transaction, &work.run_id)?;
    let binding = run_binding_in_transaction(&transaction, &work.run_id)?;
    let projection = fold_events(&events)?;
    let delivery = projection
      .notification_deliveries
      .get(&work.delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification result references an unknown delivery.".to_string(),
        )
      })?;
    if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, idempotency_key } if *attempt == work.attempt && attempt_id == &work.attempt_id && idempotency_key == &work.idempotency_key)
    {
      return Err(DurableStoreError::Contract(
        "Notification result does not match the active durable attempt.".to_string(),
      ));
    }
    let payload = match result {
      NotificationProviderDeliveryResult::Succeeded(provider_message) => {
        RunEventPayload::NotificationDeliverySucceeded(NotificationDeliverySucceededData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          provider_message,
        })
      }
      NotificationProviderDeliveryResult::Failed(failure) => {
        let final_ = !failure.retryable
          || work.attempt >= 3
          || matches!(failure.kind.as_str(), "delivery_ambiguous" | "host_crashed");
        RunEventPayload::NotificationDeliveryFailed(NotificationDeliveryFailedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          final_,
          failure,
        })
      }
    };
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      &work.run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    let mut projection = fold_events(&events)?;
    if notification_request_all_failed(&workflow, &projection, &work.approval_id, &work.request_id)
    {
      let failed_delivery_ids = workflow
        .approval(&work.approval_id)
        .unwrap()
        .notifications
        .into_iter()
        .map(|delivery| delivery.delivery_id)
        .collect::<Vec<_>>();
      let payload = RunEventPayload::RunFailed(RunFailedData::V5(RunFailedDataV5::Notification {
        approval_id: work.approval_id.clone(),
        request_id: work.request_id.clone(),
        failed_delivery_ids,
        failure: NotificationRunFailure {
          kind: "all_deliveries_failed".to_string(),
          code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
          message: "Every configured approval notification delivery failed.".to_string(),
        },
      }));
      append_to_history(
        &transaction,
        &mut events,
        &work.run_id,
        generated_event_id(),
        now,
        version,
        payload,
      )?;
      projection = fold_events(&events)?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn dispatch_ready_notifications(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
    adapter: &mut dyn NotificationProviderAdapter,
  ) -> Result<NotificationDispatchReport, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let ready = projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        next_notification_attempt(&delivery.status, now)
          .ok()
          .flatten()
          .is_some()
      })
      .map(|delivery| delivery.delivery_id.clone())
      .collect::<Vec<_>>();
    let mut report = NotificationDispatchReport::default();
    for delivery_id in ready {
      let work = self.begin_notification_delivery(run_id, &delivery_id, now)?;
      let result = adapter.deliver(&work);
      let succeeded = matches!(result, NotificationProviderDeliveryResult::Succeeded(_));
      let projection = self.complete_notification_delivery(&work, result, now)?;
      report.attempted += 1;
      if succeeded {
        report.succeeded += 1;
      } else {
        report.failed += 1;
      }
      report.run_failed = projection.status == RunStatus::Failed;
      if report.run_failed {
        break;
      }
    }
    Ok(report)
  }
}
