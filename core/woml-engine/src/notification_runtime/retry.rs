//! Private notification runtime retry.

use crate::NotificationDeliveryStatus;
use crate::NotificationMessageUpdateStatus;
use chrono::Utc;
use std::time::Duration;

pub(in crate::notification_runtime) fn delivery_retry_wait(
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

pub(in crate::notification_runtime) fn update_retry_wait(
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
