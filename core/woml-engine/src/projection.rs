//! Stable projection facade. Private modules own focused implementation responsibilities.

mod approvals;
mod attempts;
mod context;
mod control_flow;
mod correction;
mod error;
mod fold;
mod fold_approvals;
mod fold_attempts;
mod fold_control_flow;
mod fold_correction;
mod fold_lifecycle;
mod fold_locks;
mod fold_notifications;
mod fold_operations;
mod fold_run;
mod fold_waits;
mod guards;
mod iteration_fold;
mod lifecycle;
mod notifications;
mod operations;
mod run;
mod waits_and_locks;

pub use approvals::{
  ApprovalRequestProjection, ApprovalRequestStatus, RichApprovalRequestProjection,
  RichApprovalRequestStatus, RichApprovalReviewerDecisionProjection,
};
pub use attempts::{AttemptIdentity, AttemptProjection, AttemptStatus, RetryScheduleProjection};
pub use context::{LockContext, WorkflowContext};
pub use control_flow::{
  ForEachIterationExecutionProjection, ForEachIterationStatus, ForEachProjection, ForEachStatus,
  ForkBranchProjection, ForkJoinStatus, ForkProjection, ParallelGroupProjection,
  ParallelGroupStatus,
};
pub use correction::{
  CorrectionActionProjection, CorrectionActionStatus, SagaPhase, SagaProjection,
};
pub use error::FoldError;
pub use fold::fold_events;
pub(crate) use fold::ProjectionFoldState;
use fold_approvals::{
  apply_approval_invalidated, apply_approval_quorum_progressed, apply_approval_requested,
  apply_approval_resolved, apply_approval_review_requested,
  apply_approval_reviewer_decision_recorded,
};
use fold_attempts::{
  apply_step_attempt_failed, apply_step_attempt_started, apply_step_attempt_succeeded,
  apply_step_retry_scheduled,
};
use fold_control_flow::{
  apply_branch_selected, apply_choice_selected, apply_for_each_iteration_failed,
  apply_for_each_iteration_skipped, apply_for_each_iteration_started,
  apply_for_each_iteration_succeeded, apply_for_each_opened, apply_for_each_settled,
  apply_fork_branch_settled, apply_fork_join_settled, apply_fork_opened,
  apply_parallel_group_completed, apply_parallel_group_started,
};
use fold_correction::{
  apply_compensation_registered, apply_correction_attempt_failed, apply_correction_attempt_started,
  apply_correction_attempt_succeeded, apply_correction_interrupted,
  apply_correction_retry_scheduled, apply_manual_recovery_required,
  apply_saga_correction_completed, apply_saga_correction_started, apply_saga_forward_succeeded,
  apply_saga_started,
};
use fold_lifecycle::{
  apply_lifecycle_action_attempt_started, apply_lifecycle_action_failed,
  apply_lifecycle_action_succeeded, apply_lifecycle_hook_completed, apply_lifecycle_hook_requested,
  apply_reusable_lifecycle_action_failed, apply_reusable_lifecycle_action_started,
  apply_reusable_lifecycle_action_succeeded, apply_reusable_lifecycle_completed,
  apply_reusable_lifecycle_requested, apply_run_finalized, apply_run_outcome_decided,
};
use fold_locks::{
  apply_lock_acquire_timed_out, apply_lock_acquired, apply_lock_acquisition_cancelled,
  apply_lock_lease_lost, apply_lock_released, apply_lock_wait_registered,
};
use fold_notifications::{
  apply_notification_decision_accepted, apply_notification_delivery_attempt_started,
  apply_notification_delivery_failed, apply_notification_delivery_requested,
  apply_notification_delivery_succeeded, apply_notification_message_update_attempt_started,
  apply_notification_message_update_failed, apply_notification_message_update_requested,
  apply_notification_message_updated,
};
use fold_operations::{
  apply_infrastructure_policy_decision_recorded, apply_operation_failed, apply_operation_started,
  apply_operation_succeeded,
};
use fold_run::{
  apply_run_admitted, apply_run_cancellation_requested, apply_run_execution_started,
  apply_run_failed, apply_run_started, apply_run_succeeded, apply_run_timeout_reached,
};
use fold_waits::{
  apply_external_signal_received, apply_wait_cancelled, apply_wait_registered,
  apply_wait_satisfied, apply_wait_timed_out,
};
use guards::{
  identity, projection_digest, require_active_operation_attempt, require_failed_attempt,
  require_operation_runtime, require_operation_settlement, require_running,
  require_running_or_cancelling, started_attempt, validate_operation_terminal_identity,
};
use iteration_fold::apply_iteration_scoped_event;
pub use lifecycle::{
  LifecycleActionProjection, LifecycleActionStatus, LifecycleHookProjection, LifecycleHookStatus,
  LifecycleStatus, ReusableLifecycleProjection, ReusableLifecycleStatus,
};
pub use notifications::{
  NotificationDeliveryProjection, NotificationDeliveryStatus, NotificationMessageUpdateProjection,
  NotificationMessageUpdateStatus,
};
pub use operations::{OperationIdentity, OperationProjection, OperationStatus};
pub use run::{RunFailure, RunProjection, RunStatus};
pub use waits_and_locks::{DurableWaitProjection, DurableWaitStatus, LockProjection, LockStatus};
