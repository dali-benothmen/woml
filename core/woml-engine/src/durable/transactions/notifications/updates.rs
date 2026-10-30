//! Durable known-message update attempts and bounded outcome handling, distinct from sending a new notification.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableEventStore;
use crate::durable::DurableStoreError;
use crate::durable::NotificationDispatchReport;
use crate::durable::NotificationProviderAdapter;
use crate::durable::NotificationProviderUpdateResult;
use crate::durable::NotificationUpdateWork;
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::event::NotificationMessageUpdateAttemptStartedData;
use crate::event::NotificationMessageUpdateFailedData;
use crate::event::NotificationMessageUpdatedData;
use crate::fold_events;
use crate::projection::NotificationDeliveryStatus;
use crate::projection::NotificationMessageUpdateStatus;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RUN_EVENT_SCHEMA_VERSION_V5;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::TransactionBehavior;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

fn notification_update_idempotency_key(
  run_id: &str,
  request_id: &str,
  delivery_id: &str,
  update_id: &str,
) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.notification.update.v1\0");
  digest.update(run_id.as_bytes());
  digest.update(b"\0");
  digest.update(request_id.as_bytes());
  digest.update(b"\0");
  digest.update(delivery_id.as_bytes());
  digest.update(b"\0");
  digest.update(update_id.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn next_notification_update_attempt(
  status: &NotificationMessageUpdateStatus,
  now: DateTime<Utc>,
) -> Result<Option<u32>, DurableStoreError> {
  match status {
    NotificationMessageUpdateStatus::Requested => Ok(Some(1)),
    NotificationMessageUpdateStatus::Failed {
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
          DurableStoreError::Contract("Notification update retry delay is invalid.".to_string())
        })?;
      Ok((now >= *failed_at + delay).then_some(*attempt + 1))
    }
    _ => Ok(None),
  }
}

impl DurableEventStore {
  pub fn begin_notification_update(
    &mut self,
    run_id: &str,
    delivery_id: &str,
    now: DateTime<Utc>,
  ) -> Result<NotificationUpdateWork, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let update = projection
      .notification_updates
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Unknown notification message update.".to_string())
      })?;
    let attempt = next_notification_update_attempt(&update.status, now)?.ok_or_else(|| {
      DurableStoreError::Contract("Notification message update is not ready.".to_string())
    })?;
    let delivery = projection
      .notification_deliveries
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification update has no delivery.".to_string())
      })?;
    let NotificationDeliveryStatus::Succeeded {
      provider_message, ..
    } = &delivery.status
    else {
      return Err(DurableStoreError::Contract(
        "Notification update requires a successful message.".to_string(),
      ));
    };
    let approval = workflow.approval(&update.approval_id).ok_or_else(|| {
      DurableStoreError::Contract("Notification update has no compiled approval.".to_string())
    })?;
    let definition = approval
      .notifications
      .iter()
      .find(|item| item.delivery_id == delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification update has no compiled delivery.".to_string())
      })?;
    let attempt_id = format!("nattempt_{}", Uuid::new_v4().simple());
    let idempotency_key = notification_update_idempotency_key(
      run_id,
      &update.request_id,
      delivery_id,
      &update.update_id,
    );
    let payload = RunEventPayload::NotificationMessageUpdateAttemptStarted(
      NotificationMessageUpdateAttemptStartedData {
        approval_id: update.approval_id.clone(),
        request_id: update.request_id.clone(),
        delivery_id: delivery_id.to_string(),
        update_id: update.update_id.clone(),
        attempt,
        attempt_id: attempt_id.clone(),
      },
    );
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let work = NotificationUpdateWork {
      run_id: run_id.to_string(),
      approval_id: update.approval_id.clone(),
      request_id: update.request_id.clone(),
      delivery_id: delivery_id.to_string(),
      provider: definition.provider.clone(),
      credentials: definition.credentials.clone(),
      provider_message: provider_message.clone(),
      resolution: update.resolution,
      update_id: update.update_id.clone(),
      idempotency_key,
      attempt,
      attempt_id,
    };
    transaction.commit()?;
    Ok(work)
  }

  pub fn complete_notification_update(
    &mut self,
    work: &NotificationUpdateWork,
    result: NotificationProviderUpdateResult,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, &work.run_id)?;
    let workflow = definition_for_run(&transaction, &work.run_id)?;
    let binding = run_binding_in_transaction(&transaction, &work.run_id)?;
    let projection = fold_events(&events)?;
    let update = projection
      .notification_updates
      .get(&work.delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Unknown active notification update.".to_string())
      })?;
    if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == work.attempt && attempt_id == &work.attempt_id)
    {
      return Err(DurableStoreError::Contract(
        "Notification update result does not match its active attempt.".to_string(),
      ));
    }
    let payload = match result {
      NotificationProviderUpdateResult::Succeeded => {
        RunEventPayload::NotificationMessageUpdated(NotificationMessageUpdatedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          update_id: work.update_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
        })
      }
      NotificationProviderUpdateResult::Failed(failure) => {
        let final_ = !failure.retryable || work.attempt >= 3;
        RunEventPayload::NotificationMessageUpdateFailed(NotificationMessageUpdateFailedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          update_id: work.update_id.clone(),
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
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn dispatch_ready_notification_updates(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
    adapter: &mut dyn NotificationProviderAdapter,
  ) -> Result<NotificationDispatchReport, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let ready = projection
      .notification_updates
      .values()
      .filter(|update| {
        next_notification_update_attempt(&update.status, now)
          .ok()
          .flatten()
          .is_some()
      })
      .map(|update| update.delivery_id.clone())
      .collect::<Vec<_>>();
    let mut report = NotificationDispatchReport::default();
    for delivery_id in ready {
      let work = self.begin_notification_update(run_id, &delivery_id, now)?;
      let result = adapter.update(&work);
      let succeeded = matches!(result, NotificationProviderUpdateResult::Succeeded);
      self.complete_notification_update(&work, result, now)?;
      report.updates_attempted += 1;
      if succeeded {
        report.updates_succeeded += 1;
      } else {
        report.updates_failed += 1;
      }
    }
    Ok(report)
  }
}
