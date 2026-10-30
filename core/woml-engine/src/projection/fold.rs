//! Global history gate and exhaustive dispatch into typed feature transitions.
//!
//! `apply` mutates a candidate and may fail after changing it. Callers that
//! publish cached state must clone first and publish only after success; this
//! module does not introduce a second transaction or execute external effects.

use super::{
  apply_approval_invalidated, apply_approval_quorum_progressed, apply_approval_requested,
  apply_approval_resolved, apply_approval_review_requested,
  apply_approval_reviewer_decision_recorded, apply_branch_selected, apply_choice_selected,
  apply_compensation_registered, apply_correction_attempt_failed, apply_correction_attempt_started,
  apply_correction_attempt_succeeded, apply_correction_interrupted,
  apply_correction_retry_scheduled, apply_external_signal_received,
  apply_for_each_iteration_failed, apply_for_each_iteration_skipped,
  apply_for_each_iteration_started, apply_for_each_iteration_succeeded, apply_for_each_opened,
  apply_for_each_settled, apply_fork_branch_settled, apply_fork_join_settled, apply_fork_opened,
  apply_infrastructure_policy_decision_recorded, apply_iteration_scoped_event,
  apply_lifecycle_action_attempt_started, apply_lifecycle_action_failed,
  apply_lifecycle_action_succeeded, apply_lifecycle_hook_completed, apply_lifecycle_hook_requested,
  apply_lock_acquire_timed_out, apply_lock_acquired, apply_lock_acquisition_cancelled,
  apply_lock_lease_lost, apply_lock_released, apply_lock_wait_registered,
  apply_manual_recovery_required, apply_notification_decision_accepted,
  apply_notification_delivery_attempt_started, apply_notification_delivery_failed,
  apply_notification_delivery_requested, apply_notification_delivery_succeeded,
  apply_notification_message_update_attempt_started, apply_notification_message_update_failed,
  apply_notification_message_update_requested, apply_notification_message_updated,
  apply_operation_failed, apply_operation_started, apply_operation_succeeded,
  apply_parallel_group_completed, apply_parallel_group_started,
  apply_reusable_lifecycle_action_failed, apply_reusable_lifecycle_action_started,
  apply_reusable_lifecycle_action_succeeded, apply_reusable_lifecycle_completed,
  apply_reusable_lifecycle_requested, apply_run_admitted, apply_run_cancellation_requested,
  apply_run_execution_started, apply_run_failed, apply_run_finalized, apply_run_outcome_decided,
  apply_run_started, apply_run_succeeded, apply_run_timeout_reached,
  apply_saga_correction_completed, apply_saga_correction_started, apply_saga_forward_succeeded,
  apply_saga_started, apply_step_attempt_failed, apply_step_attempt_started,
  apply_step_attempt_succeeded, apply_step_retry_scheduled, apply_wait_cancelled,
  apply_wait_registered, apply_wait_satisfied, apply_wait_timed_out, AttemptIdentity, FoldError,
  RunProjection, RunStatus,
};
use crate::event::RunEvent;
use crate::event::RunEventPayload;
use std::collections::HashMap;
use std::collections::HashSet;

pub fn fold_events(events: &[RunEvent]) -> Result<RunProjection, FoldError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.fold_events");
  span.count("events", events.len());
  if let Some(run_id) = events.first().map(|event| event.run_id.clone()) {
    span.run_id(run_id);
  }
  let result = fold_events_unprofiled(events);
  if result.is_ok() {
    span.succeed();
  }
  result
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectionFoldState {
  projection: RunProjection,
  event_ids: HashSet<String>,
  attempt_indexes: HashMap<AttemptIdentity, usize>,
  previous_event: Option<RunEvent>,
}

impl ProjectionFoldState {
  pub(crate) fn from_events(events: &[RunEvent]) -> Result<Self, FoldError> {
    let mut state = Self::default();
    for event in events {
      state.apply(event)?;
    }
    Ok(state)
  }

  pub(crate) fn projection(&self) -> &RunProjection {
    &self.projection
  }

  pub(crate) fn into_projection(self) -> RunProjection {
    self.projection
  }

  pub(crate) fn last_event_id(&self) -> Option<&str> {
    self
      .previous_event
      .as_ref()
      .map(|event| event.event_id.as_str())
  }

  pub(crate) fn apply(&mut self, event: &RunEvent) -> Result<(), FoldError> {
    let Self {
      projection,
      event_ids,
      attempt_indexes,
      previous_event,
    } = self;
    let index = projection.last_sequence as usize;
    event.validate()?;
    let expected_sequence = index as u64 + 1;
    if event.sequence != expected_sequence {
      return Err(FoldError::InvalidHistory(format!(
        "Expected sequence {expected_sequence}, received {}.",
        event.sequence
      )));
    }
    if !event_ids.insert(event.event_id.clone()) {
      return Err(FoldError::InvalidHistory(format!(
        "Event ID {:?} appears more than once.",
        event.event_id
      )));
    }
    if let Some(run_id) = &projection.run_id {
      if run_id != &event.run_id {
        return Err(FoldError::InvalidHistory(format!(
          "Run history mixes run IDs {:?} and {:?}.",
          run_id, event.run_id
        )));
      }
    }
    if let Some(event_schema_version) = projection.event_schema_version {
      if event_schema_version != event.event_schema_version {
        return Err(FoldError::InvalidHistory(format!(
          "Run history mixes event schema versions {event_schema_version} and {}.",
          event.event_schema_version
        )));
      }
    }
    if matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
    ) {
      return Err(FoldError::InvalidHistory(
        "A terminal run event must be the final event.".to_string(),
      ));
    }

    let top_level_scoped_event = matches!(
      event.payload,
      RunEventPayload::ForEachIterationStarted(_)
        | RunEventPayload::ForEachIterationSucceeded(_)
        | RunEventPayload::ForEachIterationFailed(_)
        | RunEventPayload::ForEachIterationSkipped(_)
        | RunEventPayload::CompensationRegistered(_)
    );
    if event.iteration.is_some() && !top_level_scoped_event {
      apply_iteration_scoped_event(projection, event, previous_event.as_ref())?;
      projection.last_sequence = event.sequence;
      *previous_event = Some(event.clone());
      return Ok(());
    }

    match &event.payload {
      RunEventPayload::RunAdmitted(data) => apply_run_admitted(projection, event, data, index)?,
      RunEventPayload::RunExecutionStarted(data) => apply_run_execution_started(projection, data)?,
      RunEventPayload::RunTimeoutReached(data) => apply_run_timeout_reached(projection, data)?,
      RunEventPayload::RunStarted(data) => apply_run_started(projection, event, data, index)?,
      RunEventPayload::StepAttemptStarted(data) => {
        apply_step_attempt_started(projection, event, data, attempt_indexes)?
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        apply_step_attempt_succeeded(projection, data, attempt_indexes)?
      }
      RunEventPayload::StepAttemptFailed(data) => {
        apply_step_attempt_failed(projection, data, attempt_indexes)?
      }
      RunEventPayload::StepRetryScheduled(data) => {
        apply_step_retry_scheduled(projection, data, previous_event)?
      }
      RunEventPayload::BranchSelected(data) => apply_branch_selected(projection, data)?,
      RunEventPayload::ChoiceSelected(data) => apply_choice_selected(projection, data)?,
      RunEventPayload::ForkOpened(data) => apply_fork_opened(projection, data)?,
      RunEventPayload::ForkBranchSettled(data) => apply_fork_branch_settled(projection, data)?,
      RunEventPayload::ForkJoinSettled(data) => apply_fork_join_settled(projection, data)?,
      RunEventPayload::ForEachOpened(data) => apply_for_each_opened(projection, data)?,
      RunEventPayload::ForEachIterationStarted(data) => {
        apply_for_each_iteration_started(projection, event, data)?
      }
      RunEventPayload::ForEachIterationSucceeded(data) => {
        apply_for_each_iteration_succeeded(projection, event, data)?
      }
      RunEventPayload::ForEachIterationFailed(data) => {
        apply_for_each_iteration_failed(projection, event, data)?
      }
      RunEventPayload::ForEachIterationSkipped(data) => {
        apply_for_each_iteration_skipped(projection, event, data)?
      }
      RunEventPayload::ForEachSucceeded(data)
      | RunEventPayload::ForEachFailed(data)
      | RunEventPayload::ForEachCancelled(data) => apply_for_each_settled(projection, event, data)?,
      RunEventPayload::ParallelGroupStarted(data) => {
        apply_parallel_group_started(projection, data)?
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        apply_parallel_group_completed(projection, data)?
      }
      RunEventPayload::WaitRegistered(data) => apply_wait_registered(projection, data)?,
      RunEventPayload::LockWaitRegistered(data) => apply_lock_wait_registered(projection, data)?,
      RunEventPayload::LockAcquired(data) => apply_lock_acquired(projection, data)?,
      RunEventPayload::LockReleased(data) => apply_lock_released(projection, data)?,
      RunEventPayload::LockAcquireTimedOut(data) => apply_lock_acquire_timed_out(projection, data)?,
      RunEventPayload::LockAcquisitionCancelled(data) => {
        apply_lock_acquisition_cancelled(projection, data)?
      }
      RunEventPayload::LockLeaseLost(data) => apply_lock_lease_lost(projection, data)?,
      RunEventPayload::SagaStarted(data) => apply_saga_started(projection, event, data)?,
      RunEventPayload::CompensationRegistered(data) => {
        apply_compensation_registered(projection, event, data, previous_event)?
      }
      RunEventPayload::SagaForwardSucceeded(data) => {
        apply_saga_forward_succeeded(projection, event, data)?
      }
      RunEventPayload::SagaCorrectionStarted(data) => {
        apply_saga_correction_started(projection, data)?
      }
      RunEventPayload::CorrectionAttemptStarted(data) => {
        apply_correction_attempt_started(projection, event, data)?
      }
      RunEventPayload::CorrectionAttemptSucceeded(data) => {
        apply_correction_attempt_succeeded(projection, data)?
      }
      RunEventPayload::CorrectionAttemptFailed(data) => {
        apply_correction_attempt_failed(projection, data)?
      }
      RunEventPayload::CorrectionRetryScheduled(data) => {
        apply_correction_retry_scheduled(projection, data)?
      }
      RunEventPayload::CorrectionInterrupted(data) => {
        apply_correction_interrupted(projection, data)?
      }
      RunEventPayload::ManualRecoveryRequired(data) => {
        apply_manual_recovery_required(projection, data)?
      }
      RunEventPayload::SagaCorrectionCompleted(data) => {
        apply_saga_correction_completed(projection, event, data)?
      }
      RunEventPayload::ExternalSignalReceived(data) => {
        apply_external_signal_received(projection, data)?
      }
      RunEventPayload::WaitSatisfied(data) => apply_wait_satisfied(projection, data)?,
      RunEventPayload::WaitTimedOut(data) => apply_wait_timed_out(projection, data)?,
      RunEventPayload::WaitCancelled(data) => apply_wait_cancelled(projection, data)?,
      RunEventPayload::ApprovalRequested(data) => {
        apply_approval_requested(projection, event, data)?
      }
      RunEventPayload::ApprovalReviewRequested(data) => {
        apply_approval_review_requested(projection, event, data)?
      }
      RunEventPayload::ApprovalReviewerDecisionRecorded(data) => {
        apply_approval_reviewer_decision_recorded(projection, event, data)?
      }
      RunEventPayload::ApprovalQuorumProgressed(data) => {
        apply_approval_quorum_progressed(projection, data)?
      }
      RunEventPayload::NotificationDeliveryRequested(data) => {
        apply_notification_delivery_requested(projection, data)?
      }
      RunEventPayload::NotificationDeliveryAttemptStarted(data) => {
        apply_notification_delivery_attempt_started(projection, data)?
      }
      RunEventPayload::NotificationDeliverySucceeded(data) => {
        apply_notification_delivery_succeeded(projection, data)?
      }
      RunEventPayload::NotificationDeliveryFailed(data) => {
        apply_notification_delivery_failed(projection, event, data)?
      }
      RunEventPayload::NotificationDecisionAccepted(data) => {
        apply_notification_decision_accepted(projection, data)?
      }
      RunEventPayload::ApprovalResolved(data) => apply_approval_resolved(projection, event, data)?,
      RunEventPayload::ApprovalInvalidated(data) => apply_approval_invalidated(projection, data)?,
      RunEventPayload::NotificationMessageUpdateRequested(data) => {
        apply_notification_message_update_requested(projection, data)?
      }
      RunEventPayload::NotificationMessageUpdateAttemptStarted(data) => {
        apply_notification_message_update_attempt_started(projection, data)?
      }
      RunEventPayload::NotificationMessageUpdated(data) => {
        apply_notification_message_updated(projection, data)?
      }
      RunEventPayload::NotificationMessageUpdateFailed(data) => {
        apply_notification_message_update_failed(projection, event, data)?
      }
      RunEventPayload::InfrastructurePolicyDecisionRecorded(data) => {
        apply_infrastructure_policy_decision_recorded(projection, data)?
      }
      RunEventPayload::OperationStarted(data) => apply_operation_started(projection, data)?,
      RunEventPayload::OperationSucceeded(data) => apply_operation_succeeded(projection, data)?,
      RunEventPayload::OperationFailed(data) => apply_operation_failed(projection, data)?,
      RunEventPayload::RunCancellationRequested(data) => {
        apply_run_cancellation_requested(projection, data)?
      }
      RunEventPayload::LifecycleHookRequested(data) => {
        apply_lifecycle_hook_requested(projection, data)?
      }
      RunEventPayload::LifecycleActionAttemptStarted(data) => {
        apply_lifecycle_action_attempt_started(projection, data)?
      }
      RunEventPayload::LifecycleActionSucceeded(data) => {
        apply_lifecycle_action_succeeded(projection, data)?
      }
      RunEventPayload::LifecycleActionFailed(data) => {
        apply_lifecycle_action_failed(projection, data)?
      }
      RunEventPayload::LifecycleHookCompleted(data) => {
        apply_lifecycle_hook_completed(projection, data)?
      }
      RunEventPayload::ReusableLifecycleRequested(data) => {
        apply_reusable_lifecycle_requested(projection, data)?
      }
      RunEventPayload::ReusableLifecycleActionStarted(data) => {
        apply_reusable_lifecycle_action_started(projection, data)?
      }
      RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
        apply_reusable_lifecycle_action_succeeded(projection, data)?
      }
      RunEventPayload::ReusableLifecycleActionFailed(data) => {
        apply_reusable_lifecycle_action_failed(projection, data)?
      }
      RunEventPayload::ReusableLifecycleCompleted(data) => {
        apply_reusable_lifecycle_completed(projection, data)?
      }
      RunEventPayload::RunOutcomeDecided(data) => apply_run_outcome_decided(projection, data)?,
      RunEventPayload::RunFinalized(data) => apply_run_finalized(projection, data)?,
      RunEventPayload::RunSucceeded(data) => apply_run_succeeded(projection, data)?,
      RunEventPayload::RemoteDispatchTransitioned(_) => {
        // Store v20 is the authoritative dispatch projection. Event v20 keeps
        // the committed transition in run history without duplicating that
        // state machine in the workflow projection.
      }
      RunEventPayload::RunFailed(data) => apply_run_failed(projection, data, attempt_indexes)?,
    }
    projection.last_sequence = event.sequence;
    *previous_event = Some(event.clone());
    Ok(())
  }
}

fn fold_events_unprofiled(events: &[RunEvent]) -> Result<RunProjection, FoldError> {
  ProjectionFoldState::from_events(events).map(ProjectionFoldState::into_projection)
}
