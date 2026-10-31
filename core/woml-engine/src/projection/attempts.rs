//! projection attempts.

use crate::event::AttemptFailure;
use chrono::DateTime;
use chrono::Utc;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptIdentity {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttemptStatus {
  Started,
  Succeeded { output: Value },
  Failed { failure: AttemptFailure },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttemptProjection {
  pub identity: AttemptIdentity,
  pub handler: String,
  pub idempotency_key: Option<String>,
  pub status: AttemptStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryScheduleProjection {
  pub node_id: String,
  pub failed_attempt: u32,
  pub next_attempt: u32,
  pub scheduled_at: DateTime<Utc>,
}
