//! event locks.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockWaitRegisteredData {
  pub lock_id: String,
  pub key: String,
  pub waiter_sequence: u64,
  pub registered_at: DateTime<Utc>,
  pub deadline: DateTime<Utc>,
  pub execution_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockAcquiredData {
  pub lock_id: String,
  pub key: String,
  pub lease_id: String,
  pub fencing_token: String,
  pub acquired_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
  pub execution_identity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockReleasedData {
  pub lock_id: String,
  pub key: String,
  pub lease_id: String,
  pub fencing_token: String,
  pub released_at: DateTime<Utc>,
  pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockAcquireTimedOutData {
  pub lock_id: String,
  pub key: String,
  pub deadline: DateTime<Utc>,
  pub resolved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockAcquisitionCancelledData {
  pub lock_id: String,
  pub key: String,
  pub cancelled_at: DateTime<Utc>,
  pub cancellation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockLeaseLostData {
  pub lock_id: String,
  pub key: String,
  pub lease_id: String,
  pub fencing_token: String,
  pub expired_at: DateTime<Utc>,
  pub detected_at: DateTime<Utc>,
  pub ambiguous: bool,
}
