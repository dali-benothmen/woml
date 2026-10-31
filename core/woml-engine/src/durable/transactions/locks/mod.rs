//! Shared lock resource, fair queue, lease, and fencing authority. Markup and programmatic sessions share one resource high-water mark.

mod leases;
mod programmatic;
mod queue;
mod scope;

pub(in crate::durable) use programmatic::{
  finite_duration_ms, programmatic_lock_lease_ms, programmatic_lock_session_ref,
};
pub(in crate::durable) use queue::{
  ensure_lock_wait_capacity, expire_lock_owner, grant_oldest_lock_waiter,
};
pub(in crate::durable) use scope::{
  lock_descriptor_for_protected_node, resolve_lock_execution, valid_lock_key,
};
