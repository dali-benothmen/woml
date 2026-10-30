//! Bounded parallel invocation and wait-all/fail-fast sibling settlement.
//!
//! Keep boxed invocation futures, child identity, document-order failure
//! reporting, and actual completed outcomes when another sibling is cancelled.

use crate::engine::step_effect_idempotency_key;
use crate::event::ParallelFailure;
use crate::event::ParallelFailurePolicy;
use crate::event::ParallelGroupCompletedData;
use crate::event::ParallelGroupOutcome;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV3;
use crate::event::StepAttemptFailedData;
use crate::event::StepAttemptStartedData;
use crate::event::StepAttemptSucceededData;
use crate::model::ParallelGroupDefinition;
use crate::projection::AttemptStatus;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::protocol::ReusableScriptBindingV3;
use crate::protocol::RuntimeModuleBinding;
use crate::protocol::ScriptAttempt;
use crate::runtime::cleanup::ArtifactInvocationCleanup;
use crate::runtime::context::resolved_script_secrets;
use crate::runtime::context::reusable_step_binding;
use crate::runtime::context::runtime_context_for_node;
use crate::runtime::errors::FailedParallelDetails;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::progress::report_attempt_failed;
use crate::runtime::progress::report_attempt_succeeded;
use crate::runtime::progress::report_retry_scheduled;
use crate::runtime::script_execution::ensure_local_script_execution;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  append_step_attempt_success, correct_saga_failure_for_node, drive_reusable_step_lifecycle,
  generated_id, resumed_failure, settle_workflow_timeout_if_due, RuntimeDagEngine,
};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::RunEventPayload;
use crate::RunStatus;
use crate::ScriptHostClient;
use crate::StepFailureDisposition;
use crate::WorkflowContext;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::poll_fn;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

#[derive(Debug)]
pub(in crate::runtime) struct ParallelInvocationCompletion {
  pub(in crate::runtime) node_id: String,
  pub(in crate::runtime) invocation_id: String,
  pub(in crate::runtime) attempt_number: u32,
  pub(in crate::runtime) outcome: Result<Value, AttemptFailure>,
}

pub(in crate::runtime) type ParallelInvocationFuture<'a> =
  Pin<Box<dyn Future<Output = ParallelInvocationCompletion> + Send + 'a>>;

pub(in crate::runtime) struct ActiveParallelInvocation<'a> {
  pub(in crate::runtime) node_id: String,
  pub(in crate::runtime) invocation_id: String,
  pub(in crate::runtime) future: ParallelInvocationFuture<'a>,
}

pub(in crate::runtime) struct ParallelInvocationRequest {
  pub(in crate::runtime) run_id: String,
  pub(in crate::runtime) node_id: String,
  pub(in crate::runtime) invocation_id: String,
  pub(in crate::runtime) source: String,
  pub(in crate::runtime) context: WorkflowContext,
  pub(in crate::runtime) timeout_ms: u64,
  pub(in crate::runtime) max_context_bytes: Option<usize>,
  pub(in crate::runtime) attempt_number: u32,
  pub(in crate::runtime) max_attempts: u32,
  pub(in crate::runtime) idempotency_key: String,
  pub(in crate::runtime) secrets: BTreeMap<String, String>,
  pub(in crate::runtime) module_bindings: Vec<RuntimeModuleBinding>,
  pub(in crate::runtime) reusable: Option<ReusableScriptBindingV3>,
  pub(in crate::runtime) artifact_store: Option<Arc<crate::ManagedArtifactStore>>,
}

pub(in crate::runtime) async fn next_parallel_completion(
  active: &mut Vec<ActiveParallelInvocation<'_>>,
) -> Option<ParallelInvocationCompletion> {
  poll_fn(|context| {
    for index in 0..active.len() {
      if let Poll::Ready(completion) = active[index].future.as_mut().poll(context) {
        drop(active.swap_remove(index));
        return Poll::Ready(Some(completion));
      }
    }
    if active.is_empty() {
      Poll::Ready(None)
    } else {
      Poll::Pending
    }
  })
  .await
}

pub(in crate::runtime) async fn invoke_parallel_child(
  host: &ScriptHostClient,
  request: ParallelInvocationRequest,
) -> ParallelInvocationCompletion {
  let outcome = if let Some(limit) = request.max_context_bytes {
    match serde_json::to_vec(&request.context) {
      Ok(encoded) if encoded.len() > limit => Err(AttemptFailure {
        kind: AttemptFailureKind::ContextTooLarge,
        code: AttemptFailureKind::ContextTooLarge.code().to_string(),
        message: "Invocation context exceeds the configured byte limit.".to_string(),
        details: Some(crate::FailureSizeDetails {
          actual_bytes: Some(encoded.len() as u64),
          limit_bytes: Some(limit as u64),
        }),
        ..AttemptFailure::legacy_defaults()
      }),
      Err(error) => Err(AttemptFailure {
        kind: AttemptFailureKind::InvalidScriptResult,
        code: AttemptFailureKind::InvalidScriptResult.code().to_string(),
        message: format!("Invocation context could not be encoded: {error}"),
        details: None,
        ..AttemptFailure::legacy_defaults()
      }),
      _ => execute_parallel_request(host, &request).await,
    }
  } else {
    execute_parallel_request(host, &request).await
  };
  ParallelInvocationCompletion {
    node_id: request.node_id,
    invocation_id: request.invocation_id,
    attempt_number: request.attempt_number,
    outcome,
  }
}

async fn execute_parallel_request(
  host: &ScriptHostClient,
  invocation: &ParallelInvocationRequest,
) -> Result<Value, AttemptFailure> {
  let attempt = ScriptAttempt::new(
    invocation.attempt_number,
    invocation.max_attempts,
    &invocation.idempotency_key,
  )
  .map_err(|message| AttemptFailure {
    kind: AttemptFailureKind::InvalidScriptResult,
    code: AttemptFailureKind::InvalidScriptResult.code().to_string(),
    message,
    details: None,
    ..AttemptFailure::legacy_defaults()
  })?;
  let request = match invocation.reusable.as_ref() {
    Some(reusable) => ExecuteMessage::reusable_script_with_modules(
      &invocation.invocation_id,
      &invocation.run_id,
      &invocation.node_id,
      attempt,
      invocation.timeout_ms,
      &invocation.source,
      &invocation.context,
      &invocation.secrets,
      &invocation.module_bindings,
      reusable,
    ),
    None => ExecuteMessage::runtime_script_with_modules(
      &invocation.invocation_id,
      &invocation.run_id,
      &invocation.node_id,
      attempt,
      invocation.timeout_ms,
      &invocation.source,
      &invocation.context,
      &invocation.secrets,
      &invocation.module_bindings,
    ),
  };
  let _artifact_cleanup = ArtifactInvocationCleanup::from_store(
    invocation.artifact_store.clone(),
    &invocation.invocation_id,
  );
  match host.execute(&request).await {
    Ok(completed) => match completed.outcome {
      HostOutcome::Success { value } => Ok(value),
      HostOutcome::Failure { error } => Err(error.into_attempt_failure()),
    },
    Err(error) => Err(AttemptFailure {
      kind: AttemptFailureKind::HostCrashed,
      code: AttemptFailureKind::HostCrashed.code().to_string(),
      message: error.to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    }),
  }
}

pub(in crate::runtime) async fn execute_parallel_group<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  group: ParallelGroupDefinition,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<Vec<String>, RuntimeExecutionError> {
  let host_client = host;
  let policy = match group.on_error.as_str() {
    "fail-fast" => ParallelFailurePolicy::FailFast,
    "wait-all" => ParallelFailurePolicy::WaitAll,
    _ => {
      return Err(RuntimeExecutionError::Stalled(format!(
        "parallel group {:?} has an unknown failure policy",
        group.parallel_id
      )));
    }
  };
  let projection = engine.projection(run_id)?;
  let fork_context = projection
    .parallel_groups
    .get(&group.parallel_id)
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "parallel group {:?} has no durable fork event",
        group.parallel_id
      ))
    })?
    .fork_context
    .clone();
  let mut completion_order = Vec::new();
  let mut active: Vec<ActiveParallelInvocation<'_>> = Vec::new();
  let mut fail_fast_closed = false;
  let mut run_cancellation_sent = false;
  let mut workflow_timeout_sent = false;

  loop {
    let current = engine.projection(run_id)?;
    let cancelling = current.status == RunStatus::Cancelling;
    let timed_out = current.timeout_reached_at.is_some();
    if timed_out && !workflow_timeout_sent {
      workflow_timeout_sent = true;
      let active_invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in active_invocation_ids {
        let _ = host_client.cancel_run(&invocation_id).await;
      }
    }
    if cancelling && !run_cancellation_sent {
      run_cancellation_sent = true;
      let active_invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in active_invocation_ids {
        let _ = host_client.cancel_run(&invocation_id).await;
      }
    }
    while !fail_fast_closed && !cancelling && !timed_out && active.len() < group.concurrency {
      let projection = engine.projection(run_id)?;
      let now = chrono::Utc::now();
      let Some((node_id, attempt_number)) = group.child_node_ids.iter().find_map(|node_id| {
        if projection.context.steps.contains_key(node_id) {
          return None;
        }
        match projection.latest_attempt(node_id) {
          None => Some((node_id.clone(), 1)),
          Some(attempt) if matches!(attempt.status, AttemptStatus::Failed { .. }) => projection
            .pending_retries
            .get(node_id)
            .filter(|schedule| schedule.scheduled_at <= now)
            .map(|schedule| (node_id.clone(), schedule.next_attempt)),
          Some(_) => None,
        }
      }) else {
        break;
      };
      ensure_local_script_execution(engine.workflow().node(&node_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("parallel child {node_id:?} is unavailable"))
      })?)?;
      let source = engine
        .workflow()
        .node(&node_id)
        .and_then(|node| node.script_source())
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("parallel child {node_id:?} has no script source"))
        })?
        .to_string();
      let secrets = resolved_script_secrets(engine.workflow(), &node_id, options)?;
      let invocation_id = generated_id("inv");
      let max_attempts = engine
        .workflow()
        .node(&node_id)
        .and_then(|node| node.retry_policy.as_ref())
        .map_or(1, |policy| policy.max_attempts);
      let idempotency_key = step_effect_idempotency_key(run_id, engine.definition_hash(), &node_id);
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node_id.clone(),
          attempt: attempt_number,
          invocation_id: invocation_id.clone(),
          handler: "runtime.script".to_string(),
          idempotency_key: (engine.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6)
            .then(|| idempotency_key.clone()),
        }),
      )?;
      let invocation_context =
        if engine.workflow().schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
          runtime_context_for_node(engine.workflow(), &projection, &node_id)
        } else {
          fork_context.clone()
        };
      let reusable =
        reusable_step_binding(engine.workflow(), &node_id, &invocation_context, &secrets)?;
      active.push(ActiveParallelInvocation {
        node_id: node_id.clone(),
        invocation_id: invocation_id.clone(),
        future: Box::pin(invoke_parallel_child(
          host_client,
          ParallelInvocationRequest {
            run_id: run_id.to_string(),
            node_id,
            invocation_id,
            source,
            context: invocation_context,
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

    let completion = loop {
      tokio::select! {
        completion = next_parallel_completion(&mut active) => break completion,
        _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !run_cancellation_sent && !workflow_timeout_sent => {
          settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
          let current = engine.projection(run_id)?;
          if current.timeout_reached_at.is_some() {
            workflow_timeout_sent = true;
            let active_invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in active_invocation_ids {
              let _ = host_client.cancel_run(&invocation_id).await;
            }
          } else if current.status == RunStatus::Cancelling {
            run_cancellation_sent = true;
            let active_invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in active_invocation_ids {
              let _ = host_client.cancel_run(&invocation_id).await;
            }
          }
        }
      }
    };
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    if workflow_timeout_sent || engine.projection(run_id)?.timeout_reached_at.is_some() {
      drop(active);
      let projection = engine.projection(run_id)?;
      return Err(resumed_failure(engine, run_id, projection)?);
    }
    let Some(mut completion) = completion else {
      break;
    };
    if run_cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling {
      completion.outcome = Err(AttemptFailure {
        kind: AttemptFailureKind::InvocationCancelled,
        code: AttemptFailureKind::InvocationCancelled.code().to_string(),
        message: "The parallel step invocation was cancelled with its workflow run.".to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      });
    }
    match completion.outcome {
      Ok(output) => {
        completion_order.push(completion.node_id.clone());
        let completed_node_id = completion.node_id.clone();
        let completed_attempt = completion.attempt_number;
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
        report_attempt_succeeded(
          engine,
          options,
          run_id,
          &completed_node_id,
          completed_attempt,
        );
      }
      Err(failure) => {
        let cancellation = failure.kind == AttemptFailureKind::InvocationCancelled;
        let failure_code = failure.code.clone();
        let lifecycle_error = ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        };
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
          failure_code,
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
              if cancellation {
                ReusableInvocationOutcome::Cancelled
              } else {
                ReusableInvocationOutcome::Failed
              },
              None,
              (!cancellation).then_some(lifecycle_error),
              options,
              host_client,
            )
            .await?;
            if !cancellation && policy == ParallelFailurePolicy::FailFast {
              fail_fast_closed = true;
              let active_invocation_ids = active
                .iter()
                .map(|invocation| invocation.invocation_id.clone())
                .collect::<Vec<_>>();
              for invocation_id in active_invocation_ids {
                let _ = host_client.cancel(&invocation_id).await;
              }
            }
          }
          StepFailureDisposition::RunFailureDeferred => {
            return Err(RuntimeExecutionError::Stalled(
              "parallel-owned work deferred a top-level run failure".to_string(),
            ));
          }
          StepFailureDisposition::RunFailed => {
            return Err(RuntimeExecutionError::Stalled(
              "a parallel child failure terminated the run before its group settled".to_string(),
            ));
          }
        }
      }
    }
  }

  let projection = engine.projection(run_id)?;
  if projection.status == RunStatus::Cancelling {
    return Ok(completion_order);
  }
  let mut failed_node_ids = Vec::new();
  let mut cancelled_node_ids = Vec::new();
  let mut final_attempts = HashMap::new();
  let mut every_child_settled = true;
  let mut every_child_succeeded = true;
  for node_id in &group.child_node_ids {
    if projection.context.steps.contains_key(node_id) {
      continue;
    }
    every_child_succeeded = false;
    let Some(attempt) = projection.latest_attempt(node_id) else {
      every_child_settled = false;
      continue;
    };
    let AttemptStatus::Failed { failure } = &attempt.status else {
      every_child_settled = false;
      continue;
    };
    if projection.pending_retries.contains_key(node_id) {
      every_child_settled = false;
      continue;
    }
    final_attempts.insert(node_id.clone(), attempt.identity.attempt);
    if failure.kind == AttemptFailureKind::InvocationCancelled {
      cancelled_node_ids.push(node_id.clone());
    } else {
      failed_node_ids.push(node_id.clone());
    }
  }

  if every_child_succeeded {
    engine.append_payload(
      run_id,
      RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
        parallel_id: group.parallel_id,
        outcome: ParallelGroupOutcome::Succeeded,
        failed_node_ids: Vec::new(),
        cancelled_node_ids: Vec::new(),
      }),
    )?;
    return Ok(completion_order);
  }

  let group_has_final_failure = !failed_node_ids.is_empty();
  let group_must_fail = match policy {
    ParallelFailurePolicy::FailFast => group_has_final_failure,
    ParallelFailurePolicy::WaitAll => group_has_final_failure && every_child_settled,
  };
  if group_must_fail {
    let primary_node_id = engine
      .events(run_id)?
      .iter()
      .find_map(|event| match &event.payload {
        RunEventPayload::StepAttemptFailed(data)
          if failed_node_ids.contains(&data.node_id)
            && final_attempts.get(&data.node_id) == Some(&data.attempt) =>
        {
          Some(data.node_id.clone())
        }
        _ => None,
      })
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "parallel group {:?} has no durable primary final failure",
          group.parallel_id
        ))
      })?;
    let message = match policy {
      ParallelFailurePolicy::FailFast => {
        "A parallel child failed; active siblings were cancelled.".to_string()
      }
      ParallelFailurePolicy::WaitAll if failed_node_ids.len() == 1 => {
        "One parallel child failed.".to_string()
      }
      ParallelFailurePolicy::WaitAll => {
        format!("{} parallel children failed.", failed_node_ids.len())
      }
    };
    let failure = ParallelFailure {
      kind: "parallel_child_failed".to_string(),
      code: "WOML_PARALLEL_CHILD_FAILED".to_string(),
      message: message.clone(),
    };
    engine.append_payload(
      run_id,
      RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
        parallel_id: group.parallel_id.clone(),
        outcome: ParallelGroupOutcome::Failed,
        failed_node_ids: failed_node_ids.clone(),
        cancelled_node_ids: cancelled_node_ids.clone(),
      }),
    )?;
    correct_saga_failure_for_node(engine, run_id, &primary_node_id, options).await?;
    engine.append_payload(
      run_id,
      RunEventPayload::RunFailed(RunFailedData::V3(RunFailedDataV3::Parallel {
        parallel_id: group.parallel_id.clone(),
        policy,
        primary_node_id: primary_node_id.clone(),
        failed_node_ids: failed_node_ids.clone(),
        cancelled_node_ids: cancelled_node_ids.clone(),
        failure: failure.clone(),
      })),
    )?;
    return Err(RuntimeExecutionError::ParallelFailed(Box::new(
      FailedParallelDetails {
        code: failure.code.clone(),
        message,
        parallel_id: group.parallel_id,
        policy,
        primary_node_id,
        failed_node_ids,
        cancelled_node_ids,
        failure,
        events: engine.events(run_id)?,
      },
    )));
  }

  if projection.pending_retries.is_empty() {
    return Err(RuntimeExecutionError::Stalled(format!(
      "parallel group {:?} has unfinished children without a retry schedule",
      group.parallel_id
    )));
  }
  Ok(completion_order)
}
