use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::event::{
  valid_provider_message, ApprovalDecision, NotificationResolution, NotificationSafeFailure,
};
use crate::ProviderMessageIdentity;

pub const NOTIFICATION_PROVIDER_PROTOCOL: &str = "woml.notification-provider-host";
pub const NOTIFICATION_PROVIDER_PROTOCOL_VERSION: u32 = 1;
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
    if self.protocol != NOTIFICATION_PROVIDER_PROTOCOL
      || self.protocol_version != NOTIFICATION_PROVIDER_PROTOCOL_VERSION
      || self.message_type != "ready"
      || self.host_instance_id.is_empty()
      || self.host_instance_id.chars().count() > 320
      || self.providers != ["slack"]
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
#[serde(rename_all = "camelCase")]
pub struct NotificationCredentials {
  pub bot_token: NotificationSecretReference,
  pub app_token: NotificationSecretReference,
}

impl NotificationCredentials {
  pub fn from_symbolic(credentials: &BTreeMap<String, String>) -> Result<Self, String> {
    let bot_token = credentials
      .get("botToken")
      .ok_or_else(|| "Notification credentials are missing botToken.".to_string())?;
    let app_token = credentials
      .get("appToken")
      .ok_or_else(|| "Notification credentials are missing appToken.".to_string())?;
    if credentials.len() != 2 {
      return Err("Notification credentials contain an unsupported binding.".to_string());
    }
    Ok(Self {
      bot_token: NotificationSecretReference {
        kind: "secretReference",
        name: bot_token.clone(),
      },
      app_token: NotificationSecretReference {
        kind: "secretReference",
        name: app_token.clone(),
      },
    })
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
    if self.protocol != NOTIFICATION_PROVIDER_PROTOCOL
      || self.protocol_version != NOTIFICATION_PROVIDER_PROTOCOL_VERSION
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
      NotificationHostOutcome::UpdateSuccess => Ok(()),
      NotificationHostOutcome::Failure { error } => {
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
