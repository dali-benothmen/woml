//! Step lifecycle subject snapshots and supported outcome classification.

use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::model::LifecycleEventName;
use crate::projection::AttemptStatus;
use crate::AttemptFailureKind;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RunProjection;

pub(in crate::runtime) fn step_lifecycle_request(
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  event: LifecycleEventName,
  step_id: &str,
) -> Option<RunEventPayload> {
  let hook = workflow.lifecycle_hook_for_step_event(event, step_id)?;
  let subject = LifecycleSubject {
    kind: LifecycleSubjectKind::Step,
    id: step_id.to_string(),
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

pub(in crate::runtime) fn step_lifecycle_outcome(
  event: LifecycleEventName,
  projection: &RunProjection,
  step_id: &str,
) -> Option<crate::BusinessOutcome> {
  match event {
    LifecycleEventName::StepStart => None,
    LifecycleEventName::StepSuccess => Some(crate::BusinessOutcome::Succeeded),
    LifecycleEventName::StepFailure => Some(crate::BusinessOutcome::Failed),
    LifecycleEventName::StepComplete => {
      if projection.context.steps.contains_key(step_id) {
        Some(crate::BusinessOutcome::Succeeded)
      } else {
        projection.latest_attempt(step_id).and_then(|attempt| {
          let AttemptStatus::Failed { failure } = &attempt.status else {
            return None;
          };
          Some(if failure.kind == AttemptFailureKind::InvocationCancelled {
            crate::BusinessOutcome::Cancelled
          } else {
            crate::BusinessOutcome::Failed
          })
        })
      }
    }
    _ => None,
  }
}
