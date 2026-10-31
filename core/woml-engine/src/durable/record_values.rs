//! Strict stored timestamp and approval-decision conversions with unchanged errors and accepted values.

use crate::durable::DurableStoreError;
use crate::event::ApprovalDecision;
use chrono::DateTime;
use chrono::Utc;

pub(in crate::durable) fn approval_decision_text(decision: ApprovalDecision) -> &'static str {
  match decision {
    ApprovalDecision::Approved => "approved",
    ApprovalDecision::Rejected => "rejected",
  }
}

pub(in crate::durable) fn parse_approval_decision(
  value: &str,
) -> Result<ApprovalDecision, DurableStoreError> {
  match value {
    "approved" => Ok(ApprovalDecision::Approved),
    "rejected" => Ok(ApprovalDecision::Rejected),
    _ => Err(DurableStoreError::Contract(
      "Stored rich approval decision is invalid.".to_string(),
    )),
  }
}

pub(in crate::durable) fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| DurableStoreError::Contract("Stored timestamp is invalid.".to_string()))
}

pub(in crate::durable) fn parse_stored_timestamp(
  value: &str,
) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|timestamp| timestamp.with_timezone(&Utc))
    .map_err(|_| {
      DurableStoreError::Contract("Stored approval credential timestamp is invalid.".to_string())
    })
}

pub(in crate::durable) fn parse_durable_timestamp(
  value: &str,
  label: &str,
) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| DurableStoreError::Contract(format!("Stored {label} is not RFC 3339.")))
}
