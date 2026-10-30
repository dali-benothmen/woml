//! Private notification runtime diagnostics.

use crate::notification_runtime::NotificationDeliveryDiagnostic;
use crate::notification_runtime::NotificationJourneyDiagnostics;
use crate::notification_runtime::NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION;
use crate::NotificationDeliveryStatus;

pub(in crate::notification_runtime) fn notification_diagnostics(
  projection: &crate::RunProjection,
) -> NotificationJourneyDiagnostics {
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
