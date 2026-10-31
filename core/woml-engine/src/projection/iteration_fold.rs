//! projection iteration fold.

use super::{
  require_running_or_cancelling, started_attempt, AttemptStatus, FoldError, ForEachIterationStatus,
  ForEachStatus, LifecycleActionProjection, LifecycleActionStatus, LifecycleHookProjection,
  LifecycleHookStatus, LockProjection, LockStatus, OperationIdentity, OperationProjection,
  OperationStatus, ParallelGroupProjection, ParallelGroupStatus, RetryScheduleProjection,
  ReusableLifecycleProjection, ReusableLifecycleStatus, RunProjection,
};
use crate::event::LifecycleHookCompletionStatus;
use crate::event::RunEvent;
use crate::event::RunEventPayload;
use std::collections::BTreeMap;

pub(super) fn apply_iteration_scoped_event(
  projection: &mut RunProjection,
  event: &RunEvent,
  previous_event: Option<&RunEvent>,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let scope = event.iteration.as_ref().ok_or_else(|| {
    FoldError::InvalidHistory("Loop-owned work is missing iteration scope.".to_string())
  })?;
  let loop_state = projection
    .for_each
    .get_mut(&scope.for_each_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(format!(
        "Loop-owned work references unopened for-each {:?}.",
        scope.for_each_id
      ))
    })?;
  if loop_state.status != ForEachStatus::Open
    || !matches!(
      loop_state.iterations.get(&scope.index),
      Some(ForEachIterationStatus::Started)
    )
  {
    return Err(FoldError::InvalidHistory(format!(
      "Loop-owned work requires active iteration {:?}:{}.",
      scope.for_each_id, scope.index
    )));
  }
  let execution = loop_state.executions.get_mut(&scope.index).ok_or_else(|| {
    FoldError::InvalidHistory("Active iteration has no execution projection.".to_string())
  })?;
  match &event.payload {
    RunEventPayload::StepAttemptStarted(data) => {
      if execution
        .attempts
        .iter()
        .any(|attempt| attempt.identity.invocation_id == data.invocation_id)
        || execution
          .attempts
          .iter()
          .rev()
          .find(|attempt| attempt.identity.node_id == data.node_id)
          .is_some_and(|attempt| attempt.status == AttemptStatus::Started)
      {
        return Err(FoldError::InvalidHistory(format!(
          "Iteration attempt {:?} is not unique and inactive.",
          data.invocation_id
        )));
      }
      let previous = execution
        .attempts
        .iter()
        .rev()
        .find(|attempt| attempt.identity.node_id == data.node_id);
      let expected = previous.map_or(1, |attempt| attempt.identity.attempt + 1);
      if data.attempt != expected {
        return Err(FoldError::InvalidHistory(format!(
          "Iteration node {:?} expected attempt {expected}.",
          data.node_id
        )));
      }
      if let Some(previous) = previous {
        let retry = execution
          .pending_retries
          .get(&data.node_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Iteration retry has no durable schedule.".to_string())
          })?;
        if !matches!(previous.status, AttemptStatus::Failed { .. })
          || retry.next_attempt != data.attempt
          || event.occurred_at < retry.scheduled_at
        {
          return Err(FoldError::InvalidHistory(
            "Iteration retry does not match its failed attempt and schedule.".to_string(),
          ));
        }
        execution.pending_retries.remove(&data.node_id);
      }
      execution.attempts.push(started_attempt(data));
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      if execution.operations.values().any(|operation| {
        operation.identity.invocation_id == data.invocation_id
          && operation.status == OperationStatus::Started
      }) {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt cannot succeed with an active operation.".to_string(),
        ));
      }
      let attempt = execution
        .attempts
        .iter_mut()
        .find(|attempt| {
          attempt.identity.node_id == data.node_id
            && attempt.identity.attempt == data.attempt
            && attempt.identity.invocation_id == data.invocation_id
        })
        .ok_or_else(|| {
          FoldError::InvalidHistory("Iteration success has no matching attempt.".to_string())
        })?;
      if attempt.status != AttemptStatus::Started
        || execution.context.steps.contains_key(&data.node_id)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt already has a terminal outcome.".to_string(),
        ));
      }
      attempt.status = AttemptStatus::Succeeded {
        output: data.output.clone(),
      };
      execution
        .context
        .steps
        .insert(data.node_id.clone(), data.output.clone());
    }
    RunEventPayload::StepAttemptFailed(data) => {
      if execution.operations.values().any(|operation| {
        operation.identity.invocation_id == data.invocation_id
          && operation.status == OperationStatus::Started
      }) {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt cannot fail with an active operation.".to_string(),
        ));
      }
      let attempt = execution
        .attempts
        .iter_mut()
        .find(|attempt| {
          attempt.identity.node_id == data.node_id
            && attempt.identity.attempt == data.attempt
            && attempt.identity.invocation_id == data.invocation_id
        })
        .ok_or_else(|| {
          FoldError::InvalidHistory("Iteration failure has no matching attempt.".to_string())
        })?;
      if attempt.status != AttemptStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt already has a terminal outcome.".to_string(),
        ));
      }
      attempt.status = AttemptStatus::Failed {
        failure: data.failure.clone(),
      };
    }
    RunEventPayload::StepRetryScheduled(data) => {
      let adjacent = previous_event.is_some_and(|previous| {
        previous.iteration.as_ref() == Some(scope)
          && matches!(&previous.payload, RunEventPayload::StepAttemptFailed(failed)
            if failed.node_id == data.node_id && failed.attempt == data.failed_attempt)
      });
      let latest = execution
        .attempts
        .iter()
        .rev()
        .find(|attempt| attempt.identity.node_id == data.node_id);
      if !adjacent
        || !matches!(latest, Some(attempt) if attempt.identity.attempt == data.failed_attempt
          && matches!(attempt.status, AttemptStatus::Failed { .. }))
        || data.next_attempt != data.failed_attempt + 1
        || execution.pending_retries.contains_key(&data.node_id)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration retry schedule does not follow its failed attempt.".to_string(),
        ));
      }
      execution.pending_retries.insert(
        data.node_id.clone(),
        RetryScheduleProjection {
          node_id: data.node_id.clone(),
          failed_attempt: data.failed_attempt,
          next_attempt: data.next_attempt,
          scheduled_at: data.scheduled_at,
        },
      );
    }
    RunEventPayload::ChoiceSelected(data) => {
      if execution
        .choice_selections
        .insert(data.choice_id.clone(), data.arm_id.clone())
        .is_some()
      {
        return Err(FoldError::InvalidHistory(
          "Iteration choice was selected more than once.".to_string(),
        ));
      }
    }
    RunEventPayload::BranchSelected(data) => {
      if execution
        .branch_selections
        .insert(data.branch_id.clone(), data.arm_id.clone())
        .is_some()
      {
        return Err(FoldError::InvalidHistory(
          "Iteration conditional branch was selected more than once.".to_string(),
        ));
      }
    }
    RunEventPayload::ParallelGroupStarted(data) => {
      if execution.parallel_groups.contains_key(&data.parallel_id) {
        return Err(FoldError::InvalidHistory(
          "Iteration parallel group started more than once.".to_string(),
        ));
      }
      execution.parallel_groups.insert(
        data.parallel_id.clone(),
        ParallelGroupProjection {
          parallel_id: data.parallel_id.clone(),
          fork_context: execution.context.clone(),
          status: ParallelGroupStatus::Started,
        },
      );
    }
    RunEventPayload::ParallelGroupCompleted(data) => {
      let group = execution
        .parallel_groups
        .get_mut(&data.parallel_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Iteration parallel completion has no start.".to_string())
        })?;
      if group.status != ParallelGroupStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration parallel group completed more than once.".to_string(),
        ));
      }
      group.status = ParallelGroupStatus::Completed {
        outcome: data.outcome,
        failed_node_ids: data.failed_node_ids.clone(),
        cancelled_node_ids: data.cancelled_node_ids.clone(),
      };
    }
    RunEventPayload::LockWaitRegistered(data) => {
      if execution.locks.contains_key(&data.lock_id) {
        return Err(FoldError::InvalidHistory(
          "Iteration lock was registered more than once.".to_string(),
        ));
      }
      execution.locks.insert(
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
    }
    RunEventPayload::LockAcquired(data) => {
      let lock = execution.locks.get_mut(&data.lock_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lock acquisition has no waiter.".to_string())
      })?;
      if lock.status != LockStatus::Waiting
        || lock.key != data.key
        || lock.execution_identity != data.execution_identity
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lock acquisition does not match its waiter.".to_string(),
        ));
      }
      lock.status = LockStatus::Held;
      lock.lease_id = Some(data.lease_id.clone());
      lock.fencing_token = Some(data.fencing_token.clone());
      lock.acquired_at = Some(data.acquired_at);
      lock.expires_at = Some(data.expires_at);
    }
    RunEventPayload::LockReleased(data) => {
      let lock = execution.locks.get_mut(&data.lock_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lock release has no held lease.".to_string())
      })?;
      if lock.status != LockStatus::Held
        || lock.key != data.key
        || lock.lease_id.as_deref() != Some(data.lease_id.as_str())
        || lock.fencing_token.as_deref() != Some(data.fencing_token.as_str())
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lock release does not match its lease.".to_string(),
        ));
      }
      lock.status = LockStatus::Released;
      lock.resolved_at = Some(data.released_at);
      execution
        .context
        .steps
        .insert(data.lock_id.clone(), data.result.clone());
    }
    RunEventPayload::LockAcquireTimedOut(data) => {
      let lock = execution.locks.get_mut(&data.lock_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lock timeout has no waiter.".to_string())
      })?;
      if lock.status != LockStatus::Waiting
        || lock.key != data.key
        || lock.deadline != data.deadline
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lock timeout does not match its waiter.".to_string(),
        ));
      }
      lock.status = LockStatus::AcquireTimedOut;
      lock.resolved_at = Some(data.resolved_at);
    }
    RunEventPayload::LockAcquisitionCancelled(data) => {
      let lock = execution.locks.get_mut(&data.lock_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lock cancellation has no waiter.".to_string())
      })?;
      if lock.status != LockStatus::Waiting || lock.key != data.key {
        return Err(FoldError::InvalidHistory(
          "Iteration lock cancellation does not match its waiter.".to_string(),
        ));
      }
      lock.status = LockStatus::AcquisitionCancelled;
      lock.resolved_at = Some(data.cancelled_at);
    }
    RunEventPayload::LockLeaseLost(data) => {
      let lock = execution.locks.get_mut(&data.lock_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lease loss has no held lock.".to_string())
      })?;
      if lock.status != LockStatus::Held
        || lock.key != data.key
        || lock.lease_id.as_deref() != Some(data.lease_id.as_str())
        || lock.fencing_token.as_deref() != Some(data.fencing_token.as_str())
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lease loss does not match its held lock.".to_string(),
        ));
      }
      lock.status = LockStatus::LeaseLost;
      lock.resolved_at = Some(data.detected_at);
    }
    RunEventPayload::InfrastructurePolicyDecisionRecorded(_) => {
      return Err(FoldError::InvalidHistory(
        "Infrastructure policy decisions cannot be iteration-scoped.".to_string(),
      ));
    }
    RunEventPayload::OperationStarted(data) => {
      let active = execution.attempts.iter().any(|attempt| {
        attempt.identity.node_id == data.node_id
          && attempt.identity.attempt == data.attempt_number
          && attempt.identity.invocation_id == data.invocation_id
          && attempt.status == AttemptStatus::Started
      });
      let key = OperationIdentity {
        invocation_id: data.invocation_id.clone(),
        call_id: data.call_id.clone(),
      };
      if !active || execution.operations.contains_key(&key) {
        return Err(FoldError::InvalidHistory(
          "Iteration operation requires one matching active attempt.".to_string(),
        ));
      }
      execution.operations.insert(
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
    }
    RunEventPayload::OperationSucceeded(data) => {
      let key = OperationIdentity {
        invocation_id: data.invocation_id.clone(),
        call_id: data.call_id.clone(),
      };
      let operation = execution.operations.get_mut(&key).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration operation success has no start.".to_string())
      })?;
      if operation.status != OperationStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration operation already settled.".to_string(),
        ));
      }
      operation.status = OperationStatus::Succeeded {
        duration_ms: data.duration_ms,
        result_bytes: data.result_bytes,
        result_digest: data.result_digest.clone(),
      };
    }
    RunEventPayload::OperationFailed(data) => {
      let key = OperationIdentity {
        invocation_id: data.invocation_id.clone(),
        call_id: data.call_id.clone(),
      };
      let operation = execution.operations.get_mut(&key).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration operation failure has no start.".to_string())
      })?;
      if operation.status != OperationStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration operation already settled.".to_string(),
        ));
      }
      operation.status = OperationStatus::Failed {
        duration_ms: data.duration_ms,
        failure: data.failure.clone(),
      };
    }
    RunEventPayload::LifecycleHookRequested(data) => {
      if execution
        .lifecycle_hooks
        .contains_key(&data.hook_invocation_id)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle hook was requested more than once.".to_string(),
        ));
      }
      execution.lifecycle_hooks.insert(
        data.hook_invocation_id.clone(),
        LifecycleHookProjection {
          hook_invocation_id: data.hook_invocation_id.clone(),
          hook_id: data.hook_id.clone(),
          event: data.event,
          subject: data.subject.clone(),
          status: LifecycleHookStatus::Requested,
          actions: BTreeMap::new(),
          failed_actions: 0,
        },
      );
    }
    RunEventPayload::LifecycleActionAttemptStarted(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory(
            "Iteration lifecycle action started before its hook.".to_string(),
          )
        })?;
      if hook.actions.contains_key(&data.action_id)
        || hook
          .actions
          .values()
          .any(|action| action.status == LifecycleActionStatus::Started)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle action identity is duplicated.".to_string(),
        ));
      }
      hook.actions.insert(
        data.action_id.clone(),
        LifecycleActionProjection {
          action_id: data.action_id.clone(),
          attempt: data.attempt,
          status: LifecycleActionStatus::Started,
          failure: None,
        },
      );
      hook.status = LifecycleHookStatus::Running;
    }
    RunEventPayload::LifecycleActionSucceeded(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Unknown iteration lifecycle hook.".to_string())
        })?;
      let action = hook.actions.get_mut(&data.action_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lifecycle action has no start.".to_string())
      })?;
      if action.status != LifecycleActionStatus::Started || action.attempt != data.attempt {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle success does not close its action.".to_string(),
        ));
      }
      action.status = LifecycleActionStatus::Succeeded;
    }
    RunEventPayload::LifecycleActionFailed(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Unknown iteration lifecycle hook.".to_string())
        })?;
      let action = hook.actions.get_mut(&data.action_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lifecycle action has no start.".to_string())
      })?;
      if action.status != LifecycleActionStatus::Started || action.attempt != data.attempt {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle failure does not close its action.".to_string(),
        ));
      }
      action.status = LifecycleActionStatus::Failed;
      action.failure = Some(data.failure.clone());
      hook.failed_actions += 1;
    }
    RunEventPayload::LifecycleHookCompleted(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Unknown iteration lifecycle hook.".to_string())
        })?;
      if hook
        .actions
        .values()
        .any(|action| action.status == LifecycleActionStatus::Started)
        || hook.failed_actions != data.failed_actions
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle completion does not match its actions.".to_string(),
        ));
      }
      hook.status = match data.status {
        LifecycleHookCompletionStatus::Completed => LifecycleHookStatus::Completed,
        LifecycleHookCompletionStatus::CompletedWithWarnings => {
          LifecycleHookStatus::CompletedWithWarnings
        }
      };
    }
    RunEventPayload::ReusableLifecycleRequested(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      if execution.reusable_lifecycle_hooks.contains_key(&key) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle hook was requested more than once.".to_string(),
        ));
      }
      execution.reusable_lifecycle_hooks.insert(
        key,
        ReusableLifecycleProjection {
          invocation_id: data.invocation_id.clone(),
          definition_digest: data.definition_digest.clone(),
          hook: data.hook,
          status: ReusableLifecycleStatus::Requested,
          active_action_id: None,
          completed_action_ids: Vec::new(),
          warning_codes: Vec::new(),
        },
      );
    }
    RunEventPayload::ReusableLifecycleActionStarted(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.is_some() || hook.completed_action_ids.contains(&data.action_id) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle action is duplicated.".to_string(),
        ));
      }
      hook.active_action_id = Some(data.action_id.clone());
      hook.status = ReusableLifecycleStatus::Running;
    }
    RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.as_deref() != Some(data.action_id.as_str()) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle success has no active action.".to_string(),
        ));
      }
      hook.active_action_id = None;
      hook.completed_action_ids.push(data.action_id.clone());
    }
    RunEventPayload::ReusableLifecycleActionFailed(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.as_deref() != Some(data.action_id.as_str()) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle failure has no active action.".to_string(),
        ));
      }
      hook.active_action_id = None;
      hook.completed_action_ids.push(data.action_id.clone());
      hook.warning_codes.push(data.warning_code.clone());
    }
    RunEventPayload::ReusableLifecycleCompleted(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.is_some() {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle completed with an active action.".to_string(),
        ));
      }
      hook.status = if data.outcome == crate::ReusableLifecycleOutcome::CompletedWithWarnings {
        ReusableLifecycleStatus::CompletedWithWarnings
      } else {
        ReusableLifecycleStatus::Completed
      };
    }
    _ => {
      return Err(FoldError::InvalidHistory(
        "Payload is not valid iteration-scoped work.".to_string(),
      ))
    }
  }
  Ok(())
}
