//! Iteration script supervision with exact scoped invocation/attempt identity.
//!
//! Placement, secrets, interruption, host ownership, and lifecycle dispatch use
//! the existing runtime boundaries; remote placement must never fall back here.

use crate::event::CompensationRegisteredData;
use crate::event::ForEachIterationScope;
use crate::event::StepAttemptFailedData;
use crate::event::StepAttemptStartedData;
use crate::event::StepAttemptSucceededData;
use crate::event::StepRetryScheduledData;
use crate::model::LifecycleEventName;
use crate::projection::AttemptStatus;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::protocol::ScriptAttempt;
use crate::runtime::cleanup::ArtifactInvocationCleanup;
use crate::runtime::context::lock_descriptor_for_body_node;
use crate::runtime::context::reusable_step_binding_for_iteration;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::script_execution::ensure_local_script_execution;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  compensation_registration_for_iteration, drive_for_each_reusable_lifecycle,
  drive_for_each_step_lifecycle, for_each_effect_key, for_each_module_bindings, generated_id,
  RuntimeDagEngine,
};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::RunEventPayload;
use crate::RunStatus;
use crate::ScriptHostClient;
use crate::WorkflowContext;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
async fn execute_for_each_script<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  node: &crate::model::CompiledWorkflowNode,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) -> Result<Value, AttemptFailure> {
  if let Err(error) = ensure_local_script_execution(node) {
    return Err(AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_REMOTE_DISPATCH_UNAVAILABLE".to_string(),
      message: error.to_string(),
      retryable: Some(false),
      ambiguous: Some(false),
      details: None,
      ..AttemptFailure::legacy_defaults()
    });
  }
  let workflow = engine.workflow().clone();
  let definition_hash = engine.definition_hash().to_string();
  let source = node.script_source().ok_or_else(|| AttemptFailure {
    kind: AttemptFailureKind::InvalidScriptResult,
    code: "WOML_FOR_EACH_NODE_INVALID".to_string(),
    message: format!("For-each body node {:?} has no script source.", node.id),
    details: None,
    ..AttemptFailure::legacy_defaults()
  })?;
  let secrets = node
    .script_runtime
    .as_ref()
    .map(|runtime| {
      runtime
        .required_secrets
        .iter()
        .map(|name| {
          options
            .resolved_secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .map(|value| (name.clone(), value))
            .ok_or_else(|| AttemptFailure {
              kind: AttemptFailureKind::ScriptThrew,
              code: "WOML_SECRET_NOT_FOUND".to_string(),
              message: format!("Required secret {name:?} is unavailable."),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
    })
    .transpose()?
    .unwrap_or_default();
  let modules = for_each_module_bindings(options);
  let max_attempts = node
    .retry_policy
    .as_ref()
    .map_or(1, |policy| policy.max_attempts);
  let idempotency_key = for_each_effect_key(
    run_id,
    &definition_hash,
    &descriptor.for_each_id,
    index,
    &node.id,
  );
  let execution = engine
    .projection(run_id)
    .map_err(|error| AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_FOR_EACH_RECOVERY_INVALID".to_string(),
      message: error.to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?
    .for_each
    .get(&descriptor.for_each_id)
    .and_then(|loop_state| loop_state.executions.get(&index))
    .cloned()
    .ok_or_else(|| AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_FOR_EACH_RECOVERY_INVALID".to_string(),
      message: "The active iteration has no durable execution projection.".to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?;
  let latest_attempt = execution
    .attempts
    .iter()
    .rev()
    .find(|attempt| attempt.identity.node_id == node.id);
  let start_attempt = match latest_attempt {
    None => 1,
    Some(attempt) => match &attempt.status {
      AttemptStatus::Succeeded { output } => return Ok(output.clone()),
      AttemptStatus::Started => {
        return Err(AttemptFailure {
          kind: AttemptFailureKind::Interrupted,
          code: AttemptFailureKind::Interrupted.code().to_string(),
          message: "Recovery found an active iteration attempt without a durable outcome."
            .to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        });
      }
      AttemptStatus::Failed { failure } => {
        let Some(retry) = execution.pending_retries.get(&node.id) else {
          return Err(failure.clone());
        };
        while retry.scheduled_at > options.clock.now() {
          let wait = (retry.scheduled_at - options.clock.now())
            .to_std()
            .unwrap_or(CANCELLATION_POLL_INTERVAL)
            .min(CANCELLATION_POLL_INTERVAL);
          tokio::time::sleep(wait).await;
          let projection = engine.projection(run_id).map_err(|error| AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: "WOML_FOR_EACH_RECOVERY_INVALID".to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          })?;
          if projection.status == RunStatus::Cancelling || projection.timeout_reached_at.is_some() {
            return Err(AttemptFailure {
              kind: AttemptFailureKind::InvocationCancelled,
              code: AttemptFailureKind::InvocationCancelled.code().to_string(),
              message: "The pending iteration retry was cancelled with its workflow run."
                .to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            });
          }
        }
        retry.next_attempt
      }
    },
  };
  for attempt_number in start_attempt..=max_attempts {
    let invocation_id = generated_id("inv");
    let attempt =
      ScriptAttempt::new(attempt_number, max_attempts, &idempotency_key).map_err(|message| {
        AttemptFailure {
          kind: AttemptFailureKind::InvalidScriptResult,
          code: "WOML_FOR_EACH_ATTEMPT_INVALID".to_string(),
          message,
          details: None,
          ..AttemptFailure::legacy_defaults()
        }
      })?;
    let reusable = reusable_step_binding_for_iteration(
      &workflow, &node.id, context, item, index, total, &secrets,
    )
    .map_err(|error| AttemptFailure {
      kind: AttemptFailureKind::ScriptThrew,
      code: "WOML_REUSABLE_BINDING_INVALID".to_string(),
      message: error.to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?;
    let scope = ForEachIterationScope {
      for_each_id: descriptor.for_each_id.clone(),
      index,
    };
    engine
      .append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node.id.clone(),
          attempt: attempt_number,
          invocation_id: invocation_id.clone(),
          handler: node.handler.clone(),
          idempotency_key: Some(idempotency_key.clone()),
        }),
      )
      .map_err(|error| AttemptFailure {
        kind: AttemptFailureKind::Interrupted,
        code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
        message: error.to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      })?;
    let request = match reusable.as_ref() {
      Some(reusable) => ExecuteMessage::reusable_script_with_iteration_modules(
        &invocation_id,
        run_id,
        &node.id,
        attempt,
        options.script_timeout_ms,
        source,
        context,
        item,
        index,
        total,
        &secrets,
        &modules,
        reusable,
      ),
      None => ExecuteMessage::runtime_script_with_iteration_modules(
        &invocation_id,
        run_id,
        &node.id,
        attempt,
        options.script_timeout_ms,
        source,
        context,
        item,
        index,
        total,
        &secrets,
        &modules,
      ),
    }
    .map_err(|message| AttemptFailure {
      kind: AttemptFailureKind::InvalidScriptResult,
      code: "WOML_FOR_EACH_CONTEXT_INVALID".to_string(),
      message,
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?;
    if let Some(limit) = options.max_context_bytes {
      let actual = serde_json::to_vec(&request.context)
        .map_err(|error| AttemptFailure {
          kind: AttemptFailureKind::InvalidScriptResult,
          code: "WOML_FOR_EACH_CONTEXT_INVALID".to_string(),
          message: error.to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        })?
        .len();
      if actual > limit {
        let failure = AttemptFailure {
          kind: AttemptFailureKind::ContextTooLarge,
          code: AttemptFailureKind::ContextTooLarge.code().to_string(),
          message: "Invocation context exceeds the configured byte limit.".to_string(),
          details: Some(crate::FailureSizeDetails {
            actual_bytes: Some(actual as u64),
            limit_bytes: Some(limit as u64),
          }),
          ..AttemptFailure::legacy_defaults()
        };
        engine
          .append_scoped_payload(
            run_id,
            scope,
            RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
              node_id: node.id.clone(),
              attempt: attempt_number,
              invocation_id,
              failure: failure.clone(),
            }),
          )
          .map_err(|error| AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          })?;
        return Err(failure);
      }
    }
    active_invocations
      .lock()
      .await
      .insert(invocation_id.clone());
    let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
    let execution = host.execute(&request).await;
    active_invocations.lock().await.remove(&invocation_id);
    match execution {
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { value } => {
          let success = RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
            node_id: node.id.clone(),
            attempt: attempt_number,
            invocation_id,
            output: value.clone(),
          });
          let completion_sequence = engine
            .events(run_id)
            .map_err(|error| AttemptFailure {
              kind: AttemptFailureKind::Interrupted,
              code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
              message: error.to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })?
            .len() as u64
            + 1;
          let mut payloads = vec![success];
          if let Some((saga_id, action_id)) =
            compensation_registration_for_iteration(engine.workflow(), descriptor, &node.id, index)
          {
            payloads.push(RunEventPayload::CompensationRegistered(
              CompensationRegisteredData {
                saga_id,
                action_id,
                for_step_id: node.id.clone(),
                completion_sequence,
              },
            ));
          }
          engine
            .append_scoped_payloads(run_id, scope, payloads)
            .map_err(|error| AttemptFailure {
              kind: AttemptFailureKind::Interrupted,
              code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
              message: error.to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })?;
          return Ok(value);
        }
        HostOutcome::Failure { error } => {
          let failure = error.into_attempt_failure();
          engine
            .append_scoped_payload(
              run_id,
              scope.clone(),
              RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
                node_id: node.id.clone(),
                attempt: attempt_number,
                invocation_id,
                failure: failure.clone(),
              }),
            )
            .map_err(|error| AttemptFailure {
              kind: AttemptFailureKind::Interrupted,
              code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
              message: error.to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })?;
          if failure.kind == AttemptFailureKind::ScriptThrew && attempt_number < max_attempts {
            let delay = node
              .retry_policy
              .as_ref()
              .and_then(|policy| policy.delay_before_attempt(attempt_number + 1))
              .unwrap_or(0);
            engine
              .append_scoped_payload(
                run_id,
                scope,
                RunEventPayload::StepRetryScheduled(StepRetryScheduledData {
                  node_id: node.id.clone(),
                  failed_attempt: attempt_number,
                  next_attempt: attempt_number + 1,
                  scheduled_at: options.clock.now()
                    + chrono::Duration::milliseconds(i64::try_from(delay).unwrap_or(i64::MAX)),
                }),
              )
              .map_err(|error| AttemptFailure {
                kind: AttemptFailureKind::Interrupted,
                code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
                message: error.to_string(),
                details: None,
                ..AttemptFailure::legacy_defaults()
              })?;
            if delay > 0 {
              tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            continue;
          }
          return Err(failure);
        }
      },
      Err(error) => {
        let failure = AttemptFailure {
          kind: AttemptFailureKind::HostCrashed,
          code: AttemptFailureKind::HostCrashed.code().to_string(),
          message: error.to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        };
        engine
          .append_scoped_payload(
            run_id,
            scope,
            RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
              node_id: node.id.clone(),
              attempt: attempt_number,
              invocation_id,
              failure: failure.clone(),
            }),
          )
          .map_err(|error| AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          })?;
        return Err(failure);
      }
    }
  }
  unreachable!("a for-each script attempt always returns an outcome")
}

#[allow(clippy::too_many_arguments)]
pub(in crate::runtime) async fn execute_for_each_iteration_script<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) -> Result<Value, RuntimeExecutionError> {
  let node = descriptor
    .body
    .nodes
    .iter()
    .find(|node| node.id == node_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each body node disappeared".to_string()))?;
  let visible_local = descriptor
    .body
    .context_visibility
    .iter()
    .find(|visibility| visibility.node_id == node_id)
    .map(|visibility| visibility.step_ids.as_slice())
    .unwrap_or_default();
  let mut invocation_context = context.clone();
  invocation_context.steps.retain(|step_id, _| {
    descriptor.outer_step_ids.contains(step_id) || visible_local.contains(step_id)
  });
  if let Some(lock) = lock_descriptor_for_body_node(engine.workflow(), node_id) {
    if let Some(held) = engine
      .projection(run_id)?
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|loop_| loop_.executions.get(&index))
      .and_then(|execution| execution.locks.get(&lock.lock_id))
      .filter(|lock| lock.status == crate::LockStatus::Held)
    {
      invocation_context.lock = Some(crate::LockContext {
        id: held.lock_id.clone(),
        key: held.key.clone(),
        fencing_token: held.fencing_token.clone().unwrap_or_default(),
        acquired_at: held
          .acquired_at
          .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
          .unwrap_or_default(),
      });
    }
  }
  drive_for_each_step_lifecycle(
    engine,
    run_id,
    descriptor,
    node_id,
    LifecycleEventName::StepStart,
    &invocation_context,
    item,
    index,
    total,
    0,
    None,
    options,
    host,
  )
  .await?;
  match execute_for_each_script(
    engine,
    run_id,
    descriptor,
    node,
    &invocation_context,
    item,
    index,
    total,
    options,
    host,
    active_invocations,
  )
  .await
  {
    Ok(output) => {
      let mut completion_context = context.clone();
      completion_context
        .steps
        .insert(node_id.to_string(), output.clone());
      let attempts = engine
        .projection(run_id)?
        .for_each
        .get(&descriptor.for_each_id)
        .and_then(|loop_| loop_.executions.get(&index))
        .map_or(1, |execution| {
          execution
            .attempts
            .iter()
            .filter(|attempt| attempt.identity.node_id == node_id)
            .count() as u32
        });
      drive_for_each_reusable_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        &completion_context,
        item,
        index,
        total,
        ReusableInvocationOutcome::Succeeded,
        Some(output.clone()),
        None,
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepSuccess,
        &completion_context,
        item,
        index,
        total,
        attempts,
        None,
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepComplete,
        &completion_context,
        item,
        index,
        total,
        attempts,
        None,
        options,
        host,
      )
      .await?;
      Ok(output)
    }
    Err(failure) => {
      let attempts = engine
        .projection(run_id)?
        .for_each
        .get(&descriptor.for_each_id)
        .and_then(|loop_| loop_.executions.get(&index))
        .map_or(1, |execution| {
          execution
            .attempts
            .iter()
            .filter(|attempt| attempt.identity.node_id == node_id)
            .count() as u32
        });
      drive_for_each_reusable_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        context,
        item,
        index,
        total,
        ReusableInvocationOutcome::Failed,
        None,
        Some(ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        }),
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepFailure,
        context,
        item,
        index,
        total,
        attempts,
        Some(&failure),
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepComplete,
        context,
        item,
        index,
        total,
        attempts,
        Some(&failure),
        options,
        host,
      )
      .await?;
      Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "for-each {:?} iteration {index} node {node_id:?} failed [{}]: {}",
        descriptor.for_each_id, failure.code, failure.message
      )))
    }
  }
}
