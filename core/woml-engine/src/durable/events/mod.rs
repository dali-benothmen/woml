//! Event persistence helpers sharing the parent store and caller transactions.
//!
//! Compatibility expansion is also used by the in-memory runtime adapter; it
//! remains available through the original crate-visible durable facade route.

mod cache;
mod commits;
mod compatibility;
mod history;

pub(in crate::durable) use cache::{cache_projection_state, projection_state_for_events};
pub(in crate::durable) use commits::generated_event_id;
pub(in crate::durable) use compatibility::lifecycle_failure_from_attempt;
pub(crate) use compatibility::{expand_model_v11_payload, lifecycle_warnings_from_projection};
pub(in crate::durable) use history::{
  append_to_history, append_to_history_scoped, append_to_history_scoped_incremental,
  ensure_run_exists, load_events,
};
