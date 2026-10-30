//! Lifecycle notification bindings, rendering, delivery, and failure translation.
//!
//! Transport facts still settle through the existing runtime/capability path.
//! These informational actions do not resolve human approval or undo effects.

use crate::engine::resolve_context_reference;
use crate::engine::step_effect_idempotency_key;
use crate::event::LifecycleFailure;
use crate::event::LifecycleFailureKind;
use crate::event::OperationExecutionMode;
use crate::event::OperationFailedData;
use crate::event::OperationStartedData;
use crate::event::OperationSucceededData;
use crate::model::CompiledLifecycleAction;
use crate::model::CompiledReusableInvocation;
use crate::model::TemplatePart;
use crate::model::ValueExpression;
use crate::protocol::LifecycleBindingV1;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::runtime::context::resolve_json_path;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::{
  drive_reusable_lifecycle, generated_id, reusable_provider_descriptor, RuntimeDagEngine,
};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::CompiledWorkflowDefinition;
use crate::CustomNotificationHostClient;
use crate::CustomNotificationHostClientError;
use crate::CustomNotificationHostProcessOptions;
use crate::CustomNotificationKind;
use crate::CustomNotificationRequest;
use crate::CustomProviderAttempt;
use crate::CustomProviderExecuteMessage;
use crate::CustomProviderLimits;
use crate::CustomProviderOutcome;
use crate::CustomProviderScriptArtifact;
use crate::InformationalNotificationDeliverMessage;
use crate::NotificationCredentials;
use crate::NotificationHostClient;
use crate::NotificationHostClientError;
use crate::NotificationHostOutcome;
use crate::RunEventPayload;
use crate::ScriptHostClient;
use crate::WorkflowContext;
use crate::CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL;
use crate::CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
use crate::INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
use crate::NOTIFICATION_PROVIDER_PROTOCOL;
use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug)]
struct LifecycleNotificationDelivery<'a> {
  delivery_id: &'a str,
  provider: &'a str,
  destination: &'a str,
  provider_id: Option<&'a str>,
  credentials: BTreeMap<String, String>,
  message: &'a ValueExpression,
  template_name: Option<&'a str>,
  language: Option<&'a str>,
}

fn lifecycle_notification_deliveries(
  action: &CompiledLifecycleAction,
) -> Result<Vec<LifecycleNotificationDelivery<'_>>, RuntimeExecutionError> {
  let ValueExpression::Object { fields } = &action.inputs else {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "lifecycle notification action {:?} has invalid inputs",
      action.action_id
    )));
  };
  let Some(ValueExpression::Array { items }) = fields.get("deliveries") else {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "lifecycle notification action {:?} has no deliveries",
      action.action_id
    )));
  };
  items
    .iter()
    .map(|item| {
      let ValueExpression::Object { fields } = item else {
        return Err(RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification delivery is not an object".to_string(),
        ));
      };
      let literal = |name: &str| match fields.get(name) {
        Some(ValueExpression::Literal { value }) => value.as_str(),
        _ => None,
      };
      let credentials_expression = fields.get("credentials").ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification credentials are unavailable".to_string(),
        )
      })?;
      let ValueExpression::Object {
        fields: credential_fields,
      } = credentials_expression
      else {
        return Err(RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification credentials are invalid".to_string(),
        ));
      };
      let provider = literal("provider").ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification provider is unavailable".to_string(),
        )
      })?;
      let credentials = credential_fields
        .iter()
        .map(|(name, expression)| match expression {
          ValueExpression::SecretReference { name: secret } => Ok((name.clone(), secret.clone())),
          ValueExpression::Literal { value }
            if provider == "whatsapp" && name == "phoneNumberId" =>
          {
            value
              .as_str()
              .map(|value| (name.clone(), value.to_string()))
              .ok_or_else(|| {
                RuntimeExecutionError::InvalidConfiguration(
                  "WhatsApp phoneNumberId must be a literal string".to_string(),
                )
              })
          }
          _ => Err(RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification credentials must be symbolic secret references".to_string(),
          )),
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
      Ok(LifecycleNotificationDelivery {
        delivery_id: literal("deliveryId").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification deliveryId is unavailable".to_string(),
          )
        })?,
        provider,
        destination: literal("destination").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification destination is unavailable".to_string(),
          )
        })?,
        provider_id: literal("providerId"),
        credentials,
        message: fields.get("message").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification message is unavailable".to_string(),
          )
        })?,
        template_name: literal("templateName"),
        language: literal("language"),
      })
    })
    .collect()
}

fn custom_provider_runtime_artifacts(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<Vec<CustomProviderScriptArtifact>, LifecycleFailure> {
  let mut seen = HashSet::new();
  let mut artifacts = Vec::new();
  for (definition_digest, script_artifact_id) in workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::NotificationProvider {
        definition_digest,
        script_artifact_id,
        ..
      } => Some((definition_digest, script_artifact_id)),
      _ => None,
    })
  {
    if !seen.insert(script_artifact_id.as_str()) {
      continue;
    }
    let name = format!("__woml_provider__{script_artifact_id}");
    let artifact = options
      .runtime_modules
      .iter()
      .find(|artifact| artifact.name == name)
      .ok_or_else(|| LifecycleFailure {
        kind: LifecycleFailureKind::ProviderFailed,
        code: "WOML_CUSTOM_PROVIDER_ARTIFACT_MISSING".to_string(),
        message: "The immutable custom provider artifact is unavailable.".to_string(),
      })?;
    artifacts.push(CustomProviderScriptArtifact {
      script_artifact_id: script_artifact_id.clone(),
      definition_digest: definition_digest.clone(),
      source: artifact.bundle.clone(),
    });
  }
  Ok(artifacts)
}

fn custom_lifecycle_props(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>, LifecycleFailure> {
  let CompiledReusableInvocation::NotificationProvider { props, .. } = descriptor else {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_CUSTOM_PROVIDER_DESCRIPTOR_INVALID".to_string(),
      message: "The lifecycle delivery does not reference a custom provider.".to_string(),
    });
  };
  props
    .iter()
    .map(|prop| {
      let value = match &prop.expression {
        crate::model::CompiledReusablePropExpression::Literal { value } => {
          Value::String(value.clone())
        }
        crate::model::CompiledReusablePropExpression::Context { path } => {
          let path = path
            .split('.')
            .map(|segment| {
              if segment == "payload" {
                "trigger".to_string()
              } else {
                segment.to_string()
              }
            })
            .collect::<Vec<_>>();
          resolve_context_reference(&ValueExpression::ContextReference { path }, context).map_err(
            |_| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_CUSTOM_PROVIDER_PROP_UNAVAILABLE".to_string(),
              message: format!("Custom provider prop {:?} is unavailable.", prop.name),
            },
          )?
        }
        crate::model::CompiledReusablePropExpression::Secret { name } => Value::String(
          secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_CUSTOM_PROVIDER_SECRET_UNAVAILABLE".to_string(),
              message: format!("Required custom provider secret {name:?} is unavailable."),
            })?,
        ),
      };
      Ok((prop.binding_name.clone(), value))
    })
    .collect()
}

pub(in crate::runtime) fn render_template_scalar(value: Value) -> Option<String> {
  match value {
    Value::Null => Some("null".to_string()),
    Value::Bool(value) => Some(value.to_string()),
    Value::Number(value) => Some(value.to_string()),
    Value::String(value) => Some(value),
    Value::Array(_) | Value::Object(_) => None,
  }
}

fn render_lifecycle_notification_message(
  expression: &ValueExpression,
  context: &WorkflowContext,
  lifecycle: &LifecycleBindingV1,
) -> Result<String, LifecycleFailure> {
  let ValueExpression::Template { parts } = expression else {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_LIFECYCLE_TEMPLATE_INVALID".to_string(),
      message: "Lifecycle notification message is not a WOML Template v1 value.".to_string(),
    });
  };
  let lifecycle_json = serde_json::to_value(lifecycle).map_err(|_| LifecycleFailure {
    kind: LifecycleFailureKind::ProviderFailed,
    code: "WOML_LIFECYCLE_TEMPLATE_INVALID".to_string(),
    message: "Lifecycle notification data could not be rendered safely.".to_string(),
  })?;
  let mut rendered = String::new();
  for part in parts {
    match part {
      TemplatePart::Text { text } => rendered.push_str(text),
      TemplatePart::ContextReference { path } => {
        let value = resolve_context_reference(
          &ValueExpression::ContextReference { path: path.clone() },
          context,
        )
        .ok()
        .and_then(render_template_scalar)
        .ok_or_else(|| LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_LIFECYCLE_TEMPLATE_REFERENCE_INVALID".to_string(),
          message: format!(
            "Lifecycle notification reference context.{} is unavailable or is not a scalar.",
            path.join(".")
          ),
        })?;
        rendered.push_str(&value);
      }
      TemplatePart::LifecycleReference { path } => {
        let value = resolve_json_path(&lifecycle_json, path)
          .and_then(render_template_scalar)
          .ok_or_else(|| LifecycleFailure {
            kind: LifecycleFailureKind::ProviderFailed,
            code: "WOML_LIFECYCLE_TEMPLATE_REFERENCE_INVALID".to_string(),
            message: format!(
              "Lifecycle notification reference lifecycle.{} is unavailable or is not a scalar.",
              path.join(".")
            ),
          })?;
        rendered.push_str(&value);
      }
    }
    if rendered.chars().count() > 4_096 {
      return Err(LifecycleFailure {
        kind: LifecycleFailureKind::SizeLimitExceeded,
        code: "WOML_LIFECYCLE_TEMPLATE_TOO_LARGE".to_string(),
        message: "Lifecycle notification message exceeds 4096 characters.".to_string(),
      });
    }
  }
  if rendered.is_empty() {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_LIFECYCLE_TEMPLATE_EMPTY".to_string(),
      message: "Lifecycle notification message rendered to an empty value.".to_string(),
    });
  }
  Ok(rendered)
}

enum LifecycleProviderExecution {
  Succeeded {
    duration_ms: f64,
    receipt: Vec<u8>,
    metadata: Map<String, Value>,
  },
  Failed {
    duration_ms: f64,
    failure: CapabilityFailure,
    lifecycle: LifecycleFailure,
    retryable: bool,
  },
}

fn custom_provider_failure_kind(kind: crate::CustomProviderFailureKind) -> CapabilityFailureKind {
  match kind {
    crate::CustomProviderFailureKind::DeliveryAmbiguous => CapabilityFailureKind::Ambiguous,
    crate::CustomProviderFailureKind::HostCrashed
    | crate::CustomProviderFailureKind::WorkerCrashed => CapabilityFailureKind::HostCrashed,
    _ => CapabilityFailureKind::TransportFailed,
  }
}

fn notification_capability_failure(
  kind: CapabilityFailureKind,
  code: String,
  message: String,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code,
    message,
    retryable,
    ambiguous,
    details: None,
  }
}

pub(in crate::runtime) fn lifecycle_failure_from_attempt(
  failure: AttemptFailure,
) -> LifecycleFailure {
  let kind = match failure.kind {
    AttemptFailureKind::ScriptThrew => LifecycleFailureKind::ScriptThrew,
    AttemptFailureKind::ServiceFailed => LifecycleFailureKind::ProviderFailed,
    AttemptFailureKind::ScriptTimedOut => LifecycleFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => LifecycleFailureKind::NonJson,
    AttemptFailureKind::WorkerCrashed => LifecycleFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => LifecycleFailureKind::HostCrashed,
    AttemptFailureKind::Interrupted => LifecycleFailureKind::Interrupted,
    AttemptFailureKind::ContextTooLarge | AttemptFailureKind::ResultTooLarge => {
      LifecycleFailureKind::SizeLimitExceeded
    }
    AttemptFailureKind::InvocationCancelled => LifecycleFailureKind::Cancelled,
  };
  LifecycleFailure {
    kind,
    code: failure.code,
    message: failure.message,
  }
}

pub(in crate::runtime) async fn execute_lifecycle_notification<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  action: &CompiledLifecycleAction,
  hook_invocation_id: &str,
  context: &WorkflowContext,
  lifecycle: &LifecycleBindingV1,
  options: &RuntimeExecutionOptions,
) -> Result<(), LifecycleFailure> {
  if options.infrastructure_policy_required || options.infrastructure_policy.is_some() {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_POLICY_INPUT_UNSUPPORTED".to_string(),
      message: "Lifecycle notification delivery is blocked while Infrastructure Policy enforcement is active because this direct provider path has no reviewed pre-effect descriptor adapter."
        .to_string(),
    });
  }
  let deliveries = lifecycle_notification_deliveries(action).map_err(|error| LifecycleFailure {
    kind: LifecycleFailureKind::ProviderFailed,
    code: "WOML_NOTIFICATION_REQUEST_INVALID".to_string(),
    message: error.to_string(),
  })?;
  let has_builtin = deliveries.iter().any(|delivery| {
    matches!(
      delivery.provider,
      "slack" | "telegram" | "discord" | "whatsapp"
    )
  });
  let has_custom = deliveries
    .iter()
    .any(|delivery| delivery.provider == "custom");
  let host = if has_builtin {
    let host_options = options
      .notification_host
      .clone()
      .ok_or_else(|| LifecycleFailure {
        kind: LifecycleFailureKind::HostCrashed,
        code: "WOML_NOTIFICATION_HOST_UNAVAILABLE".to_string(),
        message: "The informational notification provider host is not configured.".to_string(),
      })?;
    Some(
      NotificationHostClient::spawn(host_options)
        .await
        .map_err(|error| LifecycleFailure {
          kind: LifecycleFailureKind::HostCrashed,
          code: "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
          message: error.to_string(),
        })?,
    )
  } else {
    None
  };
  let custom_host = if has_custom {
    let path = options
      .custom_notification_host_path
      .clone()
      .ok_or_else(|| LifecycleFailure {
        kind: LifecycleFailureKind::HostCrashed,
        code: "WOML_CUSTOM_PROVIDER_HOST_UNAVAILABLE".to_string(),
        message: "The custom notification provider host is not configured.".to_string(),
      })?;
    let artifacts = custom_provider_runtime_artifacts(engine.workflow(), options)?;
    Some(
      CustomNotificationHostClient::spawn(
        CustomNotificationHostProcessOptions::new(options.script_host.bun_executable.clone(), path)
          .with_artifacts(artifacts),
      )
      .await
      .map_err(|error| LifecycleFailure {
        kind: LifecycleFailureKind::HostCrashed,
        code: "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
        message: error.to_string(),
      })?,
    )
  } else {
    None
  };
  let action_key = step_effect_idempotency_key(run_id, engine.definition_hash(), &action.action_id);
  let mut failures = Vec::new();
  let mut reusable_lifecycle_host: Option<ScriptHostClient> = None;

  for delivery in deliveries {
    let message = match render_lifecycle_notification_message(delivery.message, context, lifecycle)
    {
      Ok(message) => message,
      Err(failure) => {
        failures.push(failure);
        continue;
      }
    };
    let credentials = if matches!(
      delivery.provider,
      "slack" | "telegram" | "discord" | "whatsapp"
    ) {
      match NotificationCredentials::from_symbolic(&delivery.credentials) {
        Ok(credentials) => Some(credentials),
        Err(message) => {
          failures.push(LifecycleFailure {
            kind: LifecycleFailureKind::ProviderFailed,
            code: "WOML_NOTIFICATION_REQUEST_INVALID".to_string(),
            message,
          });
          continue;
        }
      }
    } else {
      None
    };
    let custom_descriptor = if delivery.provider == "custom" {
      let Some(provider_id) = delivery.provider_id else {
        failures.push(LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_CUSTOM_PROVIDER_DESCRIPTOR_INVALID".to_string(),
          message: "The lifecycle delivery has no custom provider identity.".to_string(),
        });
        continue;
      };
      let Some(descriptor) = reusable_provider_descriptor(engine.workflow(), provider_id).cloned()
      else {
        failures.push(LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_CUSTOM_PROVIDER_DESCRIPTOR_INVALID".to_string(),
          message: "The custom lifecycle provider descriptor is unavailable.".to_string(),
        });
        continue;
      };
      Some(descriptor)
    } else {
      None
    };
    let operation_key = crate::derive_operation_key(&action_key, delivery.delivery_id);
    let mut delivered = false;
    for provider_attempt in 1..=3_u32 {
      let invocation_id = generated_id("ninv");
      let mut started_metadata = Map::new();
      started_metadata.insert(
        "provider".to_string(),
        Value::String(delivery.provider.to_string()),
      );
      started_metadata.insert(
        "destination".to_string(),
        Value::String(delivery.destination.to_string()),
      );
      started_metadata.insert("providerAttempt".to_string(), Value::from(provider_attempt));
      engine
        .append_payload(
          run_id,
          RunEventPayload::OperationStarted(OperationStartedData {
            node_id: action.action_id.clone(),
            attempt_number: 1,
            invocation_id: invocation_id.clone(),
            call_id: delivery.delivery_id.to_string(),
            operation_key: operation_key.clone(),
            capability: "notifications".to_string(),
            operation: "deliver".to_string(),
            execution_mode: OperationExecutionMode::Managed,
            metadata: started_metadata,
          }),
        )
        .map_err(|error| LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
          message: error.to_string(),
        })?;
      let execution = if let Some(descriptor) = custom_descriptor.as_ref() {
        let CompiledReusableInvocation::NotificationProvider {
          definition_digest,
          script_artifact_id,
          ..
        } = descriptor
        else {
          unreachable!()
        };
        let request = CustomProviderExecuteMessage {
          protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL.to_string(),
          protocol_version: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
          message_type: "execute".to_string(),
          invocation_id: invocation_id.clone(),
          definition_digest: definition_digest.clone(),
          script_artifact_id: script_artifact_id.clone(),
          props: custom_lifecycle_props(descriptor, context, &options.resolved_secrets)?,
          notification: CustomNotificationRequest {
            kind: CustomNotificationKind::Informational,
            message: message.clone(),
            delivery_id: delivery.delivery_id.to_string(),
            idempotency_key: operation_key.clone(),
            actions: None,
          },
          attempt: CustomProviderAttempt {
            number: provider_attempt,
            max: 3,
          },
          limits: CustomProviderLimits {
            timeout_ms: 30_000,
            max_result_bytes: 16_384,
          },
        };
        match custom_host
          .as_ref()
          .expect("custom host")
          .invoke(&request)
          .await
        {
          Ok(completed) => match completed.outcome {
            CustomProviderOutcome::Succeeded { receipt } => {
              let encoded = serde_json::to_vec(&receipt).unwrap_or_default();
              let mut metadata = Map::new();
              if let Some(message_id) = receipt.message_id {
                metadata.insert("providerMessageId".to_string(), Value::String(message_id));
              }
              LifecycleProviderExecution::Succeeded {
                duration_ms: completed.duration_ms,
                receipt: encoded,
                metadata,
              }
            }
            CustomProviderOutcome::Failed { error } => {
              let ambiguous = matches!(
                error.kind,
                crate::CustomProviderFailureKind::DeliveryAmbiguous
                  | crate::CustomProviderFailureKind::HostCrashed
                  | crate::CustomProviderFailureKind::WorkerCrashed
              );
              let retryable = error.retryable && !ambiguous;
              LifecycleProviderExecution::Failed {
                duration_ms: completed.duration_ms,
                failure: notification_capability_failure(
                  custom_provider_failure_kind(error.kind),
                  error.code.clone(),
                  error.message.clone(),
                  retryable,
                  ambiguous,
                ),
                lifecycle: LifecycleFailure {
                  kind: LifecycleFailureKind::ProviderFailed,
                  code: error.code,
                  message: error.message,
                },
                retryable,
              }
            }
          },
          Err(error) => LifecycleProviderExecution::Failed {
            duration_ms: 0.0,
            failure: notification_capability_failure(
              CapabilityFailureKind::HostCrashed,
              "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
              "The custom provider host stopped during an uncertain delivery.".to_string(),
              false,
              true,
            ),
            lifecycle: LifecycleFailure {
              kind: LifecycleFailureKind::HostCrashed,
              code: "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
              message: match error {
                CustomNotificationHostClientError::Protocol(_) => {
                  "The custom provider returned an invalid response.".to_string()
                }
                _ => "The custom provider host stopped during delivery.".to_string(),
              },
            },
            retryable: false,
          },
        }
      } else {
        let request = InformationalNotificationDeliverMessage {
          protocol: NOTIFICATION_PROVIDER_PROTOCOL,
          protocol_version: INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
          message_type: "deliver",
          mode: "informational",
          invocation_id: invocation_id.clone(),
          run_id: run_id.to_string(),
          hook_invocation_id: hook_invocation_id.to_string(),
          action_id: action.action_id.clone(),
          delivery_id: delivery.delivery_id.to_string(),
          provider: delivery.provider.to_string(),
          destination: delivery.destination.to_string(),
          idempotency_key: operation_key.clone(),
          credentials: credentials.clone().expect("built-in provider credentials"),
          message: message.clone(),
          template_name: delivery.template_name.map(str::to_string),
          language: delivery.language.map(str::to_string),
        };
        match host
          .as_ref()
          .expect("built-in notification host")
          .invoke(&invocation_id, &request)
          .await
        {
          Ok(completed) => match completed.outcome {
            NotificationHostOutcome::DeliverySuccess { provider_message } => {
              let encoded = serde_json::to_vec(&provider_message).unwrap_or_default();
              let mut metadata = Map::new();
              match provider_message {
                crate::ProviderMessageIdentity::Slack(message) => {
                  metadata.insert(
                    "workspaceId".to_string(),
                    Value::String(message.workspace_id),
                  );
                  metadata.insert("channelId".to_string(), Value::String(message.channel_id));
                  metadata.insert(
                    "providerMessageId".to_string(),
                    Value::String(message.message_id),
                  );
                }
                crate::ProviderMessageIdentity::Communication(message) => {
                  metadata.insert("accountId".to_string(), Value::String(message.account_id));
                  metadata.insert(
                    "conversationId".to_string(),
                    Value::String(message.conversation_id),
                  );
                  metadata.insert(
                    "providerMessageId".to_string(),
                    Value::String(message.message_id),
                  );
                }
              }
              LifecycleProviderExecution::Succeeded {
                duration_ms: completed.duration_ms,
                receipt: encoded,
                metadata,
              }
            }
            NotificationHostOutcome::Failure { error } => {
              let ambiguous = error.kind == "delivery_ambiguous";
              LifecycleProviderExecution::Failed {
                duration_ms: completed.duration_ms,
                failure: notification_capability_failure(
                  if ambiguous {
                    CapabilityFailureKind::Ambiguous
                  } else {
                    CapabilityFailureKind::TransportFailed
                  },
                  error.code.clone(),
                  error.message.clone(),
                  error.retryable,
                  ambiguous,
                ),
                lifecycle: LifecycleFailure {
                  kind: LifecycleFailureKind::ProviderFailed,
                  code: error.code,
                  message: error.message,
                },
                retryable: error.retryable && !ambiguous,
              }
            }
            NotificationHostOutcome::UpdateSuccess => LifecycleProviderExecution::Failed {
              duration_ms: completed.duration_ms,
              failure: notification_capability_failure(
                CapabilityFailureKind::TransportFailed,
                "WOML_NOTIFICATION_PROTOCOL_INVALID".to_string(),
                "Informational delivery returned an update result.".to_string(),
                false,
                false,
              ),
              lifecycle: LifecycleFailure {
                kind: LifecycleFailureKind::ProviderFailed,
                code: "WOML_NOTIFICATION_PROTOCOL_INVALID".to_string(),
                message: "Informational notification delivery returned an update result."
                  .to_string(),
              },
              retryable: false,
            },
          },
          Err(error) => LifecycleProviderExecution::Failed {
            duration_ms: 0.0,
            failure: notification_capability_failure(
              match error {
                NotificationHostClientError::HostCrashed(_)
                | NotificationHostClientError::Startup(_) => CapabilityFailureKind::HostCrashed,
                _ => CapabilityFailureKind::TransportFailed,
              },
              "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
              error.to_string(),
              false,
              true,
            ),
            lifecycle: LifecycleFailure {
              kind: LifecycleFailureKind::HostCrashed,
              code: "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
              message: error.to_string(),
            },
            retryable: false,
          },
        }
      };
      match execution {
        LifecycleProviderExecution::Succeeded {
          duration_ms,
          receipt,
          mut metadata,
        } => {
          let lifecycle_result = serde_json::from_slice::<Value>(&receipt).ok();
          metadata.insert(
            "provider".to_string(),
            Value::String(delivery.provider.to_string()),
          );
          metadata.insert(
            "destination".to_string(),
            Value::String(delivery.destination.to_string()),
          );
          metadata.insert("providerAttempt".to_string(), Value::from(provider_attempt));
          engine
            .append_payload(
              run_id,
              RunEventPayload::OperationSucceeded(OperationSucceededData {
                node_id: action.action_id.clone(),
                attempt_number: 1,
                invocation_id,
                call_id: delivery.delivery_id.to_string(),
                operation_key: operation_key.clone(),
                capability: "notifications".to_string(),
                operation: "deliver".to_string(),
                execution_mode: OperationExecutionMode::Managed,
                metadata,
                duration_ms,
                result_bytes: receipt.len() as u64,
                result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&receipt))),
              }),
            )
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
              message: error.to_string(),
            })?;
          if let Some(descriptor) = custom_descriptor.as_ref() {
            if reusable_lifecycle_host.is_none() {
              reusable_lifecycle_host = Some(options.acquire_script_host().await.map_err(
                |error| LifecycleFailure {
                  kind: LifecycleFailureKind::HostCrashed,
                  code: "WOML_REUSABLE_LIFECYCLE_HOST_CRASHED".to_string(),
                  message: error.to_string(),
                },
              )?);
            }
            drive_reusable_lifecycle(
              engine,
              run_id,
              descriptor,
              ReusableInvocationOutcome::Succeeded,
              lifecycle_result,
              None,
              options,
              reusable_lifecycle_host
                .as_ref()
                .expect("reusable provider lifecycle host"),
            )
            .await
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_REUSABLE_LIFECYCLE_FAILED".to_string(),
              message: error.to_string(),
            })?;
          }
          delivered = true;
          break;
        }
        LifecycleProviderExecution::Failed {
          duration_ms,
          failure,
          lifecycle,
          retryable,
        } => {
          engine
            .append_payload(
              run_id,
              RunEventPayload::OperationFailed(OperationFailedData {
                node_id: action.action_id.clone(),
                attempt_number: 1,
                invocation_id,
                call_id: delivery.delivery_id.to_string(),
                operation_key: operation_key.clone(),
                capability: "notifications".to_string(),
                operation: "deliver".to_string(),
                execution_mode: OperationExecutionMode::Managed,
                metadata: Map::new(),
                duration_ms,
                failure,
              }),
            )
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
              message: error.to_string(),
            })?;
          if retryable && provider_attempt < 3 {
            tokio::time::sleep(Duration::from_secs(if provider_attempt == 1 {
              1
            } else {
              5
            }))
            .await;
            continue;
          }
          if let Some(descriptor) = custom_descriptor.as_ref() {
            if reusable_lifecycle_host.is_none() {
              reusable_lifecycle_host = Some(options.acquire_script_host().await.map_err(
                |error| LifecycleFailure {
                  kind: LifecycleFailureKind::HostCrashed,
                  code: "WOML_REUSABLE_LIFECYCLE_HOST_CRASHED".to_string(),
                  message: error.to_string(),
                },
              )?);
            }
            drive_reusable_lifecycle(
              engine,
              run_id,
              descriptor,
              ReusableInvocationOutcome::Failed,
              None,
              Some(ReusableLifecycleErrorV1 {
                code: lifecycle.code.clone(),
                message: lifecycle.message.clone(),
              }),
              options,
              reusable_lifecycle_host
                .as_ref()
                .expect("reusable provider lifecycle host"),
            )
            .await
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_REUSABLE_LIFECYCLE_FAILED".to_string(),
              message: error.to_string(),
            })?;
          }
          failures.push(lifecycle);
          break;
        }
      }
    }
    if !delivered && failures.is_empty() {
      failures.push(LifecycleFailure {
        kind: LifecycleFailureKind::ProviderFailed,
        code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
        message: "Lifecycle notification delivery failed.".to_string(),
      });
    }
  }
  if let Some(host) = host {
    host.shutdown().await;
  }
  if let Some(host) = custom_host {
    host.shutdown().await;
  }
  if let Some(host) = reusable_lifecycle_host {
    options.release_script_host(host).await;
  }
  if failures.is_empty() {
    Ok(())
  } else {
    let failed = failures.len();
    let first = failures.remove(0);
    Err(LifecycleFailure {
      kind: first.kind,
      code: first.code,
      message: if failed == 1 {
        first.message
      } else {
        format!(
          "{failed} lifecycle deliveries failed. First failure: {}",
          first.message
        )
      },
    })
  }
}
