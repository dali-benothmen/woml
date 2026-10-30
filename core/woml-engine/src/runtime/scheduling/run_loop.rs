//! Central continuation wrapper and the intact workflow scheduling loop.
//!
//! The wrapper owns lifecycle-before/after-loop and script-host release. Its
//! boxed loop is intentional for normal test/worker stacks. Feature modules
//! retain their existing identities and settlement obligations under this loop.

use crate::engine::resolve_context_reference;
use crate::engine::selected_branch_arm;
use crate::engine::selected_choice_arm;
use crate::engine::BranchEvaluationError;
use crate::engine::BranchEvaluationErrorKind;
use crate::event::BranchSelectedData;
use crate::event::ChoiceSelectedData;
use crate::event::ForkOpenedData;
use crate::event::LifecycleFailure;
use crate::event::LifecycleFailureKind;
use crate::event::ParallelGroupStartedData;
use crate::event::RunSucceededData;
use crate::event::SagaCorrectionReason;
use crate::event::SagaIdentityData;
use crate::model::ValueExpression;
use crate::runtime::errors::BranchFailureSite;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::outcomes::WorkflowRuntimeOutcome;
use crate::runtime::script_execution::execute_script_node;
use crate::runtime::script_execution::ScriptNodeOutcome;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  attempt_run_failed_data, cancelled_run_error, completed_terminal_approval_result,
  drive_pending_reusable_step_lifecycles, drive_workflow_lifecycle, ensure_held_locks_current,
  execute_bounded_for_each, execute_fork, execute_parallel_group, execute_saga_correction,
  fail_branch, fail_choice, final_result, final_workflow_attempt_failure, ready_fork,
  register_durable_wait_node, request_approval_wait, resolve_lock_key, resumed_failure,
  reusable_step_descriptor, settle_cancelling_run, settle_workflow_timeout_if_due,
  DurableWaitNodeProgress, ForkRuntimeProgress, RuntimeDagEngine,
};
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RunStatus;
use crate::ScriptHostClient;

pub(in crate::runtime) fn runtime_result_node_id(
  workflow: &CompiledWorkflowDefinition,
) -> Option<&str> {
  workflow
    .graph
    .settlement
    .as_ref()
    .map(|settlement| settlement.main_result_node_id.as_str())
    .or_else(|| workflow.terminal_node_id())
}

pub(in crate::runtime) async fn continue_runtime<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  terminal_node_id: String,
  mut execution_order: Vec<String>,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut host = None;
  drive_workflow_lifecycle(engine, run_id, options, &mut host).await?;
  // The runtime loop intentionally supports every control-flow and correction
  // primitive. Keep its generated future off the comparatively small test and
  // worker thread stacks; adding a new primitive must not increase the stack
  // required by workflows that never use it.
  let mut execution = Box::pin(continue_runtime_loop(
    engine,
    run_id,
    terminal_node_id,
    &mut execution_order,
    options,
    &mut host,
  ))
  .await;
  if execution.is_err() && engine.projection(run_id)?.status == RunStatus::Cancelling {
    settle_cancelling_run(engine, run_id, options.clock.now(), options)?;
    execution = Err(cancelled_run_error(engine, run_id)?);
  }
  let lifecycle = drive_workflow_lifecycle(engine, run_id, options, &mut host).await;
  if let Some(host) = host {
    options.release_script_host(host).await;
  }
  lifecycle?;
  match execution {
    Ok(WorkflowRuntimeOutcome::Succeeded { execution, .. }) => Ok(
      WorkflowRuntimeOutcome::succeeded(final_result(engine, run_id, execution.execution_order)?),
    ),
    Ok(
      waiting @ (WorkflowRuntimeOutcome::Waiting { .. }
      | WorkflowRuntimeOutcome::WaitingTime { .. }
      | WorkflowRuntimeOutcome::WaitingSignal { .. }),
    ) => Ok(waiting),
    Err(RuntimeExecutionError::RunCancelled(_)) => Err(cancelled_run_error(engine, run_id)?),
    Err(error) => Err(error),
  }
}

async fn continue_runtime_loop<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  terminal_node_id: String,
  execution_order: &mut Vec<String>,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  loop {
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    ensure_held_locks_current(engine, run_id, options)?;
    drive_pending_reusable_step_lifecycles(engine, run_id, options, host).await?;
    let current = engine.projection(run_id)?;
    if current.timeout_reached_at.is_some() {
      return Err(resumed_failure(engine, run_id, current)?);
    }
    if current.status == RunStatus::Cancelling {
      let active_saga = engine
        .workflow()
        .graph
        .sagas
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find_map(|saga| {
          current.sagas.get(&saga.saga_id).and_then(|state| {
            (state.phase != crate::SagaPhase::CorrectionCompleted)
              .then_some((saga.clone(), state.reason))
          })
        });
      if let Some((saga, existing_reason)) = active_saga {
        let completed = execute_saga_correction(
          engine,
          run_id,
          &saga,
          existing_reason.unwrap_or(SagaCorrectionReason::Cancel),
          options,
          host,
        )
        .await?;
        if !completed
          && engine
            .projection(run_id)?
            .sagas
            .get(&saga.saga_id)
            .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
        {
          return Err(RuntimeExecutionError::Stalled(
            "saga cancellation requires manual recovery before the run can settle".to_string(),
          ));
        }
      }
      options.ensure_policy_execution_slot().await?;
      settle_cancelling_run(engine, run_id, options.clock.now(), options)?;
      drive_workflow_lifecycle(engine, run_id, options, host).await?;
      return Err(cancelled_run_error(engine, run_id)?);
    }
    if current.status == RunStatus::Cancelled {
      return Err(cancelled_run_error(engine, run_id)?);
    }
    if let Some((saga, reason)) = engine
      .workflow()
      .graph
      .sagas
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find_map(|saga| {
        current.sagas.get(&saga.saga_id).and_then(|state| {
          matches!(
            state.phase,
            crate::SagaPhase::Correcting | crate::SagaPhase::ManualRecovery
          )
          .then_some((saga.clone(), state.reason))
        })
      })
    {
      let reason = reason.ok_or_else(|| {
        RuntimeExecutionError::Stalled("active saga correction has no durable reason".to_string())
      })?;
      let completed = execute_saga_correction(engine, run_id, &saga, reason, options, host).await?;
      if !completed
        && engine
          .projection(run_id)?
          .sagas
          .get(&saga.saga_id)
          .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
      {
        return Err(RuntimeExecutionError::Stalled(format!(
          "saga {:?} requires manual recovery",
          saga.saga_id
        )));
      }
      if reason == SagaCorrectionReason::Failure {
        let failure = engine
          .events(run_id)?
          .into_iter()
          .rev()
          .find_map(|event| match event.payload {
            RunEventPayload::StepAttemptFailed(data)
              if engine
                .workflow()
                .saga_for_body_node(&data.node_id)
                .is_some_and(|owner| owner.saga_id == saga.saga_id) =>
            {
              Some(data)
            }
            _ => None,
          })
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(
              "failed saga correction has no durable forward failure".to_string(),
            )
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::RunFailed(attempt_run_failed_data(
            engine.event_schema_version(),
            &failure,
          )),
        )?;
        return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
      }
      continue;
    }
    let ready = engine.ready_node_ids(run_id)?;
    if ready.is_empty() {
      if let Some(fork) = ready_fork(engine.workflow(), &current, &ready) {
        if host.is_none() {
          *host = Some(options.acquire_script_host().await?);
        }
        let fork_progress = execute_fork(
          engine,
          run_id,
          fork,
          options,
          host.as_ref().expect("script host was initialized"),
        )
        .await?;
        match fork_progress {
          ForkRuntimeProgress::Continued(completed) => execution_order.extend(completed),
          ForkRuntimeProgress::Waiting { completed, outcome } => {
            execution_order.extend(completed);
            return Ok(outcome);
          }
        }
        continue;
      }
    }
    let Some(node_id) = ready.first() else {
      let projection = engine.projection(run_id)?;
      if projection.status == RunStatus::Succeeded {
        return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order.clone(),
        )?));
      }
      if let Some(result) =
        completed_terminal_approval_result(engine.workflow(), &terminal_node_id, &projection)
      {
        engine.append_payload(
          run_id,
          RunEventPayload::RunSucceeded(RunSucceededData {
            terminal_node_id: terminal_node_id.clone(),
            result,
          }),
        )?;
        return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order.clone(),
        )?));
      }
      if projection.status == RunStatus::Running && !projection.pending_retries.is_empty() {
        options.suspend_policy_execution_slot().await?;
        let scheduled_at = projection
          .pending_retries
          .values()
          .map(|retry| retry.scheduled_at)
          .min()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled("pending retry state has no next schedule".to_string())
          })?;
        let now = chrono::Utc::now();
        if scheduled_at > now {
          let wait = (scheduled_at - now).to_std().map_err(|_| {
            RuntimeExecutionError::Stalled("retry schedule exceeds the runtime clock".to_string())
          })?;
          tokio::time::sleep(wait.min(CANCELLATION_POLL_INTERVAL)).await;
        }
        // The retry may become due between the ready-node query above and this
        // clock read. Recompute readiness instead of reporting a false stall.
        continue;
      }
      return Err(RuntimeExecutionError::Stalled(
        "no node is ready before the run reached a terminal state".to_string(),
      ));
    };
    if let Some(lock) = engine
      .workflow()
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.acquire_node_id == *node_id)
      .cloned()
    {
      let projection = engine.projection(run_id)?;
      if !projection.locks.contains_key(&lock.lock_id) {
        let key = resolve_lock_key(&lock.key, &projection.context)?;
        let now = options.clock.now();
        let deadline = now
          + chrono::Duration::milliseconds(i64::try_from(lock.acquire_timeout_ms).map_err(
            |_| {
              RuntimeExecutionError::InvalidConfiguration(
                "Lock acquisition timeout exceeds the runtime clock.".to_string(),
              )
            },
          )?);
        engine.register_lock_waiter(run_id, &lock.lock_id, &key, now, deadline)?;
      }
      options.suspend_policy_execution_slot().await?;
      loop {
        let projection = engine.projection(run_id)?;
        let state = projection.locks.get(&lock.lock_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "Registered lock disappeared from durable projection.".to_string(),
          )
        })?;
        match state.status {
          crate::LockStatus::Held => {
            options.ensure_lock_renewal(run_id, &lock.lock_id, lock.lease_ms);
            break;
          }
          crate::LockStatus::Released => break,
          crate::LockStatus::Waiting => {
            if options.clock.now() >= state.deadline {
              let resolved_at = options.clock.now();
              engine.timeout_lock_waiter(run_id, &lock.lock_id, resolved_at)?;
              engine.decide_run_failed(
                run_id,
                LifecycleFailure {
                  kind: LifecycleFailureKind::TimedOut,
                  code: "WOML_LOCK_ACQUIRE_TIMED_OUT".to_string(),
                  message: format!(
                    "Lock {:?} was not acquired before its deadline.",
                    lock.lock_id
                  ),
                },
                resolved_at,
              )?;
              return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
            }
            tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
          }
          _ => {
            return Err(RuntimeExecutionError::Stalled(format!(
              "Lock {:?} acquisition did not succeed.",
              lock.lock_id
            )))
          }
        }
      }
      execution_order.push(lock.acquire_node_id);
      continue;
    }
    if let Some(lock) = engine
      .workflow()
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.release_node_id == *node_id)
      .cloned()
    {
      let projection = engine.projection(run_id)?;
      let held = projection.locks.get(&lock.lock_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled("Lock release has no projected acquisition.".to_string())
      })?;
      let acquired_at = held.acquired_at.ok_or_else(|| {
        RuntimeExecutionError::Stalled("Lock release has no acquisition timestamp.".to_string())
      })?;
      let released_at = options.clock.now();
      let waited_ms = (acquired_at - held.registered_at).num_milliseconds().max(0);
      let result = serde_json::json!({
        "status": "released", "key": held.key, "fencingToken": held.fencing_token,
        "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "waitedMs": waited_ms,
      });
      engine.release_lock(run_id, &lock.lock_id, result, released_at)?;
      options.stop_lock_renewal(run_id, &lock.lock_id);
      execution_order.push(lock.release_node_id);
      continue;
    }
    options.ensure_policy_execution_slot().await?;
    if let Some(mut group) = engine.workflow().parallel_group_for_child(node_id) {
      if ready.iter().any(|ready_id| {
        engine
          .workflow()
          .parallel_group_for_child(ready_id)
          .is_none_or(|owner| owner.parallel_id != group.parallel_id)
      }) {
        return Err(RuntimeExecutionError::Stalled(format!(
          "parallel group {:?} became ready alongside an unrelated node",
          group.parallel_id
        )));
      }
      let wait_node_ids = group
        .child_node_ids
        .iter()
        .filter(|child_id| {
          ready.contains(child_id)
            && engine
              .workflow()
              .node(child_id)
              .is_some_and(|node| node.handler == "engine.durable-wait")
        })
        .cloned()
        .collect::<Vec<_>>();
      let mut pending_wait = None;
      for wait_node_id in wait_node_ids {
        match register_durable_wait_node(engine, run_id, &wait_node_id, options).await? {
          DurableWaitNodeProgress::Resolved => execution_order.push(wait_node_id),
          DurableWaitNodeProgress::Waiting(outcome) => pending_wait = Some(outcome),
        }
      }
      group.child_node_ids.retain(|child_id| {
        ready.contains(child_id)
          && engine
            .workflow()
            .node(child_id)
            .is_some_and(|node| node.handler == "runtime.script")
      });
      if !group.child_node_ids.is_empty() {
        if host.is_none() {
          *host = Some(options.acquire_script_host().await?);
        }
        let completed = execute_parallel_group(
          engine,
          run_id,
          group,
          options,
          host.as_ref().expect("script host was initialized"),
        )
        .await?;
        execution_order.extend(completed);
      }
      if let Some(outcome) = pending_wait {
        return Ok(outcome);
      }
      continue;
    }
    if let Some(wait_node_id) = ready.iter().find(|ready_id| {
      engine
        .workflow()
        .node(ready_id)
        .is_some_and(|node| node.handler == "engine.durable-wait")
    }) {
      match register_durable_wait_node(engine, run_id, wait_node_id, options).await? {
        DurableWaitNodeProgress::Resolved => {
          execution_order.push(wait_node_id.clone());
          continue;
        }
        DurableWaitNodeProgress::Waiting(outcome) => return Ok(outcome),
      }
    }
    if let Some(fork) = ready_fork(engine.workflow(), &current, &ready) {
      if host.is_none() {
        *host = Some(options.acquire_script_host().await?);
      }
      let fork_progress = execute_fork(
        engine,
        run_id,
        fork,
        options,
        host.as_ref().expect("script host was initialized"),
      )
      .await?;
      match fork_progress {
        ForkRuntimeProgress::Continued(completed) => execution_order.extend(completed),
        ForkRuntimeProgress::Waiting { completed, outcome } => {
          execution_order.extend(completed);
          return Ok(outcome);
        }
      }
      continue;
    }
    if ready.len() != 1 {
      return Err(RuntimeExecutionError::Stalled(format!(
        "the current runtime received more than one ready node: {ready:?}"
      )));
    }

    let (handler, inputs, source) = {
      let node = engine
        .workflow()
        .node(node_id)
        .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
      (
        node.handler.clone(),
        node.inputs.clone(),
        node.script_source().map(str::to_string),
      )
    };

    match handler.as_str() {
      "engine.saga-open" => {
        let saga = engine
          .workflow()
          .saga_for_control_node(node_id)
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga open node {node_id:?} has no Model v19 descriptor"
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::SagaStarted(SagaIdentityData {
            saga_id: saga.saga_id.clone(),
          }),
        )?;
      }
      "engine.saga-commit" => {
        let saga = engine
          .workflow()
          .saga_for_control_node(node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga commit node {node_id:?} has no Model v19 descriptor"
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::SagaForwardSucceeded(SagaIdentityData {
            saga_id: saga.saga_id.clone(),
          }),
        )?;
        let cleanup_succeeded = execute_saga_correction(
          engine,
          run_id,
          &saga,
          SagaCorrectionReason::Success,
          options,
          host,
        )
        .await?;
        if !cleanup_succeeded {
          let projection = engine.projection(run_id)?;
          if projection
            .sagas
            .get(&saga.saga_id)
            .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
          {
            return Err(RuntimeExecutionError::Stalled(format!(
              "saga {:?} requires manual cleanup recovery",
              saga.saga_id
            )));
          }
          engine.decide_run_failed(
            run_id,
            LifecycleFailure {
              kind: LifecycleFailureKind::ScriptThrew,
              code: "WOML_CLEANUP_FAILED".to_string(),
              message: format!(
                "Saga {:?} completed its forward work but one or more cleanup actions failed.",
                saga.saga_id
              ),
            },
            options.clock.now(),
          )?;
          return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
        }
      }
      "engine.saga-result" => {
        let saga = engine
          .workflow()
          .saga_for_control_node(node_id)
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga result node {node_id:?} has no Model v19 descriptor"
            ))
          })?;
        let output = engine
          .projection(run_id)?
          .context
          .steps
          .get(&saga.body_result_node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga {:?} has no result from {:?}",
              saga.saga_id, saga.body_result_node_id
            ))
          })?;
        engine.publish_pure_result(run_id, node_id, output)?;
        execution_order.push(node_id.clone());
      }
      "engine.fork-open" => {
        let fork = engine
          .workflow()
          .graph
          .forks
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|fork| fork.open_node_id == *node_id)
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "fork open node {node_id:?} has no Model v13 descriptor"
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::ForkOpened(ForkOpenedData {
            fork_id: fork.fork_id.clone(),
          }),
        )?;
      }
      "engine.for-each-open" => {
        let descriptor = engine
          .workflow()
          .graph
          .for_each
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|descriptor| descriptor.open_node_id == *node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each open node {node_id:?} has no Model v16 descriptor"
            ))
          })?;
        if host.is_none() {
          *host = Some(options.acquire_script_host().await?);
        }
        execute_bounded_for_each(
          engine,
          run_id,
          &descriptor,
          options,
          host.as_ref().expect("for-each script host was initialized"),
        )
        .await?;
        execution_order.push(descriptor.for_each_id);
      }
      "engine.parallel-start" => {
        let parallel_id = node_id
          .strip_prefix("__woml_parallel__")
          .and_then(|id| id.strip_suffix("__start"))
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "parallel start node {node_id:?} has no canonical group identity"
            ))
          })?
          .to_string();
        engine.append_payload(
          run_id,
          RunEventPayload::ParallelGroupStarted(ParallelGroupStartedData { parallel_id }),
        )?;
      }
      "engine.branch-select" => {
        let context = engine.projection(run_id)?.context;
        match selected_branch_arm(engine.workflow(), node_id, &context) {
          Ok(arm_id) => {
            let branch_id = crate::engine::selector_branch_id(node_id)
              .ok_or_else(|| {
                RuntimeExecutionError::Stalled(format!(
                  "selector node {node_id:?} has no canonical branch identity"
                ))
              })?
              .to_string();
            engine.append_payload(
              run_id,
              RunEventPayload::BranchSelected(BranchSelectedData { branch_id, arm_id }),
            )?;
          }
          Err(error) => {
            let site = if matches!(error.kind, BranchEvaluationErrorKind::SelectionInvalid) {
              BranchFailureSite::Selection
            } else {
              BranchFailureSite::Test
            };
            return fail_branch(engine, run_id, error, site);
          }
        }
      }
      "engine.choice-select" => {
        let context = engine.projection(run_id)?.context;
        let (choice_id, arm_id) = match selected_choice_arm(engine.workflow(), node_id, &context) {
          Ok(selection) => selection,
          Err(error) => {
            return fail_choice(engine, run_id, error);
          }
        };
        engine.append_payload(
          run_id,
          RunEventPayload::ChoiceSelected(ChoiceSelectedData { choice_id, arm_id }),
        )?;
      }
      "engine.branch-result" => {
        let projection = engine.projection(run_id)?;
        let arm_id = projection
          .branch_selections
          .get(node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "branch result {node_id:?} became ready without a recorded selection"
            ))
          })?;
        let ValueExpression::Object { fields } = &inputs else {
          return Err(RuntimeExecutionError::Stalled(format!(
            "branch result {node_id:?} has invalid compiled inputs"
          )));
        };
        let expression = fields.get(&arm_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "branch result {node_id:?} has no value for selected arm {arm_id:?}"
          ))
        })?;
        let output = match resolve_context_reference(expression, &projection.context) {
          Ok(output) => output,
          Err(error) => {
            return fail_branch(
              engine,
              run_id,
              BranchEvaluationError {
                branch_id: node_id.clone(),
                arm_id: Some(arm_id),
                path: Some(error.path),
                kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
              },
              BranchFailureSite::Result,
            );
          }
        };
        engine.publish_pure_result(run_id, node_id, output.clone())?;
        execution_order.push(node_id.clone());
        // Model v13 workflows must pass through the explicit settlement node.
        // The last value-producing main-route step is the public result source,
        // but it is not permission to publish success while owned fork work may
        // still have failed or remain unsettled.
        if node_id == &terminal_node_id && engine.workflow().graph.settlement.is_none() {
          engine.append_payload(
            run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: output,
            }),
          )?;
          return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
            engine,
            run_id,
            execution_order.clone(),
          )?));
        }
      }
      "engine.choice-result" => {
        let projection = engine.projection(run_id)?;
        let choice = engine
          .workflow()
          .graph
          .choices
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|choice| choice.result_node_id.as_deref() == Some(node_id))
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} has no compiled choice descriptor"
            ))
          })?;
        let arm_id = projection
          .choice_selections
          .get(&choice.choice_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} became ready without a recorded selection"
            ))
          })?;
        let ValueExpression::Object { fields } = &inputs else {
          return Err(RuntimeExecutionError::Stalled(format!(
            "choice result {node_id:?} has invalid compiled inputs"
          )));
        };
        let expression = fields.get(&arm_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "choice result {node_id:?} has no value for selected arm {arm_id:?}"
          ))
        })?;
        let output = match resolve_context_reference(expression, &projection.context) {
          Ok(output) => output,
          Err(error) => {
            return fail_choice(
              engine,
              run_id,
              BranchEvaluationError {
                branch_id: choice.choice_id.clone(),
                arm_id: Some(arm_id),
                path: Some(error.path),
                kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
              },
            );
          }
        };
        engine.publish_pure_result(run_id, node_id, output.clone())?;
        execution_order.push(node_id.clone());
        if node_id == &terminal_node_id && engine.workflow().graph.settlement.is_none() {
          engine.append_payload(
            run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: output,
            }),
          )?;
          return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
            engine,
            run_id,
            execution_order.clone(),
          )?));
        }
      }
      "engine.approval-wait" => {
        return request_approval_wait(engine, run_id, node_id, options);
      }
      "engine.durable-wait" => {
        match register_durable_wait_node(engine, run_id, node_id, options).await? {
          DurableWaitNodeProgress::Resolved => {
            execution_order.push(node_id.clone());
            continue;
          }
          DurableWaitNodeProgress::Waiting(outcome) => return Ok(outcome),
        }
      }
      "runtime.script" => {
        let source = source.ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("node {node_id:?} has no script source"))
        })?;
        let publish_terminal_success = node_id == &terminal_node_id
          && engine.workflow().graph.settlement.is_none()
          && reusable_step_descriptor(engine.workflow(), node_id).is_none();
        let outcome = execute_script_node(
          engine,
          run_id,
          node_id,
          &source,
          publish_terminal_success,
          options,
          host,
        )
        .await?;
        let output = match outcome {
          ScriptNodeOutcome::Succeeded(output) => output,
          ScriptNodeOutcome::WorkflowSucceeded => {
            execution_order.push(node_id.clone());
            return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
              engine,
              run_id,
              execution_order.clone(),
            )?));
          }
          ScriptNodeOutcome::RetryScheduled | ScriptNodeOutcome::Cancelled => continue,
          ScriptNodeOutcome::SagaFailed(failure) => {
            let saga = engine
              .workflow()
              .saga_for_body_node(node_id)
              .cloned()
              .ok_or_else(|| {
                RuntimeExecutionError::Stalled(
                  "failed saga step has no owning Model v19 descriptor".to_string(),
                )
              })?;
            let correction_completed = execute_saga_correction(
              engine,
              run_id,
              &saga,
              SagaCorrectionReason::Failure,
              options,
              host,
            )
            .await?;
            if !correction_completed
              && engine
                .projection(run_id)?
                .sagas
                .get(&saga.saga_id)
                .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
            {
              return Err(RuntimeExecutionError::Stalled(format!(
                "saga {:?} requires manual recovery",
                saga.saga_id
              )));
            }
            engine.append_payload(
              run_id,
              RunEventPayload::RunFailed(attempt_run_failed_data(
                engine.event_schema_version(),
                &failure,
              )),
            )?;
            return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
          }
        };
        execution_order.push(node_id.clone());
        // In Model v13 this is only the main route's value source. The
        // workflow-settlement node decides whether all owned fork work permits
        // that value to become a public success result.
        if node_id == &terminal_node_id && engine.workflow().graph.settlement.is_none() {
          engine.append_payload(
            run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: output,
            }),
          )?;
          return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
            engine,
            run_id,
            execution_order.clone(),
          )?));
        }
      }
      "engine.workflow-settlement" => {
        let projection = engine.projection(run_id)?;
        if let Some(failure) = final_workflow_attempt_failure(&projection) {
          engine.append_payload(
            run_id,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              engine.event_schema_version(),
              &failure,
            )),
          )?;
          return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
        }
        let settlement = engine.workflow().graph.settlement.as_ref().ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "workflow settlement node has no Model v13 descriptor".to_string(),
          )
        })?;
        if settlement.node_id != *node_id {
          return Err(RuntimeExecutionError::Stalled(format!(
            "workflow settlement node {node_id:?} does not match Model v13"
          )));
        }
        let result = engine
          .projection(run_id)?
          .context
          .steps
          .get(&settlement.main_result_node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "workflow settlement has no public main result from {:?}",
              settlement.main_result_node_id
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::RunSucceeded(RunSucceededData {
            terminal_node_id: settlement.main_result_node_id.clone(),
            result,
          }),
        )?;
        return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order.clone(),
        )?));
      }
      _ => {
        return Err(RuntimeExecutionError::Stalled(format!(
          "ready node {node_id:?} uses unknown handler {handler:?}"
        )));
      }
    }
  }
}
