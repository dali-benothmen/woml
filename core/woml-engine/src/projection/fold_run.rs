//! projection fold run.

use super::{
  require_failed_attempt, require_running, ApprovalRequestStatus, AttemptIdentity, FoldError,
  LifecycleStatus, NotificationDeliveryStatus, OperationStatus, ParallelGroupStatus, RunFailure,
  RunProjection, RunStatus,
};
use crate::event::ApprovalResolution;
use crate::event::BusinessOutcome;
use crate::event::OperationExecutionMode;
use crate::event::ParallelGroupOutcome;
use crate::event::RunEvent;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV2;
use crate::event::RunFailedDataV3;
use crate::event::RunFailedDataV4;
use crate::event::RunFailedDataV5;
use std::collections::HashMap;

pub(super) fn apply_run_admitted(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::RunAdmittedData,
  index: usize,
) -> Result<(), FoldError> {
  if index != 0 || projection.status != RunStatus::NotStarted {
    return Err(FoldError::InvalidHistory(
      "run_admitted must be the first and only admission event.".to_string(),
    ));
  }
  projection.run_id = Some(event.run_id.clone());
  projection.event_schema_version = Some(event.event_schema_version);
  projection.definition_hash = Some(data.definition_hash.clone());
  projection.policy_hash = Some(data.policy_hash.clone());
  projection.trigger_id = Some(data.trigger.id.clone());
  projection.trigger_handler = Some(data.trigger.handler.clone());
  projection.context.trigger = data.payload.clone();
  projection.queue = Some(data.queue.name.clone());
  projection.occurrence_sequence = Some(data.occurrence_sequence);
  projection.admitted_at = Some(data.admitted_at);
  projection.status = RunStatus::Queued;
  Ok(())
}

pub(super) fn apply_run_execution_started(
  projection: &mut RunProjection,
  data: &crate::event::RunExecutionStartedData,
) -> Result<(), FoldError> {
  if projection.status != RunStatus::Queued || projection.started_at.is_some() {
    return Err(FoldError::InvalidHistory(
      "run_execution_started requires one queued admission.".to_string(),
    ));
  }
  projection.started_at = Some(data.started_at);
  projection.timeout_at = data.timeout_at;
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_run_timeout_reached(
  projection: &mut RunProjection,
  data: &crate::event::RunTimeoutReachedData,
) -> Result<(), FoldError> {
  if !matches!(projection.status, RunStatus::Running | RunStatus::Waiting)
    || projection.timeout_at != Some(data.deadline_at)
    || projection.timeout_reached_at.is_some()
  {
    return Err(FoldError::InvalidHistory(
      "run_timeout_reached must match the active run deadline exactly once.".to_string(),
    ));
  }
  projection.timeout_reached_at = Some(data.deadline_at);
  Ok(())
}

pub(super) fn apply_run_started(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::RunStartedData,
  index: usize,
) -> Result<(), FoldError> {
  if index != 0 || projection.status != RunStatus::NotStarted {
    return Err(FoldError::InvalidHistory(
      "run_started must be the first and only start event.".to_string(),
    ));
  }
  projection.run_id = Some(event.run_id.clone());
  projection.event_schema_version = Some(event.event_schema_version);
  projection.workflow_id = Some(data.workflow_id.clone());
  projection.definition_hash = Some(data.definition_hash.clone());
  projection.trigger_id = data.trigger_id.clone();
  projection.trigger_handler = data.trigger_handler.clone();
  projection.trigger_occurrence_id = data.trigger_occurrence_id.clone();
  projection.ingress = data.ingress.clone();
  projection.context.trigger = data.trigger.clone();
  projection.started_at = Some(event.occurred_at);
  projection.status = RunStatus::Running;
  Ok(())
}

pub(super) fn apply_run_cancellation_requested(
  projection: &mut RunProjection,
  data: &crate::event::RunCancellationRequestedData,
) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Queued | RunStatus::Running | RunStatus::Waiting
  ) {
    return Err(FoldError::InvalidHistory(
      "Cancellation may only be requested before business outcome is decided.".to_string(),
    ));
  }
  if projection.cancellation_request_id.is_some() {
    return Err(FoldError::InvalidHistory(
      "A run may contain only one durable cancellation request.".to_string(),
    ));
  }
  projection.cancellation_request_id = Some(data.request_id.clone());
  projection.status = RunStatus::Cancelling;
  Ok(())
}

pub(super) fn apply_run_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::RunSucceededData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.operations.values().any(|operation| {
    operation.execution_mode == OperationExecutionMode::Managed
      && operation.status == OperationStatus::Started
  }) {
    return Err(FoldError::InvalidHistory(
      "A run cannot succeed while a managed operation remains active.".to_string(),
    ));
  }
  if !projection.pending_retries.is_empty() {
    return Err(FoldError::InvalidHistory(
      "A run cannot succeed while a retry remains pending.".to_string(),
    ));
  }
  let terminal_output_exists = projection
    .context
    .steps
    .contains_key(&data.terminal_node_id)
    || data
      .terminal_node_id
      .strip_prefix("__woml_approval__")
      .and_then(|value| value.strip_suffix("__join"))
      .is_some_and(|approval_id| projection.context.steps.contains_key(approval_id));
  if !terminal_output_exists {
    return Err(FoldError::InvalidHistory(format!(
      "Terminal node {:?} has not published a successful output.",
      data.terminal_node_id
    )));
  }
  projection.status = RunStatus::Succeeded;
  projection.business_outcome = Some(BusinessOutcome::Succeeded);
  projection.lifecycle_status = LifecycleStatus::Completed;
  projection.terminal_node_id = Some(data.terminal_node_id.clone());
  projection.result = Some(data.result.clone());
  Ok(())
}

pub(super) fn apply_run_failed(
  projection: &mut RunProjection,
  data: &crate::event::RunFailedData,
  attempt_indexes: &mut HashMap<AttemptIdentity, usize>,
) -> Result<(), FoldError> {
  if projection.operations.values().any(|operation| {
    operation.execution_mode == OperationExecutionMode::Managed
      && operation.status == OperationStatus::Started
  }) {
    return Err(FoldError::InvalidHistory(
      "A run cannot fail while a managed operation remains active.".to_string(),
    ));
  }
  if matches!(data, RunFailedData::V5(_)) {
    if projection.status != RunStatus::Waiting {
      return Err(FoldError::InvalidHistory(
        "Notification-scoped run_failed requires a waiting approval.".to_string(),
      ));
    }
  } else {
    require_running(projection)?;
  }
  let failure = match data {
    RunFailedData::V5(RunFailedDataV5::Notification {
      approval_id,
      request_id,
      failed_delivery_ids,
      failure,
    }) => {
      let request = projection
        .approval_requests
        .get(approval_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory(
            "Notification run failure references an unknown approval.".to_string(),
          )
        })?;
      let actual_failed = projection
        .notification_deliveries
        .values()
        .filter(|delivery| {
          delivery.approval_id == *approval_id && delivery.request_id == *request_id
        })
        .filter_map(|delivery| {
          matches!(
            delivery.status,
            NotificationDeliveryStatus::Failed { final_: true, .. }
          )
          .then_some(delivery.delivery_id.clone())
        })
        .collect::<Vec<_>>();
      let any_succeeded = projection.notification_deliveries.values().any(|delivery| {
        delivery.approval_id == *approval_id
          && delivery.request_id == *request_id
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      });
      if request.request_id != *request_id || any_succeeded || actual_failed != *failed_delivery_ids
      {
        return Err(FoldError::InvalidHistory(
          "Notification run failure does not match final delivery outcomes.".to_string(),
        ));
      }
      RunFailure::Notification {
        approval_id: approval_id.clone(),
        request_id: request_id.clone(),
        failed_delivery_ids: failed_delivery_ids.clone(),
        failure: failure.clone(),
      }
    }
    RunFailedData::V1(data) => {
      if let (Some(node_id), Some(attempt), Some(invocation_id)) =
        (&data.node_id, data.attempt, &data.invocation_id)
      {
        require_failed_attempt(projection, attempt_indexes, node_id, attempt, invocation_id)?;
      }
      RunFailure::Attempt(data.failure.clone())
    }
    RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id,
      attempt,
      invocation_id,
      failure,
    }) => {
      require_failed_attempt(
        projection,
        attempt_indexes,
        node_id,
        *attempt,
        invocation_id,
      )?;
      RunFailure::Attempt(failure.clone())
    }
    RunFailedData::V2(RunFailedDataV2::Branch { failure, .. }) => {
      RunFailure::Branch(failure.clone())
    }
    RunFailedData::V3(RunFailedDataV3::Parallel {
      parallel_id,
      policy,
      primary_node_id,
      failed_node_ids,
      cancelled_node_ids,
      failure,
    }) => {
      let group = projection.parallel_groups.get(parallel_id).ok_or_else(|| {
        FoldError::InvalidHistory(format!(
          "run_failed references parallel group {parallel_id:?} before it started."
        ))
      })?;
      match &group.status {
        ParallelGroupStatus::Completed {
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: completed_failed,
          cancelled_node_ids: completed_cancelled,
        } if completed_failed == failed_node_ids && completed_cancelled == cancelled_node_ids => {}
        _ => {
          return Err(FoldError::InvalidHistory(format!(
            "run_failed does not match the failed completion of parallel group {parallel_id:?}."
          )));
        }
      }
      RunFailure::Parallel {
        parallel_id: parallel_id.clone(),
        policy: *policy,
        primary_node_id: primary_node_id.clone(),
        failed_node_ids: failed_node_ids.clone(),
        cancelled_node_ids: cancelled_node_ids.clone(),
        failure: failure.clone(),
      }
    }
    RunFailedData::V4(RunFailedDataV4::Approval {
      approval_id,
      request_id,
      failure,
    }) => {
      let request = projection
        .approval_requests
        .get(approval_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "run_failed references unknown approval {approval_id:?}."
          ))
        })?;
      if request.request_id != *request_id
        || !matches!(
          request.status,
          ApprovalRequestStatus::Resolved {
            resolution: ApprovalResolution::TimeoutFailure,
            ..
          }
        )
      {
        return Err(FoldError::InvalidHistory(format!(
          "run_failed does not match the timeout failure of approval {approval_id:?}."
        )));
      }
      RunFailure::Approval {
        approval_id: approval_id.clone(),
        request_id: request_id.clone(),
        failure: failure.clone(),
      }
    }
  };
  projection.pending_retries.clear();
  projection.status = RunStatus::Failed;
  projection.business_outcome = Some(BusinessOutcome::Failed);
  projection.lifecycle_status = LifecycleStatus::Completed;
  projection.failure = Some(failure);
  Ok(())
}
