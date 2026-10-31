//! Durable records / ownership.

use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOwnerLease {
  pub deployment_id: String,
  pub activation_id: String,
  pub runtime_instance_id: String,
  pub heartbeat_at: DateTime<Utc>,
  pub lease_expires_at: DateTime<Utc>,
}
