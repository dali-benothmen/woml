//! Durable notification journey facade.
//!
//! Journey orchestration remains separate from built-in/custom message adaptation,
//! transport hosts and messaging capability handlers. A receipt is not a decision.

mod builtin;
mod contracts;
mod custom;
mod diagnostics;
mod journey;
mod retry;

pub(in crate::notification_runtime) use builtin::{
  delivery_message, delivery_result, host_failure, update_message, update_result,
};
pub(in crate::notification_runtime) use contracts::DeliveryHostResult;
pub use contracts::{
  CustomNotificationJourneyOptions, NotificationDeliveryDiagnostic, NotificationJourneyDiagnostics,
  NotificationJourneyError, NotificationJourneyResult, NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION,
};
pub(in crate::notification_runtime) use custom::{
  custom_delivery_message, custom_delivery_result, custom_host_failure,
};
pub(in crate::notification_runtime) use diagnostics::notification_diagnostics;
pub use journey::{
  run_notification_provider_journey, run_notification_provider_journey_with_custom,
};
pub(in crate::notification_runtime) use retry::{delivery_retry_wait, update_retry_wait};
