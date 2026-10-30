//! projection fold notifications.

use super::{
  ApprovalRequestStatus, FoldError, NotificationDeliveryProjection, NotificationDeliveryStatus,
  NotificationMessageUpdateProjection, NotificationMessageUpdateStatus, RunProjection, RunStatus,
};
use crate::event::RunEvent;

pub(super) fn apply_notification_delivery_requested(
  projection: &mut RunProjection,
  data: &crate::event::NotificationDeliveryRequestedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting {
    return Err(FoldError::InvalidHistory(
      "notification_delivery_requested requires a waiting approval.".to_string(),
    ));
  }
  let request = projection
    .approval_requests
    .get(&data.approval_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Notification delivery references an unknown approval.".to_string())
    })?;
  if request.request_id != data.request_id
    || !matches!(request.status, ApprovalRequestStatus::Waiting)
    || projection
      .notification_deliveries
      .contains_key(&data.delivery_id)
  {
    return Err(FoldError::InvalidHistory(
      "Notification delivery intent does not match the active approval request.".to_string(),
    ));
  }
  projection.notification_deliveries.insert(
    data.delivery_id.clone(),
    NotificationDeliveryProjection {
      approval_id: data.approval_id.clone(),
      request_id: data.request_id.clone(),
      delivery_id: data.delivery_id.clone(),
      provider: data.provider.clone(),
      destination: data.destination.clone(),
      status: NotificationDeliveryStatus::Requested,
    },
  );
  Ok(())
}

pub(super) fn apply_notification_delivery_attempt_started(
  projection: &mut RunProjection,
  data: &crate::event::NotificationDeliveryAttemptStartedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting {
    return Err(FoldError::InvalidHistory(
      "Notification delivery attempts require a waiting approval.".to_string(),
    ));
  }
  let delivery = projection
    .notification_deliveries
    .get_mut(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(
        "Notification attempt started before its durable intent.".to_string(),
      )
    })?;
  if delivery.approval_id != data.approval_id || delivery.request_id != data.request_id {
    return Err(FoldError::InvalidHistory(
      "Notification attempt identity does not match its intent.".to_string(),
    ));
  }
  let expected_attempt = match &delivery.status {
    NotificationDeliveryStatus::Requested => 1,
    NotificationDeliveryStatus::Failed {
      attempt,
      final_: false,
      failure,
      ..
    } if failure.retryable => attempt + 1,
    _ => 0,
  };
  if data.attempt != expected_attempt {
    return Err(FoldError::InvalidHistory(
      "Notification attempt is duplicated or out of order.".to_string(),
    ));
  }
  delivery.status = NotificationDeliveryStatus::AttemptStarted {
    attempt: data.attempt,
    attempt_id: data.attempt_id.clone(),
    idempotency_key: data.idempotency_key.clone(),
  };
  Ok(())
}

pub(super) fn apply_notification_delivery_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::NotificationDeliverySucceededData,
) -> Result<(), FoldError> {
  let delivery = projection
    .notification_deliveries
    .get_mut(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Notification success has no durable intent.".to_string())
    })?;
  if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, .. } if *attempt == data.attempt && attempt_id == &data.attempt_id)
  {
    return Err(FoldError::InvalidHistory(
      "Notification success does not close its active attempt.".to_string(),
    ));
  }
  delivery.status = NotificationDeliveryStatus::Succeeded {
    attempt: data.attempt,
    attempt_id: data.attempt_id.clone(),
    provider_message: data.provider_message.clone(),
  };
  Ok(())
}

pub(super) fn apply_notification_delivery_failed(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::NotificationDeliveryFailedData,
) -> Result<(), FoldError> {
  let delivery = projection
    .notification_deliveries
    .get_mut(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Notification failure has no durable intent.".to_string())
    })?;
  if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, .. } if *attempt == data.attempt && attempt_id == &data.attempt_id)
  {
    return Err(FoldError::InvalidHistory(
      "Notification failure does not close its active attempt.".to_string(),
    ));
  }
  delivery.status = NotificationDeliveryStatus::Failed {
    attempt: data.attempt,
    attempt_id: data.attempt_id.clone(),
    final_: data.final_,
    failure: data.failure.clone(),
    failed_at: event.occurred_at,
  };
  Ok(())
}

pub(super) fn apply_notification_decision_accepted(
  projection: &mut RunProjection,
  data: &crate::event::NotificationDecisionAcceptedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting || !projection.notification_decisions.is_empty() {
    return Err(FoldError::InvalidHistory(
      "Only one notification decision may be accepted while waiting.".to_string(),
    ));
  }
  let delivery = projection
    .notification_deliveries
    .get(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Notification decision references an unknown delivery.".to_string())
    })?;
  if delivery.approval_id != data.approval_id
    || delivery.request_id != data.request_id
    || !matches!(
      delivery.status,
      NotificationDeliveryStatus::Succeeded { .. }
    )
  {
    return Err(FoldError::InvalidHistory(
      "Notification decision requires a successful delivery for the active request.".to_string(),
    ));
  }
  projection.notification_decisions.push(data.clone());
  Ok(())
}

pub(super) fn apply_notification_message_update_requested(
  projection: &mut RunProjection,
  data: &crate::event::NotificationMessageUpdateRequestedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Running {
    return Err(FoldError::InvalidHistory(
      "Message update work requires a resolved approval.".to_string(),
    ));
  }
  let delivery = projection
    .notification_deliveries
    .get(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Message update references an unknown delivery.".to_string())
    })?;
  if delivery.approval_id != data.approval_id
    || delivery.request_id != data.request_id
    || !matches!(
      delivery.status,
      NotificationDeliveryStatus::Succeeded { .. }
    )
    || projection
      .notification_updates
      .contains_key(&data.delivery_id)
  {
    return Err(FoldError::InvalidHistory(
      "Message update intent does not match a successful delivery.".to_string(),
    ));
  }
  projection.notification_updates.insert(
    data.delivery_id.clone(),
    NotificationMessageUpdateProjection {
      approval_id: data.approval_id.clone(),
      request_id: data.request_id.clone(),
      delivery_id: data.delivery_id.clone(),
      update_id: data.update_id.clone(),
      resolution: data.resolution,
      status: NotificationMessageUpdateStatus::Requested,
    },
  );
  Ok(())
}

pub(super) fn apply_notification_message_update_attempt_started(
  projection: &mut RunProjection,
  data: &crate::event::NotificationMessageUpdateAttemptStartedData,
) -> Result<(), FoldError> {
  let update = projection
    .notification_updates
    .get_mut(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Message update attempt started before its intent.".to_string())
    })?;
  let expected = match &update.status {
    NotificationMessageUpdateStatus::Requested => 1,
    NotificationMessageUpdateStatus::Failed {
      attempt,
      final_: false,
      failure,
      ..
    } if failure.retryable => attempt + 1,
    _ => 0,
  };
  if update.update_id != data.update_id || data.attempt != expected {
    return Err(FoldError::InvalidHistory(
      "Message update attempt is duplicated or out of order.".to_string(),
    ));
  }
  update.status = NotificationMessageUpdateStatus::AttemptStarted {
    attempt: data.attempt,
    attempt_id: data.attempt_id.clone(),
  };
  Ok(())
}

pub(super) fn apply_notification_message_updated(
  projection: &mut RunProjection,
  data: &crate::event::NotificationMessageUpdatedData,
) -> Result<(), FoldError> {
  let update = projection
    .notification_updates
    .get_mut(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Message update success has no intent.".to_string())
    })?;
  if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == data.attempt && attempt_id == &data.attempt_id)
  {
    return Err(FoldError::InvalidHistory(
      "Message update success does not close its attempt.".to_string(),
    ));
  }
  update.status = NotificationMessageUpdateStatus::Updated {
    attempt: data.attempt,
    attempt_id: data.attempt_id.clone(),
  };
  Ok(())
}

pub(super) fn apply_notification_message_update_failed(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::NotificationMessageUpdateFailedData,
) -> Result<(), FoldError> {
  let update = projection
    .notification_updates
    .get_mut(&data.delivery_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Message update failure has no intent.".to_string())
    })?;
  if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == data.attempt && attempt_id == &data.attempt_id)
  {
    return Err(FoldError::InvalidHistory(
      "Message update failure does not close its attempt.".to_string(),
    ));
  }
  update.status = NotificationMessageUpdateStatus::Failed {
    attempt: data.attempt,
    attempt_id: data.attempt_id.clone(),
    final_: data.final_,
    failure: data.failure.clone(),
    failed_at: event.occurred_at,
  };
  Ok(())
}
