//! Private notification runtime builtin.

use crate::durable::NotificationDeliveryWork;
use crate::durable::NotificationProviderDeliveryResult;
use crate::durable::NotificationProviderUpdateResult;
use crate::durable::NotificationUpdateWork;
use crate::notification_host::NotificationHostClientError;
use crate::notification_protocol::NotificationApprovalMessage;
use crate::notification_protocol::NotificationCredentials;
use crate::notification_protocol::NotificationDeliverMessage;
use crate::notification_protocol::NotificationHostOutcome;
use crate::notification_protocol::NotificationUpdateMessage;
use crate::notification_protocol::NOTIFICATION_PROVIDER_PROTOCOL;
use crate::notification_protocol::NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
use crate::notification_runtime::NotificationJourneyError;
use crate::NotificationSafeFailure;
use chrono::Utc;

pub(in crate::notification_runtime) fn delivery_message(
  work: &NotificationDeliveryWork,
  workflow_id: &str,
  approval_name: &str,
  approval_description: Option<&str>,
  expires_at: Option<chrono::DateTime<Utc>>,
) -> Result<NotificationDeliverMessage, NotificationJourneyError> {
  Ok(NotificationDeliverMessage {
    protocol: NOTIFICATION_PROVIDER_PROTOCOL,
    protocol_version: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    message_type: "deliver",
    invocation_id: work.attempt_id.clone(),
    run_id: work.run_id.clone(),
    approval_id: work.approval_id.clone(),
    request_id: work.request_id.clone(),
    delivery_id: work.delivery_id.clone(),
    provider: work.provider.clone(),
    destination: work.destination.clone(),
    idempotency_key: work.idempotency_key.clone(),
    credentials: NotificationCredentials::from_symbolic(&work.credentials)
      .map_err(NotificationJourneyError::Contract)?,
    decision_capability: work.decision_capability.clone(),
    message: NotificationApprovalMessage {
      workflow_id: workflow_id.to_string(),
      approval_name: approval_name.to_string(),
      approval_description: approval_description.map(str::to_string),
      expires_at,
    },
    template_name: work.template_name.clone(),
    language: work.language.clone(),
  })
}

pub(in crate::notification_runtime) fn update_message(
  work: &NotificationUpdateWork,
) -> Result<NotificationUpdateMessage, NotificationJourneyError> {
  Ok(NotificationUpdateMessage {
    protocol: NOTIFICATION_PROVIDER_PROTOCOL,
    protocol_version: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    message_type: "update",
    invocation_id: work.attempt_id.clone(),
    run_id: work.run_id.clone(),
    approval_id: work.approval_id.clone(),
    request_id: work.request_id.clone(),
    delivery_id: work.delivery_id.clone(),
    update_id: work.update_id.clone(),
    idempotency_key: work.idempotency_key.clone(),
    provider: work.provider.clone(),
    credentials: NotificationCredentials::from_symbolic(&work.credentials)
      .map_err(NotificationJourneyError::Contract)?,
    provider_message: work.provider_message.clone(),
    resolution: work.resolution,
  })
}

pub(in crate::notification_runtime) fn delivery_result(
  outcome: NotificationHostOutcome,
) -> NotificationProviderDeliveryResult {
  match outcome {
    NotificationHostOutcome::DeliverySuccess { provider_message } => {
      NotificationProviderDeliveryResult::Succeeded(provider_message)
    }
    NotificationHostOutcome::Failure { error } => NotificationProviderDeliveryResult::Failed(error),
    NotificationHostOutcome::UpdateSuccess => NotificationProviderDeliveryResult::Failed(
      invalid_response_failure("The provider returned an update result for delivery work."),
    ),
  }
}

pub(in crate::notification_runtime) fn update_result(
  outcome: NotificationHostOutcome,
) -> NotificationProviderUpdateResult {
  match outcome {
    NotificationHostOutcome::UpdateSuccess => NotificationProviderUpdateResult::Succeeded,
    NotificationHostOutcome::Failure { error } => NotificationProviderUpdateResult::Failed(error),
    NotificationHostOutcome::DeliverySuccess { .. } => NotificationProviderUpdateResult::Failed(
      invalid_response_failure("The provider returned a delivery result for update work."),
    ),
  }
}

fn invalid_response_failure(message: &str) -> NotificationSafeFailure {
  NotificationSafeFailure {
    kind: "request_invalid".to_string(),
    code: "WOML_NOTIFICATION_RESPONSE_INVALID".to_string(),
    message: message.to_string(),
    retryable: false,
    retry_after_ms: None,
  }
}

pub(in crate::notification_runtime) fn host_failure(
  error: &NotificationHostClientError,
  update: bool,
) -> NotificationSafeFailure {
  let (kind, code, retryable) = match error {
    NotificationHostClientError::Protocol(_) => (
      "request_invalid",
      "WOML_NOTIFICATION_RESPONSE_INVALID",
      false,
    ),
    _ if update => (
      "update_failed",
      "WOML_NOTIFICATION_UPDATE_HOST_CRASHED",
      true,
    ),
    _ => ("host_crashed", "WOML_NOTIFICATION_HOST_CRASHED", false),
  };
  NotificationSafeFailure {
    kind: kind.to_string(),
    code: code.to_string(),
    message: if update {
      "The notification host stopped while updating a provider message.".to_string()
    } else {
      "The notification host stopped during delivery; WOML will not replay an uncertain effect."
        .to_string()
    },
    retryable,
    retry_after_ms: None,
  }
}
