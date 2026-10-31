//! projection fold correction.

use super::{
  require_running, require_running_or_cancelling, CorrectionActionProjection,
  CorrectionActionStatus, FoldError, RunProjection, SagaPhase, SagaProjection,
};
use crate::event::RunEvent;
use crate::event::RunEventPayload;
use std::collections::BTreeMap;

pub(super) fn apply_saga_started(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::SagaIdentityData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.sagas.contains_key(&data.saga_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Saga {:?} was started more than once.",
      data.saga_id
    )));
  }
  projection.sagas.insert(
    data.saga_id.clone(),
    SagaProjection {
      saga_id: data.saga_id.clone(),
      phase: SagaPhase::Forward,
      reason: None,
      started_sequence: event.sequence,
      forward_succeeded_sequence: None,
      correction_completed_sequence: None,
      actions: BTreeMap::new(),
    },
  );
  Ok(())
}

pub(super) fn apply_compensation_registered(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::CompensationRegisteredData,
  previous_event: &Option<RunEvent>,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Compensation was registered before its saga started.".into())
  })?;
  if saga.phase != SagaPhase::Forward
    || saga.actions.contains_key(&data.action_id)
    || data.completion_sequence >= event.sequence
    || !matches!(
      previous_event.as_ref().map(|previous| &previous.payload),
      Some(RunEventPayload::StepAttemptSucceeded(succeeded))
        if succeeded.node_id == data.for_step_id
          && previous_event.as_ref().is_some_and(|previous| previous.sequence == data.completion_sequence)
    )
  {
    return Err(FoldError::InvalidHistory(
      "Compensation registration must atomically follow its proven forward success.".to_string(),
    ));
  }
  saga.actions.insert(
    data.action_id.clone(),
    CorrectionActionProjection {
      action_id: data.action_id.clone(),
      action_kind: crate::CorrectiveActionKind::Compensation,
      for_step_id: Some(data.for_step_id.clone()),
      completion_sequence: Some(data.completion_sequence),
      status: CorrectionActionStatus::Registered,
      attempts: 0,
      active_attempt: None,
      retry_at: None,
      failure_code: None,
      ambiguous: false,
    },
  );
  Ok(())
}

pub(super) fn apply_saga_forward_succeeded(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::SagaIdentityData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Saga forward success was recorded before saga start.".into())
  })?;
  if saga.phase != SagaPhase::Forward {
    return Err(FoldError::InvalidHistory(
      "Saga forward work may succeed exactly once.".to_string(),
    ));
  }
  saga.phase = SagaPhase::ForwardSucceeded;
  saga.forward_succeeded_sequence = Some(event.sequence);
  Ok(())
}

pub(super) fn apply_saga_correction_started(
  projection: &mut RunProjection,
  data: &crate::event::SagaCorrectionStartedData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Saga correction was started before saga start.".into())
  })?;
  let valid_phase = match data.reason {
    crate::SagaCorrectionReason::Success => saga.phase == SagaPhase::ForwardSucceeded,
    crate::SagaCorrectionReason::Failure | crate::SagaCorrectionReason::Cancel => {
      saga.phase == SagaPhase::Forward
    }
  };
  if !valid_phase {
    return Err(FoldError::InvalidHistory(
      "Saga correction reason does not match its forward phase.".to_string(),
    ));
  }
  saga.phase = SagaPhase::Correcting;
  saga.reason = Some(data.reason);
  Ok(())
}

pub(super) fn apply_correction_attempt_started(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::CorrectionAttemptIdentityData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction attempt references an unknown saga.".into())
  })?;
  if !matches!(
    saga.phase,
    SagaPhase::Correcting | SagaPhase::ManualRecovery
  ) || saga
    .actions
    .values()
    .any(|action| action.status == CorrectionActionStatus::Started)
  {
    return Err(FoldError::InvalidHistory(
      "A saga may have only one active corrective attempt.".to_string(),
    ));
  }
  let action = saga
    .actions
    .entry(data.action_id.clone())
    .or_insert_with(|| CorrectionActionProjection {
      action_id: data.action_id.clone(),
      action_kind: data.action_kind,
      for_step_id: None,
      completion_sequence: None,
      status: CorrectionActionStatus::Registered,
      attempts: 0,
      active_attempt: None,
      retry_at: None,
      failure_code: None,
      ambiguous: false,
    });
  let can_start = action.action_kind == data.action_kind
    && data.attempt == action.attempts + 1
    && matches!(
      action.status,
      CorrectionActionStatus::Registered | CorrectionActionStatus::RetryScheduled
    )
    && action
      .retry_at
      .is_none_or(|retry_at| event.occurred_at >= retry_at);
  if !can_start {
    return Err(FoldError::InvalidHistory(
      "Correction attempt does not match its pending action and retry schedule.".to_string(),
    ));
  }
  action.attempts = data.attempt;
  action.active_attempt = Some(data.attempt);
  action.retry_at = None;
  action.status = CorrectionActionStatus::Started;
  Ok(())
}

pub(super) fn apply_correction_attempt_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::CorrectionAttemptIdentityData,
) -> Result<(), FoldError> {
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction success references an unknown saga.".into())
  })?;
  let action = saga
    .actions
    .get_mut(&data.action_id)
    .ok_or_else(|| FoldError::InvalidHistory("Correction success has no started action.".into()))?;
  if !matches!(
    saga.phase,
    SagaPhase::Correcting | SagaPhase::ManualRecovery
  ) || action.action_kind != data.action_kind
    || action.status != CorrectionActionStatus::Started
    || action.active_attempt != Some(data.attempt)
  {
    return Err(FoldError::InvalidHistory(
      "Correction success does not close its active attempt.".to_string(),
    ));
  }
  action.status = CorrectionActionStatus::Succeeded;
  action.active_attempt = None;
  Ok(())
}

pub(super) fn apply_correction_attempt_failed(
  projection: &mut RunProjection,
  data: &crate::event::CorrectionAttemptFailedData,
) -> Result<(), FoldError> {
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction failure references an unknown saga.".into())
  })?;
  let action = saga
    .actions
    .get_mut(&data.action_id)
    .ok_or_else(|| FoldError::InvalidHistory("Correction failure has no started action.".into()))?;
  if !matches!(
    saga.phase,
    SagaPhase::Correcting | SagaPhase::ManualRecovery
  ) || action.action_kind != data.action_kind
    || action.status != CorrectionActionStatus::Started
    || action.active_attempt != Some(data.attempt)
  {
    return Err(FoldError::InvalidHistory(
      "Correction failure does not close its active attempt.".to_string(),
    ));
  }
  action.status = CorrectionActionStatus::Failed;
  action.active_attempt = None;
  action.failure_code = Some(data.code.clone());
  action.ambiguous = data.ambiguous;
  Ok(())
}

pub(super) fn apply_correction_retry_scheduled(
  projection: &mut RunProjection,
  data: &crate::event::CorrectionRetryScheduledData,
) -> Result<(), FoldError> {
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction retry references an unknown saga.".into())
  })?;
  let action = saga
    .actions
    .get_mut(&data.action_id)
    .ok_or_else(|| FoldError::InvalidHistory("Correction retry has no failed action.".into()))?;
  let automatic_retry = saga.phase == SagaPhase::Correcting
    && action.status == CorrectionActionStatus::Failed
    && !action.ambiguous;
  let operator_retry = saga.phase == SagaPhase::ManualRecovery
    && action.status == CorrectionActionStatus::ManualRecovery
    && action.ambiguous;
  if action.action_kind != data.action_kind
    || action.attempts != data.attempt
    || !(automatic_retry || operator_retry)
  {
    return Err(FoldError::InvalidHistory(
      "Correction retry does not follow an automatic failure or an explicit manual-recovery decision."
        .to_string(),
    ));
  }
  if operator_retry {
    saga.phase = SagaPhase::Correcting;
    action.ambiguous = false;
    action.failure_code = None;
  }
  action.status = CorrectionActionStatus::RetryScheduled;
  action.retry_at = Some(data.retry_at);
  Ok(())
}

pub(super) fn apply_correction_interrupted(
  projection: &mut RunProjection,
  data: &crate::event::CorrectionInterruptedData,
) -> Result<(), FoldError> {
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction interruption references an unknown saga.".into())
  })?;
  let action = saga.actions.get_mut(&data.action_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction interruption has no started action.".into())
  })?;
  if action.action_kind != data.action_kind
    || action.status != CorrectionActionStatus::Started
    || action.active_attempt != Some(data.attempt)
  {
    return Err(FoldError::InvalidHistory(
      "Correction interruption does not close its active attempt.".to_string(),
    ));
  }
  action.status = CorrectionActionStatus::Interrupted;
  action.active_attempt = None;
  action.ambiguous = data.ambiguous;
  Ok(())
}

pub(super) fn apply_manual_recovery_required(
  projection: &mut RunProjection,
  data: &crate::event::ManualRecoveryRequiredData,
) -> Result<(), FoldError> {
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Manual recovery references an unknown saga.".into())
  })?;
  let action = saga.actions.get_mut(&data.action_id).ok_or_else(|| {
    FoldError::InvalidHistory("Manual recovery references an unknown correction.".into())
  })?;
  if action.action_kind != data.action_kind
    || !matches!(
      action.status,
      CorrectionActionStatus::Failed | CorrectionActionStatus::Interrupted
    )
    || !action.ambiguous
  {
    return Err(FoldError::InvalidHistory(
      "Manual recovery requires one ambiguous failed or interrupted correction.".to_string(),
    ));
  }
  action.status = CorrectionActionStatus::ManualRecovery;
  action.failure_code = Some(data.code.clone());
  saga.phase = SagaPhase::ManualRecovery;
  Ok(())
}

pub(super) fn apply_saga_correction_completed(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::SagaIdentityData,
) -> Result<(), FoldError> {
  let saga = projection.sagas.get_mut(&data.saga_id).ok_or_else(|| {
    FoldError::InvalidHistory("Correction completion references an unknown saga.".into())
  })?;
  let success_exit = saga.reason == Some(crate::SagaCorrectionReason::Success);
  if !matches!(
    saga.phase,
    SagaPhase::Correcting | SagaPhase::ManualRecovery
  ) || saga.actions.values().any(|action| {
    matches!(
      action.status,
      CorrectionActionStatus::Started | CorrectionActionStatus::RetryScheduled
    ) || action.status == CorrectionActionStatus::Registered
      && !(success_exit && action.action_kind == crate::CorrectiveActionKind::Compensation)
  }) {
    return Err(FoldError::InvalidHistory(
      "Saga correction cannot complete while corrective work remains pending.".to_string(),
    ));
  }
  saga.phase = SagaPhase::CorrectionCompleted;
  saga.correction_completed_sequence = Some(event.sequence);
  Ok(())
}
