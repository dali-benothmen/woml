//! event validation.

use super::{
  is_approval_request_id, is_definition_hash, is_notification_delivery_id, valid_branch_arm_id,
  valid_fencing_token, valid_id, valid_lifecycle_action_identity, valid_lifecycle_failure,
  valid_lock_key, valid_ordered_id_lists, valid_prefixed_id, valid_provider_message,
  valid_public_structural_id, valid_remote_public_id, valid_rich_approval_identity,
  valid_rich_approval_roles, valid_sha256, valid_signal_name, valid_trigger_handler,
  validate_for_each_iteration_event, validate_identity, validate_notification_identity,
  validate_operation_base, ApprovalDecision, AttemptFailureKind, BranchFailure, DurableWaitKind,
  EventValidationError, FinalLifecycleStatus, ForkJoinOutcome, LifecycleHookCompletionStatus,
  LifecycleSubjectKind, ParallelGroupOutcome, ReusableLifecycleOutcome,
  RichApprovalResolutionReason, RunEvent, RunEventPayload, RunFailedData, RunFailedDataV2,
  RunFailedDataV3, RunFailedDataV4, RunFailedDataV5, RunIngress, RunOutcomeDecidedData,
};
use crate::RUN_EVENT_SCHEMA_VERSION_V1;
use crate::RUN_EVENT_SCHEMA_VERSION_V10;
use crate::RUN_EVENT_SCHEMA_VERSION_V11;
use crate::RUN_EVENT_SCHEMA_VERSION_V12;
use crate::RUN_EVENT_SCHEMA_VERSION_V13;
use crate::RUN_EVENT_SCHEMA_VERSION_V14;
use crate::RUN_EVENT_SCHEMA_VERSION_V15;
use crate::RUN_EVENT_SCHEMA_VERSION_V16;
use crate::RUN_EVENT_SCHEMA_VERSION_V17;
use crate::RUN_EVENT_SCHEMA_VERSION_V18;
use crate::RUN_EVENT_SCHEMA_VERSION_V19;
use crate::RUN_EVENT_SCHEMA_VERSION_V2;
use crate::RUN_EVENT_SCHEMA_VERSION_V20;
use crate::RUN_EVENT_SCHEMA_VERSION_V3;
use crate::RUN_EVENT_SCHEMA_VERSION_V4;
use crate::RUN_EVENT_SCHEMA_VERSION_V5;
use crate::RUN_EVENT_SCHEMA_VERSION_V6;
use crate::RUN_EVENT_SCHEMA_VERSION_V7;
use crate::RUN_EVENT_SCHEMA_VERSION_V8;
use crate::RUN_EVENT_SCHEMA_VERSION_V9;

impl RunEvent {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if !matches!(
      self.event_schema_version,
      RUN_EVENT_SCHEMA_VERSION_V1
        | RUN_EVENT_SCHEMA_VERSION_V2
        | RUN_EVENT_SCHEMA_VERSION_V3
        | RUN_EVENT_SCHEMA_VERSION_V4
        | RUN_EVENT_SCHEMA_VERSION_V5
        | RUN_EVENT_SCHEMA_VERSION_V6
        | RUN_EVENT_SCHEMA_VERSION_V7
        | RUN_EVENT_SCHEMA_VERSION_V8
        | RUN_EVENT_SCHEMA_VERSION_V9
        | RUN_EVENT_SCHEMA_VERSION_V10
        | RUN_EVENT_SCHEMA_VERSION_V11
        | RUN_EVENT_SCHEMA_VERSION_V12
        | RUN_EVENT_SCHEMA_VERSION_V13
        | RUN_EVENT_SCHEMA_VERSION_V14
        | RUN_EVENT_SCHEMA_VERSION_V15
        | RUN_EVENT_SCHEMA_VERSION_V16
        | RUN_EVENT_SCHEMA_VERSION_V17
        | RUN_EVENT_SCHEMA_VERSION_V18
        | RUN_EVENT_SCHEMA_VERSION_V19
        | RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      return Err(EventValidationError::UnsupportedSchemaVersion(
        self.event_schema_version,
      ));
    }
    if !valid_id(&self.event_id) || !valid_id(&self.run_id) || self.sequence == 0 {
      return Err(EventValidationError::Invalid(
        "Run events require valid eventId, runId, and sequence >= 1.".to_string(),
      ));
    }
    let loop_event = matches!(
      self.payload,
      RunEventPayload::ForEachOpened(_)
        | RunEventPayload::ForEachIterationStarted(_)
        | RunEventPayload::ForEachIterationSucceeded(_)
        | RunEventPayload::ForEachIterationFailed(_)
        | RunEventPayload::ForEachIterationSkipped(_)
        | RunEventPayload::ForEachSucceeded(_)
        | RunEventPayload::ForEachFailed(_)
        | RunEventPayload::ForEachCancelled(_)
    );
    let wait_event = matches!(
      self.payload,
      RunEventPayload::WaitRegistered(_)
        | RunEventPayload::ExternalSignalReceived(_)
        | RunEventPayload::WaitSatisfied(_)
        | RunEventPayload::WaitTimedOut(_)
        | RunEventPayload::WaitCancelled(_)
    );
    let lock_event = matches!(
      self.payload,
      RunEventPayload::LockWaitRegistered(_)
        | RunEventPayload::LockAcquired(_)
        | RunEventPayload::LockReleased(_)
        | RunEventPayload::LockAcquireTimedOut(_)
        | RunEventPayload::LockAcquisitionCancelled(_)
        | RunEventPayload::LockLeaseLost(_)
    );
    let correction_event = matches!(
      self.payload,
      RunEventPayload::SagaStarted(_)
        | RunEventPayload::CompensationRegistered(_)
        | RunEventPayload::SagaForwardSucceeded(_)
        | RunEventPayload::SagaCorrectionStarted(_)
        | RunEventPayload::CorrectionAttemptStarted(_)
        | RunEventPayload::CorrectionAttemptSucceeded(_)
        | RunEventPayload::CorrectionAttemptFailed(_)
        | RunEventPayload::CorrectionRetryScheduled(_)
        | RunEventPayload::CorrectionInterrupted(_)
        | RunEventPayload::SagaCorrectionCompleted(_)
        | RunEventPayload::ManualRecoveryRequired(_)
    );
    let rich_approval_event = matches!(
      self.payload,
      RunEventPayload::ApprovalReviewRequested(_)
        | RunEventPayload::ApprovalReviewerDecisionRecorded(_)
        | RunEventPayload::ApprovalQuorumProgressed(_)
        | RunEventPayload::ApprovalInvalidated(_)
    ) || matches!(
      &self.payload,
      RunEventPayload::ApprovalResolved(data) if data.is_rich()
    );
    let remote_dispatch_event =
      matches!(self.payload, RunEventPayload::RemoteDispatchTransitioned(_));
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V20 && !remote_dispatch_event {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V19;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V20
      && self.iteration.as_ref().is_some_and(|scope| {
        !valid_public_structural_id(&scope.for_each_id) || scope.index >= 10_000
      })
    {
      return Err(EventValidationError::Invalid(
        "Event v20 remote dispatch iteration scope is invalid.".to_string(),
      ));
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V19 && !rich_approval_event {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V18;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V19 && self.iteration.is_some() {
      return Err(EventValidationError::Invalid(
        "Rich approval events cannot carry an iteration scope in Event v19.".to_string(),
      ));
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V18 && !correction_event {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V17;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V18
      && self.iteration.is_some()
      && !matches!(self.payload, RunEventPayload::CompensationRegistered(_))
    {
      return Err(EventValidationError::Invalid(
        "Only compensation_registered may carry an iteration scope in Event v18.".to_string(),
      ));
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V17 && !lock_event {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V16;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V17
      && self.iteration.as_ref().is_some_and(|scope| {
        !valid_public_structural_id(&scope.for_each_id) || scope.index >= 10_000
      })
    {
      return Err(EventValidationError::Invalid(
        "Event v17 lock iteration scope is invalid.".to_string(),
      ));
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V16 && !wait_event {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V15;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V16 {
      if self.iteration.as_ref().is_some_and(|scope| {
        !valid_public_structural_id(&scope.for_each_id) || scope.index >= 10_000
      }) {
        return Err(EventValidationError::Invalid(
          "Event v16 durable-wait iteration scope is invalid.".to_string(),
        ));
      }
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V15 && !loop_event {
      if let Some(scope) = &self.iteration {
        let scoped_work = matches!(
          self.payload,
          RunEventPayload::StepAttemptStarted(_)
            | RunEventPayload::StepAttemptSucceeded(_)
            | RunEventPayload::StepAttemptFailed(_)
            | RunEventPayload::StepRetryScheduled(_)
            | RunEventPayload::BranchSelected(_)
            | RunEventPayload::ChoiceSelected(_)
            | RunEventPayload::ParallelGroupStarted(_)
            | RunEventPayload::ParallelGroupCompleted(_)
            | RunEventPayload::OperationStarted(_)
            | RunEventPayload::OperationSucceeded(_)
            | RunEventPayload::OperationFailed(_)
            | RunEventPayload::LifecycleHookRequested(_)
            | RunEventPayload::LifecycleActionAttemptStarted(_)
            | RunEventPayload::LifecycleActionSucceeded(_)
            | RunEventPayload::LifecycleActionFailed(_)
            | RunEventPayload::LifecycleHookCompleted(_)
            | RunEventPayload::ReusableLifecycleRequested(_)
            | RunEventPayload::ReusableLifecycleActionStarted(_)
            | RunEventPayload::ReusableLifecycleActionSucceeded(_)
            | RunEventPayload::ReusableLifecycleActionFailed(_)
            | RunEventPayload::ReusableLifecycleCompleted(_)
        );
        if !scoped_work || !valid_public_structural_id(&scope.for_each_id) || scope.index >= 10_000
        {
          return Err(EventValidationError::Invalid(
            "Event v15 iteration scope is invalid for this loop-owned payload.".to_string(),
          ));
        }
      }
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V14;
      inherited.iteration = None;
      return inherited.validate();
    }
    if !matches!(
      self.event_schema_version,
      RUN_EVENT_SCHEMA_VERSION_V15
        | RUN_EVENT_SCHEMA_VERSION_V16
        | RUN_EVENT_SCHEMA_VERSION_V17
        | RUN_EVENT_SCHEMA_VERSION_V18
        | RUN_EVENT_SCHEMA_VERSION_V20
    ) && self.iteration.is_some()
    {
      return Err(EventValidationError::Invalid(
        "Iteration scope is available only in Event v15 or a compatible later schema.".to_string(),
      ));
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V14 {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V13;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V13
      && !matches!(
        self.payload,
        RunEventPayload::ReusableLifecycleRequested(_)
          | RunEventPayload::ReusableLifecycleActionStarted(_)
          | RunEventPayload::ReusableLifecycleActionSucceeded(_)
          | RunEventPayload::ReusableLifecycleActionFailed(_)
          | RunEventPayload::ReusableLifecycleCompleted(_)
      )
    {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V12;
      return inherited.validate();
    }
    if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V12
      && !matches!(
        self.payload,
        RunEventPayload::ChoiceSelected(_)
          | RunEventPayload::ForkOpened(_)
          | RunEventPayload::ForkBranchSettled(_)
          | RunEventPayload::ForkJoinSettled(_)
      )
    {
      let mut inherited = self.clone();
      inherited.event_schema_version = RUN_EVENT_SCHEMA_VERSION_V11;
      return inherited.validate();
    }
    match &self.payload {
      RunEventPayload::RemoteDispatchTransitioned(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
        let meaningful = !matches!(data.state, crate::RemoteDispatchStateV1::Offered);
        let bound = !matches!(data.state, crate::RemoteDispatchStateV1::Requested);
        let terminal = matches!(
          data.state,
          crate::RemoteDispatchStateV1::Succeeded
            | crate::RemoteDispatchStateV1::Failed
            | crate::RemoteDispatchStateV1::Cancelled
        );
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V20
          || !data.dispatch_id.starts_with("dispatch_")
          || !meaningful
          || data.dispatch_id.len() != 73
          || !data.dispatch_id[9..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
          || !valid_remote_public_id(&data.target)
          || data.attempt > 10
          || bound != data.worker_id.is_some()
          || data
            .worker_id
            .as_deref()
            .is_some_and(|value| !valid_remote_public_id(value))
          || terminal != data.terminal_digest.is_some()
          || data
            .terminal_digest
            .as_deref()
            .is_some_and(|value| !valid_sha256(value))
        {
          return Err(EventValidationError::Invalid(
            "remote_dispatch_transitioned does not match the Event v20 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::RunAdmitted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V11
          || !is_definition_hash(&data.definition_hash)
          || !is_definition_hash(&data.policy_hash)
          || !valid_id(&data.trigger.id)
          || !valid_trigger_handler(&data.trigger.handler)
          || data.queue.name.is_empty()
          || data.queue.name.len() > 128
          || data.admitted_at != self.occurred_at
        {
          return Err(EventValidationError::Invalid(
            "run_admitted does not match the Event v11 admission contract.".to_string(),
          ));
        }
      }
      RunEventPayload::RunExecutionStarted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V11
          || data.started_at != self.occurred_at
          || data
            .timeout_at
            .is_some_and(|deadline| deadline <= data.started_at)
        {
          return Err(EventValidationError::Invalid(
            "run_execution_started does not match the Event v11 start contract.".to_string(),
          ));
        }
      }
      RunEventPayload::RunTimeoutReached(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V11
          || data.deadline_at != self.occurred_at
          || data.code != "WOML_WORKFLOW_TIMED_OUT"
        {
          return Err(EventValidationError::Invalid(
            "run_timeout_reached does not match the Event v11 timeout contract.".to_string(),
          ));
        }
      }
      RunEventPayload::RunStarted(data) => {
        if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V11 {
          return Err(EventValidationError::Invalid(
            "Event v11 starts with run_admitted, not run_started.".to_string(),
          ));
        }
        if !valid_id(&data.workflow_id) || !is_definition_hash(&data.definition_hash) {
          return Err(EventValidationError::Invalid(
            "run_started requires a valid workflowId and definitionHash.".to_string(),
          ));
        }
        let trigger_identity = (
          data.trigger_id.as_deref(),
          data.trigger_handler.as_deref(),
          data.trigger_occurrence_id.as_deref(),
        );
        if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V9 {
          let valid_workflow_call = trigger_identity == (None, None, None)
            && matches!(
              data.ingress.as_ref(),
              Some(RunIngress::WorkflowCall { call_key }) if is_definition_hash(call_key)
            );
          if !valid_workflow_call {
            return Err(EventValidationError::Invalid(
              "run_started v9 requires one workflow_call ingress identity and no trigger identity."
                .to_string(),
            ));
          }
        } else if self.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V10 {
          let valid_workflow_call = trigger_identity == (None, None, None)
            && matches!(
              data.ingress.as_ref(),
              Some(RunIngress::WorkflowCall { call_key }) if is_definition_hash(call_key)
            );
          let valid_source_trigger = matches!(
            trigger_identity,
            (Some(trigger_id), Some(handler), Some(occurrence_id))
              if valid_id(trigger_id)
                && valid_trigger_handler(handler)
                && valid_id(occurrence_id)
          ) && data.ingress.is_none();
          if !valid_workflow_call && !valid_source_trigger {
            return Err(EventValidationError::Invalid(
              "run_started v10 requires either a source-trigger identity or workflow_call ingress."
                .to_string(),
            ));
          }
        } else if self.event_schema_version >= RUN_EVENT_SCHEMA_VERSION_V7 {
          if !matches!(
            trigger_identity,
            (Some(trigger_id), Some(handler), Some(occurrence_id))
              if valid_id(trigger_id)
                && valid_trigger_handler(handler)
                && valid_id(occurrence_id)
          ) {
            return Err(EventValidationError::Invalid(
              "run_started v7 requires triggerId, triggerHandler, and triggerOccurrenceId."
                .to_string(),
            ));
          }
          if data.ingress.is_some() {
            return Err(EventValidationError::Invalid(
              "run_started ingress identity is available only in event schema v9.".to_string(),
            ));
          }
        } else if trigger_identity != (None, None, None) || data.ingress.is_some() {
          return Err(EventValidationError::Invalid(
            "run_started trigger occurrence identity is available only in event schema v7."
              .to_string(),
          ));
        }
      }
      RunEventPayload::StepAttemptStarted(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
        let valid_v6_contract = if self.event_schema_version >= RUN_EVENT_SCHEMA_VERSION_V6 {
          data.attempt <= 10 && data.idempotency_key.as_deref().is_some_and(valid_sha256)
        } else {
          data.idempotency_key.is_none()
        };
        if data.handler.is_empty() || data.handler.chars().count() > 256 || !valid_v6_contract {
          return Err(EventValidationError::Invalid(
            "step_attempt_started requires a valid handler.".to_string(),
          ));
        }
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
        if self.event_schema_version >= RUN_EVENT_SCHEMA_VERSION_V6 && data.attempt > 10 {
          return Err(EventValidationError::Invalid(
            "Model v6 attempts must not exceed 10.".to_string(),
          ));
        }
      }
      RunEventPayload::StepAttemptFailed(data) => {
        validate_identity(&data.node_id, data.attempt, &data.invocation_id)?;
        data.failure.validate()?;
        if self.event_schema_version >= RUN_EVENT_SCHEMA_VERSION_V6 && data.attempt > 10 {
          return Err(EventValidationError::Invalid(
            "Model v6 attempts must not exceed 10.".to_string(),
          ));
        }
        if data.failure.kind == AttemptFailureKind::InvocationCancelled
          && !matches!(
            self.event_schema_version,
            RUN_EVENT_SCHEMA_VERSION_V3
              | RUN_EVENT_SCHEMA_VERSION_V4
              | RUN_EVENT_SCHEMA_VERSION_V5
              | RUN_EVENT_SCHEMA_VERSION_V6
              | RUN_EVENT_SCHEMA_VERSION_V7
              | RUN_EVENT_SCHEMA_VERSION_V8
              | RUN_EVENT_SCHEMA_VERSION_V9
              | RUN_EVENT_SCHEMA_VERSION_V10
              | RUN_EVENT_SCHEMA_VERSION_V11
          )
        {
          return Err(EventValidationError::Invalid(
            "invocation_cancelled is available only in run-event schema v3 or later.".to_string(),
          ));
        }
      }
      RunEventPayload::StepRetryScheduled(data) => {
        if self.event_schema_version < RUN_EVENT_SCHEMA_VERSION_V6
          || !valid_id(&data.node_id)
          || !(1..=9).contains(&data.failed_attempt)
          || data.next_attempt != data.failed_attempt + 1
          || data.next_attempt > 10
          || data.scheduled_at < self.occurred_at
        {
          return Err(EventValidationError::Invalid(
            "step_retry_scheduled has an invalid v6 retry identity or schedule.".to_string(),
          ));
        }
      }
      RunEventPayload::BranchSelected(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V2
            | RUN_EVENT_SCHEMA_VERSION_V3
            | RUN_EVENT_SCHEMA_VERSION_V4
            | RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) {
          return Err(EventValidationError::Invalid(
            "branch_selected is available only in run-event schema v2 or later.".to_string(),
          ));
        }
        if !valid_public_structural_id(&data.branch_id)
          || !valid_branch_arm_id(&data.branch_id, &data.arm_id)
        {
          return Err(EventValidationError::Invalid(
            "branch_selected requires a valid branchId and matching canonical armId.".to_string(),
          ));
        }
      }
      RunEventPayload::ChoiceSelected(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V12
          || !valid_id(&data.choice_id)
          || !data.arm_id.starts_with(&format!("{}:", data.choice_id))
          || !valid_id(&data.arm_id)
        {
          return Err(EventValidationError::Invalid(
            "choice_selected requires Event v12 and one canonical choice arm identity.".to_string(),
          ));
        }
      }
      RunEventPayload::ForkOpened(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V12
          || !valid_public_structural_id(&data.fork_id)
        {
          return Err(EventValidationError::Invalid(
            "fork_opened requires Event v12 and a valid forkId.".to_string(),
          ));
        }
      }
      RunEventPayload::ForkBranchSettled(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V12
          || !valid_public_structural_id(&data.fork_id)
          || !valid_public_structural_id(&data.branch_id)
          || !valid_id(&data.terminal_node_id)
        {
          return Err(EventValidationError::Invalid(
            "fork_branch_settled requires valid Event v12 fork, branch, terminal, and outcome fields."
              .to_string(),
          ));
        }
      }
      RunEventPayload::ForkJoinSettled(data) => {
        let blocking_valid = match (data.outcome, data.blocking_branch_id.as_deref()) {
          (ForkJoinOutcome::Succeeded, None) => true,
          (ForkJoinOutcome::Failed | ForkJoinOutcome::Cancelled, Some(branch_id)) => {
            valid_public_structural_id(branch_id)
          }
          _ => false,
        };
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V12
          || !valid_public_structural_id(&data.fork_id)
          || !blocking_valid
        {
          return Err(EventValidationError::Invalid(
            "fork_join_settled requires one closed Event v12 outcome shape.".to_string(),
          ));
        }
      }
      RunEventPayload::ParallelGroupStarted(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V3
            | RUN_EVENT_SCHEMA_VERSION_V4
            | RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_public_structural_id(&data.parallel_id)
        {
          return Err(EventValidationError::Invalid(
            "parallel_group_started requires event schema v3 and a valid parallelId.".to_string(),
          ));
        }
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V3
            | RUN_EVENT_SCHEMA_VERSION_V4
            | RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_public_structural_id(&data.parallel_id)
          || !valid_ordered_id_lists(&data.failed_node_ids, &data.cancelled_node_ids)
          || (data.outcome == ParallelGroupOutcome::Succeeded
            && (!data.failed_node_ids.is_empty() || !data.cancelled_node_ids.is_empty()))
          || (data.outcome == ParallelGroupOutcome::Failed
            && data.failed_node_ids.is_empty()
            && data.cancelled_node_ids.is_empty())
        {
          return Err(EventValidationError::Invalid(
            "parallel_group_completed contains an invalid outcome or node list.".to_string(),
          ));
        }
      }
      RunEventPayload::ApprovalRequested(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V4
            | RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_public_structural_id(&data.approval_id)
          || !is_approval_request_id(&data.request_id)
          || data
            .expires_at
            .is_some_and(|expires_at| expires_at <= self.occurred_at)
        {
          return Err(EventValidationError::Invalid(
            "approval_requested requires event schema v4, valid identities, and a future expiresAt."
              .to_string(),
          ));
        }
      }
      RunEventPayload::ApprovalResolved(data) => {
        if data.is_rich() {
          let required_roles = data.required_roles.as_deref();
          let represented_roles = data.represented_roles.as_deref();
          let outcome_valid = match (data.decision, data.reason, data.required, data.received) {
            (
              Some(ApprovalDecision::Approved),
              Some(RichApprovalResolutionReason::Quorum),
              Some(required),
              Some(received),
            ) => {
              received >= required
                && required_roles
                  .zip(represented_roles)
                  .is_some_and(|(required, represented)| {
                    required.iter().all(|role| represented.contains(role))
                  })
            }
            (
              Some(ApprovalDecision::Rejected),
              Some(
                RichApprovalResolutionReason::Rejection
                | RichApprovalResolutionReason::Timeout
                | RichApprovalResolutionReason::Cancellation
                | RichApprovalResolutionReason::ArtifactInvalidated,
              ),
              Some(_),
              Some(_),
            ) => true,
            _ => false,
          };
          let valid = data.resolution.is_none()
            && data.review_digest.as_deref().is_some_and(valid_sha256)
            && data.decision.is_some()
            && data.reason.is_some()
            && data.required.is_some_and(|value| (1..=20).contains(&value))
            && data.received.is_some_and(|value| value <= 20)
            && data
              .required_roles
              .as_deref()
              .is_some_and(valid_rich_approval_roles)
            && data
              .represented_roles
              .as_deref()
              .is_some_and(valid_rich_approval_roles)
            && outcome_valid;
          if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V19
            || !valid_public_structural_id(&data.approval_id)
            || !is_approval_request_id(&data.request_id)
            || !valid
          {
            return Err(EventValidationError::Invalid(
              "approval_resolved does not match the frozen rich Event v19 contract.".to_string(),
            ));
          }
          return Ok(());
        }
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V4
            | RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_public_structural_id(&data.approval_id)
          || !is_approval_request_id(&data.request_id)
        {
          return Err(EventValidationError::Invalid(
            "approval_resolved requires event schema v4 and valid approval/request identities."
              .to_string(),
          ));
        }
        if data.decision.is_some()
          || data.reason.is_some()
          || data.required.is_some()
          || data.received.is_some()
          || data.required_roles.is_some()
          || data.represented_roles.is_some()
        {
          return Err(EventValidationError::Invalid(
            "Basic approval resolution cannot contain rich approval fields.".to_string(),
          ));
        }
        data
          .resolution
          .as_ref()
          .ok_or_else(|| {
            EventValidationError::Invalid(
              "Basic approval resolution requires a resolution.".to_string(),
            )
          })?
          .validate()?;
      }
      RunEventPayload::ApprovalReviewRequested(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V19
          || !valid_rich_approval_identity(&data.approval_id, &data.request_id, &data.review_digest)
          || data.directory_version.is_empty()
          || data.directory_version.len() > 128
          || !(1..=20).contains(&data.required)
          || !valid_rich_approval_roles(&data.required_roles)
        {
          return Err(EventValidationError::Invalid(
            "approval_review_requested does not match the frozen Event v19 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::ApprovalReviewerDecisionRecorded(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V19
          || !valid_rich_approval_identity(&data.approval_id, &data.request_id, &data.review_digest)
          || data.principal_id.is_empty()
          || data.principal_id.len() > 128
        {
          return Err(EventValidationError::Invalid(
            "approval_reviewer_decision_recorded does not match the frozen Event v19 contract."
              .to_string(),
          ));
        }
      }
      RunEventPayload::ApprovalQuorumProgressed(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V19
          || !valid_rich_approval_identity(&data.approval_id, &data.request_id, &data.review_digest)
          || !(1..=20).contains(&data.required)
          || data.received > data.required
          || !valid_rich_approval_roles(&data.required_roles)
          || !valid_rich_approval_roles(&data.represented_roles)
          || data
            .represented_roles
            .iter()
            .any(|role| !data.required_roles.contains(role))
        {
          return Err(EventValidationError::Invalid(
            "approval_quorum_progressed does not match the frozen Event v19 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::ApprovalInvalidated(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V19
          || !valid_rich_approval_identity(&data.approval_id, &data.request_id, &data.review_digest)
          || data.reason != RichApprovalResolutionReason::ArtifactInvalidated
        {
          return Err(EventValidationError::Invalid(
            "approval_invalidated does not match the frozen Event v19 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationDeliveryRequested(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !matches!(
          data.provider.as_str(),
          "slack" | "telegram" | "discord" | "whatsapp" | "custom"
        ) || data.destination.is_empty()
        {
          return Err(EventValidationError::Invalid(
            "notification_delivery_requested requires event schema v5 and a provider destination."
              .to_string(),
          ));
        }
      }
      RunEventPayload::NotificationDeliveryAttemptStarted(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !(1..=3).contains(&data.attempt)
          || !valid_prefixed_id(&data.attempt_id, "nattempt_", 12)
          || !valid_sha256(&data.idempotency_key)
        {
          return Err(EventValidationError::Invalid(
            "notification_delivery_attempt_started has an invalid attempt contract.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationDeliverySucceeded(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !(1..=3).contains(&data.attempt)
          || !valid_prefixed_id(&data.attempt_id, "nattempt_", 12)
          || !valid_provider_message(&data.provider_message)
        {
          return Err(EventValidationError::Invalid(
            "notification_delivery_succeeded has an invalid result contract.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationDeliveryFailed(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        data.failure.validate()?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !(1..=3).contains(&data.attempt)
          || !valid_prefixed_id(&data.attempt_id, "nattempt_", 12)
          || (!data.final_ && (!data.failure.retryable || data.attempt == 3))
          || matches!(
            data.failure.kind.as_str(),
            "delivery_ambiguous" | "host_crashed"
          ) && !data.final_
        {
          return Err(EventValidationError::Invalid(
            "notification_delivery_failed has an invalid retry/finality contract.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationDecisionAccepted(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !((data.provider == "slack" && valid_prefixed_id(&data.provider_actor_id, "U", 9))
          || (data.provider == "telegram"
            && data
              .provider_actor_id
              .strip_prefix("telegram:")
              .is_some_and(|value| {
                !value.is_empty()
                  && value.len() <= 32
                  && value.bytes().all(|byte| byte.is_ascii_digit())
              }))
          || (data.provider == "discord"
            && data
              .provider_actor_id
              .strip_prefix("discord:")
              .is_some_and(|value| {
                (17..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
              }))
          || (data.provider == "whatsapp"
            && data
              .provider_actor_id
              .strip_prefix("whatsapp:")
              .is_some_and(|value| {
                (8..=16).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
              }))
          || (data.provider == "custom" && data.provider_actor_id == "custom-provider"))
        {
          return Err(EventValidationError::Invalid(
            "notification_decision_accepted has an invalid provider audit identity.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationMessageUpdateRequested(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_prefixed_id(&data.update_id, "nupdate_", 11)
        {
          return Err(EventValidationError::Invalid(
            "notification_message_update_requested has an invalid update identity.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationMessageUpdateAttemptStarted(data)
      | RunEventPayload::NotificationMessageUpdated(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_prefixed_id(&data.update_id, "nupdate_", 11)
          || !(1..=3).contains(&data.attempt)
          || !valid_prefixed_id(&data.attempt_id, "nattempt_", 12)
        {
          return Err(EventValidationError::Invalid(
            "Notification message-update attempt has an invalid identity.".to_string(),
          ));
        }
      }
      RunEventPayload::NotificationMessageUpdateFailed(data) => {
        validate_notification_identity(&data.approval_id, &data.request_id, &data.delivery_id)?;
        data.failure.validate()?;
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V5
            | RUN_EVENT_SCHEMA_VERSION_V6
            | RUN_EVENT_SCHEMA_VERSION_V7
            | RUN_EVENT_SCHEMA_VERSION_V8
            | RUN_EVENT_SCHEMA_VERSION_V9
            | RUN_EVENT_SCHEMA_VERSION_V10
            | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_prefixed_id(&data.update_id, "nupdate_", 11)
          || !(1..=3).contains(&data.attempt)
          || !valid_prefixed_id(&data.attempt_id, "nattempt_", 12)
          || (!data.final_ && (!data.failure.retryable || data.attempt == 3))
        {
          return Err(EventValidationError::Invalid(
            "notification_message_update_failed has an invalid retry/finality contract."
              .to_string(),
          ));
        }
      }
      RunEventPayload::InfrastructurePolicyDecisionRecorded(data) => {
        if !valid_id(&data.invocation_id)
          || !valid_id(&data.call_id)
          || !valid_sha256(&data.operation_key)
          || !data.decision.decision_id.starts_with("decision_")
          || data.decision.decision_id.len() != 73
          || !valid_sha256(&data.decision.policy_digest)
          || !valid_sha256(&data.decision.descriptor_digest)
          || data.decision.matched_rule_ids.len() > 1024
          || data.decision.requirements.len() > 4096
          || data.decision.explanation.len() > 1024
          || data
            .decision
            .emergency_deny
            .as_ref()
            .is_some_and(|overlay| {
              overlay.policy_id.is_empty()
                || overlay.policy_version.is_empty()
                || !valid_sha256(&overlay.policy_digest)
                || overlay.matched_rule_ids.is_empty()
                || overlay.matched_rule_ids.len() > 1024
            })
          || serde_json::to_vec(&data.decision).map_or(true, |encoded| encoded.len() > 262_144)
        {
          return Err(EventValidationError::Invalid(
            "infrastructure_policy_decision_recorded exceeds its safe durable contract."
              .to_string(),
          ));
        }
      }
      RunEventPayload::OperationStarted(data) => {
        validate_operation_base(
          self.event_schema_version,
          &data.node_id,
          data.attempt_number,
          &data.invocation_id,
          &data.call_id,
          &data.operation_key,
          &data.capability,
          &data.operation,
          &data.metadata,
        )?;
      }
      RunEventPayload::OperationSucceeded(data) => {
        validate_operation_base(
          self.event_schema_version,
          &data.node_id,
          data.attempt_number,
          &data.invocation_id,
          &data.call_id,
          &data.operation_key,
          &data.capability,
          &data.operation,
          &data.metadata,
        )?;
        if !data.duration_ms.is_finite()
          || data.duration_ms < 0.0
          || data.result_bytes > 8_388_608
          || !valid_sha256(&data.result_digest)
        {
          return Err(EventValidationError::Invalid(
            "operation_succeeded has invalid duration, size, or digest fields.".to_string(),
          ));
        }
      }
      RunEventPayload::OperationFailed(data) => {
        validate_operation_base(
          self.event_schema_version,
          &data.node_id,
          data.attempt_number,
          &data.invocation_id,
          &data.call_id,
          &data.operation_key,
          &data.capability,
          &data.operation,
          &data.metadata,
        )?;
        if !data.duration_ms.is_finite() || data.duration_ms < 0.0 {
          return Err(EventValidationError::Invalid(
            "operation_failed requires a non-negative finite durationMs.".to_string(),
          ));
        }
        data
          .failure
          .validate()
          .map_err(EventValidationError::Invalid)?;
      }
      RunEventPayload::RunCancellationRequested(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_id(&data.request_id)
        {
          return Err(EventValidationError::Invalid(
            "run_cancellation_requested requires Event v10 and a valid requestId.".to_string(),
          ));
        }
      }
      RunEventPayload::LifecycleHookRequested(data) => {
        let subject_matches_event =
          data.event.is_step() == matches!(data.subject.kind, LifecycleSubjectKind::Step);
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_sha256(&data.hook_invocation_id)
          || !valid_id(&data.hook_id)
          || !valid_id(&data.subject.id)
          || !subject_matches_event
        {
          return Err(EventValidationError::Invalid(
            "lifecycle_hook_requested has an invalid Event v10 hook or subject identity."
              .to_string(),
          ));
        }
      }
      RunEventPayload::LifecycleActionAttemptStarted(data)
      | RunEventPayload::LifecycleActionSucceeded(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_lifecycle_action_identity(
          &data.hook_invocation_id,
          &data.action_id,
          data.attempt,
        ) {
          return Err(EventValidationError::Invalid(
            "Lifecycle action events require Event v10 and a valid attempt-1 identity.".to_string(),
          ));
        }
      }
      RunEventPayload::LifecycleActionFailed(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_lifecycle_action_identity(
          &data.hook_invocation_id,
          &data.action_id,
          data.attempt,
        ) || !valid_lifecycle_failure(&data.failure)
        {
          return Err(EventValidationError::Invalid(
            "lifecycle_action_failed has an invalid Event v10 identity or safe failure."
              .to_string(),
          ));
        }
      }
      RunEventPayload::LifecycleHookCompleted(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_sha256(&data.hook_invocation_id)
          || data.failed_actions > 64
          || (data.status == LifecycleHookCompletionStatus::Completed && data.failed_actions != 0)
          || (data.status == LifecycleHookCompletionStatus::CompletedWithWarnings
            && data.failed_actions == 0)
        {
          return Err(EventValidationError::Invalid(
            "lifecycle_hook_completed has an invalid Event v10 completion summary.".to_string(),
          ));
        }
      }
      RunEventPayload::ReusableLifecycleRequested(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V13
          || !valid_id(&data.invocation_id)
          || !valid_sha256(&data.definition_digest)
        {
          return Err(EventValidationError::Invalid(
            "reusable_lifecycle_requested has an invalid Event v13 identity.".to_string(),
          ));
        }
      }
      RunEventPayload::ReusableLifecycleActionStarted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V13
          || !valid_id(&data.invocation_id)
          || !valid_sha256(&data.definition_digest)
          || !valid_id(&data.action_id)
        {
          return Err(EventValidationError::Invalid(
            "reusable_lifecycle_action_started has an invalid Event v13 identity.".to_string(),
          ));
        }
      }
      RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V13
          || !valid_id(&data.invocation_id)
          || !valid_sha256(&data.definition_digest)
          || !valid_id(&data.action_id)
          || data.outcome != ReusableLifecycleOutcome::Succeeded
        {
          return Err(EventValidationError::Invalid(
            "reusable_lifecycle_action_succeeded has an invalid Event v13 outcome.".to_string(),
          ));
        }
      }
      RunEventPayload::ReusableLifecycleActionFailed(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V13
          || !valid_id(&data.invocation_id)
          || !valid_sha256(&data.definition_digest)
          || !valid_id(&data.action_id)
          || data.outcome != ReusableLifecycleOutcome::Failed
          || !data.warning_code.starts_with("WOML_")
          || data.warning_code.len() > 128
        {
          return Err(EventValidationError::Invalid(
            "reusable_lifecycle_action_failed has an invalid Event v13 warning.".to_string(),
          ));
        }
      }
      RunEventPayload::ReusableLifecycleCompleted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V13
          || !valid_id(&data.invocation_id)
          || !valid_sha256(&data.definition_digest)
          || !matches!(
            data.outcome,
            ReusableLifecycleOutcome::Succeeded | ReusableLifecycleOutcome::CompletedWithWarnings
          )
        {
          return Err(EventValidationError::Invalid(
            "reusable_lifecycle_completed has an invalid Event v13 outcome.".to_string(),
          ));
        }
      }
      RunEventPayload::ForEachOpened(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V15 | RUN_EVENT_SCHEMA_VERSION_V16
        ) || self.iteration.is_some()
          || !valid_public_structural_id(&data.for_each_id)
          || data.total > 10_000
          || !(1..=64).contains(&data.concurrency)
          || !valid_sha256(&data.items_digest)
        {
          return Err(EventValidationError::Invalid(
            "for_each_opened has an invalid Event v15 identity or bounded input.".to_string(),
          ));
        }
      }
      RunEventPayload::ForEachIterationStarted(data) => {
        validate_for_each_iteration_event(self, &data.for_each_id)?;
      }
      RunEventPayload::ForEachIterationSucceeded(data) => {
        validate_for_each_iteration_event(self, &data.for_each_id)?;
        if data.result.is_some() != data.result_digest.is_some()
          || data
            .result_digest
            .as_deref()
            .is_some_and(|digest| !valid_sha256(digest))
        {
          return Err(EventValidationError::Invalid(
            "for_each_iteration_succeeded requires a result and digest pair.".to_string(),
          ));
        }
      }
      RunEventPayload::ForEachIterationFailed(data) => {
        validate_for_each_iteration_event(self, &data.for_each_id)?;
        if !valid_id(&data.failed_node_id) {
          return Err(EventValidationError::Invalid(
            "for_each_iteration_failed requires a valid failedNodeId.".to_string(),
          ));
        }
      }
      RunEventPayload::ForEachIterationSkipped(data) => {
        validate_for_each_iteration_event(self, &data.for_each_id)?;
      }
      RunEventPayload::ForEachSucceeded(data)
      | RunEventPayload::ForEachFailed(data)
      | RunEventPayload::ForEachCancelled(data) => {
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V15 | RUN_EVENT_SCHEMA_VERSION_V16
        ) || self.iteration.is_some()
          || !valid_public_structural_id(&data.for_each_id)
          || data.total > 10_000
          || data.succeeded + data.failed + data.skipped != data.total
          || !valid_sha256(&data.aggregate_digest)
        {
          return Err(EventValidationError::Invalid(
            "for_each settlement does not match the bounded Event v15 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::WaitRegistered(data) => {
        let deadline_valid = match data.kind {
          DurableWaitKind::Duration => data
            .deadline
            .is_some_and(|deadline| deadline >= data.registered_at),
          // A wait-until target may already be in the past. The runtime still
          // records registration before atomically satisfying it.
          DurableWaitKind::Until => data.deadline.is_some(),
          DurableWaitKind::Signal => data
            .deadline
            .is_none_or(|deadline| deadline >= data.registered_at),
        };
        let shape_valid = match data.kind {
          DurableWaitKind::Duration | DurableWaitKind::Until => {
            data.deadline.is_some() && data.signals.is_none() && data.correlation_digest.is_none()
          }
          DurableWaitKind::Signal => {
            data.signals.as_ref().is_some_and(|signals| {
              (1..=16).contains(&signals.len())
                && signals.iter().all(|signal| valid_signal_name(signal))
                && signals
                  .iter()
                  .collect::<std::collections::HashSet<_>>()
                  .len()
                  == signals.len()
            }) && data.correlation_digest.as_deref().is_some_and(valid_sha256)
          }
        };
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V16
          || !valid_public_structural_id(&data.wait_id)
          || data.node_id != data.wait_id
          || data.registered_at != self.occurred_at
          || !deadline_valid
          || !shape_valid
        {
          return Err(EventValidationError::Invalid(
            "wait_registered does not match the frozen Event v16 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::ExternalSignalReceived(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V16
          || !valid_id(&data.signal_id)
          || !valid_public_structural_id(&data.wait_id)
          || !valid_signal_name(&data.signal)
          || data.received_at != self.occurred_at
          || !valid_sha256(&data.idempotency_digest)
          || data
            .correlation
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 256 || value.trim() != value)
          || data
            .authentication_class
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 64)
          || serde_json::to_vec(&data.payload).map_or(true, |payload| payload.len() > 1_048_576)
        {
          return Err(EventValidationError::Invalid(
            "external_signal_received does not match the bounded Event v16 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::WaitSatisfied(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V16
          || !valid_public_structural_id(&data.wait_id)
          || data.resolved_at != self.occurred_at
          || !data.result.is_object()
          || serde_json::to_vec(&data.result).map_or(true, |result| result.len() > 1_048_576)
        {
          return Err(EventValidationError::Invalid(
            "wait_satisfied does not match the frozen Event v16 result contract.".to_string(),
          ));
        }
      }
      RunEventPayload::WaitTimedOut(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V16
          || !valid_public_structural_id(&data.wait_id)
          || data.resolved_at != self.occurred_at
          || data.resolved_at < data.deadline
        {
          return Err(EventValidationError::Invalid(
            "wait_timed_out does not match the frozen Event v16 timeout contract.".to_string(),
          ));
        }
      }
      RunEventPayload::WaitCancelled(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V16
          || !valid_public_structural_id(&data.wait_id)
          || !valid_id(&data.cancellation_id)
          || data.cancelled_at != self.occurred_at
        {
          return Err(EventValidationError::Invalid(
            "wait_cancelled does not match the frozen Event v16 cancellation contract.".to_string(),
          ));
        }
      }
      RunEventPayload::LockWaitRegistered(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V17
          || !valid_public_structural_id(&data.lock_id)
          || !valid_lock_key(&data.key)
          || data.waiter_sequence == 0
          || data.registered_at != self.occurred_at
          || data.deadline < data.registered_at
          || !valid_id(&data.execution_identity)
        {
          return Err(EventValidationError::Invalid(
            "lock_wait_registered does not match the frozen Event v17 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::LockAcquired(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V17
          || !valid_public_structural_id(&data.lock_id)
          || !valid_lock_key(&data.key)
          || !valid_prefixed_id(&data.lease_id, "lease_", 7)
          || !valid_fencing_token(&data.fencing_token)
          || data.acquired_at != self.occurred_at
          || data.expires_at <= data.acquired_at
          || !valid_id(&data.execution_identity)
        {
          return Err(EventValidationError::Invalid(
            "lock_acquired does not match the frozen Event v17 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::LockReleased(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V17
          || !valid_public_structural_id(&data.lock_id)
          || !valid_lock_key(&data.key)
          || !valid_prefixed_id(&data.lease_id, "lease_", 7)
          || !valid_fencing_token(&data.fencing_token)
          || data.released_at != self.occurred_at
          || !data.result.is_object()
          || serde_json::to_vec(&data.result).map_or(true, |result| result.len() > 1_048_576)
        {
          return Err(EventValidationError::Invalid(
            "lock_released does not match the frozen Event v17 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::LockAcquireTimedOut(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V17
          || !valid_public_structural_id(&data.lock_id)
          || !valid_lock_key(&data.key)
          || data.resolved_at != self.occurred_at
          || data.resolved_at < data.deadline
        {
          return Err(EventValidationError::Invalid(
            "lock_acquire_timed_out does not match the frozen Event v17 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::LockAcquisitionCancelled(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V17
          || !valid_public_structural_id(&data.lock_id)
          || !valid_lock_key(&data.key)
          || data.cancelled_at != self.occurred_at
          || !valid_id(&data.cancellation_id)
        {
          return Err(EventValidationError::Invalid(
            "lock_acquisition_cancelled does not match the frozen Event v17 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::LockLeaseLost(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V17
          || !valid_public_structural_id(&data.lock_id)
          || !valid_lock_key(&data.key)
          || !valid_prefixed_id(&data.lease_id, "lease_", 7)
          || !valid_fencing_token(&data.fencing_token)
          || data.detected_at != self.occurred_at
          || data.detected_at < data.expired_at
        {
          return Err(EventValidationError::Invalid(
            "lock_lease_lost does not match the frozen Event v17 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::SagaStarted(data)
      | RunEventPayload::SagaForwardSucceeded(data)
      | RunEventPayload::SagaCorrectionCompleted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
        {
          return Err(EventValidationError::Invalid(
            "Saga lifecycle event does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::CompensationRegistered(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
          || data.action_id.is_empty()
          || data.action_id.len() > 768
          || !valid_public_structural_id(&data.for_step_id)
          || data.completion_sequence == 0
        {
          return Err(EventValidationError::Invalid(
            "compensation_registered does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::SagaCorrectionStarted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
        {
          return Err(EventValidationError::Invalid(
            "saga_correction_started does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::CorrectionAttemptStarted(data)
      | RunEventPayload::CorrectionAttemptSucceeded(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
          || data.action_id.is_empty()
          || data.action_id.len() > 768
          || !(1..=10).contains(&data.attempt)
        {
          return Err(EventValidationError::Invalid(
            "Correction attempt identity does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::CorrectionAttemptFailed(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
          || data.action_id.is_empty()
          || data.action_id.len() > 768
          || !(1..=10).contains(&data.attempt)
          || !data.code.starts_with("WOML_")
          || data.code.len() > 128
        {
          return Err(EventValidationError::Invalid(
            "correction_attempt_failed does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::CorrectionRetryScheduled(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
          || data.action_id.is_empty()
          || data.action_id.len() > 768
          || !(1..=10).contains(&data.attempt)
          || data.retry_at < self.occurred_at
        {
          return Err(EventValidationError::Invalid(
            "correction_retry_scheduled does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::CorrectionInterrupted(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
          || data.action_id.is_empty()
          || data.action_id.len() > 768
          || !(1..=10).contains(&data.attempt)
        {
          return Err(EventValidationError::Invalid(
            "correction_interrupted does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::ManualRecoveryRequired(data) => {
        if self.event_schema_version != RUN_EVENT_SCHEMA_VERSION_V18
          || !valid_public_structural_id(&data.saga_id)
          || data.action_id.is_empty()
          || data.action_id.len() > 768
          || !data.code.starts_with("WOML_")
          || data.code.len() > 128
        {
          return Err(EventValidationError::Invalid(
            "manual_recovery_required does not match the frozen Event v18 contract.".to_string(),
          ));
        }
      }
      RunEventPayload::RunOutcomeDecided(data) => {
        let valid = match data {
          RunOutcomeDecidedData::Succeeded { .. } => true,
          RunOutcomeDecidedData::Failed { failure } => valid_lifecycle_failure(failure),
          RunOutcomeDecidedData::Cancelled {
            cancellation_request_id,
          } => valid_id(cancellation_request_id),
        };
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid
        {
          return Err(EventValidationError::Invalid(
            "run_outcome_decided has an invalid Event v10 outcome payload.".to_string(),
          ));
        }
      }
      RunEventPayload::RunFinalized(data) => {
        let warnings_valid = data.warnings.len() <= 128
          && data.warnings.iter().all(|warning| {
            valid_id(&warning.hook_id)
              && valid_id(&warning.action_id)
              && warning.step_id.as_deref().is_none_or(valid_id)
              && warning
                .provider
                .as_ref()
                .is_none_or(|value| value.len() <= 64)
              && warning
                .destination
                .as_ref()
                .is_none_or(|value| value.len() <= 256)
              && warning.code.starts_with("WOML_")
          });
        if !matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !warnings_valid
          || (data.lifecycle_status == FinalLifecycleStatus::Completed && !data.warnings.is_empty())
          || (data.lifecycle_status == FinalLifecycleStatus::CompletedWithWarnings
            && data.warnings.is_empty())
        {
          return Err(EventValidationError::Invalid(
            "run_finalized has an invalid Event v10 lifecycle summary.".to_string(),
          ));
        }
      }
      RunEventPayload::RunSucceeded(data) => {
        if matches!(
          self.event_schema_version,
          RUN_EVENT_SCHEMA_VERSION_V10 | RUN_EVENT_SCHEMA_VERSION_V11
        ) || !valid_id(&data.terminal_node_id)
        {
          return Err(EventValidationError::Invalid(
            "run_succeeded requires a valid terminalNodeId.".to_string(),
          ));
        }
      }
      RunEventPayload::RunFailed(data) => match (self.event_schema_version, data) {
        (RUN_EVENT_SCHEMA_VERSION_V1, RunFailedData::V1(data)) => {
          let identity_fields = [
            data.node_id.is_some(),
            data.attempt.is_some(),
            data.invocation_id.is_some(),
          ];
          if identity_fields.iter().any(|present| *present)
            && !identity_fields.iter().all(|present| *present)
          {
            return Err(EventValidationError::Invalid(
                "run_failed attempt identity must provide nodeId, attempt, and invocationId together."
                  .to_string(),
              ));
          }
          if let (Some(node_id), Some(attempt), Some(invocation_id)) =
            (&data.node_id, data.attempt, &data.invocation_id)
          {
            validate_identity(node_id, attempt, invocation_id)?;
          }
          data.failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V2
          | RUN_EVENT_SCHEMA_VERSION_V3
          | RUN_EVENT_SCHEMA_VERSION_V4
          | RUN_EVENT_SCHEMA_VERSION_V5
          | RUN_EVENT_SCHEMA_VERSION_V6
          | RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9,
          RunFailedData::V2(RunFailedDataV2::Attempt {
            node_id,
            attempt,
            invocation_id,
            failure,
          }),
        ) => {
          validate_identity(node_id, *attempt, invocation_id)?;
          failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V2
          | RUN_EVENT_SCHEMA_VERSION_V3
          | RUN_EVENT_SCHEMA_VERSION_V4
          | RUN_EVENT_SCHEMA_VERSION_V5
          | RUN_EVENT_SCHEMA_VERSION_V6
          | RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9,
          RunFailedData::V2(RunFailedDataV2::Branch {
            branch_id,
            arm_id,
            path,
            failure,
          }),
        ) => {
          if !valid_public_structural_id(branch_id)
            || arm_id
              .as_deref()
              .is_some_and(|arm_id| !valid_branch_arm_id(branch_id, arm_id))
            || path
              .as_ref()
              .is_some_and(|path| path.is_empty() || path.iter().any(String::is_empty))
          {
            return Err(EventValidationError::Invalid(
              "Branch-scoped run_failed contains an invalid branchId, armId, or path.".to_string(),
            ));
          }
          match failure {
            BranchFailure::BranchTestNotBoolean { .. } if arm_id.is_none() || path.is_none() => {
              return Err(EventValidationError::Invalid(
                "branch_test_not_boolean requires armId and path.".to_string(),
              ));
            }
            BranchFailure::ReferenceNotAvailable { .. } if path.is_none() => {
              return Err(EventValidationError::Invalid(
                "reference_not_available requires path.".to_string(),
              ));
            }
            _ => {}
          }
          failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V3
          | RUN_EVENT_SCHEMA_VERSION_V4
          | RUN_EVENT_SCHEMA_VERSION_V5
          | RUN_EVENT_SCHEMA_VERSION_V6
          | RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9,
          RunFailedData::V3(RunFailedDataV3::Parallel {
            parallel_id,
            primary_node_id,
            failed_node_ids,
            cancelled_node_ids,
            failure,
            ..
          }),
        ) => {
          if !valid_public_structural_id(parallel_id)
            || !valid_public_structural_id(primary_node_id)
            || !valid_ordered_id_lists(failed_node_ids, cancelled_node_ids)
            || failed_node_ids.is_empty()
            || !failed_node_ids
              .iter()
              .any(|node_id| node_id == primary_node_id)
          {
            return Err(EventValidationError::Invalid(
              "Parallel-scoped run_failed contains an invalid group or child identity list."
                .to_string(),
            ));
          }
          failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V4
          | RUN_EVENT_SCHEMA_VERSION_V5
          | RUN_EVENT_SCHEMA_VERSION_V6
          | RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9,
          RunFailedData::V4(RunFailedDataV4::Approval {
            approval_id,
            request_id,
            failure,
          }),
        ) => {
          if !valid_public_structural_id(approval_id) || !is_approval_request_id(request_id) {
            return Err(EventValidationError::Invalid(
              "Approval-scoped run_failed contains an invalid approvalId or requestId.".to_string(),
            ));
          }
          failure.validate()?;
        }
        (
          RUN_EVENT_SCHEMA_VERSION_V5
          | RUN_EVENT_SCHEMA_VERSION_V6
          | RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9,
          RunFailedData::V5(RunFailedDataV5::Notification {
            approval_id,
            request_id,
            failed_delivery_ids,
            failure,
          }),
        ) => {
          if !valid_public_structural_id(approval_id)
            || !is_approval_request_id(request_id)
            || failed_delivery_ids.is_empty()
            || !failed_delivery_ids
              .iter()
              .all(|delivery_id| is_notification_delivery_id(delivery_id, approval_id))
            || failed_delivery_ids
              .iter()
              .collect::<std::collections::HashSet<_>>()
              .len()
              != failed_delivery_ids.len()
            || failure.kind != "all_deliveries_failed"
            || failure.code != "WOML_NOTIFICATION_DELIVERY_FAILED"
            || failure.message.is_empty()
          {
            return Err(EventValidationError::Invalid(
              "Notification-scoped run_failed has an invalid delivery failure contract."
                .to_string(),
            ));
          }
        }
        _ => {
          return Err(EventValidationError::Invalid(format!(
            "run_failed payload does not belong to event schema v{}.",
            self.event_schema_version
          )));
        }
      },
    }
    Ok(())
  }
}
