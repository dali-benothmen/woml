//! projection fold attempts.

use super::{
  identity, require_running, require_running_or_cancelling, started_attempt, AttemptIdentity,
  AttemptStatus, FoldError, RetryScheduleProjection, RunProjection,
};
use crate::event::RunEvent;
use crate::event::RunEventPayload;
use std::collections::HashMap;

pub(super) fn apply_step_attempt_started(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::StepAttemptStartedData,
  attempt_indexes: &mut HashMap<AttemptIdentity, usize>,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection
    .attempts
    .iter()
    .any(|attempt| attempt.identity.invocation_id == data.invocation_id)
  {
    return Err(FoldError::InvalidHistory(format!(
      "Invocation ID {:?} was already used by another attempt.",
      data.invocation_id
    )));
  }
  if projection
    .latest_attempt(&data.node_id)
    .is_some_and(|attempt| attempt.status == AttemptStatus::Started)
  {
    return Err(FoldError::InvalidHistory(format!(
      "Node {:?} already has an active attempt.",
      data.node_id
    )));
  }
  if event.event_schema_version >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
    let previous = projection.latest_attempt(&data.node_id);
    let expected_attempt = previous.map_or(1, |attempt| attempt.identity.attempt + 1);
    if data.attempt != expected_attempt {
      return Err(FoldError::InvalidHistory(format!(
        "Node {:?} expected attempt {expected_attempt}, received {}.",
        data.node_id, data.attempt
      )));
    }
    if let Some(first_key) = projection
      .attempts
      .iter()
      .find(|attempt| attempt.identity.node_id == data.node_id)
      .and_then(|attempt| attempt.idempotency_key.as_deref())
    {
      if data.idempotency_key.as_deref() != Some(first_key) {
        return Err(FoldError::InvalidHistory(format!(
          "Node {:?} changed its stable idempotency key.",
          data.node_id
        )));
      }
    }
    match (previous, projection.pending_retries.get(&data.node_id)) {
      (None, None) => {}
      (Some(previous), Some(schedule))
        if matches!(previous.status, AttemptStatus::Failed { .. })
          && schedule.failed_attempt == previous.identity.attempt
          && schedule.next_attempt == data.attempt
          && event.occurred_at >= schedule.scheduled_at => {}
      (Some(_), Some(schedule)) if event.occurred_at < schedule.scheduled_at => {
        return Err(FoldError::InvalidHistory(format!(
          "Node {:?} attempt {} started before its durable schedule.",
          data.node_id, data.attempt
        )));
      }
      _ => {
        return Err(FoldError::InvalidHistory(format!(
          "Node {:?} attempt {} has no matching pending retry.",
          data.node_id, data.attempt
        )));
      }
    }
    projection.pending_retries.remove(&data.node_id);
  }
  let key = identity(&data.node_id, data.attempt, &data.invocation_id);
  if attempt_indexes.contains_key(&key) {
    return Err(FoldError::InvalidHistory(format!(
      "Attempt {:?} was started more than once.",
      key.invocation_id
    )));
  }
  let attempt_index = projection.attempts.len();
  projection.attempts.push(started_attempt(data));
  attempt_indexes.insert(key, attempt_index);
  Ok(())
}

pub(super) fn apply_step_attempt_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::StepAttemptSucceededData,
  attempt_indexes: &mut HashMap<AttemptIdentity, usize>,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if !projection
    .active_managed_operations(&data.invocation_id)
    .is_empty()
  {
    return Err(FoldError::InvalidHistory(format!(
      "Attempt {:?} cannot succeed while a managed operation remains active.",
      data.invocation_id
    )));
  }
  let key = identity(&data.node_id, data.attempt, &data.invocation_id);
  let attempt_index = *attempt_indexes.get(&key).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "Successful attempt {:?} has no matching start event.",
      data.invocation_id
    ))
  })?;
  let attempt = &mut projection.attempts[attempt_index];
  if attempt.status != AttemptStatus::Started {
    return Err(FoldError::InvalidHistory(format!(
      "Attempt {:?} already has a terminal event.",
      data.invocation_id
    )));
  }
  if projection.context.steps.contains_key(&data.node_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Node {:?} has already published a successful output.",
      data.node_id
    )));
  }
  attempt.status = AttemptStatus::Succeeded {
    output: data.output.clone(),
  };
  projection
    .context
    .steps
    .insert(data.node_id.clone(), data.output.clone());
  Ok(())
}

pub(super) fn apply_step_attempt_failed(
  projection: &mut RunProjection,
  data: &crate::event::StepAttemptFailedData,
  attempt_indexes: &mut HashMap<AttemptIdentity, usize>,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  if !projection
    .active_managed_operations(&data.invocation_id)
    .is_empty()
  {
    return Err(FoldError::InvalidHistory(format!(
      "Attempt {:?} cannot fail while a managed operation remains active.",
      data.invocation_id
    )));
  }
  let key = identity(&data.node_id, data.attempt, &data.invocation_id);
  let attempt_index = *attempt_indexes.get(&key).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "Failed attempt {:?} has no matching start event.",
      data.invocation_id
    ))
  })?;
  let attempt = &mut projection.attempts[attempt_index];
  if attempt.status != AttemptStatus::Started {
    return Err(FoldError::InvalidHistory(format!(
      "Attempt {:?} already has a terminal event.",
      data.invocation_id
    )));
  }
  attempt.status = AttemptStatus::Failed {
    failure: data.failure.clone(),
  };
  Ok(())
}

pub(super) fn apply_step_retry_scheduled(
  projection: &mut RunProjection,
  data: &crate::event::StepRetryScheduledData,
  previous_event: &Option<RunEvent>,
) -> Result<(), FoldError> {
  require_running(projection)?;
  let closes_previous_failure = previous_event.as_ref().is_some_and(|previous| {
    matches!(
      &previous.payload,
      RunEventPayload::StepAttemptFailed(failed)
        if failed.node_id == data.node_id
          && failed.attempt == data.failed_attempt
          && failed.failure.kind == crate::AttemptFailureKind::ScriptThrew
    )
  });
  let latest = projection.latest_attempt(&data.node_id);
  if !closes_previous_failure
    || !matches!(
      latest,
      Some(attempt)
        if attempt.identity.attempt == data.failed_attempt
          && matches!(attempt.status, AttemptStatus::Failed { .. })
    )
    || data.next_attempt != data.failed_attempt + 1
    || projection.pending_retries.contains_key(&data.node_id)
  {
    return Err(FoldError::InvalidHistory(format!(
      "Retry schedule for node {:?} does not immediately follow its retryable failed attempt.",
      data.node_id
    )));
  }
  projection.pending_retries.insert(
    data.node_id.clone(),
    RetryScheduleProjection {
      node_id: data.node_id.clone(),
      failed_attempt: data.failed_attempt,
      next_attempt: data.next_attempt,
      scheduled_at: data.scheduled_at,
    },
  );
  Ok(())
}
