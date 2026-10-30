//! Corrective invocation, bindings, retry identity, and ambiguity classification.
//!
//! Preserve declared secrets, capability/placement checks, cleanup guards,
//! deadlines, and manual-recovery classification. A retry budget does not make
//! a timed-out or interrupted corrective effect safe to repeat.

use crate::event::CorrectionAttemptFailedData;
use crate::event::CorrectionAttemptIdentityData;
use crate::event::CorrectionInterruptedData;
use crate::event::CorrectionRetryScheduledData;
use crate::event::CorrectiveActionKind;
use crate::event::ManualRecoveryRequiredData;
use crate::event::SagaCorrectionReason;
use crate::model::CompiledCorrectiveAction;
use crate::protocol::CleanupBindingV1;
use crate::protocol::CleanupReasonV1;
use crate::protocol::CompensationBindingV1;
use crate::protocol::CompensationStatusV1;
use crate::protocol::CorrectionBindingRef;
use crate::protocol::CorrectionReasonV1;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::RuntimeModuleBinding;
use crate::protocol::ScriptAttempt;
use crate::runtime::cleanup::ArtifactInvocationCleanup;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::script_execution::ensure_local_script_execution;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{correction_iteration_identity, RuntimeDagEngine};
use crate::AttemptFailureKind;
use crate::RunEventPayload;
use crate::ScriptHostClient;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum CorrectiveActionOutcome {
  Succeeded,
  Failed,
  ManualRecovery,
}

fn resolved_corrective_secrets(
  action: &CompiledCorrectiveAction,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let Some(runtime) = action.node.script_runtime.as_ref() else {
    return Ok(BTreeMap::new());
  };
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
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "corrective action {:?} requires unresolved secret {name:?}",
            action.action_id
          ))
        })
    })
    .collect()
}

pub(in crate::runtime) async fn execute_corrective_action<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  saga_id: &str,
  action: &CompiledCorrectiveAction,
  runtime_action_id: &str,
  action_kind: CorrectiveActionKind,
  reason: SagaCorrectionReason,
  compensation_status: CompensationStatusV1,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<CorrectiveActionOutcome, RuntimeExecutionError> {
  ensure_local_script_execution(&action.node)?;
  let projected = engine
    .projection(run_id)?
    .sagas
    .get(saga_id)
    .and_then(|saga| saga.actions.get(runtime_action_id))
    .cloned();
  if projected
    .as_ref()
    .is_some_and(|state| state.status == crate::CorrectionActionStatus::Succeeded)
  {
    return Ok(CorrectiveActionOutcome::Succeeded);
  }
  if engine.correction_action_externally_resolved(run_id, saga_id, runtime_action_id)? {
    return Ok(CorrectiveActionOutcome::Succeeded);
  }
  if projected
    .as_ref()
    .is_some_and(|state| state.status == crate::CorrectionActionStatus::ManualRecovery)
  {
    return Ok(CorrectiveActionOutcome::ManualRecovery);
  }
  let source = action.node.script_source().ok_or_else(|| {
    RuntimeExecutionError::Stalled(format!(
      "corrective action {:?} has no script source",
      runtime_action_id
    ))
  })?;
  let secrets = resolved_corrective_secrets(action, options)?;
  let projection = engine.projection(run_id)?;
  let mut context = correction_iteration_identity(action, runtime_action_id)
    .and_then(|(for_each_id, index)| {
      projection
        .for_each
        .get(for_each_id)
        .and_then(|loop_state| loop_state.executions.get(&index))
        .map(|execution| execution.context.clone())
    })
    .unwrap_or_else(|| projection.context.clone());
  context
    .steps
    .retain(|step_id, _| action.visible_step_ids.contains(step_id));
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
  let configured_max_attempts = action
    .node
    .retry_policy
    .as_ref()
    .map_or(1, |policy| policy.max_attempts);
  let max_attempts = projected.as_ref().map_or(configured_max_attempts, |state| {
    if state.status == crate::CorrectionActionStatus::RetryScheduled {
      configured_max_attempts.max(state.attempts.saturating_add(1).min(10))
    } else {
      configured_max_attempts
    }
  });
  let timeout_ms = action.node.timeout_ms.unwrap_or(options.script_timeout_ms);
  let compensation_binding =
    (action_kind == CorrectiveActionKind::Compensation).then(|| CompensationBindingV1 {
      saga_id: saga_id.to_string(),
      for_step_id: action.for_step_id.clone().unwrap_or_default(),
      reason: match reason {
        SagaCorrectionReason::Cancel => CorrectionReasonV1::Cancel,
        SagaCorrectionReason::Failure | SagaCorrectionReason::Success => {
          CorrectionReasonV1::Failure
        }
      },
    });
  let cleanup_binding = (action_kind == CorrectiveActionKind::Cleanup).then(|| CleanupBindingV1 {
    saga_id: saga_id.to_string(),
    reason: match reason {
      SagaCorrectionReason::Success => CleanupReasonV1::Success,
      SagaCorrectionReason::Failure => CleanupReasonV1::Failure,
      SagaCorrectionReason::Cancel => CleanupReasonV1::Cancel,
    },
    compensation_status,
  });
  let correction = match action_kind {
    CorrectiveActionKind::Compensation => CorrectionBindingRef::Compensation(
      compensation_binding
        .as_ref()
        .expect("compensation binding was constructed"),
    ),
    CorrectiveActionKind::Cleanup => CorrectionBindingRef::Cleanup(
      cleanup_binding
        .as_ref()
        .expect("cleanup binding was constructed"),
    ),
  };

  let mut attempt = projected.as_ref().map_or(1, |state| state.attempts + 1);
  if let Some(retry_at) = projected.as_ref().and_then(|state| state.retry_at) {
    while retry_at > options.clock.now() {
      let wait = (retry_at - options.clock.now())
        .to_std()
        .unwrap_or(CANCELLATION_POLL_INTERVAL)
        .min(CANCELLATION_POLL_INTERVAL);
      tokio::time::sleep(wait).await;
    }
  }
  while attempt <= max_attempts {
    let identity = CorrectionAttemptIdentityData {
      saga_id: saga_id.to_string(),
      action_id: runtime_action_id.to_string(),
      action_kind,
      attempt,
    };
    engine.append_payload(
      run_id,
      RunEventPayload::CorrectionAttemptStarted(identity.clone()),
    )?;
    if host.is_none() {
      match options.acquire_script_host().await {
        Ok(client) => *host = Some(client),
        Err(_) => {
          engine.append_payloads(
            run_id,
            vec![
              RunEventPayload::CorrectionInterrupted(CorrectionInterruptedData {
                saga_id: saga_id.to_string(),
                action_id: runtime_action_id.to_string(),
                action_kind,
                attempt,
                ambiguous: true,
              }),
              RunEventPayload::ManualRecoveryRequired(ManualRecoveryRequiredData {
                saga_id: saga_id.to_string(),
                action_id: runtime_action_id.to_string(),
                action_kind,
                code: "WOML_CORRECTION_HOST_UNAVAILABLE".to_string(),
              }),
            ],
          )?;
          return Ok(CorrectiveActionOutcome::ManualRecovery);
        }
      }
    }
    let invocation_id =
      crate::durable::correction_invocation_id(run_id, saga_id, runtime_action_id, attempt);
    let idempotency_key = format!("sha256:{:x}", Sha256::digest(invocation_id.as_bytes()));
    let script_attempt = ScriptAttempt::new(attempt, max_attempts, &idempotency_key)
      .map_err(RuntimeExecutionError::Stalled)?;
    let request = ExecuteMessage::corrective_script_with_modules(
      &invocation_id,
      run_id,
      runtime_action_id,
      script_attempt,
      timeout_ms,
      source,
      &context,
      correction,
      &secrets,
      &module_bindings,
    );
    let cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
    let completed = host
      .as_ref()
      .expect("corrective script host was initialized")
      .execute(&request)
      .await;
    drop(cleanup);
    match completed {
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { .. } => {
          engine.append_payload(
            run_id,
            RunEventPayload::CorrectionAttemptSucceeded(identity),
          )?;
          return Ok(CorrectiveActionOutcome::Succeeded);
        }
        HostOutcome::Failure { error } => {
          let failure = error.into_attempt_failure();
          let ambiguous = failure.ambiguous.unwrap_or(false)
            || matches!(
              failure.kind,
              AttemptFailureKind::ScriptTimedOut
                | AttemptFailureKind::WorkerCrashed
                | AttemptFailureKind::HostCrashed
                | AttemptFailureKind::InvocationCancelled
                | AttemptFailureKind::Interrupted
            );
          let retryable = !ambiguous
            && (failure.retryable.unwrap_or(false)
              || failure.kind == AttemptFailureKind::ScriptThrew);
          engine.append_payload(
            run_id,
            RunEventPayload::CorrectionAttemptFailed(CorrectionAttemptFailedData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              attempt,
              code: failure.code,
              retryable,
              ambiguous,
            }),
          )?;
          if ambiguous {
            engine.append_payload(
              run_id,
              RunEventPayload::ManualRecoveryRequired(ManualRecoveryRequiredData {
                saga_id: saga_id.to_string(),
                action_id: runtime_action_id.to_string(),
                action_kind,
                code: "WOML_CORRECTION_OUTCOME_AMBIGUOUS".to_string(),
              }),
            )?;
            return Ok(CorrectiveActionOutcome::ManualRecovery);
          }
          if !retryable || attempt >= max_attempts {
            return Ok(CorrectiveActionOutcome::Failed);
          }
          let delay = action
            .node
            .retry_policy
            .as_ref()
            .and_then(|policy| policy.delay_before_attempt(attempt + 1))
            .unwrap_or(0);
          let retry_at = options.clock.now()
            + chrono::Duration::milliseconds(i64::try_from(delay).unwrap_or(i64::MAX));
          engine.append_payload(
            run_id,
            RunEventPayload::CorrectionRetryScheduled(CorrectionRetryScheduledData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              attempt,
              retry_at,
            }),
          )?;
          if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
          }
          attempt += 1;
        }
      },
      Err(_) => {
        *host = None;
        engine.append_payloads(
          run_id,
          vec![
            RunEventPayload::CorrectionInterrupted(CorrectionInterruptedData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              attempt,
              ambiguous: true,
            }),
            RunEventPayload::ManualRecoveryRequired(ManualRecoveryRequiredData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              code: "WOML_CORRECTION_INTERRUPTED".to_string(),
            }),
          ],
        )?;
        return Ok(CorrectiveActionOutcome::ManualRecovery);
      }
    }
  }
  Ok(CorrectiveActionOutcome::Failed)
}
