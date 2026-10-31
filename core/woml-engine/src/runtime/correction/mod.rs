//! Corrective registration, exact attempt execution, and saga settlement.
//!
//! Compensation is a new effect, not external rollback. Ambiguity requires
//! reviewed recovery instead of an automatic retry or fabricated success.

mod execution;
mod registration;
mod saga;

pub(in crate::runtime) use execution::{execute_corrective_action, CorrectiveActionOutcome};
pub(in crate::runtime) use registration::{
  append_step_attempt_success, append_step_attempt_success_payload,
  compensation_registration_for_iteration, correction_iteration_identity,
  corrective_action_matches_runtime,
};
pub(crate) use saga::resume_corrections_for_workflows_durable;
pub(in crate::runtime) use saga::{correct_saga_failure_for_node, execute_saga_correction};
