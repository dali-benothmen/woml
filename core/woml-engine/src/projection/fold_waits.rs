//! projection fold waits.

use super::{
  require_running, DurableWaitProjection, DurableWaitStatus, FoldError, RunProjection, RunStatus,
};
use chrono::SecondsFormat;
use serde_json::json;

pub(super) fn apply_wait_registered(
  projection: &mut RunProjection,
  data: &crate::event::WaitRegisteredData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.durable_waits.contains_key(&data.wait_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Durable wait {:?} was registered more than once.",
      data.wait_id
    )));
  }
  projection.durable_waits.insert(
    data.wait_id.clone(),
    DurableWaitProjection {
      wait_id: data.wait_id.clone(),
      node_id: data.node_id.clone(),
      kind: data.kind,
      registered_at: data.registered_at,
      deadline: data.deadline,
      signals: data.signals.clone().unwrap_or_default(),
      correlation_digest: data.correlation_digest.clone(),
      status: DurableWaitStatus::Registered,
      resolved_at: None,
    },
  );
  projection.status = RunStatus::Waiting;
  Ok(())
}

pub(super) fn apply_external_signal_received(
  projection: &mut RunProjection,
  data: &crate::event::ExternalSignalReceivedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting {
    return Err(FoldError::InvalidHistory(
      "external_signal_received requires a waiting run.".to_string(),
    ));
  }
  let wait = projection.durable_waits.get(&data.wait_id).ok_or_else(|| {
    FoldError::InvalidHistory(
      "External signal references an unregistered durable wait.".to_string(),
    )
  })?;
  if wait.status != DurableWaitStatus::Registered
    || !wait.signals.iter().any(|signal| signal == &data.signal)
    || projection.external_signals.iter().any(|signal| {
      signal.idempotency_digest == data.idempotency_digest || signal.signal_id == data.signal_id
    })
  {
    return Err(FoldError::InvalidHistory(
      "External signal does not match one unresolved durable wait.".to_string(),
    ));
  }
  projection.external_signals.push(data.clone());
  Ok(())
}

pub(super) fn apply_wait_satisfied(
  projection: &mut RunProjection,
  data: &crate::event::WaitSatisfiedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting {
    return Err(FoldError::InvalidHistory(
      "wait_satisfied requires a waiting run.".to_string(),
    ));
  }
  let wait = projection
    .durable_waits
    .get_mut(&data.wait_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("wait_satisfied has no registered wait.".to_string())
    })?;
  if wait.status != DurableWaitStatus::Registered
    || matches!(data.winner, crate::DurableWaitWinner::Timer)
      && (!matches!(
        wait.kind,
        crate::DurableWaitKind::Duration | crate::DurableWaitKind::Until
      ) || wait
        .deadline
        .is_none_or(|deadline| data.resolved_at < deadline))
    || matches!(data.winner, crate::DurableWaitWinner::Signal)
      && !projection
        .external_signals
        .iter()
        .any(|signal| signal.wait_id == data.wait_id && signal.received_at <= data.resolved_at)
  {
    return Err(FoldError::InvalidHistory(
      "wait_satisfied does not resolve the active wait exactly once.".to_string(),
    ));
  }
  wait.status = DurableWaitStatus::Satisfied {
    winner: data.winner,
  };
  wait.resolved_at = Some(data.resolved_at);
  projection
    .context
    .steps
    .insert(data.wait_id.clone(), data.result.clone());
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_wait_timed_out(
  projection: &mut RunProjection,
  data: &crate::event::WaitTimedOutData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Waiting {
    return Err(FoldError::InvalidHistory(
      "wait_timed_out requires a waiting run.".to_string(),
    ));
  }
  let wait = projection
    .durable_waits
    .get_mut(&data.wait_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("wait_timed_out has no registered wait.".to_string())
    })?;
  if wait.status != DurableWaitStatus::Registered
    || wait.kind != crate::DurableWaitKind::Signal
    || wait.deadline != Some(data.deadline)
  {
    return Err(FoldError::InvalidHistory(
      "wait_timed_out does not close the active signal wait.".to_string(),
    ));
  }
  wait.status = DurableWaitStatus::TimedOut {
    behavior: data.behavior,
  };
  wait.resolved_at = Some(data.resolved_at);
  if data.behavior == crate::DurableWaitTimeoutBehavior::Continue {
    projection.context.steps.insert(
      data.wait_id.clone(),
      json!({
        "status": "timed_out",
        "deadline": data.deadline.to_rfc3339_opts(SecondsFormat::Millis, true),
        "resumedAt": data.resolved_at.to_rfc3339_opts(SecondsFormat::Millis, true),
      }),
    );
  }
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_wait_cancelled(
  projection: &mut RunProjection,
  data: &crate::event::WaitCancelledData,
) -> Result<(), FoldError> {
  let wait = projection
    .durable_waits
    .get_mut(&data.wait_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("wait_cancelled has no registered wait.".to_string())
    })?;
  if wait.status != DurableWaitStatus::Registered
    || !matches!(
      projection.status,
      RunStatus::Waiting | RunStatus::Cancelling
    )
  {
    return Err(FoldError::InvalidHistory(
      "wait_cancelled does not close one active wait.".to_string(),
    ));
  }
  wait.status = DurableWaitStatus::Cancelled;
  wait.resolved_at = Some(data.cancelled_at);
  Ok(())
}
