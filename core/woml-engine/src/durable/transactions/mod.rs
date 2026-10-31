//! Private feature transaction facade. All methods share the parent store; module boundaries add no connections or commits.

mod admission;
mod approvals;
mod correction;
mod infrastructure_policy;
mod intervals;
mod locks;
mod notifications;
mod ownership;
mod policy;
mod rich_approvals;
mod run_control;
mod schedules;
mod signals;
mod waits;
mod workflow_calls;

pub(in crate::durable) use admission::{
  admit_trigger_occurrence_in_transaction, load_trigger_occurrence_by_id,
  validate_trigger_admission_request, validate_trigger_occurrence_history,
};
pub use correction::correction_invocation_id;
pub(in crate::durable) use locks::{
  ensure_lock_wait_capacity, expire_lock_owner, finite_duration_ms, grant_oldest_lock_waiter,
  lock_descriptor_for_protected_node, programmatic_lock_lease_ms, programmatic_lock_session_ref,
  resolve_lock_execution, valid_lock_key,
};
pub(in crate::durable) use notifications::notification_request_all_failed;
pub use policy::RUNTIME_POLICY_QUEUE_CEILING;
pub(in crate::durable) use rich_approvals::rich_review_row;
pub use run_control::derive_lifecycle_hook_invocation_id;
pub(in crate::durable) use signals::{
  append_signal_resolution, resolve_signal_correlation, sha256_prefixed,
};
