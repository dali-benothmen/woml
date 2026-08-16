use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::event::{
  valid_provider_message, ApprovalDecision, NotificationResolution, NotificationSafeFailure,
};
use crate::ProviderMessageIdentity;

pub const NOTIFICATION_PROVIDER_PROTOCOL: &str = "woml.notification-provider-host";
pub const NOTIFICATION_PROVIDER_PROTOCOL_VERSION: u32 = 1;
pub const INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION: u32 = 2;
pub const NOTIFICATION_PROVIDER_MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationReadyMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub host_instance_id: String,
  pub providers: Vec<String>,
}

impl NotificationReadyMessage {
  pub fn validate(&self) -> Result<(), String> {
    self.validate_for(NOTIFICATION_PROVIDER_PROTOCOL_VERSION)
  }

  pub fn validate_for(&self, protocol_version: u32) -> Result<(), String> {
    if self.protocol != NOTIFICATION_PROVIDER_PROTOCOL
      || self.protocol_version != protocol_version
      || self.message_type != "ready"
      || self.host_instance_id.is_empty()
      || self.host_instance_id.chars().count() > 320
      || (self.providers != ["slack"]
        && self.providers != ["slack", "telegram"]
        && self.providers != ["slack", "telegram", "discord"])
    {
      return Err(
        "The child did not send a valid notification-provider ready message.".to_string(),
      );
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSecretReference {
  pub kind: &'static str,
  pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum NotificationCredentials {
  Slack {
    #[serde(rename = "botToken")]
    bot_token: NotificationSecretReference,
    #[serde(rename = "appToken")]
    app_token: NotificationSecretReference,
  },
  Bot {
    #[serde(rename = "botToken")]
    bot_token: NotificationSecretReference,
  },
}

impl NotificationCredentials {
  pub fn from_symbolic(credentials: &BTreeMap<String, String>) -> Result<Self, String> {
    let bot_token = credentials
      .get("botToken")
      .ok_or_else(|| "Notification credentials are missing botToken.".to_string())?;
    let bot_token = NotificationSecretReference {
      kind: "secretReference",
      name: bot_token.clone(),
    };
    match (credentials.len(), credentials.get("appToken")) {
      (1, None) => Ok(Self::Bot { bot_token }),
      (2, Some(app_token)) => Ok(Self::Slack {
        bot_token,
        app_token: NotificationSecretReference {
          kind: "secretReference",
          name: app_token.clone(),
        },
      }),
      _ => Err("Notification credentials contain an unsupported binding.".to_string()),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationApprovalMessage {
  pub workflow_id: String,
  pub approval_name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub approval_description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDeliverMessage {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub idempotency_key: String,
  pub credentials: NotificationCredentials,
  pub decision_capability: String,
  pub message: NotificationApprovalMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InformationalNotificationDeliverMessage {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub mode: &'static str,
  pub invocation_id: String,
  pub run_id: String,
  pub hook_invocation_id: String,
  pub action_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub idempotency_key: String,
  pub credentials: NotificationCredentials,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationUpdateMessage {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub idempotency_key: String,
  pub provider: String,
  pub credentials: NotificationCredentials,
  pub provider_message: ProviderMessageIdentity,
  pub resolution: NotificationResolution,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationCompletedMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub outcome: NotificationHostOutcome,
  pub duration_ms: f64,
}

impl NotificationCompletedMessage {
  pub fn validate(&self) -> Result<(), String> {
    self.validate_for(NOTIFICATION_PROVIDER_PROTOCOL_VERSION)
  }

  pub fn validate_for(&self, protocol_version: u32) -> Result<(), String> {
    if self.protocol != NOTIFICATION_PROVIDER_PROTOCOL
      || self.protocol_version != protocol_version
      || self.message_type != "completed"
      || self.invocation_id.is_empty()
      || self.invocation_id.chars().count() > 320
      || !self.duration_ms.is_finite()
      || self.duration_ms < 0.0
    {
      return Err("The provider host returned an invalid completion envelope.".to_string());
    }
    match &self.outcome {
      NotificationHostOutcome::DeliverySuccess { provider_message } => {
        if valid_provider_message(provider_message) {
          Ok(())
        } else {
          Err("The provider host returned an invalid provider message identity.".to_string())
        }
      }
      NotificationHostOutcome::UpdateSuccess => {
        if protocol_version == NOTIFICATION_PROVIDER_PROTOCOL_VERSION {
          Ok(())
        } else {
          Err("Informational notification delivery cannot update messages.".to_string())
        }
      }
      NotificationHostOutcome::Failure { error } => {
        if protocol_version == INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION
          && error.kind == "update_failed"
        {
          return Err(
            "Informational notification delivery returned an approval-only failure kind."
              .to_string(),
          );
        }
        error.validate().map_err(|error| error.to_string())
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NotificationHostOutcome {
  DeliverySuccess {
    #[serde(rename = "providerMessage")]
    provider_message: ProviderMessageIdentity,
  },
  UpdateSuccess,
  Failure {
    error: NotificationSafeFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationInteractionMessage {
  pub protocol: String,
  pub protocol_version: u32,
  pub message_type: String,
  pub interaction_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub decision_capability: String,
  pub decision: ApprovalDecision,
  pub provider_actor_id: String,
  pub occurred_at: DateTime<Utc>,
}

impl NotificationInteractionMessage {
  pub fn validate(&self) -> Result<(), String> {
    if self.protocol != NOTIFICATION_PROVIDER_PROTOCOL
      || self.protocol_version != NOTIFICATION_PROVIDER_PROTOCOL_VERSION
      || self.message_type != "interaction"
      || self.interaction_id.is_empty()
      || self.interaction_id.chars().count() > 320
      || self.provider != "slack"
      || self.delivery_id.is_empty()
      || self.decision_capability.len() < 43
      || self.decision_capability.len() > 512
      || !valid_slack_actor(&self.provider_actor_id)
    {
      return Err("The provider host returned an invalid interaction message.".to_string());
    }
    Ok(())
  }
}

fn valid_slack_actor(value: &str) -> bool {
  value.len() >= 9
    && value.len() <= 32
    && value.starts_with('U')
    && value[1..]
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn provider_host_accepts_discord_readiness_and_message_identity() {
    let ready: NotificationReadyMessage = serde_json::from_value(serde_json::json!({
      "protocol": NOTIFICATION_PROVIDER_PROTOCOL,
      "protocolVersion": NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
      "messageType": "ready",
      "hostInstanceId": "notification-host-discord",
      "providers": ["slack", "telegram", "discord"]
    }))
    .expect("Discord-ready host message");
    ready.validate().expect("Discord-ready provider host");

    let completed: NotificationCompletedMessage = serde_json::from_value(serde_json::json!({
      "protocol": NOTIFICATION_PROVIDER_PROTOCOL,
      "protocolVersion": NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
      "messageType": "completed",
      "invocationId": "notification-discord-delivery",
      "outcome": {
        "kind": "delivery_success",
        "providerMessage": {
          "provider": "discord",
          "accountId": "123456789012345678",
          "conversationId": "345678901234567890",
          "messageId": "456789012345678901"
        }
      },
      "durationMs": 2.5
    }))
    .expect("Discord completion message");
    completed.validate().expect("Discord delivery identity");
  }
}
