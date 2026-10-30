//! projection waits and locks.

use chrono::DateTime;
use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableWaitStatus {
  Registered,
  Satisfied {
    winner: crate::DurableWaitWinner,
  },
  TimedOut {
    behavior: crate::DurableWaitTimeoutBehavior,
  },
  Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DurableWaitProjection {
  pub wait_id: String,
  pub node_id: String,
  pub kind: crate::DurableWaitKind,
  pub registered_at: DateTime<Utc>,
  pub deadline: Option<DateTime<Utc>>,
  pub signals: Vec<String>,
  pub correlation_digest: Option<String>,
  pub status: DurableWaitStatus,
  pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockStatus {
  Waiting,
  Held,
  Released,
  AcquireTimedOut,
  AcquisitionCancelled,
  LeaseLost,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LockProjection {
  pub lock_id: String,
  pub key: String,
  pub waiter_sequence: u64,
  pub execution_identity: String,
  pub registered_at: DateTime<Utc>,
  pub deadline: DateTime<Utc>,
  pub status: LockStatus,
  pub lease_id: Option<String>,
  pub fencing_token: Option<String>,
  pub acquired_at: Option<DateTime<Utc>>,
  pub expires_at: Option<DateTime<Utc>>,
  pub resolved_at: Option<DateTime<Utc>>,
}
