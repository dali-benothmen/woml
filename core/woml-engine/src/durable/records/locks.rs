//! Durable records / locks.

use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedLockStatusV1 {
  Waiting,
  Held,
  Released,
  TimedOut,
  Cancelled,
  Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockInspectionV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub lock_id: String,
  pub key: String,
  pub status: InspectedLockStatusV1,
  pub registered_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub acquired_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub released_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub fencing_token: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waited_ms: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub held_ms: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockObservationV1 {
  pub profile: &'static str,
  pub held_locks: u64,
  pub waiting_acquisitions: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub oldest_waiter_age_ms: Option<u64>,
  pub renewal_failures: u64,
  pub expired_takeovers: u64,
  pub acquisition_timeouts: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockRecoveryKindV1 {
  AcquisitionTimeout,
  LeaseExpiry,
}

/// Internal scheduler work exposed as a bounded, deterministic batch. It
/// contains coordination identities only; lease credentials, input digests,
/// script data, and payloads never cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRecoveryCandidateV1 {
  pub profile: &'static str,
  pub kind: LockRecoveryKindV1,
  pub key: String,
  pub run_id: String,
  pub execution_identity: String,
  pub due_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockLeaseMaintenance {
  Renewed,
  Lost,
  NotHeld,
}

/// Compact store-owned state for one callback-scoped programmatic lease.
/// The session identity never enters public inspection; callers receive only
/// the key, fencing token, and acquisition time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammaticLockLease {
  pub session_id: String,
  pub key: String,
  pub fencing_token: String,
  pub acquired_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgrammaticLockAcquireState {
  Waiting,
  Held,
  TimedOut,
  Cancelled,
  Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammaticLockAcquire {
  pub state: ProgrammaticLockAcquireState,
  pub lease: Option<ProgrammaticLockLease>,
}

/// Frozen Store v16 safety ceilings. These limits bound one hot resource and
/// the total local durable backlog without preventing an uncontended lock
/// from being acquired immediately.
pub const LOCK_WAITERS_PER_KEY_LIMIT: usize = 1_024;

pub const LOCK_WAITERS_PER_STORE_LIMIT: usize = 10_000;

pub const LOCK_RECOVERY_BATCH_LIMIT: usize = 250;
