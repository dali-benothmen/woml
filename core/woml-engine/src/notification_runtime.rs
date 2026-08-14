use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use thiserror::Error;
use tokio::task::JoinSet;

use sha2::{Digest, Sha256};

use crate::custom_notification_host::{
  CustomNotificationHostClient, CustomNotificationHostClientError,
  CustomNotificationHostProcessOptions, CustomProviderScriptArtifact,
};
use crate::custom_notification_provider_protocol::{
  CustomNotificationAction, CustomNotificationActions, CustomNotificationKind,
  CustomNotificationRequest, CustomProviderAttempt, CustomProviderExecuteMessage,
  CustomProviderLimits, CustomProviderOutcome, CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
use crate::durable::{
  ApprovalDecisionOutcome, ApprovalTimeoutSettlementStatus, DurableEventStore, DurableStoreError,
  NotificationDeliveryWork, NotificationDispatchReport, NotificationProviderDeliveryResult,
  NotificationProviderUpdateResult, NotificationUpdateWork,
};
use crate::model::{
  CompiledReusableInvocation, CompiledReusablePropExpression, TemplatePart, ValueExpression,
};
use crate::notification_host::{
  NotificationHostClient, NotificationHostClientError, NotificationHostProcessOptions,
};
use crate::notification_protocol::{
  NotificationApprovalMessage, NotificationCredentials, NotificationDeliverMessage,
  NotificationHostOutcome, NotificationUpdateMessage, NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
use crate::{
  ApprovalResolution, NotificationDeliveryStatus, NotificationMessageUpdateStatus,
  NotificationResolution, NotificationSafeFailure, RunStatus,
};

#[derive(Debug, Clone)]
pub struct CustomNotificationJourneyOptions {
  pub bun_executable: PathBuf,
  pub host_script_path: PathBuf,
  pub approval_base_url: String,
  pub resolved_secrets: BTreeMap<String, String>,
  pub artifacts: Vec<CustomProviderScriptArtifact>,
}

enum DeliveryHostResult {
  Slack(Result<crate::NotificationCompletedMessage, NotificationHostClientError>),
  Custom(Result<crate::CustomProviderCompletedMessage, CustomNotificationHostClientError>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationJourneyResult {
  pub run_id: String,
  pub decision: Option<ApprovalDecisionOutcome>,
  pub resolution: NotificationResolution,
  pub deliveries: NotificationDispatchReport,
  pub updates: NotificationDispatchReport,
  pub diagnostics: NotificationJourneyDiagnostics,
}

pub const NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDeliveryDiagnostic {
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub attempt: u32,
  #[serde(rename = "final")]
  pub final_: bool,
  pub failure: NotificationSafeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationJourneyDiagnostics {
  pub version: u32,
  pub delivery_failures: Vec<NotificationDeliveryDiagnostic>,
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
  DeliveryFailed(NotificationJourneyDiagnostics),
}

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

  let has_slack = approval
    .notifications
    .iter()
    .any(|definition| definition.provider == "slack");
  let has_custom = approval
    .notifications
    .iter()
    .any(|definition| definition.provider == "custom");
  let client = if has_slack {
    Some(Arc::new(NotificationHostClient::spawn(host_options).await?))
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
        let task_client = Arc::clone(client.as_ref().expect("Slack host"));
        delivery_tasks.spawn(async move {
          let result = task_client.invoke(&message.invocation_id, &message).await;
          (work, DeliveryHostResult::Slack(result))
        });
      }
    }
    while let Some(joined) = delivery_tasks.join_next().await {
      let (work, result) = joined.map_err(|error| {
        NotificationJourneyError::Host(NotificationHostClientError::HostCrashed(error.to_string()))
      })?;
      let provider_result = match result {
        DeliveryHostResult::Slack(Ok(completed)) => delivery_result(completed.outcome),
        DeliveryHostResult::Slack(Err(error)) => {
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
      let projection = store.complete_notification_delivery(&work, provider_result, Utc::now())?;
      delivery_report.attempted += 1;
      if succeeded {
        delivery_report.succeeded += 1;
      } else {
        delivery_report.failed += 1;
      }
      delivery_report.run_failed = projection.status == RunStatus::Failed;
    }
    let projection = store.projection(run_id)?;
    if projection.status == RunStatus::Failed {
      delivery_report.run_failed = true;
      break;
    }
    if projection.notification_deliveries.values().any(|delivery| {
      matches!(
        delivery.status,
        NotificationDeliveryStatus::Succeeded { .. }
      )
    }) {
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

fn notification_diagnostics(projection: &crate::RunProjection) -> NotificationJourneyDiagnostics {
  let mut delivery_failures = projection
    .notification_deliveries
    .values()
    .filter_map(|delivery| {
      let NotificationDeliveryStatus::Failed {
        attempt,
        final_,
        failure,
        ..
      } = &delivery.status
      else {
        return None;
      };
      Some(NotificationDeliveryDiagnostic {
        delivery_id: delivery.delivery_id.clone(),
        provider: delivery.provider.clone(),
        destination: delivery.destination.clone(),
        attempt: *attempt,
        final_: *final_,
        failure: failure.clone(),
      })
    })
    .collect::<Vec<_>>();
  delivery_failures.sort_by(|left, right| left.delivery_id.cmp(&right.delivery_id));
  NotificationJourneyDiagnostics {
    version: NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION,
    delivery_failures,
  }
}

fn delivery_retry_wait(
  projection: &crate::RunProjection,
  now: chrono::DateTime<Utc>,
) -> Option<Duration> {
  projection
    .notification_deliveries
    .values()
    .filter_map(|delivery| match &delivery.status {
      NotificationDeliveryStatus::Failed {
        attempt,
        final_: false,
        failure,
        failed_at,
        ..
      } if failure.retryable && *attempt < 3 => {
        let scheduled = if *attempt == 1 { 1_000 } else { 5_000 };
        let delay = scheduled.max(failure.retry_after_ms.unwrap_or(0));
        let delay = i64::try_from(delay).unwrap_or(i64::MAX);
        let due = *failed_at + chrono::Duration::milliseconds(delay);
        Some((due - now).to_std().unwrap_or(Duration::ZERO))
      }
      _ => None,
    })
    .min()
}

fn update_retry_wait(
  projection: &crate::RunProjection,
  now: chrono::DateTime<Utc>,
) -> Option<Duration> {
  projection
    .notification_updates
    .values()
    .filter_map(|update| match &update.status {
      NotificationMessageUpdateStatus::Failed {
        attempt,
        final_: false,
        failure,
        failed_at,
        ..
      } if failure.retryable && *attempt < 3 => {
        let scheduled = if *attempt == 1 { 1_000 } else { 5_000 };
        let delay = scheduled.max(failure.retry_after_ms.unwrap_or(0));
        let delay = i64::try_from(delay).unwrap_or(i64::MAX);
        let due = *failed_at + chrono::Duration::milliseconds(delay);
        Some((due - now).to_std().unwrap_or(Duration::ZERO))
      }
      _ => None,
    })
    .min()
}

fn custom_provider_descriptor<'a>(
  workflow: &'a crate::CompiledWorkflowDefinition,
  provider_id: &str,
) -> Option<&'a CompiledReusableInvocation> {
  workflow
    .reusable_definitions
    .iter()
    .flatten()
    .find(|definition| {
      matches!(definition, CompiledReusableInvocation::NotificationProvider { provider_id: id, .. } if id == provider_id)
    })
}

fn context_prop_value(path: &str, context: &crate::WorkflowContext) -> Option<serde_json::Value> {
  let mut segments = path.split('.');
  let root = match segments.next()? {
    "payload" => serde_json::Value::Object(context.trigger.clone()),
    "steps" => serde_json::Value::Object(context.steps.clone()),
    _ => return None,
  };
  let mut value = root;
  for segment in segments {
    value = value.as_object()?.get(segment)?.clone();
  }
  Some(value)
}

fn custom_provider_props(
  descriptor: &CompiledReusableInvocation,
  context: &crate::WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, serde_json::Value>, NotificationJourneyError> {
  let CompiledReusableInvocation::NotificationProvider { props, .. } = descriptor else {
    return Err(NotificationJourneyError::Contract(
      "Custom delivery references a non-provider descriptor.".to_string(),
    ));
  };
  props
    .iter()
    .map(|prop| {
      let value = match &prop.expression {
        CompiledReusablePropExpression::Literal { value } => {
          serde_json::Value::String(value.clone())
        }
        CompiledReusablePropExpression::Context { path } => context_prop_value(path, context)
          .ok_or_else(|| {
            NotificationJourneyError::Contract(format!(
              "Custom provider prop {:?} is unavailable at delivery time.",
              prop.name
            ))
          })?,
        CompiledReusablePropExpression::Secret { name } => serde_json::Value::String(
          secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| {
              NotificationJourneyError::Contract(format!(
                "Custom provider requires unresolved secret {name:?}."
              ))
            })?,
        ),
      };
      Ok((prop.binding_name.clone(), value))
    })
    .collect()
}

fn render_custom_message(
  expression: Option<&ValueExpression>,
  context: &crate::WorkflowContext,
  fallback_name: &str,
  fallback_description: Option<&str>,
) -> Result<String, NotificationJourneyError> {
  let Some(ValueExpression::Template { parts }) = expression else {
    return Ok(
      fallback_description
        .map(|description| format!("{fallback_name}\n{description}"))
        .unwrap_or_else(|| fallback_name.to_string()),
    );
  };
  let mut message = String::new();
  for part in parts {
    match part {
      TemplatePart::Text { text } => message.push_str(text),
      TemplatePart::ContextReference { path } => {
        let path = path.join(".");
        let normalized = path
          .strip_prefix("trigger.")
          .map_or(path.as_str(), |value| {
            // Compiled context.payload references use the historical internal
            // trigger root without exposing that detail to provider authors.
            // The helper below accepts payload.
            value
          });
        let lookup = if path.starts_with("trigger.") {
          format!("payload.{normalized}")
        } else {
          path
        };
        let value = context_prop_value(&lookup, context).ok_or_else(|| {
          NotificationJourneyError::Contract(
            "Custom provider message references unavailable context.".to_string(),
          )
        })?;
        match value {
          serde_json::Value::Null => message.push_str("null"),
          serde_json::Value::Bool(value) => message.push_str(&value.to_string()),
          serde_json::Value::Number(value) => message.push_str(&value.to_string()),
          serde_json::Value::String(value) => message.push_str(&value),
          _ => {
            return Err(NotificationJourneyError::Contract(
              "Custom provider messages may render scalar context values only.".to_string(),
            ))
          }
        }
      }
      TemplatePart::LifecycleReference { .. } => {
        return Err(NotificationJourneyError::Contract(
          "Approval provider messages cannot read lifecycle context.".to_string(),
        ))
      }
    }
  }
  if message.is_empty() || message.chars().count() > 16_384 {
    return Err(NotificationJourneyError::Contract(
      "Custom provider message is empty or exceeds 16384 characters.".to_string(),
    ));
  }
  Ok(message)
}

fn custom_delivery_message(
  work: &NotificationDeliveryWork,
  workflow: &crate::CompiledWorkflowDefinition,
  context: &crate::WorkflowContext,
  approval_name: &str,
  approval_description: Option<&str>,
  options: &CustomNotificationJourneyOptions,
) -> Result<CustomProviderExecuteMessage, NotificationJourneyError> {
  let provider_id = work.provider_id.as_deref().ok_or_else(|| {
    NotificationJourneyError::Contract("Custom delivery has no provider identity.".to_string())
  })?;
  let descriptor = custom_provider_descriptor(workflow, provider_id).ok_or_else(|| {
    NotificationJourneyError::Contract("Custom delivery descriptor is unavailable.".to_string())
  })?;
  let CompiledReusableInvocation::NotificationProvider {
    definition_digest,
    script_artifact_id,
    ..
  } = descriptor
  else {
    unreachable!()
  };
  let base = options.approval_base_url.trim_end_matches('/');
  let capability = &work.decision_capability;
  Ok(CustomProviderExecuteMessage {
    protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL.to_string(),
    protocol_version: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    message_type: "execute".to_string(),
    invocation_id: work.attempt_id.clone(),
    definition_digest: definition_digest.clone(),
    script_artifact_id: script_artifact_id.clone(),
    props: custom_provider_props(descriptor, context, &options.resolved_secrets)?,
    notification: CustomNotificationRequest {
      kind: CustomNotificationKind::Approval,
      message: render_custom_message(
        work.message.as_ref(),
        context,
        approval_name,
        approval_description,
      )?,
      delivery_id: work.delivery_id.clone(),
      idempotency_key: work.idempotency_key.clone(),
      actions: Some(CustomNotificationActions {
        approve: CustomNotificationAction {
          url: format!("{base}/api/v1/notification-approvals/{capability}/approved"),
        },
        reject: CustomNotificationAction {
          url: format!("{base}/api/v1/notification-approvals/{capability}/rejected"),
        },
      }),
    },
    attempt: CustomProviderAttempt {
      number: work.attempt,
      max: 3,
    },
    limits: CustomProviderLimits {
      timeout_ms: 30_000,
      max_result_bytes: 16_384,
    },
  })
}

fn synthetic_provider_message(
  work: &NotificationDeliveryWork,
  message_id: Option<&str>,
) -> crate::ProviderMessageIdentity {
  let digest = Sha256::digest(
    format!(
      "{}\0{}",
      work.delivery_id,
      message_id.unwrap_or("delivered")
    )
    .as_bytes(),
  );
  let hexadecimal = hex::encode_upper(digest);
  let seconds =
    1_000_000_000_u64 + u64::from_be_bytes(digest[..8].try_into().unwrap()) % 8_000_000_000;
  let micros = u32::from_be_bytes(digest[8..12].try_into().unwrap()) % 1_000_000;
  crate::ProviderMessageIdentity {
    workspace_id: format!("T{}", &hexadecimal[..8]),
    channel_id: format!("C{}", &hexadecimal[8..16]),
    message_id: format!("{seconds}.{micros:06}"),
  }
}

fn custom_delivery_result(
  work: &NotificationDeliveryWork,
  outcome: CustomProviderOutcome,
) -> NotificationProviderDeliveryResult {
  match outcome {
    CustomProviderOutcome::Succeeded { receipt } => NotificationProviderDeliveryResult::Succeeded(
      synthetic_provider_message(work, receipt.message_id.as_deref()),
    ),
    CustomProviderOutcome::Failed { error } => {
      let kind = match error.kind {
        crate::CustomProviderFailureKind::DeliveryAmbiguous => "delivery_ambiguous",
        crate::CustomProviderFailureKind::HostCrashed => "host_crashed",
        crate::CustomProviderFailureKind::WorkerCrashed => "delivery_ambiguous",
        crate::CustomProviderFailureKind::ContextTooLarge
        | crate::CustomProviderFailureKind::ResultTooLarge => "size_limit_exceeded",
        crate::CustomProviderFailureKind::TimedOut
        | crate::CustomProviderFailureKind::Cancelled
        | crate::CustomProviderFailureKind::ServiceFailed => "provider_unavailable",
        crate::CustomProviderFailureKind::NonJson => "request_invalid",
        crate::CustomProviderFailureKind::RequestInvalid => "request_invalid",
        crate::CustomProviderFailureKind::ScriptThrew => "request_invalid",
      };
      NotificationProviderDeliveryResult::Failed(NotificationSafeFailure {
        kind: kind.to_string(),
        code: error.code,
        message: error.message,
        retryable: error.retryable,
        retry_after_ms: None,
      })
    }
  }
}

fn custom_host_failure(error: &CustomNotificationHostClientError) -> NotificationSafeFailure {
  NotificationSafeFailure {
    kind: "host_crashed".to_string(),
    code: "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
    message: match error {
      CustomNotificationHostClientError::Protocol(_) => {
        "The custom provider returned an invalid protocol response.".to_string()
      }
      _ => "The custom provider host stopped during delivery; WOML will not replay an uncertain effect.".to_string(),
    },
    retryable: false,
    retry_after_ms: None,
  }
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
