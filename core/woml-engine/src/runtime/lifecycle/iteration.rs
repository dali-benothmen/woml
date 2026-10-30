//! Step/reusable lifecycle hooks bound to one for-each iteration identity.

use crate::event::ForEachIterationScope;
use crate::event::LifecycleActionFailedData;
use crate::event::LifecycleActionIdentityData;
use crate::event::LifecycleFailure;
use crate::event::LifecycleFailureKind;
use crate::event::LifecycleHookCompletedData;
use crate::event::LifecycleHookCompletionStatus;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::model::CompiledReusableInvocation;
use crate::model::LifecycleEventName;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::LifecycleBindingV1;
use crate::protocol::LifecycleFailureBindingV1;
use crate::protocol::LifecycleStepBindingV1;
use crate::protocol::LifecycleWorkflowBindingV1;
use crate::protocol::ReusableInvocationOutcome;
use crate::protocol::ReusableLifecycleBindingV1;
use crate::protocol::ReusableLifecycleErrorV1;
use crate::protocol::ReusableLifecycleHook as ProtocolReusableLifecycleHook;
use crate::protocol::ScriptAttempt;
use crate::runtime::cleanup::ArtifactInvocationCleanup;
use crate::runtime::context::resolved_reusable_descriptor_secrets;
use crate::runtime::context::reusable_definition_binding_for_iteration;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::modules::reusable_lifecycle_artifact_name;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::{
  for_each_effect_key, for_each_module_bindings, generated_id, lifecycle_action_source,
  lifecycle_failure_from_attempt, resolved_lifecycle_secrets, reusable_step_descriptor,
  RuntimeDagEngine,
};
use crate::AttemptFailure;
use crate::RunEventPayload;
use crate::ScriptHostClient;
use crate::ScriptHostClientError;
use crate::WorkflowContext;
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
pub(in crate::runtime) async fn drive_for_each_step_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  event: LifecycleEventName,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  attempts: u32,
  failure: Option<&AttemptFailure>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(hook) = engine
    .workflow()
    .lifecycle_hook_for_step_event(event, node_id)
    .cloned()
  else {
    return Ok(());
  };
  let scope = ForEachIterationScope {
    for_each_id: descriptor.for_each_id.clone(),
    index,
  };
  let subject_id = format!("{}[{index}].{node_id}", descriptor.for_each_id);
  let hook_invocation_id = crate::derive_lifecycle_hook_invocation_id(
    run_id,
    &hook.hook_id,
    LifecycleSubjectKind::Step,
    &subject_id,
  );
  engine.append_scoped_payload(
    run_id,
    scope.clone(),
    RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
      hook_invocation_id: hook_invocation_id.clone(),
      hook_id: hook.hook_id.clone(),
      event,
      subject: LifecycleSubject {
        kind: LifecycleSubjectKind::Step,
        id: subject_id,
      },
    }),
  )?;
  let binding = LifecycleBindingV1 {
    event,
    workflow: LifecycleWorkflowBindingV1 {
      id: engine.workflow().workflow_id.clone(),
      outcome: None,
    },
    step: Some(LifecycleStepBindingV1 {
      id: node_id.to_string(),
      outcome: match event {
        LifecycleEventName::StepSuccess => Some(crate::event::BusinessOutcome::Succeeded),
        LifecycleEventName::StepFailure => Some(crate::event::BusinessOutcome::Failed),
        _ => None,
      },
      attempts,
    }),
    failure: failure.map(|failure| LifecycleFailureBindingV1 {
      code: failure.code.clone(),
      message: failure.message.clone(),
    }),
  };
  let modules = for_each_module_bindings(options);
  let mut failed_actions = 0_u32;
  for action in &hook.actions {
    engine.append_scoped_payload(
      run_id,
      scope.clone(),
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id: action.action_id.clone(),
        attempt: 1,
      }),
    )?;
    let execution = if action.handler == "runtime.lifecycle-script" {
      let source = lifecycle_action_source(action).ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(format!(
          "for-each lifecycle action {:?} has no script source",
          action.action_id
        ))
      })?;
      let secrets = resolved_lifecycle_secrets(action, options)?;
      let idempotency_key = for_each_effect_key(
        run_id,
        engine.definition_hash(),
        &descriptor.for_each_id,
        index,
        &action.action_id,
      );
      let invocation_id = generated_id("lcinv");
      let request = ExecuteMessage::lifecycle_script_with_iteration_modules(
        &invocation_id,
        run_id,
        &action.action_id,
        ScriptAttempt::new(1, 1, &idempotency_key).map_err(RuntimeExecutionError::Stalled)?,
        options.script_timeout_ms,
        source,
        context,
        item,
        index,
        total,
        &binding,
        &secrets,
        &modules,
      )
      .map_err(RuntimeExecutionError::Stalled)?;
      let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
      host.execute(&request).await
    } else {
      Err(ScriptHostClientError::Protocol(format!(
        "iteration-scoped lifecycle action {:?} uses unsupported handler {:?}",
        action.action_id, action.handler
      )))
    };
    let payload = match execution {
      Ok(completed) if matches!(completed.outcome, HostOutcome::Success { .. }) => {
        RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
          hook_invocation_id: hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: 1,
        })
      }
      Ok(completed) => {
        failed_actions += 1;
        let HostOutcome::Failure { error } = completed.outcome else {
          unreachable!()
        };
        RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
          hook_invocation_id: hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: 1,
          failure: lifecycle_failure_from_attempt(error.into_attempt_failure()),
        })
      }
      Err(error) => {
        failed_actions += 1;
        RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
          hook_invocation_id: hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: 1,
          failure: LifecycleFailure {
            kind: LifecycleFailureKind::HostCrashed,
            code: "WOML_FOR_EACH_LIFECYCLE_FAILED".to_string(),
            message: error.to_string(),
          },
        })
      }
    };
    engine.append_scoped_payload(run_id, scope.clone(), payload)?;
  }
  engine.append_scoped_payload(
    run_id,
    scope,
    RunEventPayload::LifecycleHookCompleted(LifecycleHookCompletedData {
      hook_invocation_id,
      status: if failed_actions == 0 {
        LifecycleHookCompletionStatus::Completed
      } else {
        LifecycleHookCompletionStatus::CompletedWithWarnings
      },
      failed_actions,
    }),
  )?;
  Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::runtime) async fn drive_for_each_reusable_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  loop_descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(engine.workflow(), node_id).cloned() else {
    return Ok(());
  };
  let CompiledReusableInvocation::Step {
    invocation_id,
    definition_digest,
    lifecycle,
    ..
  } = &descriptor
  else {
    return Ok(());
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
  let scope = ForEachIterationScope {
    for_each_id: loop_descriptor.for_each_id.clone(),
    index,
  };
  let secrets = resolved_reusable_descriptor_secrets(&descriptor, options)?;
  let reusable =
    reusable_definition_binding_for_iteration(&descriptor, context, item, index, total, &secrets)?;
  let modules = for_each_module_bindings(options);
  for (protocol_hook, event_hook, actions) in hooks {
    if actions.is_empty() {
      continue;
    }
    engine.append_scoped_payload(
      run_id,
      scope.clone(),
      RunEventPayload::ReusableLifecycleRequested(crate::ReusableLifecycleRequestedData {
        invocation_id: invocation_id.clone(),
        definition_digest: definition_digest.clone(),
        hook: event_hook,
      }),
    )?;
    let binding = ReusableLifecycleBindingV1 {
      hook: protocol_hook,
      outcome,
      result: result.clone(),
      error: error.clone(),
    };
    let mut warnings = 0_u32;
    for action_id in actions {
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
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
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
      let idempotency_key = for_each_effect_key(
        run_id,
        engine.definition_hash(),
        &loop_descriptor.for_each_id,
        index,
        &action_id,
      );
      let request = ExecuteMessage::reusable_lifecycle_script_with_iteration_modules(
        &host_invocation_id,
        run_id,
        &action_id,
        ScriptAttempt::new(1, 1, &idempotency_key).map_err(RuntimeExecutionError::Stalled)?,
        options.script_timeout_ms,
        &source,
        context,
        item,
        index,
        total,
        &secrets,
        &modules,
        &reusable,
        &binding,
      )
      .map_err(RuntimeExecutionError::Stalled)?;
      let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &host_invocation_id);
      let succeeded = host
        .execute(&request)
        .await
        .is_ok_and(|completed| matches!(completed.outcome, HostOutcome::Success { .. }));
      let payload = if succeeded {
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
        warnings += 1;
        RunEventPayload::ReusableLifecycleActionFailed(crate::ReusableLifecycleActionFailedData {
          invocation_id: invocation_id.clone(),
          definition_digest: definition_digest.clone(),
          hook: event_hook,
          action_id,
          outcome: crate::ReusableLifecycleOutcome::Failed,
          warning_code: "WOML_REUSABLE_LIFECYCLE_ACTION_FAILED".to_string(),
        })
      };
      engine.append_scoped_payload(run_id, scope.clone(), payload)?;
    }
    engine.append_scoped_payload(
      run_id,
      scope.clone(),
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
