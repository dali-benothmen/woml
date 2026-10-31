//! Reconstruct the proven primary failure without replaying effects.
//!
//! An old failed attempt must not override a later success. Successful
//! compensation is not successful business work; retain original failure facts.

use crate::event::RunFailedData;
use crate::event::RunFailedDataV1;
use crate::event::RunFailedDataV2;
use crate::event::StepAttemptFailedData;
use crate::projection::AttemptStatus;
use crate::runtime::errors::BranchFailureSite;
use crate::runtime::errors::FailedApprovalDetails;
use crate::runtime::errors::FailedBranchDetails;
use crate::runtime::errors::FailedNotificationDetails;
use crate::runtime::errors::FailedParallelDetails;
use crate::runtime::errors::FailedRunDetails;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::progress::retry_max_attempts;
use crate::runtime::RuntimeDagEngine;
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::BranchFailure;
use crate::RunEventPayload;
use crate::RunFailure;
use crate::RunProjection;
use crate::RUN_EVENT_SCHEMA_VERSION_V1;
use crate::RUN_EVENT_SCHEMA_VERSION_V2;
use std::collections::HashSet;

pub(in crate::runtime) fn final_attempt_failure_in_nodes(
  projection: &RunProjection,
  node_ids: &HashSet<String>,
) -> Option<StepAttemptFailedData> {
  projection.attempts.iter().rev().find_map(|attempt| {
    if !node_ids.contains(&attempt.identity.node_id)
      || projection
        .context
        .steps
        .contains_key(&attempt.identity.node_id)
      || projection
        .pending_retries
        .contains_key(&attempt.identity.node_id)
    {
      return None;
    }
    // Only the last attempt for a node can be a final failure. Looking at the
    // latest projection directly also avoids treating an earlier failed retry
    // as fatal after a later attempt succeeded.
    let latest = projection.latest_attempt(&attempt.identity.node_id)?;
    if latest.identity != attempt.identity {
      return None;
    }
    let AttemptStatus::Failed { failure } = &attempt.status else {
      return None;
    };
    Some(StepAttemptFailedData {
      node_id: attempt.identity.node_id.clone(),
      attempt: attempt.identity.attempt,
      invocation_id: attempt.identity.invocation_id.clone(),
      failure: failure.clone(),
    })
  })
}

pub(in crate::runtime) fn final_workflow_attempt_failure(
  projection: &RunProjection,
) -> Option<StepAttemptFailedData> {
  let node_ids = projection
    .attempts
    .iter()
    .map(|attempt| attempt.identity.node_id.clone())
    .collect::<HashSet<_>>();
  final_attempt_failure_in_nodes(projection, &node_ids)
}

pub(in crate::runtime) fn resumed_failure<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  projection: RunProjection,
) -> Result<RuntimeExecutionError, RuntimeExecutionError> {
  let events = engine.events(run_id)?;
  if let Some(lifecycle_failure) = projection.lifecycle_failure.clone() {
    let kind = match lifecycle_failure.kind {
      crate::LifecycleFailureKind::TimedOut => AttemptFailureKind::ScriptTimedOut,
      crate::LifecycleFailureKind::NonJson => AttemptFailureKind::InvalidScriptResult,
      crate::LifecycleFailureKind::WorkerCrashed => AttemptFailureKind::WorkerCrashed,
      crate::LifecycleFailureKind::HostCrashed => AttemptFailureKind::HostCrashed,
      crate::LifecycleFailureKind::Interrupted => AttemptFailureKind::Interrupted,
      crate::LifecycleFailureKind::SizeLimitExceeded => AttemptFailureKind::ResultTooLarge,
      crate::LifecycleFailureKind::Cancelled => AttemptFailureKind::InvocationCancelled,
      crate::LifecycleFailureKind::ProviderFailed => AttemptFailureKind::ServiceFailed,
      crate::LifecycleFailureKind::ScriptThrew => AttemptFailureKind::ScriptThrew,
    };
    let failure = AttemptFailure {
      kind,
      code: lifecycle_failure.code.clone(),
      message: lifecycle_failure.message.clone(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    };
    let identity = events.iter().rev().find_map(|event| match &event.payload {
      RunEventPayload::StepAttemptFailed(data)
        if data.failure.code == lifecycle_failure.code
          && data.failure.message == lifecycle_failure.message =>
      {
        Some((data.node_id.clone(), data.attempt))
      }
      _ => None,
    });
    let (node_id, attempt, max_attempts) = identity
      .map(|(node_id, attempt)| {
        let maximum = retry_max_attempts(engine, &node_id);
        (Some(node_id), Some(attempt), Some(maximum))
      })
      .unwrap_or((None, None, None));
    return Ok(RuntimeExecutionError::RunFailed(Box::new(
      FailedRunDetails {
        code: lifecycle_failure.code,
        message: lifecycle_failure.message,
        node_id,
        attempt,
        max_attempts,
        failure,
        events,
      },
    )));
  }
  let failure = projection.failure.ok_or_else(|| {
    RuntimeExecutionError::Stalled("failed run has no folded failure".to_string())
  })?;
  Ok(match failure {
    RunFailure::Attempt(failure) => {
      let identity = events.iter().rev().find_map(|event| match &event.payload {
        RunEventPayload::RunFailed(RunFailedData::V1(data)) => data
          .node_id
          .as_ref()
          .zip(data.attempt)
          .map(|(node_id, attempt)| (node_id.clone(), attempt)),
        RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Attempt {
          node_id,
          attempt,
          ..
        })) => Some((node_id.clone(), *attempt)),
        _ => None,
      });
      let (node_id, attempt, max_attempts) = identity
        .map(|(node_id, attempt)| {
          let max_attempts = retry_max_attempts(engine, &node_id);
          (Some(node_id), Some(attempt), Some(max_attempts))
        })
        .unwrap_or((None, None, None));
      let exhausted = attempt
        .zip(max_attempts)
        .is_some_and(|(attempt, maximum)| maximum > 1 && attempt >= maximum)
        && failure.kind == AttemptFailureKind::ScriptThrew;
      let code = if exhausted {
        "WOML_STEP_RETRIES_EXHAUSTED".to_string()
      } else {
        failure.code.clone()
      };
      let message = if exhausted {
        format!(
          "attempt {} of {} failed [{}].",
          attempt.unwrap(),
          max_attempts.unwrap(),
          failure.code
        )
      } else {
        failure.message.clone()
      };
      RuntimeExecutionError::RunFailed(Box::new(FailedRunDetails {
        code,
        message,
        node_id,
        attempt,
        max_attempts,
        failure,
        events,
      }))
    }
    RunFailure::Parallel {
      parallel_id,
      policy,
      primary_node_id,
      failed_node_ids,
      cancelled_node_ids,
      failure,
    } => RuntimeExecutionError::ParallelFailed(Box::new(FailedParallelDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      parallel_id,
      policy,
      primary_node_id,
      failed_node_ids,
      cancelled_node_ids,
      failure,
      events,
    })),
    RunFailure::Branch(failure) => {
      let (branch_id, arm_id, path) = events
        .iter()
        .rev()
        .find_map(|event| match &event.payload {
          RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Branch {
            branch_id,
            arm_id,
            path,
            ..
          })) => Some((branch_id.clone(), arm_id.clone(), path.clone())),
          _ => None,
        })
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "failed branch run has no durable branch identity".to_string(),
          )
        })?;
      let site = match &failure {
        BranchFailure::BranchTestNotBoolean { .. } => BranchFailureSite::Test,
        BranchFailure::ReferenceNotAvailable { .. }
          if projection.branch_selections.contains_key(&branch_id) =>
        {
          BranchFailureSite::Result
        }
        BranchFailure::ReferenceNotAvailable { .. } => BranchFailureSite::Test,
        BranchFailure::BranchSelectionInvalid { .. } => BranchFailureSite::Selection,
      };
      let message = match &failure {
        BranchFailure::BranchTestNotBoolean { message, .. }
        | BranchFailure::ReferenceNotAvailable { message, .. }
        | BranchFailure::BranchSelectionInvalid { message, .. } => message.clone(),
      };
      RuntimeExecutionError::BranchFailed(Box::new(FailedBranchDetails {
        code: failure.code().to_string(),
        message,
        branch_id,
        arm_id,
        path,
        site,
        failure,
        events,
      }))
    }
    RunFailure::Approval {
      approval_id,
      request_id,
      failure,
    } => RuntimeExecutionError::ApprovalFailed(Box::new(FailedApprovalDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      approval_id,
      request_id,
      failure,
      events,
    })),
    RunFailure::Notification {
      approval_id,
      request_id,
      failed_delivery_ids,
      failure,
    } => RuntimeExecutionError::NotificationFailed(Box::new(FailedNotificationDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      approval_id,
      request_id,
      failed_delivery_ids,
      failure,
      events,
    })),
  })
}

pub(in crate::runtime) fn attempt_run_failed_data(
  event_schema_version: u32,
  failure: &StepAttemptFailedData,
) -> RunFailedData {
  match event_schema_version {
    RUN_EVENT_SCHEMA_VERSION_V1 => RunFailedData::V1(RunFailedDataV1 {
      node_id: Some(failure.node_id.clone()),
      attempt: Some(failure.attempt),
      invocation_id: Some(failure.invocation_id.clone()),
      failure: failure.failure.clone(),
    }),
    RUN_EVENT_SCHEMA_VERSION_V2
    | crate::RUN_EVENT_SCHEMA_VERSION_V3
    | crate::RUN_EVENT_SCHEMA_VERSION_V4
    | crate::RUN_EVENT_SCHEMA_VERSION_V5
    | crate::RUN_EVENT_SCHEMA_VERSION_V6
    | crate::RUN_EVENT_SCHEMA_VERSION_V7
    | crate::RUN_EVENT_SCHEMA_VERSION_V8
    | crate::RUN_EVENT_SCHEMA_VERSION_V9
    | crate::RUN_EVENT_SCHEMA_VERSION_V10
    | crate::RUN_EVENT_SCHEMA_VERSION_V11
    | crate::RUN_EVENT_SCHEMA_VERSION_V12
    | crate::RUN_EVENT_SCHEMA_VERSION_V13
    | crate::RUN_EVENT_SCHEMA_VERSION_V14
    | crate::RUN_EVENT_SCHEMA_VERSION_V15
    | crate::RUN_EVENT_SCHEMA_VERSION_V16
    | crate::RUN_EVENT_SCHEMA_VERSION_V17
    | crate::RUN_EVENT_SCHEMA_VERSION_V18
    | crate::RUN_EVENT_SCHEMA_VERSION_V19
    | crate::RUN_EVENT_SCHEMA_VERSION_V20 => RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id: failure.node_id.clone(),
      attempt: failure.attempt,
      invocation_id: failure.invocation_id.clone(),
      failure: failure.failure.clone(),
    }),
    _ => unreachable!("compiled models select a supported run-event version"),
  }
}
