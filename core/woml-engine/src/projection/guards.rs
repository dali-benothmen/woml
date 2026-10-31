//! projection guards.

use super::{
  AttemptIdentity, AttemptProjection, AttemptStatus, FoldError, LifecycleActionStatus,
  OperationProjection, OperationStatus, RunProjection, RunStatus,
};
use crate::event::OperationExecutionMode;
use crate::event::StepAttemptStartedData;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;

pub(super) fn identity(node_id: &str, attempt: u32, invocation_id: &str) -> AttemptIdentity {
  AttemptIdentity {
    node_id: node_id.to_string(),
    attempt,
    invocation_id: invocation_id.to_string(),
  }
}

pub(super) fn validate_operation_terminal_identity(
  started: &OperationProjection,
  node_id: &str,
  attempt_number: u32,
  operation_key: &str,
  capability: &str,
  operation: &str,
  execution_mode: OperationExecutionMode,
) -> Result<(), FoldError> {
  if started.status != OperationStatus::Started {
    return Err(FoldError::InvalidHistory(
      "An operation may have at most one terminal event.".to_string(),
    ));
  }
  if started.node_id != node_id
    || started.attempt_number != attempt_number
    || started.operation_key != operation_key
    || started.capability != capability
    || started.operation != operation
    || started.execution_mode != execution_mode
  {
    return Err(FoldError::InvalidHistory(
      "Terminal operation identity does not match operation_started.".to_string(),
    ));
  }
  Ok(())
}

pub(super) fn require_active_operation_attempt(
  projection: &RunProjection,
  node_id: &str,
  attempt_number: u32,
  invocation_id: &str,
) -> Result<(), FoldError> {
  if projection.attempts.iter().any(|attempt| {
    attempt.identity.node_id == node_id
      && attempt.identity.attempt == attempt_number
      && attempt.identity.invocation_id == invocation_id
      && attempt.status == AttemptStatus::Started
  }) || projection.lifecycle_hooks.values().any(|hook| {
    hook.actions.get(node_id).is_some_and(|action| {
      action.attempt == attempt_number && action.status == LifecycleActionStatus::Started
    })
  }) || super::active_correction(projection, node_id, attempt_number, invocation_id).is_some()
  {
    Ok(())
  } else {
    Err(FoldError::InvalidHistory(
      "An operation event requires its matching active step, lifecycle, or corrective attempt."
        .to_string(),
    ))
  }
}

pub(super) fn started_attempt(data: &StepAttemptStartedData) -> AttemptProjection {
  AttemptProjection {
    identity: identity(&data.node_id, data.attempt, &data.invocation_id),
    handler: data.handler.clone(),
    idempotency_key: data.idempotency_key.clone(),
    status: AttemptStatus::Started,
  }
}

pub(super) fn projection_digest(value: &Value) -> Result<String, FoldError> {
  let bytes = serde_json_canonicalizer::to_vec(value).map_err(|error| {
    FoldError::InvalidHistory(format!("A durable JSON value is not canonical: {error}"))
  })?;
  Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub(super) fn require_failed_attempt(
  projection: &RunProjection,
  attempt_indexes: &HashMap<AttemptIdentity, usize>,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Result<(), FoldError> {
  let key = identity(node_id, attempt, invocation_id);
  let attempt_index = attempt_indexes.get(&key).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "run_failed references unknown attempt {invocation_id:?}."
    ))
  })?;
  if !matches!(
    projection.attempts[*attempt_index].status,
    AttemptStatus::Failed { .. }
  ) {
    return Err(FoldError::InvalidHistory(format!(
      "run_failed references attempt {invocation_id:?} before it failed."
    )));
  }
  Ok(())
}

pub(super) fn require_running(projection: &RunProjection) -> Result<(), FoldError> {
  if projection.status != RunStatus::Running {
    return Err(FoldError::InvalidHistory(
      "A run must be started and nonterminal before attempt or terminal events.".to_string(),
    ));
  }
  Ok(())
}

pub(super) fn require_running_or_cancelling(projection: &RunProjection) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Running | RunStatus::Cancelling
  ) {
    return Err(FoldError::InvalidHistory(
      "An attempt may only settle while its run is running or cancelling.".to_string(),
    ));
  }
  Ok(())
}

pub(super) fn require_operation_runtime(
  projection: &RunProjection,
  node_id: &str,
  attempt_number: u32,
  invocation_id: &str,
) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Running | RunStatus::Finalizing
  ) && !(projection.status == RunStatus::Cancelling
    && super::active_correction(projection, node_id, attempt_number, invocation_id).is_some())
  {
    return Err(FoldError::InvalidHistory(
      "An operation requires a running or lifecycle-finalizing run.".to_string(),
    ));
  }
  Ok(())
}

pub(super) fn require_operation_settlement(projection: &RunProjection) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Running | RunStatus::Cancelling | RunStatus::Finalizing
  ) {
    return Err(FoldError::InvalidHistory(
      "An operation may only settle for an active or finalizing run.".to_string(),
    ));
  }
  Ok(())
}
