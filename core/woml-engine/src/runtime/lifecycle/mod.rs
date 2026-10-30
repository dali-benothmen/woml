//! Workflow, reusable, step, iteration, and notification lifecycle orchestration.
//!
//! Business outcome, hook health, and finalization remain separate facts.
//! Mandatory corrective cleanup belongs to correction, not informational hooks.

mod iteration;
mod notifications;
mod reusable;
mod steps;
mod workflow;

pub(in crate::runtime) use iteration::{
  drive_for_each_reusable_lifecycle, drive_for_each_step_lifecycle,
};
pub(in crate::runtime) use notifications::{
  execute_lifecycle_notification, lifecycle_failure_from_attempt, render_template_scalar,
};
pub use reusable::execute_reusable_provider_lifecycle_durable;
pub(in crate::runtime) use reusable::{
  drive_pending_reusable_step_lifecycles, drive_reusable_lifecycle, drive_reusable_step_lifecycle,
  reusable_provider_descriptor,
};
pub(in crate::runtime) use steps::{step_lifecycle_outcome, step_lifecycle_request};
pub(in crate::runtime) use workflow::{
  drive_workflow_lifecycle, lifecycle_action_source, resolved_lifecycle_secrets,
};
