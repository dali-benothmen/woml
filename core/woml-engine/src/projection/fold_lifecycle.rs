//! projection fold lifecycle.

use super::{
  FoldError, ForkJoinStatus, LifecycleActionProjection, LifecycleActionStatus,
  LifecycleHookProjection, LifecycleHookStatus, LifecycleStatus, ReusableLifecycleProjection,
  ReusableLifecycleStatus, RunProjection, RunStatus,
};
use crate::event::BusinessOutcome;
use crate::event::FinalLifecycleStatus;
use crate::event::LifecycleHookCompletionStatus;
use crate::event::RunOutcomeDecidedData;
use std::collections::BTreeMap;

pub(super) fn apply_lifecycle_hook_requested(
  projection: &mut RunProjection,
  data: &crate::event::LifecycleHookRequestedData,
) -> Result<(), FoldError> {
  if projection.run_id.is_none()
    || matches!(
      projection.status,
      RunStatus::NotStarted
        | RunStatus::Queued
        | RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
    )
    || projection
      .lifecycle_hooks
      .contains_key(&data.hook_invocation_id)
    || projection
      .lifecycle_hooks
      .values()
      .any(|hook| hook.hook_id == data.hook_id && hook.subject == data.subject)
  {
    return Err(FoldError::InvalidHistory(
      "Lifecycle hook request is duplicated or belongs to an inactive run.".to_string(),
    ));
  }
  projection.lifecycle_hooks.insert(
    data.hook_invocation_id.clone(),
    LifecycleHookProjection {
      hook_invocation_id: data.hook_invocation_id.clone(),
      hook_id: data.hook_id.clone(),
      event: data.event,
      subject: data.subject.clone(),
      status: LifecycleHookStatus::Requested,
      actions: BTreeMap::new(),
      failed_actions: 0,
    },
  );
  projection.lifecycle_status = LifecycleStatus::Running;
  Ok(())
}

pub(super) fn apply_lifecycle_action_attempt_started(
  projection: &mut RunProjection,
  data: &crate::event::LifecycleActionIdentityData,
) -> Result<(), FoldError> {
  let hook = projection
    .lifecycle_hooks
    .get_mut(&data.hook_invocation_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(
        "Lifecycle action started before its hook was requested.".to_string(),
      )
    })?;
  if matches!(
    hook.status,
    LifecycleHookStatus::Completed | LifecycleHookStatus::CompletedWithWarnings
  ) || hook.actions.contains_key(&data.action_id)
    || hook
      .actions
      .values()
      .any(|action| action.status == LifecycleActionStatus::Started)
  {
    return Err(FoldError::InvalidHistory(
      "A lifecycle hook may have only one active, uniquely identified action attempt.".to_string(),
    ));
  }
  hook.actions.insert(
    data.action_id.clone(),
    LifecycleActionProjection {
      action_id: data.action_id.clone(),
      attempt: data.attempt,
      status: LifecycleActionStatus::Started,
      failure: None,
    },
  );
  hook.status = LifecycleHookStatus::Running;
  Ok(())
}

pub(super) fn apply_lifecycle_action_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::LifecycleActionIdentityData,
) -> Result<(), FoldError> {
  let hook = projection
    .lifecycle_hooks
    .get_mut(&data.hook_invocation_id)
    .ok_or_else(|| FoldError::InvalidHistory("Unknown lifecycle hook.".to_string()))?;
  let action = hook.actions.get_mut(&data.action_id).ok_or_else(|| {
    FoldError::InvalidHistory("Lifecycle action succeeded before it started.".to_string())
  })?;
  if action.status != LifecycleActionStatus::Started || action.attempt != data.attempt {
    return Err(FoldError::InvalidHistory(
      "Lifecycle action success does not close its active attempt.".to_string(),
    ));
  }
  action.status = LifecycleActionStatus::Succeeded;
  Ok(())
}

pub(super) fn apply_lifecycle_action_failed(
  projection: &mut RunProjection,
  data: &crate::event::LifecycleActionFailedData,
) -> Result<(), FoldError> {
  let hook = projection
    .lifecycle_hooks
    .get_mut(&data.hook_invocation_id)
    .ok_or_else(|| FoldError::InvalidHistory("Unknown lifecycle hook.".to_string()))?;
  let action = hook.actions.get_mut(&data.action_id).ok_or_else(|| {
    FoldError::InvalidHistory("Lifecycle action failed before it started.".to_string())
  })?;
  if action.status != LifecycleActionStatus::Started || action.attempt != data.attempt {
    return Err(FoldError::InvalidHistory(
      "Lifecycle action failure does not close its active attempt.".to_string(),
    ));
  }
  action.status = LifecycleActionStatus::Failed;
  action.failure = Some(data.failure.clone());
  hook.failed_actions += 1;
  Ok(())
}

pub(super) fn apply_lifecycle_hook_completed(
  projection: &mut RunProjection,
  data: &crate::event::LifecycleHookCompletedData,
) -> Result<(), FoldError> {
  let hook = projection
    .lifecycle_hooks
    .get_mut(&data.hook_invocation_id)
    .ok_or_else(|| FoldError::InvalidHistory("Unknown lifecycle hook.".to_string()))?;
  if matches!(
    hook.status,
    LifecycleHookStatus::Completed | LifecycleHookStatus::CompletedWithWarnings
  ) || hook
    .actions
    .values()
    .any(|action| action.status == LifecycleActionStatus::Started)
    || hook.failed_actions != data.failed_actions
  {
    return Err(FoldError::InvalidHistory(
      "Lifecycle hook completion does not match its action outcomes.".to_string(),
    ));
  }
  hook.status = match data.status {
    LifecycleHookCompletionStatus::Completed => LifecycleHookStatus::Completed,
    LifecycleHookCompletionStatus::CompletedWithWarnings => {
      LifecycleHookStatus::CompletedWithWarnings
    }
  };
  Ok(())
}

pub(super) fn apply_reusable_lifecycle_requested(
  projection: &mut RunProjection,
  data: &crate::event::ReusableLifecycleRequestedData,
) -> Result<(), FoldError> {
  let key = format!("{}:{:?}", data.invocation_id, data.hook);
  if projection.reusable_lifecycle_hooks.contains_key(&key) {
    return Err(FoldError::InvalidHistory(
      "Reusable lifecycle hook was requested more than once.".to_string(),
    ));
  }
  projection.reusable_lifecycle_hooks.insert(
    key,
    ReusableLifecycleProjection {
      invocation_id: data.invocation_id.clone(),
      definition_digest: data.definition_digest.clone(),
      hook: data.hook,
      status: ReusableLifecycleStatus::Requested,
      active_action_id: None,
      completed_action_ids: Vec::new(),
      warning_codes: Vec::new(),
    },
  );
  Ok(())
}

pub(super) fn apply_reusable_lifecycle_action_started(
  projection: &mut RunProjection,
  data: &crate::event::ReusableLifecycleActionStartedData,
) -> Result<(), FoldError> {
  let key = format!("{}:{:?}", data.invocation_id, data.hook);
  let hook = projection
    .reusable_lifecycle_hooks
    .get_mut(&key)
    .ok_or_else(|| {
      FoldError::InvalidHistory(
        "Reusable lifecycle action started before its hook was requested.".to_string(),
      )
    })?;
  if hook.definition_digest != data.definition_digest
    || hook.active_action_id.is_some()
    || hook.completed_action_ids.contains(&data.action_id)
    || matches!(
      hook.status,
      ReusableLifecycleStatus::Completed | ReusableLifecycleStatus::CompletedWithWarnings
    )
  {
    return Err(FoldError::InvalidHistory(
      "Reusable lifecycle action has an invalid or duplicate identity.".to_string(),
    ));
  }
  hook.active_action_id = Some(data.action_id.clone());
  hook.status = ReusableLifecycleStatus::Running;
  Ok(())
}

pub(super) fn apply_reusable_lifecycle_action_succeeded(
  projection: &mut RunProjection,
  data: &crate::event::ReusableLifecycleActionSucceededData,
) -> Result<(), FoldError> {
  let key = format!("{}:{:?}", data.invocation_id, data.hook);
  let hook = projection
    .reusable_lifecycle_hooks
    .get_mut(&key)
    .ok_or_else(|| FoldError::InvalidHistory("Unknown reusable lifecycle hook.".to_string()))?;
  if hook.definition_digest != data.definition_digest
    || hook.active_action_id.as_deref() != Some(data.action_id.as_str())
  {
    return Err(FoldError::InvalidHistory(
      "Reusable lifecycle success does not close its active action.".to_string(),
    ));
  }
  hook.active_action_id = None;
  hook.completed_action_ids.push(data.action_id.clone());
  Ok(())
}

pub(super) fn apply_reusable_lifecycle_action_failed(
  projection: &mut RunProjection,
  data: &crate::event::ReusableLifecycleActionFailedData,
) -> Result<(), FoldError> {
  let key = format!("{}:{:?}", data.invocation_id, data.hook);
  let hook = projection
    .reusable_lifecycle_hooks
    .get_mut(&key)
    .ok_or_else(|| FoldError::InvalidHistory("Unknown reusable lifecycle hook.".to_string()))?;
  if hook.definition_digest != data.definition_digest
    || hook
      .active_action_id
      .as_deref()
      .is_some_and(|active| active != data.action_id)
    || hook.completed_action_ids.contains(&data.action_id)
  {
    return Err(FoldError::InvalidHistory(
      "Reusable lifecycle failure does not close its active action.".to_string(),
    ));
  }
  hook.active_action_id = None;
  hook.completed_action_ids.push(data.action_id.clone());
  hook.warning_codes.push(data.warning_code.clone());
  Ok(())
}

pub(super) fn apply_reusable_lifecycle_completed(
  projection: &mut RunProjection,
  data: &crate::event::ReusableLifecycleCompletedData,
) -> Result<(), FoldError> {
  let key = format!("{}:{:?}", data.invocation_id, data.hook);
  let hook = projection
    .reusable_lifecycle_hooks
    .get_mut(&key)
    .ok_or_else(|| FoldError::InvalidHistory("Unknown reusable lifecycle hook.".to_string()))?;
  if hook.definition_digest != data.definition_digest || hook.active_action_id.is_some() {
    return Err(FoldError::InvalidHistory(
      "Reusable lifecycle completion does not match its hook.".to_string(),
    ));
  }
  hook.status = if data.outcome == crate::ReusableLifecycleOutcome::CompletedWithWarnings {
    ReusableLifecycleStatus::CompletedWithWarnings
  } else {
    ReusableLifecycleStatus::Completed
  };
  Ok(())
}

pub(super) fn apply_run_outcome_decided(
  projection: &mut RunProjection,
  data: &crate::event::RunOutcomeDecidedData,
) -> Result<(), FoldError> {
  if projection.business_outcome.is_some()
    || !matches!(
      projection.status,
      RunStatus::Running | RunStatus::Waiting | RunStatus::Cancelling
    )
  {
    return Err(FoldError::InvalidHistory(
      "A run business outcome may be decided exactly once.".to_string(),
    ));
  }
  if projection.forks.values().any(|fork| {
    fork.join_status == ForkJoinStatus::Pending
      || fork
        .branches
        .values()
        .any(|branch| branch.outcome.is_none())
  }) {
    return Err(FoldError::InvalidHistory(
      "A run outcome cannot be decided while opened fork work remains unsettled.".to_string(),
    ));
  }
  match data {
    RunOutcomeDecidedData::Succeeded { result } => {
      if projection.status != RunStatus::Running {
        return Err(FoldError::InvalidHistory(
          "A successful outcome requires a running workflow.".to_string(),
        ));
      }
      projection.result = Some(result.clone());
    }
    RunOutcomeDecidedData::Failed { failure } => {
      projection.lifecycle_failure = Some(failure.clone());
    }
    RunOutcomeDecidedData::Cancelled {
      cancellation_request_id,
    } => {
      if projection.cancellation_request_id.as_deref() != Some(cancellation_request_id.as_str()) {
        return Err(FoldError::InvalidHistory(
          "Cancelled outcome does not match the durable cancellation request.".to_string(),
        ));
      }
    }
  }
  projection.business_outcome = Some(data.outcome());
  projection.lifecycle_status = LifecycleStatus::Finalizing;
  projection.status = RunStatus::Finalizing;
  Ok(())
}

pub(super) fn apply_run_finalized(
  projection: &mut RunProjection,
  data: &crate::event::RunFinalizedData,
) -> Result<(), FoldError> {
  let failed_actions = projection
    .lifecycle_hooks
    .values()
    .flat_map(|hook| {
      hook.actions.values().filter_map(|action| {
        action.failure.as_ref().map(|failure| {
          (
            hook.hook_id.as_str(),
            action.action_id.as_str(),
            matches!(hook.subject.kind, crate::event::LifecycleSubjectKind::Step)
              .then_some(hook.subject.id.as_str()),
            failure.code.as_str(),
          )
        })
      })
    })
    .collect::<Vec<_>>();
  let warnings_match = failed_actions.len() == data.warnings.len()
    && failed_actions
      .iter()
      .all(|(hook_id, action_id, step_id, code)| {
        data.warnings.iter().any(|warning| {
          warning.hook_id == *hook_id
            && warning.action_id == *action_id
            && warning.step_id.as_deref() == *step_id
            && warning.code == *code
        })
      });
  if projection.status != RunStatus::Finalizing
    || projection.business_outcome != Some(data.outcome)
    || !warnings_match
    || projection.lifecycle_hooks.values().any(|hook| {
      matches!(
        hook.status,
        LifecycleHookStatus::Requested | LifecycleHookStatus::Running
      )
    })
  {
    return Err(FoldError::InvalidHistory(
      "run_finalized requires the decided outcome and no unfinished lifecycle hook.".to_string(),
    ));
  }
  projection.lifecycle_warnings = data.warnings.clone();
  projection.lifecycle_status = match data.lifecycle_status {
    FinalLifecycleStatus::Completed => LifecycleStatus::Completed,
    FinalLifecycleStatus::CompletedWithWarnings => LifecycleStatus::CompletedWithWarnings,
  };
  projection.status = match data.outcome {
    BusinessOutcome::Succeeded => RunStatus::Succeeded,
    BusinessOutcome::Failed => RunStatus::Failed,
    BusinessOutcome::Cancelled => RunStatus::Cancelled,
  };
  Ok(())
}
