//! Scoped iteration coordination, input views, invocation, and shared adapter.
//!
//! Iteration futures share one event authority; the item limit and active
//! concurrency limit are different quantities. Aggregation retains input order.

mod context;
mod coordinator;
mod script;
mod shared;

pub(crate) use context::for_each_effect_key;
pub(in crate::runtime) use context::{
  for_each_digest, for_each_module_bindings, for_each_ready_nodes, for_each_selected_branch,
  for_each_selected_choice, resolve_for_each_reference,
};
pub(in crate::runtime) use coordinator::{
  execute_bounded_for_each, settle_open_for_each, ForEachTerminalDisposition,
};
pub(in crate::runtime) use script::execute_for_each_iteration_script;
pub(in crate::runtime) use shared::SharedForEachEngine;
