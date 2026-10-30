//! Private notification runtime journey.

use crate::custom_notification_host::CustomNotificationHostClient;
use crate::custom_notification_host::CustomNotificationHostProcessOptions;
use crate::durable::ApprovalTimeoutSettlementStatus;
use crate::durable::DurableEventStore;
use crate::durable::DurableStoreError;
use crate::durable::NotificationDispatchReport;
use crate::durable::NotificationProviderDeliveryResult;
use crate::durable::NotificationProviderUpdateResult;
use crate::execute_reusable_provider_lifecycle_durable;
use crate::notification_host::NotificationHostClient;
use crate::notification_host::NotificationHostClientError;
use crate::notification_host::NotificationHostProcessOptions;
use crate::notification_protocol::NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
use crate::notification_runtime::custom_delivery_message;
use crate::notification_runtime::custom_delivery_result;
use crate::notification_runtime::custom_host_failure;
use crate::notification_runtime::delivery_message;
use crate::notification_runtime::delivery_result;
use crate::notification_runtime::delivery_retry_wait;
use crate::notification_runtime::host_failure;
use crate::notification_runtime::notification_diagnostics;
use crate::notification_runtime::update_message;
use crate::notification_runtime::update_result;
use crate::notification_runtime::update_retry_wait;
use crate::notification_runtime::CustomNotificationJourneyOptions;
use crate::notification_runtime::DeliveryHostResult;
use crate::notification_runtime::NotificationJourneyError;
use crate::notification_runtime::NotificationJourneyResult;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::ApprovalResolution;
use crate::NotificationDeliveryStatus;
use crate::NotificationResolution;
use crate::RunStatus;
use crate::RuntimeExecutionOptions;
use crate::ScriptHostProcessOptions;
use chrono::Utc;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

pub async fn run_notification_provider_journey(
  event_store_path: impl AsRef<Path>,
  run_id: &str,
  host_options: NotificationHostProcessOptions,
  interaction_timeout: Duration,
) -> Result<NotificationJourneyResult, NotificationJourneyError> {
  run_notification_provider_journey_with_custom(
    event_store_path,
    run_id,
    host_options,
    interaction_timeout,
    None,
  )
  .await
}

pub async fn run_notification_provider_journey_with_custom(
  event_store_path: impl AsRef<Path>,
  run_id: &str,
  host_options: NotificationHostProcessOptions,
  interaction_timeout: Duration,
  custom_options: Option<CustomNotificationJourneyOptions>,
) -> Result<NotificationJourneyResult, NotificationJourneyError> {
  let event_store_path = event_store_path.as_ref().to_path_buf();
  let mut store = DurableEventStore::open(&event_store_path)?;
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

  let has_builtin = approval.notifications.iter().any(|definition| {
    matches!(
      definition.provider.as_str(),
      "slack" | "telegram" | "discord" | "whatsapp"
    )
  });
  let has_custom = approval
    .notifications
    .iter()
    .any(|definition| definition.provider == "custom");
  let client = if has_builtin {
    // Runtime execution configures its default provider host for informational
    // lifecycle notifications (v2). Approval delivery is a separate, durable
    // protocol journey and must always start an approval-capable v1 session.
    Some(Arc::new(
      NotificationHostClient::spawn(
        host_options.with_protocol_version(NOTIFICATION_PROVIDER_PROTOCOL_VERSION),
      )
      .await?,
    ))
  } else {
    None
  };
  let custom_client = if has_custom {
    let options = custom_options.as_ref().ok_or_else(|| {
      NotificationJourneyError::Contract(
        "Custom notification provider runtime artifacts are unavailable.".to_string(),
      )
    })?;
    Some(Arc::new(
      CustomNotificationHostClient::spawn(
        CustomNotificationHostProcessOptions::new(
          &options.bun_executable,
          &options.host_script_path,
        )
        .with_artifacts(options.artifacts.clone()),
      )
      .await
      .map_err(|error| NotificationJourneyError::Contract(error.to_string()))?,
    ))
  } else {
    None
  };
  let mut delivery_report = NotificationDispatchReport::default();
  loop {
    let mut delivery_tasks = JoinSet::new();
    for definition in &approval.notifications {
      let work =
        match store.begin_notification_delivery(run_id, &definition.delivery_id, Utc::now()) {
          Ok(work) => work,
          Err(DurableStoreError::Contract(_)) => continue,
          Err(error) => return Err(error.into()),
        };
      if work.provider == "custom" {
        let options = custom_options.as_ref().ok_or_else(|| {
          NotificationJourneyError::Contract("Custom provider options are unavailable.".to_string())
        })?;
        let message = custom_delivery_message(
          &work,
          &workflow,
          &projection.context,
          approval.name.as_deref().unwrap_or(approval_id),
          approval.description.as_deref(),
          options,
        )?;
        let task_client = Arc::clone(custom_client.as_ref().expect("custom host"));
        delivery_tasks.spawn(async move {
          let result = task_client.invoke(&message).await;
          (work, DeliveryHostResult::Custom(result))
        });
      } else {
        let message = delivery_message(
          &work,
          &workflow.workflow_id,
          approval.name.as_deref().unwrap_or(approval_id),
          approval.description.as_deref(),
          request.expires_at,
        )?;
        let task_client = Arc::clone(client.as_ref().expect("built-in provider host"));
        delivery_tasks.spawn(async move {
          let result = task_client.invoke(&message.invocation_id, &message).await;
          (work, DeliveryHostResult::BuiltIn(result))
        });
      }
    }
    while let Some(joined) = delivery_tasks.join_next().await {
      let (work, result) = joined.map_err(|error| {
        NotificationJourneyError::Host(NotificationHostClientError::HostCrashed(error.to_string()))
      })?;
      let provider_result = match result {
        DeliveryHostResult::BuiltIn(Ok(completed)) => delivery_result(completed.outcome),
        DeliveryHostResult::BuiltIn(Err(error)) => {
          NotificationProviderDeliveryResult::Failed(host_failure(&error, false))
        }
        DeliveryHostResult::Custom(Ok(completed)) => {
          custom_delivery_result(&work, completed.outcome)
        }
        DeliveryHostResult::Custom(Err(error)) => {
          NotificationProviderDeliveryResult::Failed(custom_host_failure(&error))
        }
      };
      let succeeded = matches!(
        provider_result,
        NotificationProviderDeliveryResult::Succeeded(_)
      );
      let lifecycle_result = provider_result.clone();
      let projection = store.complete_notification_delivery(&work, provider_result, Utc::now())?;
      delivery_report.attempted += 1;
      if succeeded {
        delivery_report.succeeded += 1;
      } else {
        delivery_report.failed += 1;
      }
      delivery_report.run_failed = projection.status == RunStatus::Failed;
      let delivery_is_final = projection
        .notification_deliveries
        .get(&work.delivery_id)
        .is_some_and(|delivery| {
          matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
              | NotificationDeliveryStatus::Failed { final_: true, .. }
          )
        });
      if work.provider == "custom" && delivery_is_final {
        let provider_id = work.provider_id.as_deref().ok_or_else(|| {
          NotificationJourneyError::Contract(
            "Custom delivery has no provider lifecycle identity.".to_string(),
          )
        })?;
        let custom = custom_options.as_ref().ok_or_else(|| {
          NotificationJourneyError::Contract(
            "Custom provider lifecycle options are unavailable.".to_string(),
          )
        })?;
        let (outcome, result, error) = match lifecycle_result {
          NotificationProviderDeliveryResult::Succeeded(provider_message) => (
            ReusableInvocationOutcome::Succeeded,
            Some(serde_json::json!({
              "messageId": match provider_message {
                crate::ProviderMessageIdentity::Slack(message) => message.message_id,
                crate::ProviderMessageIdentity::Communication(message) => message.message_id,
              }
            })),
            None,
          ),
          NotificationProviderDeliveryResult::Failed(failure) => (
            ReusableInvocationOutcome::Failed,
            None,
            Some(ReusableLifecycleErrorV1 {
              code: failure.code,
              message: failure.message,
            }),
          ),
        };
        execute_reusable_provider_lifecycle_durable(
          event_store_path.clone(),
          run_id,
          provider_id,
          outcome,
          result,
          error,
          RuntimeExecutionOptions::new(
            ScriptHostProcessOptions::new(
              custom.bun_executable.clone(),
              custom.script_host_path.clone(),
            ),
            30_000,
          )
          .with_resolved_secrets(custom.resolved_secrets.clone()),
        )
        .await
        .map_err(|error| NotificationJourneyError::Contract(error.to_string()))?;
      }
    }
    let projection = store.projection(run_id)?;
    if projection.status == RunStatus::Failed {
      delivery_report.run_failed = true;
      break;
    }
    // A successful delivery makes the approval usable, but it must not abandon
    // a retryable sibling provider. Settle every configured destination first
    // so a mixed <notify> has deterministic, restart-safe delivery state.
    let all_deliveries_settled = projection.notification_deliveries.values().all(|delivery| {
      matches!(
        delivery.status,
        NotificationDeliveryStatus::Succeeded { .. }
          | NotificationDeliveryStatus::Failed { final_: true, .. }
      )
    });
    if all_deliveries_settled {
      break;
    }
    let Some(wait) = delivery_retry_wait(&projection, Utc::now()) else {
      break;
    };
    tokio::time::sleep(wait).await;
  }
  if delivery_report.run_failed {
    let diagnostics = notification_diagnostics(&store.projection(run_id)?);
    shutdown_clients(client, custom_client).await;
    return Err(NotificationJourneyError::DeliveryFailed(diagnostics));
  }
  if !store
    .projection(run_id)?
    .notification_deliveries
    .values()
    .any(|delivery| {
      matches!(
        delivery.status,
        NotificationDeliveryStatus::Succeeded { .. }
      )
    })
  {
    shutdown_clients(client, custom_client).await;
    return Err(NotificationJourneyError::Contract(
      "No notification delivery is currently available for a provider decision.".to_string(),
    ));
  }

  let wait_started = std::time::Instant::now();
  let (decision, resolution) = loop {
    let current = store.projection(run_id)?;
    let current_request = current.approval_requests.get(approval_id).ok_or_else(|| {
      NotificationJourneyError::Contract("The approval request disappeared.".to_string())
    })?;
    if let crate::ApprovalRequestStatus::Resolved { resolution, .. } = &current_request.status {
      let resolution = match resolution {
        ApprovalResolution::Decision {
          decision: crate::ApprovalDecision::Approved,
          ..
        } => NotificationResolution::Approved,
        ApprovalResolution::Decision {
          decision: crate::ApprovalDecision::Rejected,
          ..
        } => NotificationResolution::Rejected,
        ApprovalResolution::TimeoutFailure => NotificationResolution::TimeoutFailed,
      };
      break (None, resolution);
    }
    if wait_started.elapsed() >= interaction_timeout {
      let settlement = store.settle_approval_timeout(run_id, approval_id, Utc::now())?;
      if settlement.status == ApprovalTimeoutSettlementStatus::NotDue {
        shutdown_clients(client, custom_client).await;
        return Err(NotificationHostClientError::InteractionTimedOut.into());
      }
      let resolution = match settlement.resolution {
        Some(ApprovalResolution::Decision {
          decision: crate::ApprovalDecision::Approved,
          ..
        }) => NotificationResolution::Approved,
        Some(ApprovalResolution::Decision {
          decision: crate::ApprovalDecision::Rejected,
          ..
        }) => NotificationResolution::Rejected,
        Some(ApprovalResolution::TimeoutFailure) => NotificationResolution::TimeoutFailed,
        None => {
          return Err(NotificationJourneyError::Contract(
            "A settled approval timeout did not provide a resolution.".to_string(),
          ))
        }
      };
      break (None, resolution);
    }
    let Some(slack) = client.as_ref() else {
      tokio::time::sleep(Duration::from_millis(100)).await;
      continue;
    };
    match slack.next_interaction(Duration::from_millis(100)).await {
      Ok(interaction) => {
        let decision = store.resolve_notification_approval_from_provider(
          &interaction.decision_capability,
          &interaction.delivery_id,
          &interaction.provider,
          &interaction.provider_actor_id,
          interaction.decision,
          Utc::now(),
        )?;
        let resolution = match decision.decision {
          crate::ApprovalDecision::Approved => NotificationResolution::Approved,
          crate::ApprovalDecision::Rejected => NotificationResolution::Rejected,
        };
        break (Some(decision), resolution);
      }
      Err(NotificationHostClientError::InteractionTimedOut) => continue,
      Err(error) => return Err(error.into()),
    }
  };

  let mut update_report = NotificationDispatchReport::default();
  loop {
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
      let task_client = Arc::clone(client.as_ref().ok_or_else(|| {
        NotificationJourneyError::Contract("Slack message update host is unavailable.".to_string())
      })?);
      update_tasks.spawn(async move {
        let result = task_client.invoke(&message.invocation_id, &message).await;
        (work, result)
      });
    }
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
    let projection = store.projection(run_id)?;
    let Some(wait) = update_retry_wait(&projection, Utc::now()) else {
      break;
    };
    tokio::time::sleep(wait).await;
  }
  shutdown_clients(client, custom_client).await;
  Ok(NotificationJourneyResult {
    run_id: run_id.to_string(),
    decision,
    resolution,
    deliveries: delivery_report,
    updates: update_report,
    diagnostics: notification_diagnostics(&store.projection(run_id)?),
  })
}

async fn shutdown_shared(client: Arc<NotificationHostClient>) {
  if let Ok(client) = Arc::try_unwrap(client) {
    client.shutdown().await;
  }
}

async fn shutdown_clients(
  slack: Option<Arc<NotificationHostClient>>,
  custom: Option<Arc<CustomNotificationHostClient>>,
) {
  if let Some(slack) = slack {
    shutdown_shared(slack).await;
  }
  if let Some(custom) = custom {
    if let Ok(custom) = Arc::try_unwrap(custom) {
      custom.shutdown().await;
    }
  }
}
