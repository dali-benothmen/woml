//! projection notifications.

use crate::event::NotificationResolution;
use crate::event::NotificationSafeFailure;
use crate::event::ProviderMessageIdentity;
use chrono::DateTime;
use chrono::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationDeliveryStatus {
  Requested,
  AttemptStarted {
    attempt: u32,
    attempt_id: String,
    idempotency_key: String,
  },
  Succeeded {
    attempt: u32,
    attempt_id: String,
    provider_message: ProviderMessageIdentity,
  },
  Failed {
    attempt: u32,
    attempt_id: String,
    final_: bool,
    failure: NotificationSafeFailure,
    failed_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationDeliveryProjection {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub status: NotificationDeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationMessageUpdateStatus {
  Requested,
  AttemptStarted {
    attempt: u32,
    attempt_id: String,
  },
  Updated {
    attempt: u32,
    attempt_id: String,
  },
  Failed {
    attempt: u32,
    attempt_id: String,
    final_: bool,
    failure: NotificationSafeFailure,
    failed_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationMessageUpdateProjection {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub resolution: NotificationResolution,
  pub status: NotificationMessageUpdateStatus,
}
