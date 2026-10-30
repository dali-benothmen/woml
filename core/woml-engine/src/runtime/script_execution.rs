//! Ordinary script-attempt invocation and failure settlement.
//!
//! Preserve placement checks, exact attempt identity, boxed host execution,
//! interruption deadlines, and cleanup/drop order. Scheduling, correction, and
//! engine adapters are called through the parent and keep their existing owners.

use super::{
  append_step_attempt_success_payload, attempt_run_failed_data, drive_reusable_step_lifecycle,
  generated_id, lock_descriptor_for_body_node, report_attempt_failed, report_attempt_succeeded,
  report_retry_scheduled, resolved_script_secrets, resumed_failure, retry_max_attempts,
  reusable_step_binding, reusable_step_descriptor, runtime_context_for_node,
  settle_workflow_timeout_if_due, ArtifactInvocationCleanup, FailedRunDetails, RuntimeDagEngine,
  RuntimeExecutionError, RuntimeExecutionOptions,
};
use crate::engine::step_effect_idempotency_key;
use crate::event::RunSucceededData;
use crate::event::StepAttemptFailedData;
use crate::event::StepAttemptStartedData;
use crate::event::StepAttemptSucceededData;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::protocol::RuntimeModuleBinding;
use crate::protocol::ScriptAttempt;
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::RunEventPayload;
use crate::RunStatus;
use crate::ScriptHostClient;
use crate::ScriptHostClientError;
use crate::StepFailureDisposition;
use serde_json::Value;
use std::time::Duration;

pub(super) fn ensure_local_script_execution(
  node: &crate::model::CompiledWorkflowNode,
) -> Result<(), RuntimeExecutionError> {
  let Some(execution) = node.execution.as_ref() else {
    return Ok(());
  };
  if execution.target == "local" {
    return Ok(());
  }
  Err(RuntimeExecutionError::Stalled(format!(
    "remote target {:?} for node {:?} requires an activated remote dispatch coordinator",
    execution.target, node.id
  )))
}

pub(super) async fn execute_script_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  source: &str,
  publish_terminal_success: bool,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<ScriptNodeOutcome, RuntimeExecutionError> {
  let node = engine.workflow().node(node_id).ok_or_else(|| {
    RuntimeExecutionError::Stalled(format!("script node {node_id:?} is unavailable"))
  })?;
  ensure_local_script_execution(node)?;
  let secrets = resolved_script_secrets(engine.workflow(), node_id, options)?;
  let invocation_id = generated_id("inv");
  let projection = engine.projection(run_id)?;
  let attempt_number = match projection.latest_attempt(node_id) {
    None => 1,
    Some(_) => projection
      .pending_retries
      .get(node_id)
      .map(|retry| retry.next_attempt)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "node {node_id:?} has prior attempts but no pending retry"
        ))
      })?,
  };
  let max_attempts = engine
    .workflow()
    .node(node_id)
    .and_then(|node| node.retry_policy.as_ref())
    .map_or(1, |policy| policy.max_attempts);
  let idempotency_key = step_effect_idempotency_key(run_id, engine.definition_hash(), node_id);
  engine.append_payload(
    run_id,
    RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
      node_id: node_id.to_string(),
      attempt: attempt_number,
      invocation_id: invocation_id.clone(),
      handler: "runtime.script".to_string(),
      idempotency_key: (engine.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6)
        .then(|| idempotency_key.clone()),
    }),
  )?;

  let projection = engine.projection(run_id)?;
  let context = runtime_context_for_node(engine.workflow(), &projection, node_id);
  if let Some(limit) = options.max_context_bytes {
    let actual = serde_json::to_vec(&context)
      .map_err(|error| RuntimeExecutionError::Stalled(error.to_string()))?
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
      return settle_script_attempt_failure(
        engine,
        options,
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        failure,
      )
      .await;
    }
  }

  if host.is_none() {
    match options.acquire_script_host().await {
      Ok(client) => *host = Some(client),
      Err(error) => {
        return settle_script_attempt_failure(
          engine,
          options,
          host,
          run_id,
          node_id,
          attempt_number,
          &invocation_id,
          AttemptFailure {
            kind: AttemptFailureKind::HostCrashed,
            code: AttemptFailureKind::HostCrashed.code().to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        )
        .await;
      }
    }
  }

  let module_bindings = options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect::<Vec<_>>();
  let attempt = ScriptAttempt::new(attempt_number, max_attempts, &idempotency_key)
    .map_err(RuntimeExecutionError::Stalled)?;
  let reusable = reusable_step_binding(engine.workflow(), node_id, &context, &secrets)?;
  let request = match reusable.as_ref() {
    Some(reusable) => ExecuteMessage::reusable_script_with_modules(
      &invocation_id,
      run_id,
      node_id,
      attempt,
      options.script_timeout_ms,
      source,
      &context,
      &secrets,
      &module_bindings,
      reusable,
    ),
    None => ExecuteMessage::runtime_script_with_modules(
      &invocation_id,
      run_id,
      node_id,
      attempt,
      options.script_timeout_ms,
      source,
      &context,
      &secrets,
      &module_bindings,
    ),
  };
  let host_client = host.as_ref().expect("script host was initialized");
  let artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
  let mut execution = Box::pin(host_client.execute(&request));
  let mut cancellation_sent = false;
  let mut timeout_sent = false;
  let mut lease_loss_sent = false;
  let mut retire_interrupted_host = false;
  let completed = loop {
    tokio::select! {
      result = &mut execution => break result,
      _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent && !timeout_sent && !lease_loss_sent => {
        settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
        let current = engine.projection(run_id)?;
        let protected_lease_lost = lock_descriptor_for_body_node(engine.workflow(), node_id)
          .and_then(|lock| current.locks.get(&lock.lock_id))
          .is_some_and(|lock| lock.status == crate::LockStatus::LeaseLost);
        if protected_lease_lost {
          lease_loss_sent = true;
          if let Err(error) = interrupt_script_invocation(host_client, &invocation_id).await {
            retire_interrupted_host = true;
            break Err(error);
          }
        } else if current.timeout_reached_at.is_some() {
          timeout_sent = true;
          if let Err(error) = interrupt_script_invocation(host_client, &invocation_id).await {
            retire_interrupted_host = true;
            break Err(error);
          }
        } else if current.status == RunStatus::Cancelling {
          cancellation_sent = true;
          if let Err(error) = interrupt_script_invocation(host_client, &invocation_id).await {
            retire_interrupted_host = true;
            break Err(error);
          }
        }
      }
      _ = tokio::time::sleep(SCRIPT_HOST_INTERRUPT_GRACE), if cancellation_sent || timeout_sent || lease_loss_sent => {
        retire_interrupted_host = true;
        break Err(ScriptHostClientError::HostCrashed(
          "the script host did not acknowledge invocation interruption before its deadline"
            .to_string(),
        ));
      }
    }
  };
  drop(execution);
  drop(artifact_cleanup);
  if retire_interrupted_host {
    // A host that cannot settle an interrupted Worker cannot safely execute
    // correction or later workflow work. Dropping it terminates the child
    // process (`kill_on_drop`) and the next action acquires a fresh host.
    *host = None;
  }
  settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
  if timeout_sent || engine.projection(run_id)?.timeout_reached_at.is_some() {
    let projection = engine.projection(run_id)?;
    return Err(resumed_failure(engine, run_id, projection)?);
  }
  if lease_loss_sent {
    let failure = AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_LOCK_LEASE_LOST".to_string(),
      message: "The protected step was stopped because its durable lock lease expired.".to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    };
    return settle_script_attempt_failure(
      engine,
      options,
      host,
      run_id,
      node_id,
      attempt_number,
      &invocation_id,
      failure,
    )
    .await;
  }
  let outcome = match completed {
    _ if cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling => {
      return settle_script_attempt_failure(
        engine,
        options,
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        AttemptFailure {
          kind: AttemptFailureKind::InvocationCancelled,
          code: AttemptFailureKind::InvocationCancelled.code().to_string(),
          message: "The step invocation was cancelled with its workflow run.".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      )
      .await;
    }
    Ok(completed) => completed.outcome,
    Err(error) => {
      let failure = AttemptFailure {
        kind: AttemptFailureKind::HostCrashed,
        code: AttemptFailureKind::HostCrashed.code().to_string(),
        message: error.to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      };
      return settle_script_attempt_failure(
        engine,
        options,
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        failure,
      )
      .await;
    }
  };

  match outcome {
    HostOutcome::Success { value } => {
      let succeeded_data = StepAttemptSucceededData {
        node_id: node_id.to_string(),
        attempt: attempt_number,
        invocation_id,
        output: value.clone(),
      };
      let succeeded = RunEventPayload::StepAttemptSucceeded(succeeded_data);
      if publish_terminal_success {
        engine.append_payloads(
          run_id,
          vec![
            succeeded,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: node_id.to_string(),
              result: value.clone(),
            }),
          ],
        )?;
        report_attempt_succeeded(engine, options, run_id, node_id, attempt_number);
        return Ok(ScriptNodeOutcome::WorkflowSucceeded);
      }
      append_step_attempt_success_payload(engine, run_id, succeeded)?;
      drive_reusable_step_lifecycle(
        engine,
        run_id,
        node_id,
        ReusableInvocationOutcome::Succeeded,
        Some(value.clone()),
        None,
        options,
        host.as_ref().expect("script host was initialized"),
      )
      .await?;
      report_attempt_succeeded(engine, options, run_id, node_id, attempt_number);
      Ok(ScriptNodeOutcome::Succeeded(value))
    }
    HostOutcome::Failure { error } => {
      settle_script_attempt_failure(
        engine,
        options,
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        error.into_attempt_failure(),
      )
      .await
    }
  }
}

pub(super) enum ScriptNodeOutcome {
  Succeeded(Value),
  WorkflowSucceeded,
  RetryScheduled,
  Cancelled,
  SagaFailed(Box<StepAttemptFailedData>),
}

pub(super) const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

const SCRIPT_HOST_INTERRUPT_GRACE: Duration = Duration::from_secs(2);

async fn interrupt_script_invocation(
  host: &ScriptHostClient,
  invocation_id: &str,
) -> Result<(), ScriptHostClientError> {
  tokio::time::timeout(SCRIPT_HOST_INTERRUPT_GRACE, host.cancel_run(invocation_id))
    .await
    .map_err(|_| {
      ScriptHostClientError::HostCrashed(
        "the script host did not accept invocation interruption before its deadline".to_string(),
      )
    })?
}

async fn settle_script_attempt_failure<E: RuntimeDagEngine>(
  engine: &mut E,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
  run_id: &str,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
  failure: AttemptFailure,
) -> Result<ScriptNodeOutcome, RuntimeExecutionError> {
  let failure_event = StepAttemptFailedData {
    node_id: node_id.to_string(),
    attempt,
    invocation_id: invocation_id.to_string(),
    failure: failure.clone(),
  };
  let disposition =
    engine.record_step_attempt_failure(run_id, options.clock.now(), failure_event.clone())?;
  report_attempt_failed(
    engine,
    options,
    run_id,
    node_id,
    attempt,
    failure.code.clone(),
  );
  let saga_failure_deferred = matches!(&disposition, StepFailureDisposition::RunFailureDeferred)
    && engine.workflow().saga_for_body_node(node_id).is_some();
  match disposition {
    StepFailureDisposition::RetryScheduled {
      next_attempt,
      scheduled_at,
    } => {
      report_retry_scheduled(engine, options, run_id, node_id, next_attempt, scheduled_at);
      return Ok(ScriptNodeOutcome::RetryScheduled);
    }
    StepFailureDisposition::StepFailed => {
      if failure.kind == AttemptFailureKind::InvocationCancelled
        && engine.projection(run_id)?.status == RunStatus::Cancelling
      {
        return Ok(ScriptNodeOutcome::Cancelled);
      }
      return Err(RuntimeExecutionError::Stalled(format!(
        "non-parallel node {node_id:?} returned a group-owned failure"
      )));
    }
    StepFailureDisposition::RunFailureDeferred => {}
    StepFailureDisposition::RunFailed => {}
  }
  if saga_failure_deferred {
    return Ok(ScriptNodeOutcome::SagaFailed(Box::new(failure_event)));
  }
  if reusable_step_descriptor(engine.workflow(), node_id).is_some() && host.is_none() {
    *host = Some(options.acquire_script_host().await?);
  }
  if let Some(host) = host.as_ref() {
    drive_reusable_step_lifecycle(
      engine,
      run_id,
      node_id,
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
  }
  release_held_lock_after_body_failure(engine, run_id, node_id, options.clock.now(), options)?;
  if engine.projection(run_id)?.business_outcome.is_none() {
    engine.append_payload(
      run_id,
      RunEventPayload::RunFailed(attempt_run_failed_data(
        engine.event_schema_version(),
        &StepAttemptFailedData {
          node_id: node_id.to_string(),
          attempt,
          invocation_id: invocation_id.to_string(),
          failure: failure.clone(),
        },
      )),
    )?;
  }
  Err(RuntimeExecutionError::RunFailed(Box::new(
    FailedRunDetails {
      code: if retry_max_attempts(engine, node_id) > 1
        && attempt >= retry_max_attempts(engine, node_id)
        && failure.kind == AttemptFailureKind::ScriptThrew
      {
        "WOML_STEP_RETRIES_EXHAUSTED".to_string()
      } else {
        failure.code.clone()
      },
      message: if retry_max_attempts(engine, node_id) > 1
        && attempt >= retry_max_attempts(engine, node_id)
        && failure.kind == AttemptFailureKind::ScriptThrew
      {
        format!(
          "attempt {} of {} failed [{}].",
          attempt,
          retry_max_attempts(engine, node_id),
          failure.code
        )
      } else {
        failure.message.clone()
      },
      node_id: Some(node_id.to_string()),
      attempt: Some(attempt),
      max_attempts: Some(retry_max_attempts(engine, node_id)),
      failure,
      events: engine.events(run_id)?,
    },
  )))
}

fn release_held_lock_after_body_failure<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  released_at: chrono::DateTime<chrono::Utc>,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = lock_descriptor_for_body_node(engine.workflow(), node_id).cloned() else {
    return Ok(());
  };
  let projection = engine.projection(run_id)?;
  let Some(lock) = projection
    .locks
    .get(&descriptor.lock_id)
    .filter(|lock| lock.status == crate::LockStatus::Held)
    .cloned()
  else {
    return Ok(());
  };
  let acquired_at = lock.acquired_at.ok_or_else(|| {
    RuntimeExecutionError::Stalled("A held lock has no acquisition timestamp.".to_string())
  })?;
  let result = serde_json::json!({
    "status": "released",
    "key": lock.key,
    "fencingToken": lock.fencing_token,
    "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    "waitedMs": (acquired_at - lock.registered_at).num_milliseconds().max(0),
  });
  engine.release_lock(run_id, &descriptor.lock_id, result, released_at)?;
  options.stop_lock_renewal(run_id, &descriptor.lock_id);
  Ok(())
}
