//! projection fold locks.

use super::{
  require_running, require_running_or_cancelling, FoldError, LockProjection, LockStatus,
  RunProjection, RunStatus,
};

pub(super) fn apply_lock_wait_registered(
  projection: &mut RunProjection,
  data: &crate::event::LockWaitRegisteredData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.locks.contains_key(&data.lock_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Lock {:?} was registered more than once.",
      data.lock_id
    )));
  }
  projection.locks.insert(
    data.lock_id.clone(),
    LockProjection {
      lock_id: data.lock_id.clone(),
      key: data.key.clone(),
      waiter_sequence: data.waiter_sequence,
      execution_identity: data.execution_identity.clone(),
      registered_at: data.registered_at,
      deadline: data.deadline,
      status: LockStatus::Waiting,
      lease_id: None,
      fencing_token: None,
      acquired_at: None,
      expires_at: None,
      resolved_at: None,
    },
  );
  projection.status = RunStatus::Waiting;
  Ok(())
}

pub(super) fn apply_lock_acquired(
  projection: &mut RunProjection,
  data: &crate::event::LockAcquiredData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting {
    return Err(FoldError::InvalidHistory(
      "lock_acquired requires a waiting run.".to_string(),
    ));
  }
  let lock = projection.locks.get_mut(&data.lock_id).ok_or_else(|| {
    FoldError::InvalidHistory("lock_acquired has no registered waiter.".to_string())
  })?;
  if lock.status != LockStatus::Waiting
    || lock.key != data.key
    || lock.execution_identity != data.execution_identity
  {
    return Err(FoldError::InvalidHistory(
      "lock_acquired does not match the active waiter.".to_string(),
    ));
  }
  lock.status = LockStatus::Held;
  lock.lease_id = Some(data.lease_id.clone());
  lock.fencing_token = Some(data.fencing_token.clone());
  lock.acquired_at = Some(data.acquired_at);
  lock.expires_at = Some(data.expires_at);
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_lock_released(
  projection: &mut RunProjection,
  data: &crate::event::LockReleasedData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let lock = projection
    .locks
    .get_mut(&data.lock_id)
    .ok_or_else(|| FoldError::InvalidHistory("lock_released has no acquired lease.".to_string()))?;
  if lock.status != LockStatus::Held
    || lock.key != data.key
    || lock.lease_id.as_deref() != Some(data.lease_id.as_str())
    || lock.fencing_token.as_deref() != Some(data.fencing_token.as_str())
  {
    return Err(FoldError::InvalidHistory(
      "lock_released does not match the held lease.".to_string(),
    ));
  }
  lock.status = LockStatus::Released;
  lock.resolved_at = Some(data.released_at);
  projection
    .context
    .steps
    .insert(data.lock_id.clone(), data.result.clone());
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_lock_acquire_timed_out(
  projection: &mut RunProjection,
  data: &crate::event::LockAcquireTimedOutData,
) -> Result<(), FoldError> {
  let lock = projection.locks.get_mut(&data.lock_id).ok_or_else(|| {
    FoldError::InvalidHistory("lock_acquire_timed_out has no waiter.".to_string())
  })?;
  if lock.status != LockStatus::Waiting || lock.key != data.key || lock.deadline != data.deadline {
    return Err(FoldError::InvalidHistory(
      "lock_acquire_timed_out does not match the active waiter.".to_string(),
    ));
  }
  lock.status = LockStatus::AcquireTimedOut;
  lock.resolved_at = Some(data.resolved_at);
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_lock_acquisition_cancelled(
  projection: &mut RunProjection,
  data: &crate::event::LockAcquisitionCancelledData,
) -> Result<(), FoldError> {
  let lock = projection.locks.get_mut(&data.lock_id).ok_or_else(|| {
    FoldError::InvalidHistory("lock_acquisition_cancelled has no waiter.".to_string())
  })?;
  if lock.status != LockStatus::Waiting || lock.key != data.key {
    return Err(FoldError::InvalidHistory(
      "lock_acquisition_cancelled does not match the active waiter.".to_string(),
    ));
  }
  lock.status = LockStatus::AcquisitionCancelled;
  lock.resolved_at = Some(data.cancelled_at);
  Ok(())
}

pub(super) fn apply_lock_lease_lost(
  projection: &mut RunProjection,
  data: &crate::event::LockLeaseLostData,
) -> Result<(), FoldError> {
  let lock = projection.locks.get_mut(&data.lock_id).ok_or_else(|| {
    FoldError::InvalidHistory("lock_lease_lost has no acquired lease.".to_string())
  })?;
  if lock.status != LockStatus::Held
    || lock.key != data.key
    || lock.lease_id.as_deref() != Some(data.lease_id.as_str())
    || lock.fencing_token.as_deref() != Some(data.fencing_token.as_str())
  {
    return Err(FoldError::InvalidHistory(
      "lock_lease_lost does not match the held lease.".to_string(),
    ));
  }
  lock.status = LockStatus::LeaseLost;
  lock.resolved_at = Some(data.detected_at);
  projection.status = RunStatus::Running;
  Ok(())
}
