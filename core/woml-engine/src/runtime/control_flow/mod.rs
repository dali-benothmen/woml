//! Execution families subordinate to the central runtime continuation loop.
//!
//! Branches, waits, parallel groups, forks, and iteration have distinct
//! contracts, but none gains independent durable scheduling authority.

mod branches;
mod for_each;
mod forks;
mod parallel;
mod waits;

pub(in crate::runtime) use branches::{fail_branch, fail_choice};
pub(crate) use for_each::for_each_effect_key;
pub(in crate::runtime) use for_each::{
  execute_bounded_for_each, execute_for_each_iteration_script, for_each_digest,
  for_each_module_bindings, for_each_ready_nodes, for_each_selected_branch,
  for_each_selected_choice, resolve_for_each_reference, settle_open_for_each,
  ForEachTerminalDisposition, SharedForEachEngine,
};
pub(in crate::runtime) use forks::{
  execute_fork, ready_fork, settle_open_fork_cancellation, ForkRuntimeProgress,
};
pub(in crate::runtime) use parallel::{
  execute_parallel_group, invoke_parallel_child, next_parallel_completion,
  ActiveParallelInvocation, ParallelInvocationRequest,
};
pub(in crate::runtime) use waits::{
  completed_terminal_approval_result, register_durable_wait_node, request_approval_wait,
  rich_waiting_outcome, waiting_outcome, DurableWaitNodeProgress,
};
