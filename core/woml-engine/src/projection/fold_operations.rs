//! projection fold operations.

use super::{
  require_active_operation_attempt, require_operation_runtime, require_operation_settlement,
  validate_operation_terminal_identity, AttemptStatus, FoldError, LifecycleActionStatus,
  OperationIdentity, OperationProjection, OperationStatus, RunProjection,
};

pub(super) fn apply_infrastructure_policy_decision_recorded(
  projection: &mut RunProjection,
  data: &crate::event::InfrastructurePolicyDecisionRecordedData,
) -> Result<(), FoldError> {
  require_operation_settlement(projection)?;
  if projection
    .infrastructure_policy_decisions
    .iter()
    .any(|existing| existing.decision.decision_id == data.decision.decision_id)
  {
    return Err(FoldError::InvalidHistory(
      "Infrastructure policy decision identity was recorded more than once.".to_string(),
    ));
  }
  projection
    .infrastructure_policy_decisions
    .push(data.clone());
  Ok(())
}

pub(super) fn apply_operation_started(
  projection: &mut RunProjection,
  data: &crate::event::OperationStartedData,
) -> Result<(), FoldError> {
  require_operation_runtime(projection)?;
  let attempt_active = projection.attempts.iter().any(|attempt| {
    attempt.identity.node_id == data.node_id
      && attempt.identity.attempt == data.attempt_number
      && attempt.identity.invocation_id == data.invocation_id
      && attempt.status == AttemptStatus::Started
  }) || projection.lifecycle_hooks.values().any(|hook| {
    hook.actions.get(&data.node_id).is_some_and(|action| {
      action.attempt == data.attempt_number && action.status == LifecycleActionStatus::Started
    })
  });
  if !attempt_active {
    return Err(FoldError::InvalidHistory(
      "operation_started requires its matching active step or lifecycle action attempt."
        .to_string(),
    ));
  }
  let key = OperationIdentity {
    invocation_id: data.invocation_id.clone(),
    call_id: data.call_id.clone(),
  };
  if projection.operations.contains_key(&key) {
    return Err(FoldError::InvalidHistory(format!(
      "Operation call {:?} was started more than once in invocation {:?}.",
      data.call_id, data.invocation_id
    )));
  }
  if projection.operations.values().any(|operation| {
    operation.identity.invocation_id == data.invocation_id
      && operation.operation_key == data.operation_key
  }) {
    return Err(FoldError::InvalidHistory(format!(
      "Logical operation key {:?} was reused within invocation {:?}.",
      data.operation_key, data.invocation_id
    )));
  }
  projection.operations.insert(
    key.clone(),
    OperationProjection {
      identity: key,
      node_id: data.node_id.clone(),
      attempt_number: data.attempt_number,
      operation_key: data.operation_key.clone(),
      capability: data.capability.clone(),
      operation: data.operation.clone(),
      execution_mode: data.execution_mode,
      metadata: data.metadata.clone(),
      status: OperationStatus::Started,
    },
  );
  Ok(())
}

pub(super) fn apply_operation_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::OperationSucceededData,
) -> Result<(), FoldError> {
  require_operation_runtime(projection)?;
  require_active_operation_attempt(
    projection,
    &data.node_id,
    data.attempt_number,
    &data.invocation_id,
  )?;
  let key = OperationIdentity {
    invocation_id: data.invocation_id.clone(),
    call_id: data.call_id.clone(),
  };
  let operation = projection.operations.get_mut(&key).ok_or_else(|| {
    FoldError::InvalidHistory("operation_succeeded has no matching start.".to_string())
  })?;
  validate_operation_terminal_identity(
    operation,
    &data.node_id,
    data.attempt_number,
    &data.operation_key,
    &data.capability,
    &data.operation,
    data.execution_mode,
  )?;
  operation.metadata = data.metadata.clone();
  operation.status = OperationStatus::Succeeded {
    duration_ms: data.duration_ms,
    result_bytes: data.result_bytes,
    result_digest: data.result_digest.clone(),
  };
  Ok(())
}

pub(super) fn apply_operation_failed(
  projection: &mut RunProjection,
  data: &crate::event::OperationFailedData,
) -> Result<(), FoldError> {
  require_operation_settlement(projection)?;
  require_active_operation_attempt(
    projection,
    &data.node_id,
    data.attempt_number,
    &data.invocation_id,
  )?;
  let key = OperationIdentity {
    invocation_id: data.invocation_id.clone(),
    call_id: data.call_id.clone(),
  };
  let operation = projection.operations.get_mut(&key).ok_or_else(|| {
    FoldError::InvalidHistory("operation_failed has no matching start.".to_string())
  })?;
  validate_operation_terminal_identity(
    operation,
    &data.node_id,
    data.attempt_number,
    &data.operation_key,
    &data.capability,
    &data.operation,
    data.execution_mode,
  )?;
  operation.metadata = data.metadata.clone();
  operation.status = OperationStatus::Failed {
    duration_ms: data.duration_ms,
    failure: data.failure.clone(),
  };
  Ok(())
}
