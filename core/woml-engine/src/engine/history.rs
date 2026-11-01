//! engine history.

mod prefix;
#[cfg(test)]
mod tests;

use prefix::HistoryPrefix;

use super::{
  node_is_complete, ready_node_ids_for_projection, ready_node_ids_for_projection_at,
  step_effect_idempotency_key, validate_payload_against_definition,
  validate_scoped_payload_against_definition,
};
use crate::event::AttemptFailureKind;
use crate::event::DurableWaitTimeoutBehavior;
use crate::event::ForkBranchOutcome;
use crate::event::ForkJoinOutcome;
use crate::event::ParallelGroupOutcome;
use crate::event::RunEventPayload;
use crate::event::RunOutcomeDecidedData;
use crate::CompiledWorkflowDefinition;
use crate::ParallelGroupStatus;
use crate::RunEvent;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParallelChildState {
  Active,
  RetryPending,
  Succeeded,
  Failed(AttemptFailureKind),
}

pub(crate) fn validate_event_history_against_definition(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &[RunEvent],
) -> Result<(), String> {
  validate_history_with_prefixes(workflow, definition_hash, events, HistoryPrefix::new)
}

fn validate_history_with_prefixes<'a>(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &'a [RunEvent],
  make_prefixes: impl FnOnce(&'a [RunEvent]) -> HistoryPrefix<'a>,
) -> Result<(), String> {
  // Preserve the full structural gate and its error precedence. Prefix state
  // belongs only to this immutable slice and never escapes to a store/cache.
  crate::fold_events(events).map_err(|error| error.to_string())?;
  let mut prefixes = make_prefixes(events);
  let mut child_states: HashMap<String, ParallelChildState> = HashMap::new();

  for (index, event) in events.iter().enumerate() {
    if let Some(scope) = &event.iteration {
      validate_scoped_payload_against_definition(
        workflow,
        definition_hash,
        &event.run_id,
        scope,
        &event.payload,
      )?;
      if !matches!(
        event.payload,
        RunEventPayload::ForEachIterationStarted(_)
          | RunEventPayload::ForEachIterationSucceeded(_)
          | RunEventPayload::ForEachIterationFailed(_)
          | RunEventPayload::ForEachIterationSkipped(_)
      ) {
        continue;
      }
    } else {
      validate_payload_against_definition(workflow, definition_hash, &event.payload)?;
    }
    match &event.payload {
      RunEventPayload::ChoiceSelected(data) => {
        let choice = workflow
          .graph
          .choices
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|choice| choice.choice_id == data.choice_id)
          .ok_or_else(|| format!("Unknown control choice {:?}.", data.choice_id))?;
        let prefix = prefixes.at(index)?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, prefix)?;
        if !ready.contains(&choice.selector_node_id) {
          return Err(format!(
            "Control choice {:?} was selected before its selector was ready.",
            data.choice_id
          ));
        }
      }
      RunEventPayload::ForkOpened(data) => {
        let fork = workflow
          .graph
          .forks
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|fork| fork.fork_id == data.fork_id)
          .ok_or_else(|| format!("Unknown fork {:?}.", data.fork_id))?;
        let prefix = prefixes.at(index)?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, prefix)?;
        if !ready.contains(&fork.open_node_id) {
          return Err(format!(
            "Fork {:?} opened before it was ready.",
            data.fork_id
          ));
        }
      }
      RunEventPayload::ForkBranchSettled(data) => {
        let fork = workflow
          .graph
          .forks
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|fork| fork.fork_id == data.fork_id)
          .ok_or_else(|| format!("Unknown fork {:?}.", data.fork_id))?;
        let branch = fork
          .branches
          .iter()
          .find(|branch| branch.branch_id == data.branch_id)
          .ok_or_else(|| format!("Unknown fork branch {:?}.", data.branch_id))?;
        if data.outcome == ForkBranchOutcome::Succeeded {
          let prefix = prefixes.at(index)?;
          let terminal_ready = workflow
            .graph
            .edges
            .iter()
            .filter(|edge| edge.to == branch.terminal_node_id)
            .all(|edge| {
              workflow
                .node(&edge.from)
                .is_some_and(|node| node_is_complete(workflow, node, prefix))
            });
          if !terminal_ready {
            return Err(format!(
              "Fork branch {:?}.{:?} succeeded before its terminal route completed.",
              data.fork_id, data.branch_id
            ));
          }
        }
      }
      RunEventPayload::ForkJoinSettled(data) => {
        let fork = workflow
          .graph
          .forks
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|fork| fork.fork_id == data.fork_id)
          .ok_or_else(|| format!("Unknown fork {:?}.", data.fork_id))?;
        let prefix = prefixes.at(index)?;
        let projected = prefix
          .forks
          .get(&data.fork_id)
          .ok_or_else(|| format!("Fork {:?} join settled before opening.", data.fork_id))?;
        if data.outcome == ForkJoinOutcome::Succeeded
          && !fork.joined_branch_ids.iter().all(|branch_id| {
            projected
              .branches
              .get(branch_id)
              .and_then(|branch| branch.outcome)
              == Some(ForkBranchOutcome::Succeeded)
          })
        {
          return Err(format!(
            "Fork {:?} join succeeded before every joined branch succeeded.",
            data.fork_id
          ));
        }
      }
      RunEventPayload::SagaStarted(data) => {
        let saga = workflow
          .saga(&data.saga_id)
          .ok_or_else(|| format!("Unknown saga {:?}.", data.saga_id))?;
        let prefix = prefixes.at(index)?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, prefix)?;
        if !ready.contains(&saga.open_node_id) {
          return Err(format!(
            "Saga {:?} started before its open node was ready.",
            data.saga_id
          ));
        }
      }
      RunEventPayload::CorrectionAttemptStarted(data) => {
        let saga = workflow
          .saga(&data.saga_id)
          .ok_or_else(|| format!("Unknown saga {:?}.", data.saga_id))?;
        let prefix = prefixes.at(index)?;
        let state = prefix
          .sagas
          .get(&data.saga_id)
          .ok_or_else(|| format!("Saga {:?} has no projected correction state.", data.saga_id))?;
        match data.action_kind {
          crate::CorrectiveActionKind::Compensation => {
            if state.reason == Some(crate::SagaCorrectionReason::Success) {
              return Err("Compensation cannot run after successful forward work.".to_string());
            }
            let next = state
              .actions
              .values()
              .filter(|action| {
                action.action_kind == crate::CorrectiveActionKind::Compensation
                  && matches!(
                    action.status,
                    crate::CorrectionActionStatus::Registered
                      | crate::CorrectionActionStatus::RetryScheduled
                  )
              })
              .max_by_key(|action| action.completion_sequence)
              .map(|action| action.action_id.as_str());
            if next != Some(data.action_id.as_str()) {
              return Err(format!(
                "Compensation {:?} is not next in reverse durable completion order.",
                data.action_id
              ));
            }
          }
          crate::CorrectiveActionKind::Cleanup => {
            if state.reason != Some(crate::SagaCorrectionReason::Success)
              && state.actions.values().any(|action| {
                action.action_kind == crate::CorrectiveActionKind::Compensation
                  && matches!(
                    action.status,
                    crate::CorrectionActionStatus::Registered
                      | crate::CorrectionActionStatus::Started
                      | crate::CorrectionActionStatus::RetryScheduled
                  )
              })
            {
              return Err("Cleanup cannot start before compensation settles.".to_string());
            }
            let position = saga
              .cleanup
              .iter()
              .position(|action| action.action_id == data.action_id)
              .ok_or_else(|| "Cleanup action is absent from Model v19.".to_string())?;
            if saga.cleanup[..position].iter().any(|action| {
              !state
                .actions
                .get(&action.action_id)
                .is_some_and(|projected| {
                  matches!(
                    projected.status,
                    crate::CorrectionActionStatus::Succeeded
                      | crate::CorrectionActionStatus::Failed
                      | crate::CorrectionActionStatus::ManualRecovery
                  )
                })
            }) {
              return Err(format!(
                "Cleanup {:?} is not next in compiled source order.",
                data.action_id
              ));
            }
          }
        }
      }
      RunEventPayload::SagaCorrectionCompleted(data) => {
        let saga = workflow
          .saga(&data.saga_id)
          .ok_or_else(|| format!("Unknown saga {:?}.", data.saga_id))?;
        let prefix = prefixes.at(index)?;
        let state = prefix
          .sagas
          .get(&data.saga_id)
          .ok_or_else(|| format!("Saga {:?} has no projected correction state.", data.saga_id))?;
        if saga.cleanup.iter().any(|action| {
          !state
            .actions
            .get(&action.action_id)
            .is_some_and(|projected| {
              matches!(
                projected.status,
                crate::CorrectionActionStatus::Succeeded | crate::CorrectionActionStatus::Failed
              )
            })
        }) {
          return Err(format!(
            "Saga {:?} completed before every cleanup action settled.",
            data.saga_id
          ));
        }
      }
      RunEventPayload::RunOutcomeDecided(_) => {
        let prefix = prefixes.at(index)?;
        for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
          let Some(projected) = prefix.forks.get(&fork.fork_id) else {
            continue;
          };
          if projected.join_status == crate::projection::ForkJoinStatus::Pending
            || fork
              .branches
              .iter()
              .any(|branch| !projected.branches.contains_key(&branch.branch_id))
          {
            return Err(format!(
              "Run outcome was decided before fork {:?} fully settled.",
              fork.fork_id
            ));
          }
        }
      }
      RunEventPayload::ParallelGroupStarted(data) => {
        let group = workflow
          .parallel_group(&data.parallel_id)
          .ok_or_else(|| format!("Unknown parallel group {:?}.", data.parallel_id))?;
        let prefix = prefixes.at(index)?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, prefix)?;
        if !ready.iter().any(|node_id| node_id == &group.start_node_id) {
          return Err(format!(
            "Parallel group {:?} was started before its fork was ready.",
            data.parallel_id
          ));
        }
      }
      RunEventPayload::StepAttemptStarted(data) => {
        let prefix = prefixes.at(index)?;
        let ready =
          ready_node_ids_for_projection_at(workflow, definition_hash, prefix, event.occurred_at)?;
        if !ready.iter().any(|node_id| node_id == &data.node_id) {
          return Err(format!(
            "Node {:?} started before it was ready.",
            data.node_id
          ));
        }
        if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
          let expected_key =
            step_effect_idempotency_key(&event.run_id, definition_hash, &data.node_id);
          if data.idempotency_key.as_deref() != Some(expected_key.as_str()) {
            return Err(format!(
              "Node {:?} attempt does not carry its derived stable idempotency key.",
              data.node_id
            ));
          }
        }
        let Some(group) = workflow.parallel_group_for_child(&data.node_id) else {
          continue;
        };
        if !matches!(
          prefix
            .parallel_groups
            .get(&group.parallel_id)
            .map(|state| &state.status),
          Some(ParallelGroupStatus::Started)
        ) {
          return Err(format!(
            "Parallel child {:?} started outside its active group {:?}.",
            data.node_id, group.parallel_id
          ));
        }
        let active = group
          .child_node_ids
          .iter()
          .filter(|node_id| child_states.get(*node_id) == Some(&ParallelChildState::Active))
          .count();
        if active >= group.concurrency {
          return Err(format!(
            "Parallel group {:?} exceeds its concurrency cap of {}.",
            group.parallel_id, group.concurrency
          ));
        }
        child_states.insert(data.node_id.clone(), ParallelChildState::Active);
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(data.node_id.clone(), ParallelChildState::Succeeded);
        }
      }
      RunEventPayload::StepAttemptFailed(data) => {
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(
            data.node_id.clone(),
            ParallelChildState::Failed(data.failure.kind),
          );
        }
        if let Some(policy) = workflow
          .node(&data.node_id)
          .and_then(|node| node.retry_policy.as_ref())
        {
          let should_retry = data.failure.kind == AttemptFailureKind::ScriptThrew
            && data.attempt < policy.max_attempts;
          if should_retry
            && !matches!(
              events.get(index + 1).map(|event| &event.payload),
              Some(RunEventPayload::StepRetryScheduled(schedule))
                if schedule.node_id == data.node_id
                  && schedule.failed_attempt == data.attempt
                  && schedule.next_attempt == data.attempt + 1
            )
          {
            return Err(format!(
              "Retryable failure for node {:?} must be atomically followed by step_retry_scheduled.",
              data.node_id
            ));
          }
        }
      }
      RunEventPayload::StepRetryScheduled(data) => {
        let previous = events
          .get(index.wrapping_sub(1))
          .ok_or_else(|| "step_retry_scheduled cannot be the first run event.".to_string())?;
        let RunEventPayload::StepAttemptFailed(failed) = &previous.payload else {
          return Err(
            "step_retry_scheduled must immediately follow step_attempt_failed.".to_string(),
          );
        };
        let policy = workflow
          .node(&data.node_id)
          .and_then(|node| node.retry_policy.as_ref())
          .ok_or_else(|| format!("Node {:?} has no retry policy.", data.node_id))?;
        let delay_ms = policy
          .delay_before_attempt(data.next_attempt)
          .ok_or_else(|| "Compiled retry delay is invalid.".to_string())?;
        let delay = chrono::Duration::milliseconds(
          i64::try_from(delay_ms)
            .map_err(|_| "Retry delay exceeds the clock range.".to_string())?,
        );
        if failed.node_id != data.node_id
          || failed.attempt != data.failed_attempt
          || failed.failure.kind != AttemptFailureKind::ScriptThrew
          || data.scheduled_at != previous.occurred_at + delay
        {
          return Err(format!(
            "Retry schedule for node {:?} does not match its failed attempt and compiled backoff.",
            data.node_id
          ));
        }
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(data.node_id.clone(), ParallelChildState::RetryPending);
        }
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        let group = workflow
          .parallel_group(&data.parallel_id)
          .ok_or_else(|| format!("Unknown parallel group {:?}.", data.parallel_id))?;
        let failed = group
          .child_node_ids
          .iter()
          .filter(|node_id| {
            matches!(
              child_states.get(*node_id),
              Some(ParallelChildState::Failed(kind))
                if *kind != AttemptFailureKind::InvocationCancelled
            )
          })
          .cloned()
          .collect::<Vec<_>>();
        let cancelled = group
          .child_node_ids
          .iter()
          .filter(|node_id| {
            matches!(
              child_states.get(*node_id),
              Some(ParallelChildState::Failed(
                AttemptFailureKind::InvocationCancelled
              ))
            )
          })
          .cloned()
          .collect::<Vec<_>>();
        let active = group
          .child_node_ids
          .iter()
          .any(|node_id| child_states.get(node_id) == Some(&ParallelChildState::Active));
        let every_succeeded = group
          .child_node_ids
          .iter()
          .all(|node_id| child_states.get(node_id) == Some(&ParallelChildState::Succeeded));
        let every_terminal = group.child_node_ids.iter().all(|node_id| {
          matches!(
            child_states.get(node_id),
            Some(ParallelChildState::Succeeded | ParallelChildState::Failed(_))
          )
        });
        let valid_outcome = match data.outcome {
          ParallelGroupOutcome::Succeeded => every_succeeded,
          ParallelGroupOutcome::Failed if group.on_error == "wait-all" => every_terminal,
          ParallelGroupOutcome::Failed => !active,
        };
        if !valid_outcome || data.failed_node_ids != failed || data.cancelled_node_ids != cancelled
        {
          return Err(format!(
            "Parallel completion for {:?} does not match its durable child outcomes.",
            data.parallel_id
          ));
        }
      }
      RunEventPayload::ApprovalRequested(data) => {
        let approval = workflow
          .approval(&data.approval_id)
          .ok_or_else(|| format!("Unknown approval {:?}.", data.approval_id))?;
        let prefix = prefixes.at(index)?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, prefix)?;
        if !ready.iter().any(|node_id| node_id == &data.approval_id) {
          return Err(format!(
            "Approval {:?} was requested before its wait node was ready.",
            data.approval_id
          ));
        }
        let expected_expires_at = approval
          .timeout_ms
          .map(|milliseconds| {
            i64::try_from(milliseconds)
              .map(chrono::Duration::milliseconds)
              .map(|duration| event.occurred_at + duration)
          })
          .transpose()
          .map_err(|_| {
            format!(
              "Approval {:?} timeout exceeds the clock range.",
              data.approval_id
            )
          })?;
        if data.expires_at != expected_expires_at {
          return Err(format!(
            "Approval {:?} request deadline does not match its compiled timeout.",
            data.approval_id
          ));
        }
      }
      RunEventPayload::ApprovalReviewRequested(data) => {
        let prefix = prefixes.at(index)?;
        let ready = ready_node_ids_for_projection(workflow, definition_hash, prefix)?;
        if !ready.iter().any(|node_id| node_id == &data.approval_id) {
          return Err(format!(
            "Rich approval {:?} was requested before its wait node was ready.",
            data.approval_id
          ));
        }
      }
      RunEventPayload::ApprovalResolved(_) => {}
      RunEventPayload::WaitRegistered(data) => {
        let prefix = prefixes.at(index)?;
        let ready =
          ready_node_ids_for_projection_at(workflow, definition_hash, prefix, event.occurred_at)?;
        if !ready.iter().any(|node_id| node_id == &data.node_id) {
          return Err(format!(
            "Durable wait {:?} was registered before its node was ready.",
            data.wait_id
          ));
        }
        if workflow.parallel_group_for_child(&data.node_id).is_some() {
          child_states.insert(data.node_id.clone(), ParallelChildState::Active);
        }
      }
      RunEventPayload::WaitSatisfied(data) => {
        if let Some(node_id) = workflow
          .durable_wait(&data.wait_id)
          .map(|wait| wait.node_id())
          .filter(|node_id| workflow.parallel_group_for_child(node_id).is_some())
        {
          child_states.insert(node_id.to_string(), ParallelChildState::Succeeded);
        }
      }
      RunEventPayload::WaitTimedOut(data) => {
        if let Some(node_id) = workflow
          .durable_wait(&data.wait_id)
          .map(|wait| wait.node_id())
          .filter(|node_id| workflow.parallel_group_for_child(node_id).is_some())
        {
          child_states.insert(
            node_id.to_string(),
            if data.behavior == DurableWaitTimeoutBehavior::Continue {
              ParallelChildState::Succeeded
            } else {
              ParallelChildState::Failed(AttemptFailureKind::ServiceFailed)
            },
          );
        }
      }
      RunEventPayload::WaitCancelled(data) => {
        if let Some(node_id) = workflow
          .durable_wait(&data.wait_id)
          .map(|wait| wait.node_id())
          .filter(|node_id| workflow.parallel_group_for_child(node_id).is_some())
        {
          child_states.insert(
            node_id.to_string(),
            ParallelChildState::Failed(AttemptFailureKind::InvocationCancelled),
          );
        }
      }
      RunEventPayload::ExternalSignalReceived(_) => {}
      RunEventPayload::RunSucceeded(_) => {
        let projection = prefixes.at(index + 1)?;
        if projection.parallel_groups.values().any(|group| {
          !matches!(
            group.status,
            ParallelGroupStatus::Completed {
              outcome: ParallelGroupOutcome::Succeeded,
              ..
            }
          )
        }) {
          return Err(
            "A run cannot succeed while a started parallel group is incomplete or failed."
              .to_string(),
          );
        }
      }
      RunEventPayload::LifecycleHookRequested(data) => {
        let expected = crate::derive_lifecycle_hook_invocation_id(
          &event.run_id,
          &data.hook_id,
          data.subject.kind,
          &data.subject.id,
        );
        if data.hook_invocation_id != expected
          || (matches!(data.subject.kind, crate::LifecycleSubjectKind::Workflow)
            && data.subject.id != event.run_id)
        {
          return Err(
            "Lifecycle hook request has a non-canonical invocation identity.".to_string(),
          );
        }
        let eligible = events
          .get(index.wrapping_sub(1))
          .is_some_and(|previous| match data.event {
            crate::LifecycleEventName::RunStart => {
              matches!(
                previous.payload,
                RunEventPayload::RunStarted(_) | RunEventPayload::RunExecutionStarted(_)
              )
            }
            crate::LifecycleEventName::RunCancel => matches!(
              previous.payload,
              RunEventPayload::RunCancellationRequested(_)
                | RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Cancelled { .. })
            ),
            crate::LifecycleEventName::RunSuccess => matches!(
              previous.payload,
              RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Succeeded { .. })
            ),
            crate::LifecycleEventName::RunFailure => matches!(
              previous.payload,
              RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Failed { .. })
            ),
            crate::LifecycleEventName::RunComplete => matches!(
              previous.payload,
              RunEventPayload::LifecycleHookCompleted(_) | RunEventPayload::RunOutcomeDecided(_)
            ),
            crate::LifecycleEventName::StepStart => matches!(
              previous.payload,
              RunEventPayload::StepAttemptStarted(ref step) if step.node_id == data.subject.id
            ),
            crate::LifecycleEventName::StepSuccess => matches!(
              previous.payload,
              RunEventPayload::StepAttemptSucceeded(ref step) if step.node_id == data.subject.id
            ),
            crate::LifecycleEventName::StepFailure => matches!(
              previous.payload,
              RunEventPayload::StepAttemptFailed(ref step) if step.node_id == data.subject.id
            ),
            crate::LifecycleEventName::StepComplete => matches!(
              previous.payload,
              RunEventPayload::LifecycleHookCompleted(_)
                | RunEventPayload::StepAttemptSucceeded(_)
                | RunEventPayload::StepAttemptFailed(_)
            ),
          });
        if !eligible {
          return Err(
            "Lifecycle hook request is not adjacent to its eligibility event.".to_string(),
          );
        }
      }
      RunEventPayload::LifecycleActionAttemptStarted(data) => {
        let prefix = prefixes.at(index)?;
        let hook_projection = prefix
          .lifecycle_hooks
          .get(&data.hook_invocation_id)
          .ok_or_else(|| "Lifecycle action has no requested hook.".to_string())?;
        let hook = workflow
          .lifecycle_hook(&hook_projection.hook_id)
          .ok_or_else(|| "Lifecycle action hook is absent from Model v11.".to_string())?;
        let expected = hook
          .actions
          .get(hook_projection.actions.len())
          .ok_or_else(|| {
            "Lifecycle hook has no remaining action at this source-order position.".to_string()
          })?;
        if expected.action_id != data.action_id {
          return Err("Lifecycle actions must start in compiled source order.".to_string());
        }
      }
      RunEventPayload::LifecycleHookCompleted(data) => {
        let prefix = prefixes.at(index)?;
        let hook_projection = prefix
          .lifecycle_hooks
          .get(&data.hook_invocation_id)
          .ok_or_else(|| "Lifecycle completion has no requested hook.".to_string())?;
        let hook = workflow
          .lifecycle_hook(&hook_projection.hook_id)
          .ok_or_else(|| "Lifecycle completion hook is absent from Model v11.".to_string())?;
        if hook_projection.actions.len() != hook.actions.len() {
          return Err(
            "Lifecycle hook cannot complete before every compiled action has an outcome."
              .to_string(),
          );
        }
      }
      RunEventPayload::RunAdmitted(_)
      | RunEventPayload::RunExecutionStarted(_)
      | RunEventPayload::RunTimeoutReached(_)
      | RunEventPayload::RunStarted(_)
      | RunEventPayload::BranchSelected(_)
      | RunEventPayload::NotificationDeliveryRequested(_)
      | RunEventPayload::NotificationDeliveryAttemptStarted(_)
      | RunEventPayload::NotificationDeliverySucceeded(_)
      | RunEventPayload::NotificationDeliveryFailed(_)
      | RunEventPayload::NotificationDecisionAccepted(_)
      | RunEventPayload::NotificationMessageUpdateRequested(_)
      | RunEventPayload::NotificationMessageUpdateAttemptStarted(_)
      | RunEventPayload::NotificationMessageUpdated(_)
      | RunEventPayload::NotificationMessageUpdateFailed(_)
      | RunEventPayload::OperationStarted(_)
      | RunEventPayload::OperationSucceeded(_)
      | RunEventPayload::OperationFailed(_)
      | RunEventPayload::RunCancellationRequested(_)
      | RunEventPayload::LifecycleActionSucceeded(_)
      | RunEventPayload::LifecycleActionFailed(_)
      | RunEventPayload::ReusableLifecycleRequested(_)
      | RunEventPayload::ReusableLifecycleActionStarted(_)
      | RunEventPayload::ReusableLifecycleActionSucceeded(_)
      | RunEventPayload::ReusableLifecycleActionFailed(_)
      | RunEventPayload::ReusableLifecycleCompleted(_)
      | RunEventPayload::ForEachOpened(_)
      | RunEventPayload::ForEachIterationStarted(_)
      | RunEventPayload::ForEachIterationSucceeded(_)
      | RunEventPayload::ForEachIterationFailed(_)
      | RunEventPayload::ForEachIterationSkipped(_)
      | RunEventPayload::ForEachSucceeded(_)
      | RunEventPayload::ForEachFailed(_)
      | RunEventPayload::ForEachCancelled(_)
      | RunEventPayload::LockWaitRegistered(_)
      | RunEventPayload::LockAcquired(_)
      | RunEventPayload::LockReleased(_)
      | RunEventPayload::LockAcquireTimedOut(_)
      | RunEventPayload::LockAcquisitionCancelled(_)
      | RunEventPayload::LockLeaseLost(_)
      | RunEventPayload::ApprovalReviewerDecisionRecorded(_)
      | RunEventPayload::ApprovalQuorumProgressed(_)
      | RunEventPayload::ApprovalInvalidated(_)
      | RunEventPayload::CompensationRegistered(_)
      | RunEventPayload::SagaForwardSucceeded(_)
      | RunEventPayload::SagaCorrectionStarted(_)
      | RunEventPayload::CorrectionAttemptSucceeded(_)
      | RunEventPayload::CorrectionAttemptFailed(_)
      | RunEventPayload::CorrectionRetryScheduled(_)
      | RunEventPayload::CorrectionInterrupted(_)
      | RunEventPayload::ManualRecoveryRequired(_)
      | RunEventPayload::InfrastructurePolicyDecisionRecorded(_)
      | RunEventPayload::RemoteDispatchTransitioned(_)
      | RunEventPayload::RunFinalized(_)
      | RunEventPayload::RunFailed(_) => {}
    }
  }
  prefixes.succeed();
  Ok(())
}

pub(super) fn validate_for_each_id(
  workflow: &CompiledWorkflowDefinition,
  for_each_id: &str,
) -> Result<(), String> {
  if workflow
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .any(|descriptor| descriptor.for_each_id == for_each_id)
  {
    Ok(())
  } else {
    Err(format!(
      "For-each event references unknown loop {for_each_id:?}."
    ))
  }
}

pub(super) fn validate_reusable_lifecycle_binding(
  workflow: &CompiledWorkflowDefinition,
  invocation_id: &str,
  definition_digest: &str,
  action_id: Option<&str>,
) -> Result<(), String> {
  if !matches!(
    workflow.schema_version,
    crate::COMPILED_MODEL_SCHEMA_VERSION_V14
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
  ) {
    return Err("Reusable lifecycle events require compiled Model v14 or later.".to_string());
  }
  let descriptor = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .find(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step {
        invocation_id: candidate,
        definition_digest: digest,
        ..
      } => candidate == invocation_id && digest == definition_digest,
      crate::model::CompiledReusableInvocation::NotificationProvider {
        provider_id: candidate,
        definition_digest: digest,
        ..
      } => candidate == invocation_id && digest == definition_digest,
    })
    .ok_or_else(|| {
      "Reusable lifecycle event does not match an immutable Model v14 invocation.".to_string()
    })?;
  if let Some(action_id) = action_id {
    let lifecycle = match descriptor {
      crate::model::CompiledReusableInvocation::Step { lifecycle, .. }
      | crate::model::CompiledReusableInvocation::NotificationProvider { lifecycle, .. } => {
        lifecycle.as_ref()
      }
    };
    let found = lifecycle.is_some_and(|lifecycle| {
      lifecycle
        .on_success
        .iter()
        .chain(&lifecycle.on_error)
        .chain(&lifecycle.on_complete)
        .flat_map(|actions| actions.iter())
        .any(|candidate| candidate == action_id)
    });
    if !found {
      return Err("Reusable lifecycle event references an unknown action.".to_string());
    }
  }
  Ok(())
}

pub(super) fn validate_operation_node(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
) -> Result<(), String> {
  if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V8 {
    return Err("Operation events require compiled workflow Model v8.".to_string());
  }
  if workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
    lifecycle
      .hooks
      .iter()
      .flat_map(|hook| &hook.actions)
      .any(|action| {
        action.action_id == node_id
          && matches!(
            action.handler.as_str(),
            "runtime.lifecycle-script" | "notification.informational"
          )
      })
  }) {
    return Ok(());
  }
  if workflow
    .graph
    .sagas
    .as_deref()
    .unwrap_or_default()
    .iter()
    .any(|saga| {
      [
        crate::CorrectiveActionKind::Compensation,
        crate::CorrectiveActionKind::Cleanup,
      ]
      .into_iter()
      .any(|kind| {
        super::validation::validate_correction_action(workflow, &saga.saga_id, node_id, kind)
          .is_ok()
      })
    })
  {
    return Ok(());
  }
  let node = workflow
    .node(node_id)
    .ok_or_else(|| format!("Operation references unknown node or lifecycle action {node_id:?}."))?;
  if node.handler != "runtime.script" || node.script_runtime.is_none() {
    return Err(format!(
      "Operation node {node_id:?} is not a Model v8 runtime.script node."
    ));
  }
  Ok(())
}
