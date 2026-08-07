use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use thiserror::Error;
use tokio::task::JoinSet;

use crate::durable::{
  ApprovalDecisionOutcome, DurableEventStore, DurableStoreError, NotificationDeliveryWork,
  NotificationDispatchReport, NotificationProviderDeliveryResult, NotificationProviderUpdateResult,
  NotificationUpdateWork,
};
use crate::notification_host::{
  NotificationHostClient, NotificationHostClientError, NotificationHostProcessOptions,
};
use crate::notification_protocol::{
  NotificationApprovalMessage, NotificationCredentials, NotificationDeliverMessage,
  NotificationHostOutcome, NotificationUpdateMessage, NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
use crate::{NotificationSafeFailure, RunStatus};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationJourneyResult {
  pub run_id: String,
  pub decision: ApprovalDecisionOutcome,
  pub deliveries: NotificationDispatchReport,
  pub updates: NotificationDispatchReport,
}

#[derive(Debug, Error)]
pub enum NotificationJourneyError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
  #[error(transparent)]
  Host(#[from] NotificationHostClientError),
  #[error("notification provider work is invalid: {0}")]
  Contract(String),
  #[error("every approval notification delivery failed")]
  DeliveryFailed,
}

pub async fn run_notification_provider_journey(
  event_store_path: impl AsRef<Path>,
  run_id: &str,
  host_options: NotificationHostProcessOptions,
  interaction_timeout: Duration,
) -> Result<NotificationJourneyResult, NotificationJourneyError> {
  let mut store = DurableEventStore::open(event_store_path)?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  let projection = store.projection(run_id)?;
  if projection.status != RunStatus::Waiting {
    return Err(NotificationJourneyError::Contract(
      "A provider journey requires a workflow waiting for approval.".to_string(),
    ));
  }
  let (approval_id, request) = projection
    .approval_requests
    .iter()
    .find(|(_, request)| matches!(request.status, crate::ApprovalRequestStatus::Waiting))
    .ok_or_else(|| {
      NotificationJourneyError::Contract(
        "The waiting workflow has no unresolved approval request.".to_string(),
      )
    })?;
  let approval = workflow.approval(approval_id).ok_or_else(|| {
    NotificationJourneyError::Contract("The compiled approval is missing.".to_string())
  })?;
  if approval.notifications.is_empty() {
    return Err(NotificationJourneyError::Contract(
      "The waiting approval has no notification delivery definitions.".to_string(),
    ));
  }

  let client = Arc::new(NotificationHostClient::spawn(host_options).await?);
  let mut delivery_report = NotificationDispatchReport::default();
  let mut delivery_tasks = JoinSet::new();
  for definition in &approval.notifications {
    let work = match store.begin_notification_delivery(run_id, &definition.delivery_id, Utc::now())
    {
      Ok(work) => work,
      Err(DurableStoreError::Contract(_)) => continue,
      Err(error) => return Err(error.into()),
    };
    let message = delivery_message(
      &work,
      &workflow.workflow_id,
      approval.name.as_deref().unwrap_or(approval_id),
      approval.description.as_deref(),
      request.expires_at,
    )?;
    let task_client = Arc::clone(&client);
    delivery_tasks.spawn(async move {
      let result = task_client.invoke(&message.invocation_id, &message).await;
      (work, result)
    });
  }
  while let Some(joined) = delivery_tasks.join_next().await {
    let (work, result) = joined.map_err(|error| {
      NotificationJourneyError::Host(NotificationHostClientError::HostCrashed(error.to_string()))
    })?;
    let provider_result = match result {
      Ok(completed) => delivery_result(completed.outcome),
      Err(error) => NotificationProviderDeliveryResult::Failed(host_failure(&error, false)),
    };
    let succeeded = matches!(
      provider_result,
      NotificationProviderDeliveryResult::Succeeded(_)
    );
    let projection = store.complete_notification_delivery(&work, provider_result, Utc::now())?;
    delivery_report.attempted += 1;
    if succeeded {
      delivery_report.succeeded += 1;
    } else {
      delivery_report.failed += 1;
    }
    delivery_report.run_failed = projection.status == RunStatus::Failed;
  }
  if delivery_report.run_failed {
    shutdown_shared(client).await;
    return Err(NotificationJourneyError::DeliveryFailed);
  }
  if delivery_report.succeeded == 0 {
    shutdown_shared(client).await;
    return Err(NotificationJourneyError::Contract(
      "No notification delivery is currently available for a provider decision.".to_string(),
    ));
  }

  let interaction = client.next_interaction(interaction_timeout).await?;
  let decision = store.resolve_notification_approval_from_provider(
    &interaction.decision_capability,
    &interaction.delivery_id,
    &interaction.provider,
    &interaction.provider_actor_id,
    interaction.decision,
    Utc::now(),
  )?;

  let projection = store.projection(run_id)?;
  let update_ids = projection
    .notification_updates
    .keys()
    .cloned()
    .collect::<Vec<_>>();
  let mut update_tasks = JoinSet::new();
  for delivery_id in update_ids {
    let work = match store.begin_notification_update(run_id, &delivery_id, Utc::now()) {
      Ok(work) => work,
      Err(DurableStoreError::Contract(_)) => continue,
      Err(error) => return Err(error.into()),
    };
    let message = update_message(&work)?;
    let task_client = Arc::clone(&client);
    update_tasks.spawn(async move {
      let result = task_client.invoke(&message.invocation_id, &message).await;
      (work, result)
    });
  }
  let mut update_report = NotificationDispatchReport::default();
  while let Some(joined) = update_tasks.join_next().await {
    let (work, result) = joined.map_err(|error| {
      NotificationJourneyError::Host(NotificationHostClientError::HostCrashed(error.to_string()))
    })?;
    let provider_result = match result {
      Ok(completed) => update_result(completed.outcome),
      Err(error) => NotificationProviderUpdateResult::Failed(host_failure(&error, true)),
    };
    let succeeded = matches!(provider_result, NotificationProviderUpdateResult::Succeeded);
    store.complete_notification_update(&work, provider_result, Utc::now())?;
    update_report.updates_attempted += 1;
    if succeeded {
      update_report.updates_succeeded += 1;
    } else {
      update_report.updates_failed += 1;
    }
  }
  shutdown_shared(client).await;
  Ok(NotificationJourneyResult {
    run_id: run_id.to_string(),
    decision,
    deliveries: delivery_report,
    updates: update_report,
  })
}

fn delivery_message(
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
  })
}

fn update_message(
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

fn delivery_result(outcome: NotificationHostOutcome) -> NotificationProviderDeliveryResult {
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

fn update_result(outcome: NotificationHostOutcome) -> NotificationProviderUpdateResult {
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

fn host_failure(error: &NotificationHostClientError, update: bool) -> NotificationSafeFailure {
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

async fn shutdown_shared(client: Arc<NotificationHostClient>) {
  if let Ok(client) = Arc::try_unwrap(client) {
    client.shutdown().await;
  }
}
