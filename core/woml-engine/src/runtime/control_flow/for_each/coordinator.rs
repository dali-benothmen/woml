//! Bounded iteration admission, body progress, and ordered loop settlement.
//!
//! Preserve active-sibling settlement and pending-item treatment before closing
//! a failed/cancelled loop. Completion timing does not choose aggregate order.

use crate::engine::resolve_context_reference;
use crate::event::BranchSelectedData;
use crate::event::ChoiceSelectedData;
use crate::event::ForEachIterationFailedData;
use crate::event::ForEachIterationFailureKind;
use crate::event::ForEachIterationIdentityData;
use crate::event::ForEachIterationScope;
use crate::event::ForEachIterationSkippedData;
use crate::event::ForEachIterationSucceededData;
use crate::event::ForEachOpenedData;
use crate::event::ForEachSettledData;
use crate::event::ForEachSkipReason;
use crate::event::OperationFailedData;
use crate::event::ParallelGroupCompletedData;
use crate::event::ParallelGroupOutcome;
use crate::event::ParallelGroupStartedData;
use crate::event::StepAttemptFailedData;
use crate::model::ValueExpression;
use crate::projection::AttemptStatus;
use crate::projection::ForEachIterationStatus;
use crate::projection::ForEachStatus;
use crate::projection::ParallelGroupStatus;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::progress::report_for_each_progress;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  cancelled_run_error, execute_for_each_iteration_script, for_each_digest, for_each_ready_nodes,
  for_each_selected_branch, for_each_selected_choice, lifecycle_failure_from_attempt,
  resolve_for_each_lock_key, resolve_for_each_reference, resumed_failure,
  settle_workflow_timeout_if_due, RuntimeDagEngine, SharedForEachEngine,
};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::OperationStatus;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use crate::ScriptHostClient;
use crate::WorkflowContext;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum ForEachTerminalDisposition {
  Failed,
  Cancelled,
}

fn for_each_latest_iteration_failure(
  projection: &RunProjection,
  for_each_id: &str,
  index: u32,
) -> Option<(String, AttemptFailure)> {
  let execution = projection
    .for_each
    .get(for_each_id)?
    .executions
    .get(&index)?;
  execution
    .attempts
    .iter()
    .rev()
    .filter_map(|attempt| match &attempt.status {
      AttemptStatus::Failed { failure } => {
        Some((attempt.identity.node_id.clone(), failure.clone()))
      }
      _ => None,
    })
    .min_by_key(|(_, failure)| matches!(failure.kind, AttemptFailureKind::InvocationCancelled))
}

fn for_each_settlement_digest(
  total: u32,
  succeeded: u32,
  failed: u32,
  skipped: u32,
) -> Result<String, RuntimeExecutionError> {
  for_each_digest(&serde_json::json!({
    "total": total,
    "succeeded": succeeded,
    "failed": failed,
    "skipped": skipped,
  }))
}

pub(in crate::runtime) fn settle_open_for_each<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  disposition: ForEachTerminalDisposition,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let Some(loop_state) = projection.for_each.get(&descriptor.for_each_id) else {
    return Ok(());
  };
  if loop_state.status != ForEachStatus::Open {
    return Ok(());
  }

  let active_indexes = loop_state
    .iterations
    .iter()
    .filter_map(|(index, status)| {
      matches!(status, ForEachIterationStatus::Started).then_some(*index)
    })
    .collect::<Vec<_>>();
  for index in active_indexes {
    let current = engine.projection(run_id)?;
    let execution = current
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|state| state.executions.get(&index))
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "for-each {:?} active iteration {index} has no execution projection",
          descriptor.for_each_id
        ))
      })?;
    let scope = ForEachIterationScope {
      for_each_id: descriptor.for_each_id.clone(),
      index,
    };
    for operation in execution
      .operations
      .values()
      .filter(|operation| operation.status == OperationStatus::Started)
    {
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id.clone(),
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id.clone(),
          call_id: operation.identity.call_id.clone(),
          operation_key: operation.operation_key.clone(),
          capability: operation.capability.clone(),
          operation: operation.operation.clone(),
          execution_mode: operation.execution_mode,
          metadata: operation.metadata.clone(),
          duration_ms: 0.0,
          failure: CapabilityFailure {
            kind: CapabilityFailureKind::Ambiguous,
            code: "WOML_CAPABILITY_CANCELLATION_AMBIGUOUS".to_string(),
            message: "Loop settlement found an operation without a durable terminal event; its external outcome is ambiguous and it will not be replayed.".to_string(),
            retryable: false,
            ambiguous: true,
            details: None,
          },
        }),
      )?;
    }
    for attempt in execution
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
    {
      let kind = match disposition {
        ForEachTerminalDisposition::Failed => AttemptFailureKind::Interrupted,
        ForEachTerminalDisposition::Cancelled => AttemptFailureKind::InvocationCancelled,
      };
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.identity.node_id.clone(),
          attempt: attempt.identity.attempt,
          invocation_id: attempt.identity.invocation_id.clone(),
          failure: AttemptFailure {
            kind,
            code: kind.code().to_string(),
            message: "The loop settled before this invocation produced a durable terminal outcome."
              .to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        }),
      )?;
    }
    for group in execution
      .parallel_groups
      .values()
      .filter(|group| group.status == ParallelGroupStatus::Started)
    {
      let start_id = format!("__woml_parallel__{}__start", group.parallel_id);
      let cancelled_node_ids = descriptor
        .body
        .edges
        .iter()
        .filter(|edge| {
          edge.from == start_id && edge.parallel_id.as_deref() == Some(group.parallel_id.as_str())
        })
        .map(|edge| edge.to.clone())
        .collect::<Vec<_>>();
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: Vec::new(),
          cancelled_node_ids,
        }),
      )?;
    }
    let current = engine.projection(run_id)?;
    let (failed_node_id, failure_kind) =
      for_each_latest_iteration_failure(&current, &descriptor.for_each_id, index)
        .map(|(node_id, failure)| (node_id, for_each_failure_kind(failure.kind)))
        .unwrap_or_else(|| {
          (
            descriptor.body.terminal_node_id.clone(),
            match disposition {
              ForEachTerminalDisposition::Failed => ForEachIterationFailureKind::Interrupted,
              ForEachTerminalDisposition::Cancelled => ForEachIterationFailureKind::Cancelled,
            },
          )
        });
    engine.append_scoped_payload(
      run_id,
      scope,
      RunEventPayload::ForEachIterationFailed(ForEachIterationFailedData {
        for_each_id: descriptor.for_each_id.clone(),
        failed_node_id,
        failure_kind,
      }),
    )?;
  }

  let projection = engine.projection(run_id)?;
  let loop_state = projection
    .for_each
    .get(&descriptor.for_each_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each disappeared".to_string()))?;
  let skip_reason = match disposition {
    ForEachTerminalDisposition::Failed => ForEachSkipReason::Failure,
    ForEachTerminalDisposition::Cancelled => ForEachSkipReason::Cancellation,
  };
  let pending_indexes = (0..loop_state.total)
    .filter(|index| !loop_state.iterations.contains_key(index))
    .collect::<Vec<_>>();
  for index in pending_indexes {
    engine.append_scoped_payload(
      run_id,
      ForEachIterationScope {
        for_each_id: descriptor.for_each_id.clone(),
        index,
      },
      RunEventPayload::ForEachIterationSkipped(ForEachIterationSkippedData {
        for_each_id: descriptor.for_each_id.clone(),
        reason: skip_reason,
      }),
    )?;
  }

  let projection = engine.projection(run_id)?;
  let loop_state = projection
    .for_each
    .get(&descriptor.for_each_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each disappeared".to_string()))?;
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
  let settled = ForEachSettledData {
    for_each_id: descriptor.for_each_id.clone(),
    total: loop_state.total,
    succeeded,
    failed,
    skipped,
    aggregate_digest: for_each_settlement_digest(loop_state.total, succeeded, failed, skipped)?,
  };
  let payload = match disposition {
    ForEachTerminalDisposition::Failed => RunEventPayload::ForEachFailed(settled),
    ForEachTerminalDisposition::Cancelled => RunEventPayload::ForEachCancelled(settled),
  };
  engine.append_payload(run_id, payload)
}

fn for_each_failure_kind(kind: AttemptFailureKind) -> ForEachIterationFailureKind {
  match kind {
    AttemptFailureKind::ScriptThrew => ForEachIterationFailureKind::ScriptThrew,
    AttemptFailureKind::ScriptTimedOut => ForEachIterationFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => ForEachIterationFailureKind::NonJson,
    AttemptFailureKind::ContextTooLarge => ForEachIterationFailureKind::ContextTooLarge,
    AttemptFailureKind::ResultTooLarge => ForEachIterationFailureKind::ResultTooLarge,
    AttemptFailureKind::WorkerCrashed => ForEachIterationFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => ForEachIterationFailureKind::HostCrashed,
    AttemptFailureKind::InvocationCancelled => ForEachIterationFailureKind::Cancelled,
    AttemptFailureKind::Interrupted => ForEachIterationFailureKind::Interrupted,
    AttemptFailureKind::ServiceFailed => ForEachIterationFailureKind::CapabilityFailed,
  }
}

async fn cancel_active_for_each_invocations(
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) {
  let invocation_ids = active_invocations
    .lock()
    .await
    .iter()
    .cloned()
    .collect::<Vec<_>>();
  for invocation_id in invocation_ids {
    let _ = host.cancel_run(&invocation_id).await;
  }
}

pub(in crate::runtime) async fn execute_bounded_for_each<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let root_projection = engine.projection(run_id)?;
  let root_context = root_projection.context.clone();
  let items = resolve_context_reference(&descriptor.items, &root_context).map_err(|_| {
    RuntimeExecutionError::InvalidConfiguration(format!(
      "for-each {:?} items reference is unavailable",
      descriptor.for_each_id
    ))
  })?;
  let Value::Array(items) = items else {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "for-each {:?} items must resolve to an array",
      descriptor.for_each_id
    )));
  };
  if items.len() > 10_000 {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "for-each {:?} exceeds the 10000-item limit",
      descriptor.for_each_id
    )));
  }

  let total = items.len() as u32;
  let concurrency = descriptor.concurrency as usize;
  let items_digest = for_each_digest(&Value::Array(items.clone()))?;
  let mut shared_engine = SharedForEachEngine::new(engine);
  match root_projection.for_each.get(&descriptor.for_each_id) {
    None => shared_engine.append_payload(
      run_id,
      RunEventPayload::ForEachOpened(ForEachOpenedData {
        for_each_id: descriptor.for_each_id.clone(),
        total,
        items_digest: items_digest.clone(),
        concurrency: descriptor.concurrency,
      }),
    )?,
    Some(loop_state)
      if loop_state.total == total
        && loop_state.items_digest == items_digest
        && loop_state.concurrency == descriptor.concurrency =>
    {
      if loop_state.status == ForEachStatus::Succeeded {
        return Ok(());
      }
      if loop_state.status != ForEachStatus::Open {
        return Err(RuntimeExecutionError::Stalled(format!(
          "for-each {:?} is already terminal",
          descriptor.for_each_id
        )));
      }
    }
    Some(_) => {
      return Err(RuntimeExecutionError::Stalled(format!(
        "for-each {:?} recovery input does not match its captured count, digest, or concurrency",
        descriptor.for_each_id
      )));
    }
  }
  report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;

  let active_invocations = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
  let mut aggregate_results = vec![None; items.len()];
  let loop_projection = shared_engine.projection(run_id)?;
  let loop_state = loop_projection
    .for_each
    .get(&descriptor.for_each_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each disappeared".to_string()))?;
  let mut resumed_indexes = loop_state
    .iterations
    .iter()
    .filter_map(|(index, status)| match status {
      ForEachIterationStatus::Started => Some(*index),
      ForEachIterationStatus::Succeeded { result } => {
        aggregate_results[*index as usize] = result.clone();
        None
      }
      ForEachIterationStatus::Failed { .. } | ForEachIterationStatus::Skipped => None,
    })
    .collect::<VecDeque<_>>();
  let mut pending_indexes = (0..total)
    .filter(|index| !loop_state.iterations.contains_key(index))
    .collect::<VecDeque<_>>();
  let mut stop_admission = loop_state
    .iterations
    .values()
    .any(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    || matches!(
      root_projection.status,
      RunStatus::Cancelling | RunStatus::Finalizing
    )
    || root_projection.timeout_reached_at.is_some();
  let mut first_error = None;
  let mut active = FuturesUnordered::new();

  loop {
    while !stop_admission && active.len() < concurrency {
      let (index, resumed) = if let Some(index) = resumed_indexes.pop_front() {
        (index, true)
      } else if let Some(index) = pending_indexes.pop_front() {
        (index, false)
      } else {
        break;
      };
      if !resumed {
        let scope = ForEachIterationScope {
          for_each_id: descriptor.for_each_id.clone(),
          index,
        };
        shared_engine.append_scoped_payload(
          run_id,
          scope,
          RunEventPayload::ForEachIterationStarted(ForEachIterationIdentityData {
            for_each_id: descriptor.for_each_id.clone(),
          }),
        )?;
      }

      let mut iteration_engine = shared_engine.clone();
      let iteration_descriptor = descriptor.clone();
      let iteration_context = root_context.clone();
      let item = items[index as usize].clone();
      let iteration_options = options.clone();
      let iteration_invocations = Arc::clone(&active_invocations);
      active.push(async move {
        let result = execute_for_each_iteration(
          &mut iteration_engine,
          run_id,
          &iteration_descriptor,
          &iteration_context,
          &item,
          index,
          total,
          &iteration_options,
          host,
          &iteration_invocations,
        )
        .await;
        (index, result)
      });
    }

    if active.is_empty() {
      break;
    }

    tokio::select! {
      completion = active.next() => {
        let Some((index, result)) = completion else { break };
        match result {
          Ok(result) => aggregate_results[index as usize] = result,
          Err(error) => {
            let projection = shared_engine.projection(run_id)?;
            let iteration_is_open = projection
              .for_each
              .get(&descriptor.for_each_id)
              .and_then(|state| state.iterations.get(&index))
              .is_some_and(|status| matches!(status, ForEachIterationStatus::Started));
            if iteration_is_open {
              let (failed_node_id, failure_kind) = for_each_latest_iteration_failure(
                &projection,
                &descriptor.for_each_id,
                index,
              )
              .map(|(node_id, failure)| (node_id, for_each_failure_kind(failure.kind)))
              .unwrap_or_else(|| {
                (
                  descriptor.body.terminal_node_id.clone(),
                  ForEachIterationFailureKind::Interrupted,
                )
              });
              shared_engine.append_scoped_payload(
                run_id,
                ForEachIterationScope {
                  for_each_id: descriptor.for_each_id.clone(),
                  index,
                },
                RunEventPayload::ForEachIterationFailed(ForEachIterationFailedData {
                  for_each_id: descriptor.for_each_id.clone(),
                  failed_node_id,
                  failure_kind,
                }),
              )?;
            }
            if first_error.is_none() {
              first_error = Some(error);
              stop_admission = true;
              cancel_active_for_each_invocations(host, &active_invocations).await;
            }
          }
        }
        report_for_each_progress(
          &shared_engine,
          options,
          run_id,
          &descriptor.for_each_id,
        )?;
      }
      _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !stop_admission => {
        settle_workflow_timeout_if_due(
          &mut shared_engine,
          run_id,
          options.clock.now(),
          options,
        )?;
        let projection = shared_engine.projection(run_id)?;
        if projection.timeout_reached_at.is_some() || projection.status == RunStatus::Cancelling {
          stop_admission = true;
          cancel_active_for_each_invocations(host, &active_invocations).await;
        }
      }
    }
  }

  let projection = shared_engine.projection(run_id)?;
  if projection.status == RunStatus::Cancelling {
    settle_open_for_each(
      &mut shared_engine,
      run_id,
      descriptor,
      ForEachTerminalDisposition::Cancelled,
    )?;
    report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;
    return Err(cancelled_run_error(&shared_engine, run_id)?);
  }
  if projection.timeout_reached_at.is_some() {
    return Err(resumed_failure(&shared_engine, run_id, projection)?);
  }
  if first_error.is_some() || stop_admission {
    let projection = shared_engine.projection(run_id)?;
    let primary_failure = projection
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|loop_state| {
        loop_state
          .iterations
          .keys()
          .filter_map(|index| {
            for_each_latest_iteration_failure(&projection, &descriptor.for_each_id, *index)
          })
          .find(|(_, failure)| failure.kind != AttemptFailureKind::InvocationCancelled)
      })
      .map(|(_, failure)| failure)
      .unwrap_or_else(|| AttemptFailure {
        kind: AttemptFailureKind::Interrupted,
        code: "WOML_FOR_EACH_INTERRUPTED".to_string(),
        message: format!(
          "For-each {:?} could not complete one of its iterations.",
          descriptor.for_each_id
        ),
        details: None,
        ..AttemptFailure::legacy_defaults()
      });
    settle_open_for_each(
      &mut shared_engine,
      run_id,
      descriptor,
      ForEachTerminalDisposition::Failed,
    )?;
    report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;
    if shared_engine.projection(run_id)?.business_outcome.is_none() {
      shared_engine.decide_run_failed(
        run_id,
        lifecycle_failure_from_attempt(primary_failure),
        options.clock.now(),
      )?;
    }
    return Err(resumed_failure(
      &shared_engine,
      run_id,
      shared_engine.projection(run_id)?,
    )?);
  }

  let mut public_output = Map::new();
  public_output.insert("total".to_string(), Value::from(total));
  public_output.insert("succeeded".to_string(), Value::from(total));
  if descriptor.result.is_some() && !aggregate_results.is_empty() {
    let results = aggregate_results
      .into_iter()
      .enumerate()
      .map(|(index, result)| {
        result.ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "for-each {:?} completed without result slot {index}",
            descriptor.for_each_id
          ))
        })
      })
      .collect::<Result<Vec<_>, _>>()?;
    public_output.insert("results".to_string(), Value::Array(results));
  }
  shared_engine.append_payload(
    run_id,
    RunEventPayload::ForEachSucceeded(ForEachSettledData {
      for_each_id: descriptor.for_each_id.clone(),
      total,
      succeeded: total,
      failed: 0,
      skipped: 0,
      aggregate_digest: for_each_digest(&Value::Object(public_output))?,
    }),
  )?;
  report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;
  Ok(())
}

fn for_each_ready_script_concurrency(
  descriptor: &crate::model::CompiledForEach,
  node_ids: &[String],
) -> usize {
  let mut parallel_id = None;
  for node_id in node_ids {
    let owner = descriptor
      .body
      .edges
      .iter()
      .find(|edge| edge.to == *node_id)
      .and_then(|edge| edge.parallel_id.as_deref());
    match (parallel_id, owner) {
      (None, Some(owner)) => parallel_id = Some(owner),
      (Some(expected), Some(owner)) if expected == owner => {}
      _ => return 1,
    }
  }
  let Some(parallel_id) = parallel_id else {
    return 1;
  };
  let start_id = format!("__woml_parallel__{parallel_id}__start");
  let Some(ValueExpression::Literal {
    value: Value::Number(value),
  }) = descriptor
    .body
    .nodes
    .iter()
    .find(|node| node.id == start_id)
    .and_then(|node| match &node.inputs {
      ValueExpression::Object { fields } => fields.get("concurrency"),
      _ => None,
    })
  else {
    return 1;
  };
  value
    .as_u64()
    .and_then(|value| usize::try_from(value).ok())
    .unwrap_or(1)
    .clamp(1, node_ids.len())
}

#[allow(clippy::too_many_arguments)]
async fn execute_for_each_iteration<E: RuntimeDagEngine + Clone>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  root_context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) -> Result<Option<Value>, RuntimeExecutionError> {
  let scope = ForEachIterationScope {
    for_each_id: descriptor.for_each_id.clone(),
    index,
  };

  let projection = engine.projection(run_id)?;
  let execution = projection
    .for_each
    .get(&descriptor.for_each_id)
    .and_then(|loop_state| loop_state.executions.get(&index))
    .cloned()
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "for-each {:?} iteration {index} has no durable execution projection",
        descriptor.for_each_id
      ))
    })?;
  let mut context = execution.context.clone();
  if context.steps.is_empty() {
    context = root_context.clone();
  }
  context.steps.retain(|step_id, _| {
    descriptor.outer_step_ids.contains(step_id)
      || descriptor.body.nodes.iter().any(|node| node.id == *step_id)
  });
  let mut completed = execution
    .attempts
    .iter()
    .filter_map(|attempt| {
      matches!(attempt.status, AttemptStatus::Succeeded { .. })
        .then_some(attempt.identity.node_id.clone())
    })
    .collect::<HashSet<_>>();
  let mut selections = execution.choice_selections.clone();
  selections.extend(execution.branch_selections.clone());
  for choice in &descriptor.body.choices {
    if selections.contains_key(&choice.choice_id) {
      completed.insert(choice.selector_node_id.clone());
    }
  }
  for branch_id in execution.branch_selections.keys() {
    completed.insert(format!("__woml_branch__{branch_id}__select"));
  }
  for (parallel_id, group) in &execution.parallel_groups {
    completed.insert(format!("__woml_parallel__{parallel_id}__start"));
    if !matches!(group.status, ParallelGroupStatus::Started) {
      completed.insert(parallel_id.clone());
    }
  }
  while !completed.contains(&descriptor.body.terminal_node_id) {
    let ready = for_each_ready_nodes(descriptor, &completed, &selections);
    if ready.is_empty() {
      return Err(RuntimeExecutionError::Stalled(format!(
        "for-each {:?} iteration {index} local DAG stalled",
        descriptor.for_each_id
      )));
    }
    let ready_scripts = ready
      .iter()
      .filter(|node_id| {
        descriptor
          .body
          .nodes
          .iter()
          .find(|node| node.id == node_id.as_str())
          .is_some_and(|node| node.handler == "runtime.script")
      })
      .cloned()
      .collect::<Vec<_>>();
    let inner_concurrency = for_each_ready_script_concurrency(descriptor, &ready_scripts);
    if ready_scripts.len() > 1 && inner_concurrency > 1 {
      let script_engine = Clone::clone(&*engine);
      let script_descriptor = descriptor.clone();
      let script_context = context.clone();
      let script_item = item.clone();
      let script_options = options.clone();
      let script_invocations = Arc::clone(active_invocations);
      let mut active_scripts = futures_util::stream::iter(ready_scripts)
        .map(move |node_id| {
          let mut script_engine = script_engine.clone();
          let script_descriptor = script_descriptor.clone();
          let script_context = script_context.clone();
          let script_item = script_item.clone();
          let script_options = script_options.clone();
          let script_invocations = Arc::clone(&script_invocations);
          async move {
            let result = execute_for_each_iteration_script(
              &mut script_engine,
              run_id,
              &script_descriptor,
              &node_id,
              &script_context,
              &script_item,
              index,
              total,
              &script_options,
              host,
              &script_invocations,
            )
            .await;
            (node_id, result)
          }
        })
        .buffer_unordered(inner_concurrency);
      let mut outputs = Vec::new();
      let mut first_error = None;
      while let Some((node_id, result)) = active_scripts.next().await {
        match result {
          Ok(output) => outputs.push((node_id, output)),
          Err(error) if first_error.is_none() => first_error = Some(error),
          Err(_) => {}
        }
      }
      if let Some(error) = first_error {
        return Err(error);
      }
      for (node_id, output) in outputs {
        context.steps.insert(node_id.clone(), output);
        completed.insert(node_id);
      }
      continue;
    }
    for node_id in ready {
      let node = descriptor
        .body
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled("for-each body node disappeared".to_string())
        })?;
      match node.handler.as_str() {
        "engine.lock-acquire" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.acquire_node_id == node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock acquire node has no compiled descriptor.".to_string(),
              )
            })?;
          let projected = engine
            .projection(run_id)?
            .for_each
            .get(&descriptor.for_each_id)
            .and_then(|loop_| loop_.executions.get(&index))
            .and_then(|execution| execution.locks.get(&lock.lock_id))
            .cloned();
          if projected.is_none() {
            let key = resolve_for_each_lock_key(&lock.key, &context, item, index, total)?;
            let now = options.clock.now();
            let deadline = now
              + chrono::Duration::milliseconds(i64::try_from(lock.acquire_timeout_ms).map_err(
                |_| {
                  RuntimeExecutionError::InvalidConfiguration(
                    "Iteration lock acquisition timeout exceeds the runtime clock.".to_string(),
                  )
                },
              )?);
            engine.register_iteration_lock_waiter(
              run_id,
              &lock.lock_id,
              &key,
              scope.clone(),
              now,
              deadline,
            )?;
          }
          options.suspend_policy_execution_slot().await?;
          loop {
            let state = engine
              .projection(run_id)?
              .for_each
              .get(&descriptor.for_each_id)
              .and_then(|loop_| loop_.executions.get(&index))
              .and_then(|execution| execution.locks.get(&lock.lock_id))
              .cloned()
              .ok_or_else(|| {
                RuntimeExecutionError::Stalled(
                  "Iteration lock waiter disappeared from durable projection.".to_string(),
                )
              })?;
            match state.status {
              crate::LockStatus::Held => {
                match engine.renew_iteration_lock_lease(
                  run_id,
                  &lock.lock_id,
                  scope.clone(),
                  options.clock.now(),
                )? {
                  crate::LockLeaseMaintenance::Renewed => {
                    options.ensure_iteration_lock_renewal(
                      run_id,
                      &lock.lock_id,
                      scope.clone(),
                      lock.lease_ms,
                    );
                    break;
                  }
                  crate::LockLeaseMaintenance::Lost | crate::LockLeaseMaintenance::NotHeld => {
                    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                      "Iteration lock {:?} expired before its protected continuation resumed.",
                      lock.lock_id
                    )));
                  }
                }
              }
              crate::LockStatus::Waiting if options.clock.now() < state.deadline => {
                tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
              }
              crate::LockStatus::Waiting => {
                engine.timeout_iteration_lock_waiter(
                  run_id,
                  &lock.lock_id,
                  scope.clone(),
                  options.clock.now(),
                )?;
                return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                  "Iteration lock {:?} was not acquired before its deadline.",
                  lock.lock_id
                )));
              }
              _ => {
                return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                  "Iteration lock {:?} did not acquire safely.",
                  lock.lock_id
                )));
              }
            }
          }
        }
        "engine.lock-release" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.release_node_id == node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock release node has no compiled descriptor.".to_string(),
              )
            })?;
          let held = engine
            .projection(run_id)?
            .for_each
            .get(&descriptor.for_each_id)
            .and_then(|loop_| loop_.executions.get(&index))
            .and_then(|execution| execution.locks.get(&lock.lock_id))
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock release has no acquired lease.".to_string(),
              )
            })?;
          let acquired_at = held.acquired_at.ok_or_else(|| {
            RuntimeExecutionError::Stalled(
              "Iteration lock release has no acquisition timestamp.".to_string(),
            )
          })?;
          let released_at = options.clock.now();
          let result = serde_json::json!({
            "status": "released",
            "key": held.key,
            "fencingToken": held.fencing_token,
            "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "waitedMs": (acquired_at - held.registered_at).num_milliseconds().max(0),
          });
          engine.release_iteration_lock(
            run_id,
            &lock.lock_id,
            scope.clone(),
            result.clone(),
            released_at,
          )?;
          options.stop_iteration_lock_renewal(run_id, &lock.lock_id, &scope);
          context.steps.insert(lock.lock_id, result);
        }
        "engine.lock-result" => {}
        "runtime.script" => {
          let output = execute_for_each_iteration_script(
            engine,
            run_id,
            descriptor,
            &node_id,
            &context,
            item,
            index,
            total,
            options,
            host,
            active_invocations,
          )
          .await?;
          context.steps.insert(node_id.clone(), output);
        }
        "engine.choice-select" => {
          let (choice_id, arm_id) =
            for_each_selected_choice(descriptor, &node_id, &context, item, index, total)?;
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::ChoiceSelected(ChoiceSelectedData {
              choice_id: choice_id.clone(),
              arm_id: arm_id.clone(),
            }),
          )?;
          selections.insert(choice_id, arm_id);
        }
        "engine.branch-select" => {
          let (branch_id, arm_id) =
            for_each_selected_branch(descriptor, &node_id, &context, item, index, total)?;
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::BranchSelected(BranchSelectedData {
              branch_id: branch_id.clone(),
              arm_id: arm_id.clone(),
            }),
          )?;
          selections.insert(branch_id, arm_id);
        }
        "engine.branch-result" => {
          let arm_id = selections.get(&node_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each conditional branch {node_id:?} has no durable selection"
            ))
          })?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "for-each conditional result {node_id:?} has invalid inputs"
            )));
          };
          let expression = fields.get(arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each conditional result {node_id:?} has no selected-arm value"
            ))
          })?;
          let output = resolve_for_each_reference(expression, &context, item, index, total)?;
          context.steps.insert(node_id.clone(), output);
        }
        "engine.choice-result" => {
          let choice = descriptor
            .body
            .choices
            .iter()
            .find(|choice| choice.result_node_id.as_deref() == Some(node_id.as_str()))
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "for-each result node {node_id:?} has no choice descriptor"
              ))
            })?;
          let arm_id = selections.get(&choice.choice_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each choice {:?} has no durable selection",
              choice.choice_id
            ))
          })?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "for-each choice result {node_id:?} has invalid inputs"
            )));
          };
          let expression = fields.get(arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each choice result {node_id:?} has no selected arm value"
            ))
          })?;
          let output = resolve_for_each_reference(expression, &context, item, index, total)?;
          context.steps.insert(node_id.clone(), output);
        }
        "engine.choice-join" => {}
        "engine.parallel-start" => {
          let parallel_id = node_id
            .strip_prefix("__woml_parallel__")
            .and_then(|id| id.strip_suffix("__start"))
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "for-each parallel start node {node_id:?} has no canonical group identity"
              ))
            })?
            .to_string();
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::ParallelGroupStarted(ParallelGroupStartedData { parallel_id }),
          )?;
        }
        "engine.parallel-join" => {
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
              parallel_id: node_id.clone(),
              outcome: ParallelGroupOutcome::Succeeded,
              failed_node_ids: Vec::new(),
              cancelled_node_ids: Vec::new(),
            }),
          )?;
        }
        handler => {
          return Err(RuntimeExecutionError::Stalled(format!(
            "for-each body node {node_id:?} uses unsupported handler {handler:?}"
          )))
        }
      }
      completed.insert(node_id);
    }
  }

  let result = descriptor
    .result
    .as_ref()
    .map(|expression| resolve_for_each_reference(expression, &context, item, index, total))
    .transpose()?;
  let result_digest = result.as_ref().map(for_each_digest).transpose()?;
  engine.append_scoped_payload(
    run_id,
    scope,
    RunEventPayload::ForEachIterationSucceeded(ForEachIterationSucceededData {
      for_each_id: descriptor.for_each_id.clone(),
      result: result.clone(),
      result_digest,
    }),
  )?;
  Ok(result)
}
