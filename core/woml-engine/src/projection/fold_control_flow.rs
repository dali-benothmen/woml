//! projection fold control flow.

use super::{
  projection_digest, require_running, require_running_or_cancelling, AttemptStatus, FoldError,
  ForEachIterationExecutionProjection, ForEachIterationStatus, ForEachProjection, ForEachStatus,
  ForkBranchProjection, ForkJoinStatus, ForkProjection, LifecycleHookStatus, OperationStatus,
  ParallelGroupProjection, ParallelGroupStatus, ReusableLifecycleStatus, RunProjection,
};
use crate::event::ForkBranchOutcome;
use crate::event::ForkJoinOutcome;
use crate::event::RunEvent;
use crate::event::RunEventPayload;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn apply_branch_selected(
  projection: &mut RunProjection,
  data: &crate::event::BranchSelectedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if let Some(selected_arm) = projection.branch_selections.get(&data.branch_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Branch {:?} was already selected as arm {:?}.",
      data.branch_id, selected_arm
    )));
  }
  projection
    .branch_selections
    .insert(data.branch_id.clone(), data.arm_id.clone());
  Ok(())
}

pub(super) fn apply_choice_selected(
  projection: &mut RunProjection,
  data: &crate::event::ChoiceSelectedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.choice_selections.contains_key(&data.choice_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Choice {:?} was selected more than once.",
      data.choice_id
    )));
  }
  projection
    .choice_selections
    .insert(data.choice_id.clone(), data.arm_id.clone());
  Ok(())
}

pub(super) fn apply_fork_opened(
  projection: &mut RunProjection,
  data: &crate::event::ForkOpenedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.forks.contains_key(&data.fork_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Fork {:?} was opened more than once.",
      data.fork_id
    )));
  }
  projection.forks.insert(
    data.fork_id.clone(),
    ForkProjection {
      fork_id: data.fork_id.clone(),
      branches: BTreeMap::new(),
      join_status: ForkJoinStatus::Pending,
      blocking_branch_id: None,
    },
  );
  Ok(())
}

pub(super) fn apply_fork_branch_settled(
  projection: &mut RunProjection,
  data: &crate::event::ForkBranchSettledData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let fork = projection.forks.get_mut(&data.fork_id).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "Fork branch {:?}.{:?} settled before its fork opened.",
      data.fork_id, data.branch_id
    ))
  })?;
  if fork.branches.contains_key(&data.branch_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Fork branch {:?}.{:?} settled more than once.",
      data.fork_id, data.branch_id
    )));
  }
  fork.branches.insert(
    data.branch_id.clone(),
    ForkBranchProjection {
      branch_id: data.branch_id.clone(),
      terminal_node_id: Some(data.terminal_node_id.clone()),
      outcome: Some(data.outcome),
    },
  );
  Ok(())
}

pub(super) fn apply_fork_join_settled(
  projection: &mut RunProjection,
  data: &crate::event::ForkJoinSettledData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let fork = projection.forks.get_mut(&data.fork_id).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "Fork {:?} join settled before its fork opened.",
      data.fork_id
    ))
  })?;
  if fork.join_status != ForkJoinStatus::Pending {
    return Err(FoldError::InvalidHistory(format!(
      "Fork {:?} join settled more than once.",
      data.fork_id
    )));
  }
  if let Some(blocking_branch_id) = data.blocking_branch_id.as_deref() {
    let matching_outcome = fork
      .branches
      .get(blocking_branch_id)
      .and_then(|branch| branch.outcome)
      .is_some_and(|outcome| match data.outcome {
        ForkJoinOutcome::Failed => outcome == ForkBranchOutcome::Failed,
        ForkJoinOutcome::Cancelled => outcome == ForkBranchOutcome::Cancelled,
        ForkJoinOutcome::Succeeded => false,
      });
    if !matching_outcome {
      return Err(FoldError::InvalidHistory(format!(
        "Fork {:?} join outcome is not proven by blocking branch {:?}.",
        data.fork_id, blocking_branch_id
      )));
    }
  }
  fork.join_status = match data.outcome {
    ForkJoinOutcome::Succeeded => ForkJoinStatus::Succeeded,
    ForkJoinOutcome::Failed => ForkJoinStatus::Failed,
    ForkJoinOutcome::Cancelled => ForkJoinStatus::Cancelled,
  };
  fork.blocking_branch_id = data.blocking_branch_id.clone();
  Ok(())
}

pub(super) fn apply_for_each_opened(
  projection: &mut RunProjection,
  data: &crate::event::ForEachOpenedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.for_each.contains_key(&data.for_each_id) {
    return Err(FoldError::InvalidHistory(format!(
      "For-each {:?} was opened more than once.",
      data.for_each_id
    )));
  }
  projection.for_each.insert(
    data.for_each_id.clone(),
    ForEachProjection {
      for_each_id: data.for_each_id.clone(),
      total: data.total,
      items_digest: data.items_digest.clone(),
      concurrency: data.concurrency,
      status: ForEachStatus::Open,
      iterations: BTreeMap::new(),
      executions: BTreeMap::new(),
    },
  );
  Ok(())
}

pub(super) fn apply_for_each_iteration_started(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ForEachIterationIdentityData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  let scope = event.iteration.as_ref().expect("validated iteration scope");
  let loop_state = projection
    .for_each
    .get_mut(&data.for_each_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(format!(
        "For-each iteration {:?}:{} started before its loop opened.",
        data.for_each_id, scope.index
      ))
    })?;
  if loop_state.status != ForEachStatus::Open
    || scope.index >= loop_state.total
    || loop_state.iterations.contains_key(&scope.index)
  {
    return Err(FoldError::InvalidHistory(format!(
      "For-each iteration {:?}:{} cannot start in its current state.",
      data.for_each_id, scope.index
    )));
  }
  loop_state
    .iterations
    .insert(scope.index, ForEachIterationStatus::Started);
  loop_state.executions.insert(
    scope.index,
    ForEachIterationExecutionProjection {
      context: projection.context.clone(),
      ..ForEachIterationExecutionProjection::default()
    },
  );
  Ok(())
}

pub(super) fn apply_for_each_iteration_succeeded(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ForEachIterationSucceededData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  let scope = event.iteration.as_ref().expect("validated iteration scope");
  if let (Some(result), Some(expected_digest)) = (&data.result, &data.result_digest) {
    if projection_digest(result)? != *expected_digest {
      return Err(FoldError::InvalidHistory(format!(
        "For-each iteration {:?}:{} result digest does not match its value.",
        data.for_each_id, scope.index
      )));
    }
  }
  let loop_state = projection
    .for_each
    .get_mut(&data.for_each_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("For-each iteration succeeded before opening.".to_string())
    })?;
  match loop_state.iterations.get_mut(&scope.index) {
    Some(status @ ForEachIterationStatus::Started) => {
      *status = ForEachIterationStatus::Succeeded {
        result: data.result.clone(),
      };
    }
    _ => {
      return Err(FoldError::InvalidHistory(format!(
        "For-each iteration {:?}:{} succeeded without one active start.",
        data.for_each_id, scope.index
      )));
    }
  }
  Ok(())
}

pub(super) fn apply_for_each_iteration_failed(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ForEachIterationFailedData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let scope = event.iteration.as_ref().expect("validated iteration scope");
  let loop_state = projection
    .for_each
    .get_mut(&data.for_each_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("For-each iteration failed before opening.".to_string())
    })?;
  match loop_state.iterations.get_mut(&scope.index) {
    Some(status @ ForEachIterationStatus::Started) => {
      *status = ForEachIterationStatus::Failed {
        failed_node_id: data.failed_node_id.clone(),
      };
      if let Some(execution) = loop_state.executions.get_mut(&scope.index) {
        execution.pending_retries.clear();
      }
    }
    _ => {
      return Err(FoldError::InvalidHistory(format!(
        "For-each iteration {:?}:{} failed without one active start.",
        data.for_each_id, scope.index
      )));
    }
  }
  Ok(())
}

pub(super) fn apply_for_each_iteration_skipped(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ForEachIterationSkippedData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let scope = event.iteration.as_ref().expect("validated iteration scope");
  let loop_state = projection
    .for_each
    .get_mut(&data.for_each_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("For-each iteration skipped before opening.".to_string())
    })?;
  if loop_state.status != ForEachStatus::Open
    || scope.index >= loop_state.total
    || loop_state.iterations.contains_key(&scope.index)
  {
    return Err(FoldError::InvalidHistory(format!(
      "For-each iteration {:?}:{} cannot be skipped in its current state.",
      data.for_each_id, scope.index
    )));
  }
  loop_state
    .iterations
    .insert(scope.index, ForEachIterationStatus::Skipped);
  Ok(())
}

pub(super) fn apply_for_each_settled(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ForEachSettledData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let is_success = matches!(&event.payload, RunEventPayload::ForEachSucceeded(_));
  let is_cancelled = matches!(&event.payload, RunEventPayload::ForEachCancelled(_));
  let loop_state = projection
    .for_each
    .get_mut(&data.for_each_id)
    .ok_or_else(|| FoldError::InvalidHistory("For-each settled before opening.".to_string()))?;
  let succeeded = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
    .count() as u32;
  let failed = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    .count() as u32;
  let skipped = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
    .count() as u32;
  let pending_retry_count = loop_state
    .executions
    .values()
    .map(|execution| execution.pending_retries.len())
    .sum::<usize>();
  let active_attempt_count = loop_state
    .executions
    .values()
    .flat_map(|execution| &execution.attempts)
    .filter(|attempt| attempt.status == AttemptStatus::Started)
    .count();
  let active_attempts = loop_state
    .executions
    .iter()
    .flat_map(|(index, execution)| {
      execution
        .attempts
        .iter()
        .filter(|attempt| attempt.status == AttemptStatus::Started)
        .map(move |attempt| {
          format!(
            "{}:{}:{}:{}",
            index,
            attempt.identity.node_id,
            attempt.identity.attempt,
            attempt.identity.invocation_id
          )
        })
    })
    .collect::<Vec<_>>();
  let active_operation_count = loop_state
    .executions
    .values()
    .flat_map(|execution| execution.operations.values())
    .filter(|operation| operation.status == OperationStatus::Started)
    .count();
  let active_lifecycle_count = loop_state
    .executions
    .values()
    .flat_map(|execution| execution.lifecycle_hooks.values())
    .filter(|hook| {
      matches!(
        hook.status,
        LifecycleHookStatus::Requested | LifecycleHookStatus::Running
      )
    })
    .count();
  let active_reusable_count = loop_state
    .executions
    .values()
    .flat_map(|execution| execution.reusable_lifecycle_hooks.values())
    .filter(|hook| {
      matches!(
        hook.status,
        ReusableLifecycleStatus::Requested | ReusableLifecycleStatus::Running
      )
    })
    .count();
  let active_parallel_count = loop_state
    .executions
    .values()
    .flat_map(|execution| execution.parallel_groups.values())
    .filter(|group| group.status == ParallelGroupStatus::Started)
    .count();
  let owns_active_work = pending_retry_count != 0
    || active_attempt_count != 0
    || active_operation_count != 0
    || active_lifecycle_count != 0
    || active_reusable_count != 0
    || active_parallel_count != 0;
  if loop_state.status != ForEachStatus::Open
    || data.total != loop_state.total
    || (data.succeeded, data.failed, data.skipped) != (succeeded, failed, skipped)
    || succeeded + failed + skipped != loop_state.total
    || (is_success && (failed != 0 || skipped != 0))
    || owns_active_work
  {
    return Err(FoldError::InvalidHistory(format!(
      "For-each {:?} settlement does not match its folded iterations: event=({},{},{}), folded=({succeeded},{failed},{skipped}), total={}, activeWork={owns_active_work} (retries={pending_retry_count}, attempts={active_attempt_count} {active_attempts:?}, operations={active_operation_count}, lifecycle={active_lifecycle_count}, reusable={active_reusable_count}, parallel={active_parallel_count}).",
      data.for_each_id, data.succeeded, data.failed, data.skipped, loop_state.total
    )));
  }
  loop_state.status = if is_success {
    ForEachStatus::Succeeded
  } else if is_cancelled {
    ForEachStatus::Cancelled
  } else {
    ForEachStatus::Failed
  };
  if is_success {
    let ordered_results = (0..loop_state.total)
      .filter_map(|index| match loop_state.iterations.get(&index) {
        Some(ForEachIterationStatus::Succeeded {
          result: Some(result),
        }) => Some(result.clone()),
        _ => None,
      })
      .collect::<Vec<_>>();
    let mut output = Map::new();
    output.insert("total".to_string(), Value::from(loop_state.total));
    output.insert("succeeded".to_string(), Value::from(succeeded));
    if !ordered_results.is_empty() {
      output.insert("results".to_string(), Value::Array(ordered_results));
    }
    let output = Value::Object(output);
    if projection_digest(&output)? != data.aggregate_digest {
      return Err(FoldError::InvalidHistory(format!(
        "For-each {:?} aggregate digest does not match its public output.",
        data.for_each_id
      )));
    }
    projection
      .context
      .steps
      .insert(data.for_each_id.clone(), output);
  }
  Ok(())
}

pub(super) fn apply_parallel_group_started(
  projection: &mut RunProjection,
  data: &crate::event::ParallelGroupStartedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.parallel_groups.contains_key(&data.parallel_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Parallel group {:?} was started more than once.",
      data.parallel_id
    )));
  }
  projection.parallel_groups.insert(
    data.parallel_id.clone(),
    ParallelGroupProjection {
      parallel_id: data.parallel_id.clone(),
      fork_context: projection.context.clone(),
      status: ParallelGroupStatus::Started,
    },
  );
  Ok(())
}

pub(super) fn apply_parallel_group_completed(
  projection: &mut RunProjection,
  data: &crate::event::ParallelGroupCompletedData,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let group = projection
    .parallel_groups
    .get_mut(&data.parallel_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(format!(
        "Parallel group {:?} completed before it started.",
        data.parallel_id
      ))
    })?;
  if !matches!(group.status, ParallelGroupStatus::Started) {
    return Err(FoldError::InvalidHistory(format!(
      "Parallel group {:?} was completed more than once.",
      data.parallel_id
    )));
  }
  group.status = ParallelGroupStatus::Completed {
    outcome: data.outcome,
    failed_node_ids: data.failed_node_ids.clone(),
    cancelled_node_ids: data.cancelled_node_ids.clone(),
  };
  Ok(())
}
