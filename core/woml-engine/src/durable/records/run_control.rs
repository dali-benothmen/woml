//! Durable records / run control.

use crate::RunProjection;
use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedBusinessOutcome {
  Undecided,
  Succeeded,
  Failed,
  Cancelled,
}

impl From<Option<crate::event::BusinessOutcome>> for InspectedBusinessOutcome {
  fn from(value: Option<crate::event::BusinessOutcome>) -> Self {
    match value {
      None => Self::Undecided,
      Some(crate::event::BusinessOutcome::Succeeded) => Self::Succeeded,
      Some(crate::event::BusinessOutcome::Failed) => Self::Failed,
      Some(crate::event::BusinessOutcome::Cancelled) => Self::Cancelled,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCancellationStatus {
  Accepted,
  AlreadyRequested,
  AlreadyCancelled,
  Rejected,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunTimeoutSettlement {
  NotConfigured,
  NotDue { deadline_at: DateTime<Utc> },
  LostRace { projection: RunProjection },
  TimedOut { projection: RunProjection },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunCancellationCode {
  WomlRunNotFound,
  WomlRunOutcomeAlreadyDecided,
  WomlRunAlreadyTerminal,
  WomlRunControlVersionUnsupported,
  WomlRunCancellationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCancellationResult {
  pub profile: &'static str,
  pub command_id: String,
  pub run_id: String,
  pub status: RunCancellationStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<RunCancellationCode>,
}
