//! Private notification runtime custom.

use crate::custom_notification_host::CustomNotificationHostClientError;
use crate::custom_notification_provider_protocol::CustomNotificationAction;
use crate::custom_notification_provider_protocol::CustomNotificationActions;
use crate::custom_notification_provider_protocol::CustomNotificationKind;
use crate::custom_notification_provider_protocol::CustomNotificationRequest;
use crate::custom_notification_provider_protocol::CustomProviderAttempt;
use crate::custom_notification_provider_protocol::CustomProviderExecuteMessage;
use crate::custom_notification_provider_protocol::CustomProviderLimits;
use crate::custom_notification_provider_protocol::CustomProviderOutcome;
use crate::custom_notification_provider_protocol::CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL;
use crate::custom_notification_provider_protocol::CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
use crate::durable::NotificationDeliveryWork;
use crate::durable::NotificationProviderDeliveryResult;
use crate::model::CompiledReusableInvocation;
use crate::model::CompiledReusablePropExpression;
use crate::model::TemplatePart;
use crate::model::ValueExpression;
use crate::notification_runtime::CustomNotificationJourneyOptions;
use crate::notification_runtime::NotificationJourneyError;
use crate::NotificationSafeFailure;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;

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

pub(in crate::notification_runtime) fn custom_delivery_message(
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
  crate::ProviderMessageIdentity::Slack(crate::event::SlackProviderMessageIdentity {
    workspace_id: format!("T{}", &hexadecimal[..8]),
    channel_id: format!("C{}", &hexadecimal[8..16]),
    message_id: format!("{seconds}.{micros:06}"),
  })
}

pub(in crate::notification_runtime) fn custom_delivery_result(
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

pub(in crate::notification_runtime) fn custom_host_failure(
  error: &CustomNotificationHostClientError,
) -> NotificationSafeFailure {
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
