//! Derived and mixed read-model maintenance on the caller's connection/transaction.
//!
//! Rebuilding summaries differs from rebuilding correction rows that also retain
//! operator decisions. Never create an independent commit or erase that evidence.

mod correction;
mod policy;
mod run_summaries;

pub(in crate::durable) use correction::{
  rebuild_correction_indexes, write_correction_indexes_from_projection,
};
pub(in crate::durable) use policy::{
  rebuild_runtime_policy_indexes, runtime_policy_indexes_need_rebuild,
  write_runtime_policy_indexes_for_run, write_runtime_policy_indexes_from_projection,
};
pub(in crate::durable) use run_summaries::{
  rebuild_run_summary_index, write_run_summary, write_run_summary_from_projection,
};
