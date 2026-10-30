//! Ordered workflow hooks, lifecycle snapshots, observation, and finalization.
//!
//! Outcome hooks observe durable business truth; a failed notification cannot
//! manufacture business failure/success. Cancellation hooks respect outcome
//! settlement rather than bypassing saga correction.

use crate::engine::step_effect_idempotency_key;
use crate::event::FinalLifecycleStatus;
use crate::event::LifecycleActionFailedData;
use crate::event::LifecycleActionIdentityData;
use crate::event::LifecycleFailure;
use crate::event::LifecycleFailureKind;
use crate::event::LifecycleHookCompletedData;
use crate::event::LifecycleHookCompletionStatus;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::event::RunFinalizedData;
use crate::model::CompiledLifecycleAction;
use crate::model::LifecycleEventName;
use crate::model::ValueExpression;
use crate::projection::AttemptStatus;
use crate::projection::LifecycleActionStatus;
use crate::projection::LifecycleHookProjection;
use crate::projection::LifecycleHookStatus;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::LifecycleBindingV1;
use crate::protocol::LifecycleFailureBindingV1;
use crate::protocol::LifecycleStepBindingV1;
use crate::protocol::LifecycleWorkflowBindingV1;
use crate::protocol::RuntimeModuleBinding;
use crate::protocol::ScriptAttempt;
use crate::runtime::cleanup::ArtifactInvocationCleanup;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::progress::LifecycleProgress;
use crate::runtime::progress::LifecycleProgressPhase;
use crate::runtime::progress::LIFECYCLE_PROGRESS_PROFILE;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  execute_lifecycle_notification, generated_id, lifecycle_failure_from_attempt,
  step_lifecycle_outcome, step_lifecycle_request, RuntimeDagEngine,
};
use crate::CompiledWorkflowDefinition;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use crate::ScriptHostClient;
use std::collections::BTreeMap;

pub(in crate::runtime) fn lifecycle_action_source(
  action: &CompiledLifecycleAction,
) -> Option<&str> {
  let ValueExpression::Object { fields } = &action.inputs else {
    return None;
  };
  let ValueExpression::Literal { value } = fields.get("source")? else {
    return None;
  };
  value.as_str()
}

pub(in crate::runtime) fn resolved_lifecycle_secrets(
  action: &CompiledLifecycleAction,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let Some(runtime) = &action.script_runtime else {
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
            "lifecycle action {:?} requires unresolved secret {name:?}",
            action.action_id
          ))
        })
    })
    .collect()
}

fn workflow_lifecycle_request(
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  event: LifecycleEventName,
) -> Option<RunEventPayload> {
  let hook = workflow.lifecycle_hook_for_event(event)?;
  let subject = LifecycleSubject {
    kind: LifecycleSubjectKind::Workflow,
    id: run_id.to_string(),
  };
  Some(RunEventPayload::LifecycleHookRequested(
    LifecycleHookRequestedData {
      hook_invocation_id: crate::derive_lifecycle_hook_invocation_id(
        run_id,
        &hook.hook_id,
        subject.kind,
        &subject.id,
      ),
      hook_id: hook.hook_id.clone(),
      event: hook.event,
      subject,
    },
  ))
}

fn report_lifecycle(
  options: &RuntimeExecutionOptions,
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  phase: LifecycleProgressPhase,
  hook_id: &str,
  action_id: &str,
  step_id: Option<&str>,
  code: Option<String>,
) {
  options.report_lifecycle(LifecycleProgress {
    profile: LIFECYCLE_PROGRESS_PROFILE,
    run_id: run_id.to_string(),
    workflow_id: workflow.workflow_id.clone(),
    phase,
    hook_id: hook_id.to_string(),
    action_id: action_id.to_string(),
    step_id: step_id.map(str::to_string),
    code,
  });
}

fn pending_lifecycle_hook(
  projection: &RunProjection,
  events: &[RunEvent],
) -> Option<LifecycleHookProjection> {
  let pending = |hook: &LifecycleHookProjection| {
    matches!(
      hook.status,
      LifecycleHookStatus::Requested | LifecycleHookStatus::Running
    )
  };
  for subject_kind in [LifecycleSubjectKind::Step, LifecycleSubjectKind::Workflow] {
    if let Some(hook) = events.iter().find_map(|event| {
      let RunEventPayload::LifecycleHookRequested(request) = &event.payload else {
        return None;
      };
      let hook = projection
        .lifecycle_hooks
        .get(&request.hook_invocation_id)?;
      (hook.subject.kind == subject_kind && pending(hook)).then(|| hook.clone())
    }) {
      return Some(hook);
    }
  }
  None
}

fn lifecycle_request_snapshot(
  events: &[RunEvent],
  hook_invocation_id: &str,
) -> Result<RunProjection, RuntimeExecutionError> {
  let request_index = events
    .iter()
    .position(|event| {
      matches!(
        &event.payload,
        RunEventPayload::LifecycleHookRequested(request)
          if request.hook_invocation_id == hook_invocation_id
      )
    })
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled("lifecycle request event disappeared".to_string())
    })?;
  crate::fold_events(&events[..=request_index])
    .map_err(|error| RuntimeExecutionError::Stalled(error.to_string()))
}

pub(in crate::runtime) async fn drive_workflow_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<(), RuntimeExecutionError> {
  if !matches!(
    engine.workflow().schema_version,
    crate::COMPILED_MODEL_SCHEMA_VERSION_V11
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
  ) {
    return Ok(());
  }
  loop {
    let projection = engine.projection(run_id)?;
    let events = engine.events(run_id)?;
    let pending = pending_lifecycle_hook(&projection, &events);
    let Some(hook_projection) = pending else {
      if projection.status == RunStatus::Finalizing {
        let outcome = projection.business_outcome.ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "finalizing workflow has no durable business outcome".to_string(),
          )
        })?;
        let warnings = crate::durable::lifecycle_warnings_from_projection(&projection);
        let final_hook = projection
          .lifecycle_hooks
          .values()
          .find(|hook| hook.event == LifecycleEventName::RunComplete)
          .or_else(|| {
            projection.lifecycle_hooks.values().find(|hook| {
              matches!(
                hook.event,
                LifecycleEventName::RunSuccess
                  | LifecycleEventName::RunFailure
                  | LifecycleEventName::RunCancel
              )
            })
          });
        let hook_id = final_hook.map_or("lifecycle:run_complete", |hook| hook.hook_id.as_str());
        let action_id = final_hook
          .and_then(|hook| hook.actions.keys().next())
          .map_or("lifecycle:run_complete:action:0", String::as_str);
        report_lifecycle(
          options,
          engine.workflow(),
          run_id,
          LifecycleProgressPhase::RunFinalizing,
          hook_id,
          action_id,
          None,
          None,
        );
        engine.append_payload(
          run_id,
          RunEventPayload::RunFinalized(RunFinalizedData {
            outcome,
            lifecycle_status: if warnings.is_empty() {
              FinalLifecycleStatus::Completed
            } else {
              FinalLifecycleStatus::CompletedWithWarnings
            },
            warnings,
          }),
        )?;
        report_lifecycle(
          options,
          engine.workflow(),
          run_id,
          LifecycleProgressPhase::RunFinalized,
          hook_id,
          action_id,
          None,
          None,
        );
      }
      return Ok(());
    };
    if hook_projection.event == LifecycleEventName::RunCancel
      && projection.business_outcome.is_none()
    {
      // The cancellation request admits this hook durably, but it must not run
      // until all pre-outcome work has settled and cancellation is the durable
      // business outcome.
      return Ok(());
    }
    let hook = engine
      .workflow()
      .lifecycle_hook(&hook_projection.hook_id)
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "durable lifecycle hook is not in the definition".to_string(),
        )
      })?;
    let progress_action_id = hook.actions.first().map_or_else(
      || format!("{}:action:0", hook.hook_id),
      |action| action.action_id.clone(),
    );
    if hook_projection.status == LifecycleHookStatus::Requested {
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        LifecycleProgressPhase::HookRequested,
        &hook.hook_id,
        &progress_action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        None,
      );
    }
    if hook_projection
      .actions
      .values()
      .any(|action| action.status == LifecycleActionStatus::Started)
    {
      return Err(RuntimeExecutionError::Stalled(
        "an interrupted lifecycle action must be recovered before resume".to_string(),
      ));
    }
    let next_action = hook
      .actions
      .iter()
      .find(|action| !hook_projection.actions.contains_key(&action.action_id))
      .cloned();
    let Some(action) = next_action else {
      let failed_actions = hook_projection.failed_actions;
      let mut payloads = vec![RunEventPayload::LifecycleHookCompleted(
        LifecycleHookCompletedData {
          hook_invocation_id: hook_projection.hook_invocation_id.clone(),
          status: if failed_actions == 0 {
            LifecycleHookCompletionStatus::Completed
          } else {
            LifecycleHookCompletionStatus::CompletedWithWarnings
          },
          failed_actions,
        },
      )];
      if matches!(
        hook.event,
        LifecycleEventName::RunSuccess
          | LifecycleEventName::RunFailure
          | LifecycleEventName::RunCancel
      ) {
        if let Some(request) =
          workflow_lifecycle_request(engine.workflow(), run_id, LifecycleEventName::RunComplete)
        {
          payloads.push(request);
        }
      } else if matches!(
        hook.event,
        LifecycleEventName::StepSuccess | LifecycleEventName::StepFailure
      ) {
        if let Some(request) = step_lifecycle_request(
          engine.workflow(),
          run_id,
          LifecycleEventName::StepComplete,
          &hook_projection.subject.id,
        ) {
          payloads.push(request);
        }
      }
      engine.append_payloads(run_id, payloads)?;
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        LifecycleProgressPhase::HookCompleted,
        &hook.hook_id,
        &progress_action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        None,
      );
      continue;
    };
    if projection.cancellation_request_id.is_some()
      && matches!(
        hook.event,
        LifecycleEventName::RunStart | LifecycleEventName::StepStart
      )
    {
      let attempt_number = 1;
      engine.append_payloads(
        run_id,
        vec![
          RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
          }),
          RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
            failure: LifecycleFailure {
              kind: LifecycleFailureKind::Cancelled,
              code: "WOML_LIFECYCLE_ACTION_CANCELLED".to_string(),
              message: "The pending pre-outcome lifecycle action was cancelled without execution."
                .to_string(),
            },
          }),
        ],
      )?;
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        LifecycleProgressPhase::ActionFailed,
        &hook.hook_id,
        &action.action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        Some("WOML_LIFECYCLE_ACTION_CANCELLED".to_string()),
      );
      continue;
    }
    if !matches!(
      action.handler.as_str(),
      "runtime.lifecycle-script" | "notification.informational"
    ) {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "lifecycle action {:?} uses an unsupported runtime handler",
        action.action_id
      )));
    }
    let attempt_number = 1;
    let invocation_id = generated_id("lcinv");
    let idempotency_key =
      step_effect_idempotency_key(run_id, engine.definition_hash(), &action.action_id);
    engine.append_payload(
      run_id,
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_projection.hook_invocation_id.clone(),
        action_id: action.action_id.clone(),
        attempt: attempt_number,
      }),
    )?;
    report_lifecycle(
      options,
      engine.workflow(),
      run_id,
      LifecycleProgressPhase::ActionStarted,
      &hook.hook_id,
      &action.action_id,
      matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
        .then_some(hook_projection.subject.id.as_str()),
      None,
    );
    let snapshot = lifecycle_request_snapshot(&events, &hook_projection.hook_invocation_id)?;
    let context = snapshot.context.clone();
    let step = matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step).then(|| {
      let step_id = hook_projection.subject.id.clone();
      LifecycleStepBindingV1 {
        id: step_id.clone(),
        outcome: step_lifecycle_outcome(hook.event, &snapshot, &step_id),
        attempts: snapshot
          .latest_attempt(&step_id)
          .map_or(0, |attempt| attempt.identity.attempt),
      }
    });
    let step_failure = step.as_ref().and_then(|step| {
      snapshot.latest_attempt(&step.id).and_then(|attempt| {
        let AttemptStatus::Failed { failure } = &attempt.status else {
          return None;
        };
        Some(LifecycleFailureBindingV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        })
      })
    });
    let binding = LifecycleBindingV1 {
      event: hook.event,
      workflow: LifecycleWorkflowBindingV1 {
        id: engine.workflow().workflow_id.clone(),
        outcome: snapshot.business_outcome,
      },
      step,
      failure: step_failure.or_else(|| {
        snapshot
          .lifecycle_failure
          .as_ref()
          .map(|failure| LifecycleFailureBindingV1 {
            code: failure.code.clone(),
            message: failure.message.clone(),
          })
      }),
    };
    if action.handler == "notification.informational" {
      let result = execute_lifecycle_notification(
        engine,
        run_id,
        &action,
        &hook_projection.hook_invocation_id,
        &context,
        &binding,
        options,
      )
      .await;
      let (payload, phase, failure_code) = match result {
        Ok(()) => (
          RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
          }),
          LifecycleProgressPhase::ActionSucceeded,
          None,
        ),
        Err(failure) => {
          let code = failure.code.clone();
          (
            RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
              hook_invocation_id: hook_projection.hook_invocation_id.clone(),
              action_id: action.action_id.clone(),
              attempt: attempt_number,
              failure,
            }),
            LifecycleProgressPhase::ActionFailed,
            Some(code),
          )
        }
      };
      engine.append_payload(run_id, payload)?;
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        phase,
        &hook.hook_id,
        &action.action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        failure_code,
      );
      continue;
    }
    let source = lifecycle_action_source(&action).ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(format!(
        "lifecycle action {:?} has no script source",
        action.action_id
      ))
    })?;
    let secrets = resolved_lifecycle_secrets(&action, options)?;
    if host.is_none() {
      match options.acquire_script_host().await {
        Ok(client) => *host = Some(client),
        Err(error) => {
          engine.append_payload(
            run_id,
            RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
              hook_invocation_id: hook_projection.hook_invocation_id.clone(),
              action_id: action.action_id.clone(),
              attempt: attempt_number,
              failure: LifecycleFailure {
                kind: LifecycleFailureKind::HostCrashed,
                code: "WOML_SCRIPT_HOST_CRASHED".to_string(),
                message: error.to_string(),
              },
            }),
          )?;
          report_lifecycle(
            options,
            engine.workflow(),
            run_id,
            LifecycleProgressPhase::ActionFailed,
            &hook.hook_id,
            &progress_action_id,
            matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
              .then_some(hook_projection.subject.id.as_str()),
            Some("WOML_SCRIPT_HOST_CRASHED".to_string()),
          );
          continue;
        }
      }
    }
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
    let request = ExecuteMessage::lifecycle_script_with_modules(
      &invocation_id,
      run_id,
      &action.action_id,
      ScriptAttempt::new(attempt_number, 1, &idempotency_key)
        .map_err(RuntimeExecutionError::Stalled)?,
      options.script_timeout_ms,
      source,
      &context,
      &binding,
      &secrets,
      &modules,
    );
    let host_client = host.as_ref().expect("script host was initialized");
    let artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
    let mut execution = Box::pin(host_client.execute(&request));
    let mut cancellation_sent = false;
    let result = loop {
      tokio::select! {
        result = &mut execution => break result,
        _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent => {
          let current = engine.projection(run_id)?;
          let pre_outcome_action = current.business_outcome.is_none()
            && hook.event != LifecycleEventName::RunCancel;
          if current.status == RunStatus::Cancelling && pre_outcome_action {
            cancellation_sent = true;
            if let Err(error) = host_client.cancel_run(&invocation_id).await {
              break Err(error);
            }
          }
        }
      }
    };
    drop(execution);
    drop(artifact_cleanup);
    let (payload, phase, failure_code) = match result {
      _ if cancellation_sent => (
        RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
          hook_invocation_id: hook_projection.hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: attempt_number,
          failure: LifecycleFailure {
            kind: LifecycleFailureKind::Cancelled,
            code: "WOML_LIFECYCLE_ACTION_CANCELLED".to_string(),
            message: "The lifecycle action was cancelled with its workflow run.".to_string(),
          },
        }),
        LifecycleProgressPhase::ActionFailed,
        Some("WOML_LIFECYCLE_ACTION_CANCELLED".to_string()),
      ),
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { .. } => (
          RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
          }),
          LifecycleProgressPhase::ActionSucceeded,
          None,
        ),
        HostOutcome::Failure { error } => {
          let failure = lifecycle_failure_from_attempt(error.into_attempt_failure());
          let code = failure.code.clone();
          (
            RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
              hook_invocation_id: hook_projection.hook_invocation_id.clone(),
              action_id: action.action_id.clone(),
              attempt: attempt_number,
              failure,
            }),
            LifecycleProgressPhase::ActionFailed,
            Some(code),
          )
        }
      },
      Err(error) => {
        *host = None;
        (
          RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
            failure: LifecycleFailure {
              kind: LifecycleFailureKind::HostCrashed,
              code: "WOML_SCRIPT_HOST_CRASHED".to_string(),
              message: error.to_string(),
            },
          }),
          LifecycleProgressPhase::ActionFailed,
          Some("WOML_SCRIPT_HOST_CRASHED".to_string()),
        )
      }
    };
    engine.append_payload(run_id, payload)?;
    report_lifecycle(
      options,
      engine.workflow(),
      run_id,
      phase,
      &hook.hook_id,
      &progress_action_id,
      matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
        .then_some(hook_projection.subject.id.as_str()),
      failure_code,
    );
  }
}
