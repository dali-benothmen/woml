//! Durable records / notifications.

use crate::event::NotificationSafeFailure;
use crate::event::ProviderMessageIdentity;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationDeliveryWork {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub credentials: BTreeMap<String, String>,
  pub provider_id: Option<String>,
  pub message: Option<crate::model::ValueExpression>,
  pub template_name: Option<String>,
  pub language: Option<String>,
  pub attempt: u32,
  pub attempt_id: String,
  pub idempotency_key: String,
  pub decision_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProviderDeliveryResult {
  Succeeded(ProviderMessageIdentity),
  Failed(NotificationSafeFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationUpdateWork {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub credentials: BTreeMap<String, String>,
  pub provider_message: ProviderMessageIdentity,
  pub resolution: crate::event::NotificationResolution,
  pub update_id: String,
  pub idempotency_key: String,
  pub attempt: u32,
  pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProviderUpdateResult {
  Succeeded,
  Failed(NotificationSafeFailure),
}

pub trait NotificationProviderAdapter {
  fn deliver(&mut self, work: &NotificationDeliveryWork) -> NotificationProviderDeliveryResult;
  fn update(&mut self, work: &NotificationUpdateWork) -> NotificationProviderUpdateResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDispatchReport {
  pub attempted: usize,
  pub succeeded: usize,
  pub failed: usize,
  pub run_failed: bool,
  pub updates_attempted: usize,
  pub updates_succeeded: usize,
  pub updates_failed: usize,
}
