//! Reusable step/provider descriptors and durable lifecycle continuation.
//!
//! Recorded completed hooks are not silently replayed. Provider lifecycle uses
//! the same identity-bound modules and runtime authority as its calling run.

use crate::engine::step_effect_idempotency_key;
use crate::model::CompiledReusableInvocation;
use crate::projection::AttemptStatus;
use crate::projection::ReusableLifecycleStatus;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleBindingV1;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::protocol::ReusableLifecycleHook as ProtocolReusableLifecycleHook;
use crate::protocol::RuntimeModuleBinding;
use crate::protocol::ScriptAttempt;
use crate::runtime::cleanup::ArtifactInvocationCleanup;
use crate::runtime::composition::attach_durable_capability_authority;
use crate::runtime::context::resolved_reusable_descriptor_secrets;
use crate::runtime::context::reusable_definition_binding;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::modules::reusable_lifecycle_artifact_name;
use crate::runtime::modules::runtime_modules_from_store;
use crate::runtime::modules::validate_runtime_modules;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::{generated_id, reusable_step_descriptor, RuntimeDagEngine};
use crate::AttemptFailureKind;
use crate::CompiledWorkflowDefinition;
use crate::DurableDagEngine;
use crate::DurableEventStore;
use crate::RunEventPayload;
use crate::ScriptHostClient;
use serde_json::Value;
use std::path::PathBuf;

/// Finalizes one custom notification provider's definition-owned lifecycle
/// after its durable delivery outcome is known. Notification journeys use this
/// adapter so provider hooks share the same Event v13 authority, immutable
/// artifacts, managed services, and recovery rules as custom-step hooks.
pub async fn execute_reusable_provider_lifecycle_durable(
  database_path: PathBuf,
  run_id: &str,
  provider_id: &str,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let store = DurableEventStore::open(database_path)?;
  let binding = store.run_binding(run_id)?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  validate_runtime_modules(engine.workflow(), &options)?;
  let descriptor_has_lifecycle = reusable_provider_descriptor(engine.workflow(), provider_id)
    .is_some_and(|descriptor| {
      matches!(
        descriptor,
        CompiledReusableInvocation::NotificationProvider {
          lifecycle: Some(_),
          ..
        }
      )
    });
  if !descriptor_has_lifecycle {
    return Ok(());
  }
  let host = options.acquire_script_host().await?;
  let lifecycle = drive_reusable_provider_lifecycle(
    &mut engine,
    run_id,
    provider_id,
    outcome,
    result,
    error,
    &options,
    &host,
  )
  .await;
  options.release_script_host(host).await;
  lifecycle
}

pub(in crate::runtime) fn reusable_provider_descriptor<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  provider_id: &str,
) -> Option<&'a CompiledReusableInvocation> {
  workflow
    .reusable_definitions
    .iter()
    .flatten()
    .find(|definition| {
      matches!(definition, CompiledReusableInvocation::NotificationProvider { provider_id: value, .. } if value == provider_id)
    })
}

pub(in crate::runtime) async fn drive_reusable_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &CompiledReusableInvocation,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let (invocation_id, definition_digest, lifecycle) = match descriptor {
    CompiledReusableInvocation::Step {
      invocation_id,
      definition_digest,
      lifecycle,
      ..
    } => (invocation_id, definition_digest, lifecycle),
    CompiledReusableInvocation::NotificationProvider {
      provider_id,
      definition_digest,
      lifecycle,
      ..
    } => (provider_id, definition_digest, lifecycle),
  };
  let Some(lifecycle) = lifecycle else {
    return Ok(());
  };

  let hooks = match outcome {
    ReusableInvocationOutcome::Succeeded => vec![
      (
        ProtocolReusableLifecycleHook::OnSuccess,
        crate::ReusableLifecycleHook::OnSuccess,
        lifecycle.on_success.clone().unwrap_or_default(),
      ),
      (
        ProtocolReusableLifecycleHook::OnComplete,
        crate::ReusableLifecycleHook::OnComplete,
        lifecycle.on_complete.clone().unwrap_or_default(),
      ),
    ],
    ReusableInvocationOutcome::Failed => vec![
      (
        ProtocolReusableLifecycleHook::OnError,
        crate::ReusableLifecycleHook::OnError,
        lifecycle.on_error.clone().unwrap_or_default(),
      ),
      (
        ProtocolReusableLifecycleHook::OnComplete,
        crate::ReusableLifecycleHook::OnComplete,
        lifecycle.on_complete.clone().unwrap_or_default(),
      ),
    ],
    ReusableInvocationOutcome::Cancelled => vec![(
      ProtocolReusableLifecycleHook::OnComplete,
      crate::ReusableLifecycleHook::OnComplete,
      lifecycle.on_complete.clone().unwrap_or_default(),
    )],
  };
  let context = engine.projection(run_id)?.context;
  let secrets = resolved_reusable_descriptor_secrets(descriptor, options)?;
  let reusable = reusable_definition_binding(descriptor, &context, &secrets)?;
  let modules = options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect::<Vec<_>>();

  for (protocol_hook, event_hook, actions) in hooks {
    if actions.is_empty() {
      continue;
    }
    let key = format!("{}:{:?}", invocation_id, event_hook);
    let projected = engine
      .projection(run_id)?
      .reusable_lifecycle_hooks
      .get(&key)
      .cloned();
    if projected.as_ref().is_some_and(|hook| {
      matches!(
        hook.status,
        ReusableLifecycleStatus::Completed | ReusableLifecycleStatus::CompletedWithWarnings
      )
    }) {
      continue;
    }
    if projected.is_none() {
      engine.append_payload(
        run_id,
        RunEventPayload::ReusableLifecycleRequested(crate::ReusableLifecycleRequestedData {
          invocation_id: invocation_id.clone(),
          definition_digest: definition_digest.clone(),
          hook: event_hook,
        }),
      )?;
    }
    let binding = ReusableLifecycleBindingV1 {
      hook: protocol_hook,
      outcome,
      result: result.clone(),
      error: error.clone(),
    };
    for action_id in actions {
      let projected = engine
        .projection(run_id)?
        .reusable_lifecycle_hooks
        .get(&key)
        .cloned()
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled("reusable lifecycle request was not projected".to_string())
        })?;
      if projected.completed_action_ids.contains(&action_id) {
        continue;
      }
      if projected.active_action_id.as_deref() == Some(action_id.as_str()) {
        engine.append_payload(
          run_id,
          RunEventPayload::ReusableLifecycleActionFailed(
            crate::ReusableLifecycleActionFailedData {
              invocation_id: invocation_id.clone(),
              definition_digest: definition_digest.clone(),
              hook: event_hook,
              action_id: action_id.clone(),
              outcome: crate::ReusableLifecycleOutcome::Failed,
              warning_code: "WOML_REUSABLE_LIFECYCLE_INTERRUPTED".to_string(),
            },
          ),
        )?;
        continue;
      }
      let artifact_name = reusable_lifecycle_artifact_name(&action_id);
      let source = options
        .runtime_modules
        .iter()
        .find(|artifact| artifact.name == artifact_name)
        .map(|artifact| artifact.bundle.clone())
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "reusable lifecycle artifact for {action_id:?} is unavailable"
          ))
        })?;
      engine.append_payload(
        run_id,
        RunEventPayload::ReusableLifecycleActionStarted(
          crate::ReusableLifecycleActionStartedData {
            invocation_id: invocation_id.clone(),
            definition_digest: definition_digest.clone(),
            hook: event_hook,
            action_id: action_id.clone(),
          },
        ),
      )?;
      let host_invocation_id = generated_id("rlcinv");
      let idempotency_key =
        step_effect_idempotency_key(run_id, engine.definition_hash(), &action_id);
      let request = ExecuteMessage::reusable_lifecycle_script_with_modules(
        &host_invocation_id,
        run_id,
        &action_id,
        ScriptAttempt::new(1, 1, &idempotency_key).map_err(RuntimeExecutionError::Stalled)?,
        options.script_timeout_ms,
        &source,
        &context,
        &secrets,
        &modules,
        &reusable,
        &binding,
      );
      let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &host_invocation_id);
      let succeeded = host
        .execute(&request)
        .await
        .is_ok_and(|completed| matches!(completed.outcome, HostOutcome::Success { .. }));
      engine.append_payload(
        run_id,
        if succeeded {
          RunEventPayload::ReusableLifecycleActionSucceeded(
            crate::ReusableLifecycleActionSucceededData {
              invocation_id: invocation_id.clone(),
              definition_digest: definition_digest.clone(),
              hook: event_hook,
              action_id,
              outcome: crate::ReusableLifecycleOutcome::Succeeded,
            },
          )
        } else {
          RunEventPayload::ReusableLifecycleActionFailed(crate::ReusableLifecycleActionFailedData {
            invocation_id: invocation_id.clone(),
            definition_digest: definition_digest.clone(),
            hook: event_hook,
            action_id,
            outcome: crate::ReusableLifecycleOutcome::Failed,
            warning_code: "WOML_REUSABLE_LIFECYCLE_ACTION_FAILED".to_string(),
          })
        },
      )?;
    }
    let warnings = engine
      .projection(run_id)?
      .reusable_lifecycle_hooks
      .get(&key)
      .map_or(0, |hook| hook.warning_codes.len());
    engine.append_payload(
      run_id,
      RunEventPayload::ReusableLifecycleCompleted(crate::ReusableLifecycleCompletedData {
        invocation_id: invocation_id.clone(),
        definition_digest: definition_digest.clone(),
        hook: event_hook,
        outcome: if warnings == 0 {
          crate::ReusableLifecycleOutcome::Succeeded
        } else {
          crate::ReusableLifecycleOutcome::CompletedWithWarnings
        },
      }),
    )?;
  }
  Ok(())
}

pub(in crate::runtime) async fn drive_reusable_step_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(engine.workflow(), node_id).cloned() else {
    return Ok(());
  };
  drive_reusable_lifecycle(
    engine,
    run_id,
    &descriptor,
    outcome,
    result,
    error,
    options,
    host,
  )
  .await
}

async fn drive_reusable_provider_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  provider_id: &str,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = reusable_provider_descriptor(engine.workflow(), provider_id).cloned()
  else {
    return Ok(());
  };
  drive_reusable_lifecycle(
    engine,
    run_id,
    &descriptor,
    outcome,
    result,
    error,
    options,
    host,
  )
  .await
}

pub(in crate::runtime) async fn drive_pending_reusable_step_lifecycles<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<(), RuntimeExecutionError> {
  let descriptors = engine
    .workflow()
    .reusable_definitions
    .clone()
    .unwrap_or_default();
  for descriptor in descriptors {
    let CompiledReusableInvocation::Step {
      node_id,
      lifecycle: Some(_),
      ..
    } = descriptor
    else {
      continue;
    };
    let projection = engine.projection(run_id)?;
    let Some(attempt) = projection.latest_attempt(&node_id) else {
      continue;
    };
    let (outcome, result, error) = match &attempt.status {
      AttemptStatus::Started => continue,
      AttemptStatus::Succeeded { output } => (
        ReusableInvocationOutcome::Succeeded,
        Some(output.clone()),
        None,
      ),
      AttemptStatus::Failed { .. } if projection.pending_retries.contains_key(&node_id) => continue,
      AttemptStatus::Failed { failure }
        if failure.kind == AttemptFailureKind::InvocationCancelled =>
      {
        (ReusableInvocationOutcome::Cancelled, None, None)
      }
      AttemptStatus::Failed { failure } => (
        ReusableInvocationOutcome::Failed,
        None,
        Some(ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        }),
      ),
    };
    if host.is_none() {
      *host = Some(options.acquire_script_host().await?);
    }
    drive_reusable_step_lifecycle(
      engine,
      run_id,
      &node_id,
      outcome,
      result,
      error,
      options,
      host
        .as_ref()
        .expect("reusable lifecycle host was initialized"),
    )
    .await?;
  }
  Ok(())
}
