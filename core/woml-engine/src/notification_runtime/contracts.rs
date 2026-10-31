//! Private notification runtime contracts.

use crate::custom_notification_host::CustomNotificationHostClientError;
use crate::custom_notification_host::CustomProviderScriptArtifact;
use crate::durable::ApprovalDecisionOutcome;
use crate::durable::DurableStoreError;
use crate::durable::NotificationDispatchReport;
use crate::notification_host::NotificationHostClientError;
use crate::NotificationResolution;
use crate::NotificationSafeFailure;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CustomNotificationJourneyOptions {
  pub bun_executable: PathBuf,
  pub host_script_path: PathBuf,
  pub script_host_path: PathBuf,
  pub approval_base_url: String,
  pub resolved_secrets: BTreeMap<String, String>,
  pub artifacts: Vec<CustomProviderScriptArtifact>,
}

pub(in crate::notification_runtime) enum DeliveryHostResult {
  BuiltIn(Result<crate::NotificationCompletedMessage, NotificationHostClientError>),
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
