//! Shared historical outcome/lifecycle expansion and warning reconstruction.
//!
//! Both in-memory and durable adapters use these semantics. This is not a
//! database-only translation or permission to reinterpret retained event bytes.

use crate::durable::{derive_lifecycle_hook_invocation_id, DurableStoreError};
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV2;
use crate::event::RunFailedDataV3;
use crate::event::RunFailedDataV4;
use crate::event::RunFailedDataV5;
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RunProjection;

pub(crate) fn lifecycle_warnings_from_projection(
  projection: &RunProjection,
) -> Vec<crate::event::LifecycleWarning> {
  projection
    .lifecycle_hooks
    .values()
    .flat_map(|hook| {
      hook.actions.values().filter_map(|action| {
        let failure = action.failure.as_ref()?;
        Some(crate::event::LifecycleWarning {
          hook_id: hook.hook_id.clone(),
          action_id: action.action_id.clone(),
          step_id: matches!(hook.subject.kind, LifecycleSubjectKind::Step)
            .then(|| hook.subject.id.clone()),
          provider: None,
          destination: None,
          code: failure.code.clone(),
        })
      })
    })
    .collect()
}

pub(in crate::durable) fn lifecycle_failure_from_attempt(
  failure: &AttemptFailure,
) -> crate::event::LifecycleFailure {
  let kind = match failure.kind {
    AttemptFailureKind::ScriptThrew => crate::event::LifecycleFailureKind::ScriptThrew,
    AttemptFailureKind::ScriptTimedOut => crate::event::LifecycleFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => crate::event::LifecycleFailureKind::NonJson,
    AttemptFailureKind::ContextTooLarge | AttemptFailureKind::ResultTooLarge => {
      crate::event::LifecycleFailureKind::SizeLimitExceeded
    }
    AttemptFailureKind::WorkerCrashed => crate::event::LifecycleFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => crate::event::LifecycleFailureKind::HostCrashed,
    AttemptFailureKind::InvocationCancelled => crate::event::LifecycleFailureKind::Cancelled,
    AttemptFailureKind::Interrupted => crate::event::LifecycleFailureKind::Interrupted,
    AttemptFailureKind::ServiceFailed => crate::event::LifecycleFailureKind::ProviderFailed,
  };
  crate::event::LifecycleFailure {
    kind,
    code: failure.code.clone(),
    message: failure.message.clone(),
  }
}

fn lifecycle_failure_from_legacy(data: &RunFailedData) -> crate::event::LifecycleFailure {
  match data {
    RunFailedData::V1(data) => lifecycle_failure_from_attempt(&data.failure),
    RunFailedData::V2(RunFailedDataV2::Attempt { failure, .. }) => {
      lifecycle_failure_from_attempt(failure)
    }
    RunFailedData::V2(RunFailedDataV2::Branch { failure, .. }) => {
      let message = match failure {
        crate::event::BranchFailure::BranchTestNotBoolean { message, .. }
        | crate::event::BranchFailure::ReferenceNotAvailable { message, .. }
        | crate::event::BranchFailure::BranchSelectionInvalid { message, .. } => message.clone(),
      };
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::ProviderFailed,
        code: failure.code().to_string(),
        message,
      }
    }
    RunFailedData::V3(RunFailedDataV3::Parallel { failure, .. }) => {
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::ProviderFailed,
        code: failure.code.clone(),
        message: failure.message.clone(),
      }
    }
    RunFailedData::V4(RunFailedDataV4::Approval { failure, .. }) => {
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::TimedOut,
        code: failure.code.clone(),
        message: failure.message.clone(),
      }
    }
    RunFailedData::V5(RunFailedDataV5::Notification { failure, .. }) => {
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::ProviderFailed,
        code: failure.code.clone(),
        message: failure.message.clone(),
      }
    }
  }
}

pub(crate) fn expand_model_v11_payload(
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  payload: RunEventPayload,
) -> Result<Vec<RunEventPayload>, DurableStoreError> {
  fn step_hook_request(
    workflow: &CompiledWorkflowDefinition,
    run_id: &str,
    event: crate::model::LifecycleEventName,
    step_id: &str,
  ) -> Option<RunEventPayload> {
    let hook = workflow.lifecycle_hook_for_step_event(event, step_id)?;
    let subject = LifecycleSubject {
      kind: LifecycleSubjectKind::Step,
      id: step_id.to_string(),
    };
    Some(RunEventPayload::LifecycleHookRequested(
      LifecycleHookRequestedData {
        hook_invocation_id: derive_lifecycle_hook_invocation_id(
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

  let payload = if matches!(
    &payload,
    RunEventPayload::RunStarted(_) | RunEventPayload::RunExecutionStarted(_)
  ) {
    let mut payloads = vec![payload];
    if let Some(hook) =
      workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunStart)
    {
      let subject = LifecycleSubject {
        kind: LifecycleSubjectKind::Workflow,
        id: run_id.to_string(),
      };
      payloads.push(RunEventPayload::LifecycleHookRequested(
        LifecycleHookRequestedData {
          hook_invocation_id: derive_lifecycle_hook_invocation_id(
            run_id,
            &hook.hook_id,
            subject.kind,
            &subject.id,
          ),
          hook_id: hook.hook_id.clone(),
          event: hook.event,
          subject,
        },
      ));
    }
    return Ok(payloads);
  } else {
    payload
  };
  let payload = match payload {
    RunEventPayload::StepAttemptStarted(data) if data.attempt == 1 => {
      let request = step_hook_request(
        workflow,
        run_id,
        crate::model::LifecycleEventName::StepStart,
        &data.node_id,
      );
      let mut payloads = vec![RunEventPayload::StepAttemptStarted(data)];
      payloads.extend(request);
      return Ok(payloads);
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      let request = step_hook_request(
        workflow,
        run_id,
        crate::model::LifecycleEventName::StepSuccess,
        &data.node_id,
      )
      .or_else(|| {
        step_hook_request(
          workflow,
          run_id,
          crate::model::LifecycleEventName::StepComplete,
          &data.node_id,
        )
      });
      let mut payloads = vec![RunEventPayload::StepAttemptSucceeded(data)];
      payloads.extend(request);
      return Ok(payloads);
    }
    RunEventPayload::StepAttemptFailed(data) => {
      let retryable = workflow
        .node(&data.node_id)
        .and_then(|node| node.retry_policy.as_ref())
        .is_some_and(|policy| {
          data.failure.kind == AttemptFailureKind::ScriptThrew && data.attempt < policy.max_attempts
        });
      if retryable {
        return Ok(vec![RunEventPayload::StepAttemptFailed(data)]);
      }
      let event = if data.failure.kind == AttemptFailureKind::InvocationCancelled {
        crate::model::LifecycleEventName::StepComplete
      } else {
        crate::model::LifecycleEventName::StepFailure
      };
      let request = step_hook_request(workflow, run_id, event, &data.node_id).or_else(|| {
        (event != crate::model::LifecycleEventName::StepComplete)
          .then(|| {
            step_hook_request(
              workflow,
              run_id,
              crate::model::LifecycleEventName::StepComplete,
              &data.node_id,
            )
          })
          .flatten()
      });
      let mut payloads = vec![RunEventPayload::StepAttemptFailed(data)];
      payloads.extend(request);
      return Ok(payloads);
    }
    payload => payload,
  };
  let outcome = match payload {
    RunEventPayload::RunSucceeded(data) => crate::event::RunOutcomeDecidedData::Succeeded {
      result: data.result,
    },
    RunEventPayload::RunFailed(data) => crate::event::RunOutcomeDecidedData::Failed {
      failure: lifecycle_failure_from_legacy(&data),
    },
    payload => return Ok(vec![payload]),
  };
  let business_outcome = outcome.outcome();
  let lifecycle_event = match business_outcome {
    crate::event::BusinessOutcome::Succeeded => crate::model::LifecycleEventName::RunSuccess,
    crate::event::BusinessOutcome::Failed => crate::model::LifecycleEventName::RunFailure,
    crate::event::BusinessOutcome::Cancelled => crate::model::LifecycleEventName::RunCancel,
  };
  let mut payloads = vec![RunEventPayload::RunOutcomeDecided(outcome)];
  let next_hook = workflow
    .lifecycle_hook_for_event(lifecycle_event)
    .or_else(|| workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete));
  if let Some(hook) = next_hook {
    let subject = LifecycleSubject {
      kind: LifecycleSubjectKind::Workflow,
      id: run_id.to_string(),
    };
    payloads.push(RunEventPayload::LifecycleHookRequested(
      LifecycleHookRequestedData {
        hook_invocation_id: derive_lifecycle_hook_invocation_id(
          run_id,
          &hook.hook_id,
          subject.kind,
          &subject.id,
        ),
        hook_id: hook.hook_id.clone(),
        event: hook.event,
        subject,
      },
    ));
  } else if workflow.lifecycle.is_none() {
    payloads.push(RunEventPayload::RunFinalized(
      crate::event::RunFinalizedData {
        outcome: business_outcome,
        lifecycle_status: crate::event::FinalLifecycleStatus::Completed,
        warnings: Vec::new(),
      },
    ));
  }
  Ok(payloads)
}
