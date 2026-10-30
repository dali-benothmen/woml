//! Fork opening snapshots, owned branches, join modes, and branch settlement.
//!
//! Main-route eligibility and final ownership settlement are different gates.
//! Unjoined work must not leak input visibility or disappear on cancellation.

use crate::engine::resolve_context_reference;
use crate::engine::selected_branch_arm;
use crate::engine::selected_choice_arm;
use crate::engine::step_effect_idempotency_key;
use crate::event::BranchSelectedData;
use crate::event::ChoiceSelectedData;
use crate::event::ForkBranchOutcome;
use crate::event::ForkBranchSettledData;
use crate::event::ForkJoinOutcome;
use crate::event::ForkJoinSettledData;
use crate::event::ParallelGroupStartedData;
use crate::event::StepAttemptFailedData;
use crate::event::StepAttemptStartedData;
use crate::event::StepAttemptSucceededData;
use crate::model::ValueExpression;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::protocol::RuntimeModuleBinding;
use crate::runtime::context::resolved_script_secrets;
use crate::runtime::context::reusable_step_binding;
use crate::runtime::context::runtime_context_for_node;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::outcomes::WorkflowRuntimeOutcome;
use crate::runtime::progress::report_attempt_failed;
use crate::runtime::progress::report_retry_scheduled;
use crate::runtime::script_execution::ensure_local_script_execution;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  append_step_attempt_success, attempt_run_failed_data, correct_saga_failure_for_node,
  drive_reusable_step_lifecycle, execute_parallel_group, fail_choice,
  final_attempt_failure_in_nodes, final_workflow_attempt_failure, generated_id,
  invoke_parallel_child, next_parallel_completion, register_durable_wait_node,
  request_approval_wait, resolve_lock_key, resumed_failure, settle_workflow_timeout_if_due,
  ActiveParallelInvocation, DurableWaitNodeProgress, ParallelInvocationRequest, RuntimeDagEngine,
};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use crate::ScriptHostClient;
use crate::StepFailureDisposition;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::collections::VecDeque;

fn fork_branch_routes(
  workflow: &CompiledWorkflowDefinition,
  fork: &crate::model::CompiledFork,
) -> BTreeMap<String, HashSet<String>> {
  fork
    .branches
    .iter()
    .map(|branch| {
      let mut route = HashSet::new();
      let mut pending = VecDeque::from([branch.entry_node_id.as_str()]);
      while let Some(node_id) = pending.pop_front() {
        if !route.insert(node_id.to_string()) || node_id == branch.terminal_node_id {
          continue;
        }
        pending.extend(
          workflow
            .graph
            .edges
            .iter()
            .filter(|edge| edge.from == node_id)
            .map(|edge| edge.to.as_str()),
        );
      }
      (branch.branch_id.clone(), route)
    })
    .collect()
}

pub(in crate::runtime) fn ready_fork(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  ready: &[String],
) -> Option<crate::model::CompiledFork> {
  workflow
    .graph
    .forks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|fork| {
      let Some(state) = projection.forks.get(&fork.fork_id) else {
        return false;
      };
      if state.join_status != crate::ForkJoinStatus::Pending {
        return false;
      }
      let routes = fork_branch_routes(workflow, fork);
      let has_ready_work = ready.iter().any(|node_id| {
        routes.values().any(|route| route.contains(node_id)) || node_id == &fork.join_node_id
      });
      let has_recovered_failure = fork.branches.iter().any(|branch| {
        !state.branches.contains_key(&branch.branch_id)
          && routes
            .get(&branch.branch_id)
            .is_some_and(|route| final_attempt_failure_in_nodes(projection, route).is_some())
      });
      let needs_join_settlement = fork
        .branches
        .iter()
        .all(|branch| state.branches.contains_key(&branch.branch_id));
      has_ready_work || has_recovered_failure || needs_join_settlement
    })
    .cloned()
}

pub(in crate::runtime) fn settle_open_fork_cancellation<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  fork: &crate::model::CompiledFork,
) -> Result<(), RuntimeExecutionError> {
  let mut projection = engine.projection(run_id)?;
  for branch in &fork.branches {
    if projection
      .forks
      .get(&fork.fork_id)
      .is_some_and(|state| state.branches.contains_key(&branch.branch_id))
    {
      continue;
    }
    engine.append_payload(
      run_id,
      RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
        fork_id: fork.fork_id.clone(),
        branch_id: branch.branch_id.clone(),
        terminal_node_id: branch.terminal_node_id.clone(),
        outcome: ForkBranchOutcome::Cancelled,
      }),
    )?;
    projection = engine.projection(run_id)?;
  }

  if projection
    .forks
    .get(&fork.fork_id)
    .is_some_and(|state| state.join_status == crate::ForkJoinStatus::Pending)
  {
    let join = if let Some(blocking_branch_id) = fork.joined_branch_ids.first() {
      ForkJoinSettledData {
        fork_id: fork.fork_id.clone(),
        outcome: ForkJoinOutcome::Cancelled,
        blocking_branch_id: Some(blocking_branch_id.clone()),
      }
    } else {
      ForkJoinSettledData {
        fork_id: fork.fork_id.clone(),
        outcome: ForkJoinOutcome::Succeeded,
        blocking_branch_id: None,
      }
    };
    engine.append_payload(run_id, RunEventPayload::ForkJoinSettled(join))?;
  }
  Ok(())
}

pub(in crate::runtime) enum ForkRuntimeProgress {
  Continued(Vec<String>),
  Waiting {
    completed: Vec<String>,
    outcome: WorkflowRuntimeOutcome,
  },
}

pub(in crate::runtime) async fn execute_fork<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  fork: crate::model::CompiledFork,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<ForkRuntimeProgress, RuntimeExecutionError> {
  let host_client = host;
  let routes = fork_branch_routes(engine.workflow(), &fork);
  let mut active: Vec<ActiveParallelInvocation<'_>> = Vec::new();
  let mut completion_order = Vec::new();
  let mut cancellation_sent = false;
  let mut suspended_outcome = None;

  loop {
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    let mut projection = engine.projection(run_id)?;
    if projection.timeout_reached_at.is_some() {
      let invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in invocation_ids {
        let _ = host_client.cancel_run(&invocation_id).await;
      }
      return Err(resumed_failure(engine, run_id, projection)?);
    }
    if projection.status == RunStatus::Cancelling && active.is_empty() {
      settle_open_fork_cancellation(engine, run_id, &fork)?;
      return Ok(ForkRuntimeProgress::Continued(completion_order));
    }
    if projection.status == RunStatus::Cancelling && !cancellation_sent {
      cancellation_sent = true;
      let invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in invocation_ids {
        let _ = host_client.cancel_run(&invocation_id).await;
      }
    }
    let ready = engine.ready_node_ids(run_id)?;

    for branch in &fork.branches {
      if ready.contains(&branch.terminal_node_id)
        && projection
          .forks
          .get(&fork.fork_id)
          .is_some_and(|state| !state.branches.contains_key(&branch.branch_id))
      {
        engine.append_payload(
          run_id,
          RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
            fork_id: fork.fork_id.clone(),
            branch_id: branch.branch_id.clone(),
            terminal_node_id: branch.terminal_node_id.clone(),
            outcome: ForkBranchOutcome::Succeeded,
          }),
        )?;
      }
    }

    projection = engine.projection(run_id)?;
    for branch in &fork.branches {
      let branch_is_settled = projection
        .forks
        .get(&fork.fork_id)
        .is_some_and(|state| state.branches.contains_key(&branch.branch_id));
      if branch_is_settled {
        continue;
      }
      let route = routes.get(&branch.branch_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "fork {:?} lost branch route {:?}",
          fork.fork_id, branch.branch_id
        ))
      })?;
      let Some(failure) = final_attempt_failure_in_nodes(&projection, route) else {
        continue;
      };
      let outcome = if failure.failure.kind == AttemptFailureKind::InvocationCancelled {
        ForkBranchOutcome::Cancelled
      } else {
        ForkBranchOutcome::Failed
      };
      engine.append_payload(
        run_id,
        RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
          fork_id: fork.fork_id.clone(),
          branch_id: branch.branch_id.clone(),
          terminal_node_id: branch.terminal_node_id.clone(),
          outcome,
        }),
      )?;
    }

    projection = engine.projection(run_id)?;
    let join_is_pending = projection
      .forks
      .get(&fork.fork_id)
      .is_some_and(|state| state.join_status == crate::ForkJoinStatus::Pending);
    let joined_blocker = fork.joined_branch_ids.iter().find_map(|branch_id| {
      projection
        .forks
        .get(&fork.fork_id)
        .and_then(|state| state.branches.get(branch_id))
        .and_then(|branch| branch.outcome)
        .filter(|outcome| *outcome != ForkBranchOutcome::Succeeded)
        .map(|outcome| (branch_id.clone(), outcome))
    });
    let joined_branches_succeeded = fork.joined_branch_ids.iter().all(|branch_id| {
      projection
        .forks
        .get(&fork.fork_id)
        .and_then(|state| state.branches.get(branch_id))
        .and_then(|branch| branch.outcome)
        == Some(ForkBranchOutcome::Succeeded)
    });
    if join_is_pending {
      let join = if let Some((branch_id, outcome)) = joined_blocker {
        Some(ForkJoinSettledData {
          fork_id: fork.fork_id.clone(),
          outcome: if outcome == ForkBranchOutcome::Cancelled {
            ForkJoinOutcome::Cancelled
          } else {
            ForkJoinOutcome::Failed
          },
          blocking_branch_id: Some(branch_id),
        })
      } else if joined_branches_succeeded {
        Some(ForkJoinSettledData {
          fork_id: fork.fork_id.clone(),
          outcome: ForkJoinOutcome::Succeeded,
          blocking_branch_id: None,
        })
      } else {
        None
      };
      if let Some(join) = join {
        engine.append_payload(run_id, RunEventPayload::ForkJoinSettled(join))?;
      }
    }

    projection = engine.projection(run_id)?;
    let every_branch_settled = fork.branches.iter().all(|branch| {
      projection
        .forks
        .get(&fork.fork_id)
        .is_some_and(|state| state.branches.contains_key(&branch.branch_id))
    });
    if every_branch_settled && active.is_empty() {
      if projection.status == RunStatus::Cancelling {
        return Ok(ForkRuntimeProgress::Continued(completion_order));
      }
      if let Some(failure) = final_workflow_attempt_failure(&projection) {
        let join_failed = projection
          .forks
          .get(&fork.fork_id)
          .is_some_and(|state| state.join_status != crate::ForkJoinStatus::Succeeded);
        let main_result_recorded =
          engine
            .workflow()
            .graph
            .settlement
            .as_ref()
            .is_some_and(|settlement| {
              projection
                .context
                .steps
                .contains_key(&settlement.main_result_node_id)
            });
        if join_failed || main_result_recorded {
          correct_saga_failure_for_node(engine, run_id, &failure.node_id, options).await?;
          engine.append_payload(
            run_id,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              engine.event_schema_version(),
              &failure,
            )),
          )?;
          return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
        }
      }
      return Ok(ForkRuntimeProgress::Continued(completion_order));
    }

    let ready = engine.ready_node_ids(run_id)?;
    let mut pure_node_completed = false;
    for node_id in &ready {
      if !routes.values().any(|route| route.contains(node_id)) {
        continue;
      }
      let handler = engine
        .workflow()
        .node(node_id)
        .map(|node| node.handler.as_str())
        .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
      match handler {
        "engine.lock-acquire" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.acquire_node_id == *node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Fork lock acquire node has no compiled descriptor.".to_string(),
              )
            })?;
          if !projection.locks.contains_key(&lock.lock_id) {
            let key = resolve_lock_key(&lock.key, &projection.context)?;
            let now = options.clock.now();
            let deadline = now
              + chrono::Duration::milliseconds(i64::try_from(lock.acquire_timeout_ms).map_err(
                |_| {
                  RuntimeExecutionError::InvalidConfiguration(
                    "Fork lock acquisition timeout exceeds the runtime clock.".to_string(),
                  )
                },
              )?);
            engine.register_lock_waiter(run_id, &lock.lock_id, &key, now, deadline)?;
          }
          let state = engine
            .projection(run_id)?
            .locks
            .get(&lock.lock_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Fork lock waiter disappeared from durable projection.".to_string(),
              )
            })?;
          match state.status {
            crate::LockStatus::Held => {
              options.ensure_lock_renewal(run_id, &lock.lock_id, lock.lease_ms);
              completion_order.push(lock.acquire_node_id);
              pure_node_completed = true;
              break;
            }
            crate::LockStatus::Waiting if options.clock.now() < state.deadline => {}
            crate::LockStatus::Waiting => {
              engine.timeout_lock_waiter(run_id, &lock.lock_id, options.clock.now())?;
              return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                "Fork lock {:?} was not acquired before its deadline.",
                lock.lock_id
              )));
            }
            _ => {
              return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                "Fork lock {:?} did not acquire safely.",
                lock.lock_id
              )));
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
            .find(|lock| lock.release_node_id == *node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Fork lock release node has no compiled descriptor.".to_string(),
              )
            })?;
          let held = projection
            .locks
            .get(&lock.lock_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled("Fork lock release has no lease.".to_string())
            })?;
          let acquired_at = held.acquired_at.ok_or_else(|| {
            RuntimeExecutionError::Stalled(
              "Fork lock release has no acquisition timestamp.".to_string(),
            )
          })?;
          let released_at = options.clock.now();
          let result = serde_json::json!({
            "status": "released", "key": held.key, "fencingToken": held.fencing_token,
            "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "waitedMs": (acquired_at - held.registered_at).num_milliseconds().max(0),
          });
          engine.release_lock(run_id, &lock.lock_id, result, released_at)?;
          options.stop_lock_renewal(run_id, &lock.lock_id);
          completion_order.push(lock.release_node_id);
          pure_node_completed = true;
          break;
        }
        "engine.approval-wait" if active.is_empty() => {
          let outcome = request_approval_wait(engine, run_id, node_id, options)?;
          return Ok(ForkRuntimeProgress::Waiting {
            completed: completion_order,
            outcome,
          });
        }
        "engine.durable-wait" => {
          match register_durable_wait_node(engine, run_id, node_id, options).await? {
            DurableWaitNodeProgress::Resolved => {
              completion_order.push(node_id.clone());
              pure_node_completed = true;
              break;
            }
            DurableWaitNodeProgress::Waiting(outcome) => {
              suspended_outcome.get_or_insert(outcome);
            }
          }
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
          pure_node_completed = true;
          break;
        }
        "engine.choice-select" => {
          let context = runtime_context_for_node(engine.workflow(), &projection, node_id);
          let (choice_id, arm_id) = match selected_choice_arm(engine.workflow(), node_id, &context)
          {
            Ok(selection) => selection,
            Err(error) => {
              return fail_choice(engine, run_id, error);
            }
          };
          engine.append_payload(
            run_id,
            RunEventPayload::ChoiceSelected(ChoiceSelectedData { choice_id, arm_id }),
          )?;
          pure_node_completed = true;
          break;
        }
        "engine.branch-select" => {
          let arm_id = selected_branch_arm(engine.workflow(), node_id, &projection.context)
            .map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "fork branch choice {:?} could not select an arm: {:?}",
                error.branch_id, error.kind
              ))
            })?;
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
          pure_node_completed = true;
          break;
        }
        "engine.branch-result" => {
          let arm_id = projection
            .branch_selections
            .get(node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "branch result {node_id:?} became ready without a recorded selection"
              ))
            })?;
          let node = engine
            .workflow()
            .node(node_id)
            .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "branch result {node_id:?} has invalid compiled inputs"
            )));
          };
          let expression = fields.get(&arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "branch result {node_id:?} has no value for selected arm {arm_id:?}"
            ))
          })?;
          let output =
            resolve_context_reference(expression, &projection.context).map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "branch result {node_id:?} cannot read {:?}",
                error.path
              ))
            })?;
          engine.publish_pure_result(run_id, node_id, output)?;
          completion_order.push(node_id.clone());
          pure_node_completed = true;
          break;
        }
        "engine.choice-result" => {
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
          let node = engine
            .workflow()
            .node(node_id)
            .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} has invalid compiled inputs"
            )));
          };
          let expression = fields.get(&arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} has no value for selected arm {arm_id:?}"
            ))
          })?;
          let output =
            resolve_context_reference(expression, &projection.context).map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "choice result {node_id:?} cannot read {:?}",
                error.path
              ))
            })?;
          engine.publish_pure_result(run_id, node_id, output)?;
          completion_order.push(node_id.clone());
          pure_node_completed = true;
          break;
        }
        _ => {}
      }
    }
    if pure_node_completed {
      continue;
    }

    if active.is_empty() {
      if let Some(mut group) = ready.iter().find_map(|node_id| {
        let group = engine.workflow().parallel_group_for_child(node_id)?;
        routes
          .values()
          .any(|route| {
            group
              .child_node_ids
              .iter()
              .all(|child| route.contains(child))
          })
          .then_some(group)
      }) {
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
        for wait_node_id in wait_node_ids {
          match register_durable_wait_node(engine, run_id, &wait_node_id, options).await? {
            DurableWaitNodeProgress::Resolved => completion_order.push(wait_node_id),
            DurableWaitNodeProgress::Waiting(outcome) => {
              suspended_outcome.get_or_insert(outcome);
            }
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
          let completed = execute_parallel_group(engine, run_id, group, options, host).await?;
          completion_order.extend(completed);
        }
        if let Some(outcome) = suspended_outcome.take() {
          return Ok(ForkRuntimeProgress::Waiting {
            completed: completion_order,
            outcome,
          });
        }
        continue;
      }
    }

    options.ensure_policy_execution_slot().await?;
    for node_id in &ready {
      if !engine
        .workflow()
        .node(node_id)
        .is_some_and(|node| node.handler == "runtime.script")
      {
        continue;
      }
      let branch_owner = engine
        .workflow()
        .fork_branch_owner(node_id)
        .map(|(owner_fork, branch)| (owner_fork.fork_id.clone(), branch.branch_id.clone()));
      match branch_owner.as_ref() {
        Some((fork_id, branch_id)) if fork_id == &fork.fork_id => {
          let route = routes.get(branch_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "fork {:?} lost branch route {:?}",
              fork.fork_id, branch_id
            ))
          })?;
          let branch_is_settled = projection
            .forks
            .get(&fork.fork_id)
            .is_some_and(|state| state.branches.contains_key(branch_id));
          let branch_is_active = active
            .iter()
            .any(|invocation| route.contains(&invocation.node_id));
          if branch_is_settled || branch_is_active {
            continue;
          }
        }
        Some(_) => continue,
        None => {
          let main_is_active = active.iter().any(|invocation| {
            engine
              .workflow()
              .fork_branch_owner(&invocation.node_id)
              .is_none()
          });
          if main_is_active {
            continue;
          }
        }
      }
      ensure_local_script_execution(engine.workflow().node(node_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("fork branch script {node_id:?} is unavailable"))
      })?)?;
      let source = engine
        .workflow()
        .node(node_id)
        .and_then(|node| node.script_source())
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("fork branch script {node_id:?} has no source"))
        })?
        .to_string();
      let secrets = resolved_script_secrets(engine.workflow(), node_id, options)?;
      let attempt_number = projection
        .pending_retries
        .get(node_id)
        .map_or(1, |retry| retry.next_attempt);
      let max_attempts = engine
        .workflow()
        .node(node_id)
        .and_then(|node| node.retry_policy.as_ref())
        .map_or(1, |policy| policy.max_attempts);
      let invocation_id = generated_id("inv");
      let idempotency_key = step_effect_idempotency_key(run_id, engine.definition_hash(), node_id);
      let context = runtime_context_for_node(engine.workflow(), &projection, node_id);
      let reusable = reusable_step_binding(engine.workflow(), node_id, &context, &secrets)?;
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node_id.clone(),
          attempt: attempt_number,
          invocation_id: invocation_id.clone(),
          handler: "runtime.script".to_string(),
          idempotency_key: Some(idempotency_key.clone()),
        }),
      )?;
      active.push(ActiveParallelInvocation {
        node_id: node_id.clone(),
        invocation_id: invocation_id.clone(),
        future: Box::pin(invoke_parallel_child(
          host_client,
          ParallelInvocationRequest {
            run_id: run_id.to_string(),
            node_id: node_id.clone(),
            invocation_id,
            source,
            context,
            timeout_ms: options.script_timeout_ms,
            max_context_bytes: options.max_context_bytes,
            attempt_number,
            max_attempts,
            idempotency_key,
            secrets,
            module_bindings: options
              .runtime_modules
              .iter()
              .filter(|module| !module.name.starts_with("__woml_"))
              .map(|module| RuntimeModuleBinding {
                name: module.name.clone(),
                bundle_digest: module.bundle_digest.clone(),
                exports: module.exports.clone(),
              })
              .collect(),
            reusable,
            artifact_store: options.managed_artifact_store.clone(),
          },
        )),
      });
    }

    if active.is_empty() {
      if let Some(outcome) = suspended_outcome.take() {
        return Ok(ForkRuntimeProgress::Waiting {
          completed: completion_order,
          outcome,
        });
      }
      let projection = engine.projection(run_id)?;
      if let Some(scheduled_at) = projection
        .pending_retries
        .values()
        .map(|retry| retry.scheduled_at)
        .min()
      {
        options.suspend_policy_execution_slot().await?;
        let now = chrono::Utc::now();
        if scheduled_at > now {
          let wait = (scheduled_at - now).to_std().map_err(|_| {
            RuntimeExecutionError::Stalled(
              "fork retry schedule exceeds the runtime clock".to_string(),
            )
          })?;
          tokio::time::sleep(wait.min(CANCELLATION_POLL_INTERVAL)).await;
        }
        continue;
      }
      let fork_waits_for_lock = projection.locks.values().any(|lock| {
        lock.status == crate::LockStatus::Waiting
          && engine
            .workflow()
            .durable_lock(&lock.lock_id)
            .is_some_and(|descriptor| {
              routes
                .values()
                .any(|route| route.contains(&descriptor.acquire_node_id))
            })
      });
      if fork_waits_for_lock {
        options.suspend_policy_execution_slot().await?;
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
        continue;
      }
    }

    let completion = loop {
      tokio::select! {
        completion = next_parallel_completion(&mut active) => break completion,
        _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent => {
          settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
          let current = engine.projection(run_id)?;
          if current.timeout_reached_at.is_some() {
            let invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in invocation_ids {
              let _ = host_client.cancel_run(&invocation_id).await;
            }
            return Err(resumed_failure(engine, run_id, current)?);
          }
          if current.status == RunStatus::Cancelling {
            cancellation_sent = true;
            let invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in invocation_ids {
              let _ = host_client.cancel_run(&invocation_id).await;
            }
          }
        }
      }
    };
    let Some(mut completion) = completion else {
      return Err(RuntimeExecutionError::Stalled(format!(
        "fork {:?} has unfinished owned work but no safe executable operation",
        fork.fork_id
      )));
    };
    if cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling {
      completion.outcome = Err(AttemptFailure {
        kind: AttemptFailureKind::InvocationCancelled,
        code: AttemptFailureKind::InvocationCancelled.code().to_string(),
        message: "The fork branch invocation was cancelled with its workflow run.".to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      });
    }
    match completion.outcome {
      Ok(output) => {
        completion_order.push(completion.node_id.clone());
        let completed_node_id = completion.node_id.clone();
        append_step_attempt_success(
          engine,
          run_id,
          StepAttemptSucceededData {
            node_id: completion.node_id,
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            output: output.clone(),
          },
        )?;
        drive_reusable_step_lifecycle(
          engine,
          run_id,
          &completed_node_id,
          ReusableInvocationOutcome::Succeeded,
          Some(output),
          None,
          options,
          host_client,
        )
        .await?;
      }
      Err(failure) => {
        let code = failure.code.clone();
        let lifecycle_error = ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        };
        let cancelled = failure.kind == AttemptFailureKind::InvocationCancelled;
        let disposition = engine.record_step_attempt_failure(
          run_id,
          options.clock.now(),
          StepAttemptFailedData {
            node_id: completion.node_id.clone(),
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            failure,
          },
        )?;
        report_attempt_failed(
          engine,
          options,
          run_id,
          &completion.node_id,
          completion.attempt_number,
          code,
        );
        match disposition {
          StepFailureDisposition::RetryScheduled {
            next_attempt,
            scheduled_at,
          } => report_retry_scheduled(
            engine,
            options,
            run_id,
            &completion.node_id,
            next_attempt,
            scheduled_at,
          ),
          StepFailureDisposition::StepFailed => {
            drive_reusable_step_lifecycle(
              engine,
              run_id,
              &completion.node_id,
              if cancelled {
                ReusableInvocationOutcome::Cancelled
              } else {
                ReusableInvocationOutcome::Failed
              },
              None,
              (!cancelled).then_some(lifecycle_error),
              options,
              host_client,
            )
            .await?;
          }
          StepFailureDisposition::RunFailureDeferred => {
            return Err(RuntimeExecutionError::Stalled(
              "fork-owned work deferred a top-level run failure".to_string(),
            ));
          }
          StepFailureDisposition::RunFailed => {
            return Err(RuntimeExecutionError::Stalled(
              "fork-coordinated work failed the run before all owned branches settled".to_string(),
            ));
          }
        }
      }
    }
  }
}
