use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::event::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalResolution,
  ApprovalTimeoutPolicy, AttemptFailure, BranchFailure, BusinessOutcome, EventValidationError,
  FinalLifecycleStatus, ForkBranchOutcome, ForkJoinOutcome, LifecycleFailure,
  LifecycleHookCompletionStatus, LifecycleSubject, LifecycleWarning, NotificationResolution,
  NotificationSafeFailure, OperationExecutionMode, ParallelFailure, ParallelFailurePolicy,
  ParallelGroupOutcome, ProviderMessageIdentity, RunEvent, RunEventPayload, RunFailedData,
  RunFailedDataV2, RunFailedDataV3, RunFailedDataV4, RunFailedDataV5, RunOutcomeDecidedData,
  StepAttemptStartedData,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
  #[default]
  NotStarted,
  Queued,
  Running,
  Waiting,
  Cancelling,
  Finalizing,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
  #[default]
  Idle,
  Running,
  Finalizing,
  CompletedWithWarnings,
  Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleActionStatus {
  Started,
  Succeeded,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleActionProjection {
  pub action_id: String,
  pub attempt: u32,
  pub status: LifecycleActionStatus,
  pub failure: Option<LifecycleFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookStatus {
  Requested,
  Running,
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleHookProjection {
  pub hook_invocation_id: String,
  pub hook_id: String,
  pub event: crate::model::LifecycleEventName,
  pub subject: LifecycleSubject,
  pub status: LifecycleHookStatus,
  pub actions: BTreeMap<String, LifecycleActionProjection>,
  pub failed_actions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReusableLifecycleStatus {
  Requested,
  Running,
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReusableLifecycleProjection {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: crate::ReusableLifecycleHook,
  pub status: ReusableLifecycleStatus,
  pub active_action_id: Option<String>,
  pub completed_action_ids: Vec<String>,
  pub warning_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkflowContext {
  pub trigger: Map<String, Value>,
  pub steps: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptIdentity {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttemptStatus {
  Started,
  Succeeded { output: Value },
  Failed { failure: AttemptFailure },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttemptProjection {
  pub identity: AttemptIdentity,
  pub handler: String,
  pub idempotency_key: Option<String>,
  pub status: AttemptStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationIdentity {
  pub invocation_id: String,
  pub call_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationStatus {
  Started,
  Succeeded {
    duration_ms: f64,
    result_bytes: u64,
    result_digest: String,
  },
  Failed {
    duration_ms: f64,
    failure: crate::CapabilityFailure,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationProjection {
  pub identity: OperationIdentity,
  pub node_id: String,
  pub attempt_number: u32,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
  pub status: OperationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryScheduleProjection {
  pub node_id: String,
  pub failed_attempt: u32,
  pub next_attempt: u32,
  pub scheduled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelGroupStatus {
  Started,
  Completed {
    outcome: ParallelGroupOutcome,
    failed_node_ids: Vec<String>,
    cancelled_node_ids: Vec<String>,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelGroupProjection {
  pub parallel_id: String,
  pub fork_context: WorkflowContext,
  pub status: ParallelGroupStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkJoinStatus {
  Pending,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkBranchProjection {
  pub branch_id: String,
  pub terminal_node_id: Option<String>,
  pub outcome: Option<ForkBranchOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkProjection {
  pub fork_id: String,
  pub branches: BTreeMap<String, ForkBranchProjection>,
  pub join_status: ForkJoinStatus,
  pub blocking_branch_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForEachStatus {
  Open,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForEachIterationStatus {
  Started,
  Succeeded { result: Option<Value> },
  Failed { failed_node_id: String },
  Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForEachProjection {
  pub for_each_id: String,
  pub total: u32,
  pub items_digest: String,
  pub concurrency: u32,
  pub status: ForEachStatus,
  pub iterations: BTreeMap<u32, ForEachIterationStatus>,
  pub executions: BTreeMap<u32, ForEachIterationExecutionProjection>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ForEachIterationExecutionProjection {
  pub context: WorkflowContext,
  pub attempts: Vec<AttemptProjection>,
  pub operations: BTreeMap<OperationIdentity, OperationProjection>,
  pub pending_retries: BTreeMap<String, RetryScheduleProjection>,
  pub branch_selections: BTreeMap<String, String>,
  pub choice_selections: BTreeMap<String, String>,
  pub parallel_groups: BTreeMap<String, ParallelGroupProjection>,
  pub lifecycle_hooks: BTreeMap<String, LifecycleHookProjection>,
  pub reusable_lifecycle_hooks: BTreeMap<String, ReusableLifecycleProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalRequestStatus {
  Waiting,
  Resolved {
    resolution: ApprovalResolution,
    resolved_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestProjection {
  pub approval_id: String,
  pub request_id: String,
  pub requested_at: DateTime<Utc>,
  pub expires_at: Option<DateTime<Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
  pub status: ApprovalRequestStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationDeliveryStatus {
  Requested,
  AttemptStarted {
    attempt: u32,
    attempt_id: String,
    idempotency_key: String,
  },
  Succeeded {
    attempt: u32,
    attempt_id: String,
    provider_message: ProviderMessageIdentity,
  },
  Failed {
    attempt: u32,
    attempt_id: String,
    final_: bool,
    failure: NotificationSafeFailure,
    failed_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationDeliveryProjection {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub status: NotificationDeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationMessageUpdateStatus {
  Requested,
  AttemptStarted {
    attempt: u32,
    attempt_id: String,
  },
  Updated {
    attempt: u32,
    attempt_id: String,
  },
  Failed {
    attempt: u32,
    attempt_id: String,
    final_: bool,
    failure: NotificationSafeFailure,
    failed_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationMessageUpdateProjection {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub resolution: NotificationResolution,
  pub status: NotificationMessageUpdateStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
  Attempt(AttemptFailure),
  Branch(BranchFailure),
  Parallel {
    parallel_id: String,
    policy: ParallelFailurePolicy,
    primary_node_id: String,
    failed_node_ids: Vec<String>,
    cancelled_node_ids: Vec<String>,
    failure: ParallelFailure,
  },
  Approval {
    approval_id: String,
    request_id: String,
    failure: ApprovalFailure,
  },
  Notification {
    approval_id: String,
    request_id: String,
    failed_delivery_ids: Vec<String>,
    failure: crate::event::NotificationRunFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProjection {
  pub event_schema_version: Option<u32>,
  pub run_id: Option<String>,
  pub workflow_id: Option<String>,
  pub definition_hash: Option<String>,
  pub policy_hash: Option<String>,
  pub queue: Option<String>,
  pub occurrence_sequence: Option<u64>,
  pub admitted_at: Option<DateTime<Utc>>,
  pub started_at: Option<DateTime<Utc>>,
  pub timeout_at: Option<DateTime<Utc>>,
  pub timeout_reached_at: Option<DateTime<Utc>>,
  pub trigger_id: Option<String>,
  pub trigger_handler: Option<String>,
  pub trigger_occurrence_id: Option<String>,
  pub ingress: Option<crate::RunIngress>,
  pub status: RunStatus,
  pub business_outcome: Option<BusinessOutcome>,
  pub lifecycle_status: LifecycleStatus,
  pub cancellation_request_id: Option<String>,
  pub lifecycle_hooks: BTreeMap<String, LifecycleHookProjection>,
  pub reusable_lifecycle_hooks: BTreeMap<String, ReusableLifecycleProjection>,
  pub lifecycle_warnings: Vec<LifecycleWarning>,
  pub context: WorkflowContext,
  pub attempts: Vec<AttemptProjection>,
  pub operations: BTreeMap<OperationIdentity, OperationProjection>,
  pub pending_retries: BTreeMap<String, RetryScheduleProjection>,
  pub branch_selections: BTreeMap<String, String>,
  pub choice_selections: BTreeMap<String, String>,
  pub parallel_groups: BTreeMap<String, ParallelGroupProjection>,
  pub forks: BTreeMap<String, ForkProjection>,
  pub for_each: BTreeMap<String, ForEachProjection>,
  pub approval_requests: BTreeMap<String, ApprovalRequestProjection>,
  pub notification_deliveries: BTreeMap<String, NotificationDeliveryProjection>,
  pub notification_updates: BTreeMap<String, NotificationMessageUpdateProjection>,
  pub notification_decisions: Vec<crate::event::NotificationDecisionAcceptedData>,
  pub terminal_node_id: Option<String>,
  pub result: Option<Value>,
  pub failure: Option<RunFailure>,
  pub lifecycle_failure: Option<LifecycleFailure>,
  pub last_sequence: u64,
}

impl RunProjection {
  pub fn completed_node_ids(&self) -> HashSet<&str> {
    self.context.steps.keys().map(String::as_str).collect()
  }

  pub fn attempted_node_ids(&self) -> HashSet<&str> {
    self
      .attempts
      .iter()
      .map(|attempt| attempt.identity.node_id.as_str())
      .collect()
  }

  pub fn active_attempt_node_ids(&self) -> HashSet<&str> {
    self
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
      .map(|attempt| attempt.identity.node_id.as_str())
      .collect()
  }

  pub fn latest_attempt(&self, node_id: &str) -> Option<&AttemptProjection> {
    self
      .attempts
      .iter()
      .rev()
      .find(|attempt| attempt.identity.node_id == node_id)
  }

  pub fn active_managed_operations(&self, invocation_id: &str) -> Vec<&OperationProjection> {
    self
      .operations
      .values()
      .filter(|operation| {
        operation.identity.invocation_id == invocation_id
          && operation.execution_mode == OperationExecutionMode::Managed
          && operation.status == OperationStatus::Started
      })
      .collect()
  }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FoldError {
  #[error(transparent)]
  InvalidEvent(#[from] EventValidationError),
  #[error("{0}")]
  InvalidHistory(String),
}

fn identity(node_id: &str, attempt: u32, invocation_id: &str) -> AttemptIdentity {
  AttemptIdentity {
    node_id: node_id.to_string(),
    attempt,
    invocation_id: invocation_id.to_string(),
  }
}

fn validate_operation_terminal_identity(
  started: &OperationProjection,
  node_id: &str,
  attempt_number: u32,
  operation_key: &str,
  capability: &str,
  operation: &str,
  execution_mode: OperationExecutionMode,
) -> Result<(), FoldError> {
  if started.status != OperationStatus::Started {
    return Err(FoldError::InvalidHistory(
      "An operation may have at most one terminal event.".to_string(),
    ));
  }
  if started.node_id != node_id
    || started.attempt_number != attempt_number
    || started.operation_key != operation_key
    || started.capability != capability
    || started.operation != operation
    || started.execution_mode != execution_mode
  {
    return Err(FoldError::InvalidHistory(
      "Terminal operation identity does not match operation_started.".to_string(),
    ));
  }
  Ok(())
}

fn require_active_operation_attempt(
  projection: &RunProjection,
  node_id: &str,
  attempt_number: u32,
  invocation_id: &str,
) -> Result<(), FoldError> {
  if projection.attempts.iter().any(|attempt| {
    attempt.identity.node_id == node_id
      && attempt.identity.attempt == attempt_number
      && attempt.identity.invocation_id == invocation_id
      && attempt.status == AttemptStatus::Started
  }) || projection.lifecycle_hooks.values().any(|hook| {
    hook.actions.get(node_id).is_some_and(|action| {
      action.attempt == attempt_number && action.status == LifecycleActionStatus::Started
    })
  }) {
    Ok(())
  } else {
    Err(FoldError::InvalidHistory(
      "An operation event requires its matching active step or lifecycle action attempt."
        .to_string(),
    ))
  }
}

fn started_attempt(data: &StepAttemptStartedData) -> AttemptProjection {
  AttemptProjection {
    identity: identity(&data.node_id, data.attempt, &data.invocation_id),
    handler: data.handler.clone(),
    idempotency_key: data.idempotency_key.clone(),
    status: AttemptStatus::Started,
  }
}

fn apply_iteration_scoped_event(
  projection: &mut RunProjection,
  event: &RunEvent,
  previous_event: Option<&RunEvent>,
) -> Result<(), FoldError> {
  require_running_or_cancelling(projection)?;
  let scope = event.iteration.as_ref().ok_or_else(|| {
    FoldError::InvalidHistory("Loop-owned work is missing iteration scope.".to_string())
  })?;
  let loop_state = projection
    .for_each
    .get_mut(&scope.for_each_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(format!(
        "Loop-owned work references unopened for-each {:?}.",
        scope.for_each_id
      ))
    })?;
  if loop_state.status != ForEachStatus::Open
    || !matches!(
      loop_state.iterations.get(&scope.index),
      Some(ForEachIterationStatus::Started)
    )
  {
    return Err(FoldError::InvalidHistory(format!(
      "Loop-owned work requires active iteration {:?}:{}.",
      scope.for_each_id, scope.index
    )));
  }
  let execution = loop_state.executions.get_mut(&scope.index).ok_or_else(|| {
    FoldError::InvalidHistory("Active iteration has no execution projection.".to_string())
  })?;
  match &event.payload {
    RunEventPayload::StepAttemptStarted(data) => {
      if execution
        .attempts
        .iter()
        .any(|attempt| attempt.identity.invocation_id == data.invocation_id)
        || execution
          .attempts
          .iter()
          .rev()
          .find(|attempt| attempt.identity.node_id == data.node_id)
          .is_some_and(|attempt| attempt.status == AttemptStatus::Started)
      {
        return Err(FoldError::InvalidHistory(format!(
          "Iteration attempt {:?} is not unique and inactive.",
          data.invocation_id
        )));
      }
      let previous = execution
        .attempts
        .iter()
        .rev()
        .find(|attempt| attempt.identity.node_id == data.node_id);
      let expected = previous.map_or(1, |attempt| attempt.identity.attempt + 1);
      if data.attempt != expected {
        return Err(FoldError::InvalidHistory(format!(
          "Iteration node {:?} expected attempt {expected}.",
          data.node_id
        )));
      }
      if let Some(previous) = previous {
        let retry = execution
          .pending_retries
          .get(&data.node_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Iteration retry has no durable schedule.".to_string())
          })?;
        if !matches!(previous.status, AttemptStatus::Failed { .. })
          || retry.next_attempt != data.attempt
          || event.occurred_at < retry.scheduled_at
        {
          return Err(FoldError::InvalidHistory(
            "Iteration retry does not match its failed attempt and schedule.".to_string(),
          ));
        }
        execution.pending_retries.remove(&data.node_id);
      }
      execution.attempts.push(started_attempt(data));
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      if execution.operations.values().any(|operation| {
        operation.identity.invocation_id == data.invocation_id
          && operation.status == OperationStatus::Started
      }) {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt cannot succeed with an active operation.".to_string(),
        ));
      }
      let attempt = execution
        .attempts
        .iter_mut()
        .find(|attempt| {
          attempt.identity.node_id == data.node_id
            && attempt.identity.attempt == data.attempt
            && attempt.identity.invocation_id == data.invocation_id
        })
        .ok_or_else(|| {
          FoldError::InvalidHistory("Iteration success has no matching attempt.".to_string())
        })?;
      if attempt.status != AttemptStatus::Started
        || execution.context.steps.contains_key(&data.node_id)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt already has a terminal outcome.".to_string(),
        ));
      }
      attempt.status = AttemptStatus::Succeeded {
        output: data.output.clone(),
      };
      execution
        .context
        .steps
        .insert(data.node_id.clone(), data.output.clone());
    }
    RunEventPayload::StepAttemptFailed(data) => {
      if execution.operations.values().any(|operation| {
        operation.identity.invocation_id == data.invocation_id
          && operation.status == OperationStatus::Started
      }) {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt cannot fail with an active operation.".to_string(),
        ));
      }
      let attempt = execution
        .attempts
        .iter_mut()
        .find(|attempt| {
          attempt.identity.node_id == data.node_id
            && attempt.identity.attempt == data.attempt
            && attempt.identity.invocation_id == data.invocation_id
        })
        .ok_or_else(|| {
          FoldError::InvalidHistory("Iteration failure has no matching attempt.".to_string())
        })?;
      if attempt.status != AttemptStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration attempt already has a terminal outcome.".to_string(),
        ));
      }
      attempt.status = AttemptStatus::Failed {
        failure: data.failure.clone(),
      };
    }
    RunEventPayload::StepRetryScheduled(data) => {
      let adjacent = previous_event.is_some_and(|previous| {
        previous.iteration.as_ref() == Some(scope)
          && matches!(&previous.payload, RunEventPayload::StepAttemptFailed(failed)
            if failed.node_id == data.node_id && failed.attempt == data.failed_attempt)
      });
      let latest = execution
        .attempts
        .iter()
        .rev()
        .find(|attempt| attempt.identity.node_id == data.node_id);
      if !adjacent
        || !matches!(latest, Some(attempt) if attempt.identity.attempt == data.failed_attempt
          && matches!(attempt.status, AttemptStatus::Failed { .. }))
        || data.next_attempt != data.failed_attempt + 1
        || execution.pending_retries.contains_key(&data.node_id)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration retry schedule does not follow its failed attempt.".to_string(),
        ));
      }
      execution.pending_retries.insert(
        data.node_id.clone(),
        RetryScheduleProjection {
          node_id: data.node_id.clone(),
          failed_attempt: data.failed_attempt,
          next_attempt: data.next_attempt,
          scheduled_at: data.scheduled_at,
        },
      );
    }
    RunEventPayload::ChoiceSelected(data) => {
      if execution
        .choice_selections
        .insert(data.choice_id.clone(), data.arm_id.clone())
        .is_some()
      {
        return Err(FoldError::InvalidHistory(
          "Iteration choice was selected more than once.".to_string(),
        ));
      }
    }
    RunEventPayload::BranchSelected(data) => {
      if execution
        .branch_selections
        .insert(data.branch_id.clone(), data.arm_id.clone())
        .is_some()
      {
        return Err(FoldError::InvalidHistory(
          "Iteration conditional branch was selected more than once.".to_string(),
        ));
      }
    }
    RunEventPayload::ParallelGroupStarted(data) => {
      if execution.parallel_groups.contains_key(&data.parallel_id) {
        return Err(FoldError::InvalidHistory(
          "Iteration parallel group started more than once.".to_string(),
        ));
      }
      execution.parallel_groups.insert(
        data.parallel_id.clone(),
        ParallelGroupProjection {
          parallel_id: data.parallel_id.clone(),
          fork_context: execution.context.clone(),
          status: ParallelGroupStatus::Started,
        },
      );
    }
    RunEventPayload::ParallelGroupCompleted(data) => {
      let group = execution
        .parallel_groups
        .get_mut(&data.parallel_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Iteration parallel completion has no start.".to_string())
        })?;
      if group.status != ParallelGroupStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration parallel group completed more than once.".to_string(),
        ));
      }
      group.status = ParallelGroupStatus::Completed {
        outcome: data.outcome,
        failed_node_ids: data.failed_node_ids.clone(),
        cancelled_node_ids: data.cancelled_node_ids.clone(),
      };
    }
    RunEventPayload::OperationStarted(data) => {
      let active = execution.attempts.iter().any(|attempt| {
        attempt.identity.node_id == data.node_id
          && attempt.identity.attempt == data.attempt_number
          && attempt.identity.invocation_id == data.invocation_id
          && attempt.status == AttemptStatus::Started
      });
      let key = OperationIdentity {
        invocation_id: data.invocation_id.clone(),
        call_id: data.call_id.clone(),
      };
      if !active || execution.operations.contains_key(&key) {
        return Err(FoldError::InvalidHistory(
          "Iteration operation requires one matching active attempt.".to_string(),
        ));
      }
      execution.operations.insert(
        key.clone(),
        OperationProjection {
          identity: key,
          node_id: data.node_id.clone(),
          attempt_number: data.attempt_number,
          operation_key: data.operation_key.clone(),
          capability: data.capability.clone(),
          operation: data.operation.clone(),
          execution_mode: data.execution_mode,
          metadata: data.metadata.clone(),
          status: OperationStatus::Started,
        },
      );
    }
    RunEventPayload::OperationSucceeded(data) => {
      let key = OperationIdentity {
        invocation_id: data.invocation_id.clone(),
        call_id: data.call_id.clone(),
      };
      let operation = execution.operations.get_mut(&key).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration operation success has no start.".to_string())
      })?;
      if operation.status != OperationStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration operation already settled.".to_string(),
        ));
      }
      operation.status = OperationStatus::Succeeded {
        duration_ms: data.duration_ms,
        result_bytes: data.result_bytes,
        result_digest: data.result_digest.clone(),
      };
    }
    RunEventPayload::OperationFailed(data) => {
      let key = OperationIdentity {
        invocation_id: data.invocation_id.clone(),
        call_id: data.call_id.clone(),
      };
      let operation = execution.operations.get_mut(&key).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration operation failure has no start.".to_string())
      })?;
      if operation.status != OperationStatus::Started {
        return Err(FoldError::InvalidHistory(
          "Iteration operation already settled.".to_string(),
        ));
      }
      operation.status = OperationStatus::Failed {
        duration_ms: data.duration_ms,
        failure: data.failure.clone(),
      };
    }
    RunEventPayload::LifecycleHookRequested(data) => {
      if execution
        .lifecycle_hooks
        .contains_key(&data.hook_invocation_id)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle hook was requested more than once.".to_string(),
        ));
      }
      execution.lifecycle_hooks.insert(
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
    }
    RunEventPayload::LifecycleActionAttemptStarted(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory(
            "Iteration lifecycle action started before its hook.".to_string(),
          )
        })?;
      if hook.actions.contains_key(&data.action_id)
        || hook
          .actions
          .values()
          .any(|action| action.status == LifecycleActionStatus::Started)
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle action identity is duplicated.".to_string(),
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
    }
    RunEventPayload::LifecycleActionSucceeded(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Unknown iteration lifecycle hook.".to_string())
        })?;
      let action = hook.actions.get_mut(&data.action_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lifecycle action has no start.".to_string())
      })?;
      if action.status != LifecycleActionStatus::Started || action.attempt != data.attempt {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle success does not close its action.".to_string(),
        ));
      }
      action.status = LifecycleActionStatus::Succeeded;
    }
    RunEventPayload::LifecycleActionFailed(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Unknown iteration lifecycle hook.".to_string())
        })?;
      let action = hook.actions.get_mut(&data.action_id).ok_or_else(|| {
        FoldError::InvalidHistory("Iteration lifecycle action has no start.".to_string())
      })?;
      if action.status != LifecycleActionStatus::Started || action.attempt != data.attempt {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle failure does not close its action.".to_string(),
        ));
      }
      action.status = LifecycleActionStatus::Failed;
      action.failure = Some(data.failure.clone());
      hook.failed_actions += 1;
    }
    RunEventPayload::LifecycleHookCompleted(data) => {
      let hook = execution
        .lifecycle_hooks
        .get_mut(&data.hook_invocation_id)
        .ok_or_else(|| {
          FoldError::InvalidHistory("Unknown iteration lifecycle hook.".to_string())
        })?;
      if hook
        .actions
        .values()
        .any(|action| action.status == LifecycleActionStatus::Started)
        || hook.failed_actions != data.failed_actions
      {
        return Err(FoldError::InvalidHistory(
          "Iteration lifecycle completion does not match its actions.".to_string(),
        ));
      }
      hook.status = match data.status {
        LifecycleHookCompletionStatus::Completed => LifecycleHookStatus::Completed,
        LifecycleHookCompletionStatus::CompletedWithWarnings => {
          LifecycleHookStatus::CompletedWithWarnings
        }
      };
    }
    RunEventPayload::ReusableLifecycleRequested(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      if execution.reusable_lifecycle_hooks.contains_key(&key) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle hook was requested more than once.".to_string(),
        ));
      }
      execution.reusable_lifecycle_hooks.insert(
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
    }
    RunEventPayload::ReusableLifecycleActionStarted(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.is_some() || hook.completed_action_ids.contains(&data.action_id) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle action is duplicated.".to_string(),
        ));
      }
      hook.active_action_id = Some(data.action_id.clone());
      hook.status = ReusableLifecycleStatus::Running;
    }
    RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.as_deref() != Some(data.action_id.as_str()) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle success has no active action.".to_string(),
        ));
      }
      hook.active_action_id = None;
      hook.completed_action_ids.push(data.action_id.clone());
    }
    RunEventPayload::ReusableLifecycleActionFailed(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.as_deref() != Some(data.action_id.as_str()) {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle failure has no active action.".to_string(),
        ));
      }
      hook.active_action_id = None;
      hook.completed_action_ids.push(data.action_id.clone());
      hook.warning_codes.push(data.warning_code.clone());
    }
    RunEventPayload::ReusableLifecycleCompleted(data) => {
      let key = format!("{}:{:?}", data.invocation_id, data.hook);
      let hook = execution
        .reusable_lifecycle_hooks
        .get_mut(&key)
        .ok_or_else(|| FoldError::InvalidHistory("Unknown iteration reusable hook.".to_string()))?;
      if hook.active_action_id.is_some() {
        return Err(FoldError::InvalidHistory(
          "Iteration reusable lifecycle completed with an active action.".to_string(),
        ));
      }
      hook.status = if data.outcome == crate::ReusableLifecycleOutcome::CompletedWithWarnings {
        ReusableLifecycleStatus::CompletedWithWarnings
      } else {
        ReusableLifecycleStatus::Completed
      };
    }
    _ => {
      return Err(FoldError::InvalidHistory(
        "Payload is not valid iteration-scoped work.".to_string(),
      ))
    }
  }
  Ok(())
}

pub fn fold_events(events: &[RunEvent]) -> Result<RunProjection, FoldError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.fold_events");
  span.count("events", events.len());
  if let Some(run_id) = events.first().map(|event| event.run_id.clone()) {
    span.run_id(run_id);
  }
  let result = fold_events_unprofiled(events);
  if result.is_ok() {
    span.succeed();
  }
  result
}

fn fold_events_unprofiled(events: &[RunEvent]) -> Result<RunProjection, FoldError> {
  let mut projection = RunProjection::default();
  let mut event_ids = HashSet::new();
  let mut attempt_indexes: HashMap<AttemptIdentity, usize> = HashMap::new();

  for (index, event) in events.iter().enumerate() {
    event.validate()?;
    let expected_sequence = index as u64 + 1;
    if event.sequence != expected_sequence {
      return Err(FoldError::InvalidHistory(format!(
        "Expected sequence {expected_sequence}, received {}.",
        event.sequence
      )));
    }
    if !event_ids.insert(event.event_id.as_str()) {
      return Err(FoldError::InvalidHistory(format!(
        "Event ID {:?} appears more than once.",
        event.event_id
      )));
    }
    if let Some(run_id) = &projection.run_id {
      if run_id != &event.run_id {
        return Err(FoldError::InvalidHistory(format!(
          "Run history mixes run IDs {:?} and {:?}.",
          run_id, event.run_id
        )));
      }
    }
    if let Some(event_schema_version) = projection.event_schema_version {
      if event_schema_version != event.event_schema_version {
        return Err(FoldError::InvalidHistory(format!(
          "Run history mixes event schema versions {event_schema_version} and {}.",
          event.event_schema_version
        )));
      }
    }
    if matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
    ) {
      return Err(FoldError::InvalidHistory(
        "A terminal run event must be the final event.".to_string(),
      ));
    }

    let scoped_loop_event = matches!(
      event.payload,
      RunEventPayload::ForEachIterationStarted(_)
        | RunEventPayload::ForEachIterationSucceeded(_)
        | RunEventPayload::ForEachIterationFailed(_)
        | RunEventPayload::ForEachIterationSkipped(_)
    );
    if event.iteration.is_some() && !scoped_loop_event {
      apply_iteration_scoped_event(&mut projection, event, events.get(index.wrapping_sub(1)))?;
      projection.last_sequence = event.sequence;
      continue;
    }

    match &event.payload {
      RunEventPayload::RunAdmitted(data) => {
        if index != 0 || projection.status != RunStatus::NotStarted {
          return Err(FoldError::InvalidHistory(
            "run_admitted must be the first and only admission event.".to_string(),
          ));
        }
        projection.run_id = Some(event.run_id.clone());
        projection.event_schema_version = Some(event.event_schema_version);
        projection.definition_hash = Some(data.definition_hash.clone());
        projection.policy_hash = Some(data.policy_hash.clone());
        projection.trigger_id = Some(data.trigger.id.clone());
        projection.trigger_handler = Some(data.trigger.handler.clone());
        projection.context.trigger = data.payload.clone();
        projection.queue = Some(data.queue.name.clone());
        projection.occurrence_sequence = Some(data.occurrence_sequence);
        projection.admitted_at = Some(data.admitted_at);
        projection.status = RunStatus::Queued;
      }
      RunEventPayload::RunExecutionStarted(data) => {
        if projection.status != RunStatus::Queued || projection.started_at.is_some() {
          return Err(FoldError::InvalidHistory(
            "run_execution_started requires one queued admission.".to_string(),
          ));
        }
        projection.started_at = Some(data.started_at);
        projection.timeout_at = data.timeout_at;
        projection.status = RunStatus::Running;
      }
      RunEventPayload::RunTimeoutReached(data) => {
        if !matches!(projection.status, RunStatus::Running | RunStatus::Waiting)
          || projection.timeout_at != Some(data.deadline_at)
          || projection.timeout_reached_at.is_some()
        {
          return Err(FoldError::InvalidHistory(
            "run_timeout_reached must match the active run deadline exactly once.".to_string(),
          ));
        }
        projection.timeout_reached_at = Some(data.deadline_at);
      }
      RunEventPayload::RunStarted(data) => {
        if index != 0 || projection.status != RunStatus::NotStarted {
          return Err(FoldError::InvalidHistory(
            "run_started must be the first and only start event.".to_string(),
          ));
        }
        projection.run_id = Some(event.run_id.clone());
        projection.event_schema_version = Some(event.event_schema_version);
        projection.workflow_id = Some(data.workflow_id.clone());
        projection.definition_hash = Some(data.definition_hash.clone());
        projection.trigger_id = data.trigger_id.clone();
        projection.trigger_handler = data.trigger_handler.clone();
        projection.trigger_occurrence_id = data.trigger_occurrence_id.clone();
        projection.ingress = data.ingress.clone();
        projection.context.trigger = data.trigger.clone();
        projection.started_at = Some(event.occurred_at);
        projection.status = RunStatus::Running;
      }
      RunEventPayload::StepAttemptStarted(data) => {
        require_running(&projection)?;
        if projection
          .attempts
          .iter()
          .any(|attempt| attempt.identity.invocation_id == data.invocation_id)
        {
          return Err(FoldError::InvalidHistory(format!(
            "Invocation ID {:?} was already used by another attempt.",
            data.invocation_id
          )));
        }
        if projection
          .latest_attempt(&data.node_id)
          .is_some_and(|attempt| attempt.status == AttemptStatus::Started)
        {
          return Err(FoldError::InvalidHistory(format!(
            "Node {:?} already has an active attempt.",
            data.node_id
          )));
        }
        if event.event_schema_version >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
          let previous = projection.latest_attempt(&data.node_id);
          let expected_attempt = previous.map_or(1, |attempt| attempt.identity.attempt + 1);
          if data.attempt != expected_attempt {
            return Err(FoldError::InvalidHistory(format!(
              "Node {:?} expected attempt {expected_attempt}, received {}.",
              data.node_id, data.attempt
            )));
          }
          if let Some(first_key) = projection
            .attempts
            .iter()
            .find(|attempt| attempt.identity.node_id == data.node_id)
            .and_then(|attempt| attempt.idempotency_key.as_deref())
          {
            if data.idempotency_key.as_deref() != Some(first_key) {
              return Err(FoldError::InvalidHistory(format!(
                "Node {:?} changed its stable idempotency key.",
                data.node_id
              )));
            }
          }
          match (previous, projection.pending_retries.get(&data.node_id)) {
            (None, None) => {}
            (Some(previous), Some(schedule))
              if matches!(previous.status, AttemptStatus::Failed { .. })
                && schedule.failed_attempt == previous.identity.attempt
                && schedule.next_attempt == data.attempt
                && event.occurred_at >= schedule.scheduled_at => {}
            (Some(_), Some(schedule)) if event.occurred_at < schedule.scheduled_at => {
              return Err(FoldError::InvalidHistory(format!(
                "Node {:?} attempt {} started before its durable schedule.",
                data.node_id, data.attempt
              )));
            }
            _ => {
              return Err(FoldError::InvalidHistory(format!(
                "Node {:?} attempt {} has no matching pending retry.",
                data.node_id, data.attempt
              )));
            }
          }
          projection.pending_retries.remove(&data.node_id);
        }
        let key = identity(&data.node_id, data.attempt, &data.invocation_id);
        if attempt_indexes.contains_key(&key) {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} was started more than once.",
            key.invocation_id
          )));
        }
        let attempt_index = projection.attempts.len();
        projection.attempts.push(started_attempt(data));
        attempt_indexes.insert(key, attempt_index);
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        require_running(&projection)?;
        if !projection
          .active_managed_operations(&data.invocation_id)
          .is_empty()
        {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} cannot succeed while a managed operation remains active.",
            data.invocation_id
          )));
        }
        let key = identity(&data.node_id, data.attempt, &data.invocation_id);
        let attempt_index = *attempt_indexes.get(&key).ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "Successful attempt {:?} has no matching start event.",
            data.invocation_id
          ))
        })?;
        let attempt = &mut projection.attempts[attempt_index];
        if attempt.status != AttemptStatus::Started {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} already has a terminal event.",
            data.invocation_id
          )));
        }
        if projection.context.steps.contains_key(&data.node_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Node {:?} has already published a successful output.",
            data.node_id
          )));
        }
        attempt.status = AttemptStatus::Succeeded {
          output: data.output.clone(),
        };
        projection
          .context
          .steps
          .insert(data.node_id.clone(), data.output.clone());
      }
      RunEventPayload::StepAttemptFailed(data) => {
        require_running_or_cancelling(&projection)?;
        if !projection
          .active_managed_operations(&data.invocation_id)
          .is_empty()
        {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} cannot fail while a managed operation remains active.",
            data.invocation_id
          )));
        }
        let key = identity(&data.node_id, data.attempt, &data.invocation_id);
        let attempt_index = *attempt_indexes.get(&key).ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "Failed attempt {:?} has no matching start event.",
            data.invocation_id
          ))
        })?;
        let attempt = &mut projection.attempts[attempt_index];
        if attempt.status != AttemptStatus::Started {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} already has a terminal event.",
            data.invocation_id
          )));
        }
        attempt.status = AttemptStatus::Failed {
          failure: data.failure.clone(),
        };
      }
      RunEventPayload::StepRetryScheduled(data) => {
        require_running(&projection)?;
        let previous_event = events.get(index.wrapping_sub(1));
        let closes_previous_failure = previous_event.is_some_and(|previous| {
          matches!(
            &previous.payload,
            RunEventPayload::StepAttemptFailed(failed)
              if failed.node_id == data.node_id
                && failed.attempt == data.failed_attempt
                && failed.failure.kind == crate::AttemptFailureKind::ScriptThrew
          )
        });
        let latest = projection.latest_attempt(&data.node_id);
        if !closes_previous_failure
          || !matches!(
            latest,
            Some(attempt)
              if attempt.identity.attempt == data.failed_attempt
                && matches!(attempt.status, AttemptStatus::Failed { .. })
          )
          || data.next_attempt != data.failed_attempt + 1
          || projection.pending_retries.contains_key(&data.node_id)
        {
          return Err(FoldError::InvalidHistory(format!(
            "Retry schedule for node {:?} does not immediately follow its retryable failed attempt.",
            data.node_id
          )));
        }
        projection.pending_retries.insert(
          data.node_id.clone(),
          RetryScheduleProjection {
            node_id: data.node_id.clone(),
            failed_attempt: data.failed_attempt,
            next_attempt: data.next_attempt,
            scheduled_at: data.scheduled_at,
          },
        );
      }
      RunEventPayload::BranchSelected(data) => {
        require_running(&projection)?;
        if let Some(selected_arm) = projection.branch_selections.get(&data.branch_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Branch {:?} was already selected as arm {:?}.",
            data.branch_id, selected_arm
          )));
        }
        projection
          .branch_selections
          .insert(data.branch_id.clone(), data.arm_id.clone());
      }
      RunEventPayload::ChoiceSelected(data) => {
        require_running(&projection)?;
        if projection.choice_selections.contains_key(&data.choice_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Choice {:?} was selected more than once.",
            data.choice_id
          )));
        }
        projection
          .choice_selections
          .insert(data.choice_id.clone(), data.arm_id.clone());
      }
      RunEventPayload::ForkOpened(data) => {
        require_running(&projection)?;
        if projection.forks.contains_key(&data.fork_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Fork {:?} was opened more than once.",
            data.fork_id
          )));
        }
        projection.forks.insert(
          data.fork_id.clone(),
          ForkProjection {
            fork_id: data.fork_id.clone(),
            branches: BTreeMap::new(),
            join_status: ForkJoinStatus::Pending,
            blocking_branch_id: None,
          },
        );
      }
      RunEventPayload::ForkBranchSettled(data) => {
        require_running_or_cancelling(&projection)?;
        let fork = projection.forks.get_mut(&data.fork_id).ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "Fork branch {:?}.{:?} settled before its fork opened.",
            data.fork_id, data.branch_id
          ))
        })?;
        if fork.branches.contains_key(&data.branch_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Fork branch {:?}.{:?} settled more than once.",
            data.fork_id, data.branch_id
          )));
        }
        fork.branches.insert(
          data.branch_id.clone(),
          ForkBranchProjection {
            branch_id: data.branch_id.clone(),
            terminal_node_id: Some(data.terminal_node_id.clone()),
            outcome: Some(data.outcome),
          },
        );
      }
      RunEventPayload::ForkJoinSettled(data) => {
        require_running_or_cancelling(&projection)?;
        let fork = projection.forks.get_mut(&data.fork_id).ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "Fork {:?} join settled before its fork opened.",
            data.fork_id
          ))
        })?;
        if fork.join_status != ForkJoinStatus::Pending {
          return Err(FoldError::InvalidHistory(format!(
            "Fork {:?} join settled more than once.",
            data.fork_id
          )));
        }
        if let Some(blocking_branch_id) = data.blocking_branch_id.as_deref() {
          let matching_outcome = fork
            .branches
            .get(blocking_branch_id)
            .and_then(|branch| branch.outcome)
            .is_some_and(|outcome| match data.outcome {
              ForkJoinOutcome::Failed => outcome == ForkBranchOutcome::Failed,
              ForkJoinOutcome::Cancelled => outcome == ForkBranchOutcome::Cancelled,
              ForkJoinOutcome::Succeeded => false,
            });
          if !matching_outcome {
            return Err(FoldError::InvalidHistory(format!(
              "Fork {:?} join outcome is not proven by blocking branch {:?}.",
              data.fork_id, blocking_branch_id
            )));
          }
        }
        fork.join_status = match data.outcome {
          ForkJoinOutcome::Succeeded => ForkJoinStatus::Succeeded,
          ForkJoinOutcome::Failed => ForkJoinStatus::Failed,
          ForkJoinOutcome::Cancelled => ForkJoinStatus::Cancelled,
        };
        fork.blocking_branch_id = data.blocking_branch_id.clone();
      }
      RunEventPayload::ForEachOpened(data) => {
        require_running(&projection)?;
        if projection.for_each.contains_key(&data.for_each_id) {
          return Err(FoldError::InvalidHistory(format!(
            "For-each {:?} was opened more than once.",
            data.for_each_id
          )));
        }
        projection.for_each.insert(
          data.for_each_id.clone(),
          ForEachProjection {
            for_each_id: data.for_each_id.clone(),
            total: data.total,
            items_digest: data.items_digest.clone(),
            concurrency: data.concurrency,
            status: ForEachStatus::Open,
            iterations: BTreeMap::new(),
            executions: BTreeMap::new(),
          },
        );
      }
      RunEventPayload::ForEachIterationStarted(data) => {
        require_running(&projection)?;
        let scope = event.iteration.as_ref().expect("validated iteration scope");
        let loop_state = projection
          .for_each
          .get_mut(&data.for_each_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(format!(
              "For-each iteration {:?}:{} started before its loop opened.",
              data.for_each_id, scope.index
            ))
          })?;
        if loop_state.status != ForEachStatus::Open
          || scope.index >= loop_state.total
          || loop_state.iterations.contains_key(&scope.index)
        {
          return Err(FoldError::InvalidHistory(format!(
            "For-each iteration {:?}:{} cannot start in its current state.",
            data.for_each_id, scope.index
          )));
        }
        loop_state
          .iterations
          .insert(scope.index, ForEachIterationStatus::Started);
        loop_state.executions.insert(
          scope.index,
          ForEachIterationExecutionProjection {
            context: projection.context.clone(),
            ..ForEachIterationExecutionProjection::default()
          },
        );
      }
      RunEventPayload::ForEachIterationSucceeded(data) => {
        require_running(&projection)?;
        let scope = event.iteration.as_ref().expect("validated iteration scope");
        if let (Some(result), Some(expected_digest)) = (&data.result, &data.result_digest) {
          if projection_digest(result)? != *expected_digest {
            return Err(FoldError::InvalidHistory(format!(
              "For-each iteration {:?}:{} result digest does not match its value.",
              data.for_each_id, scope.index
            )));
          }
        }
        let loop_state = projection
          .for_each
          .get_mut(&data.for_each_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("For-each iteration succeeded before opening.".to_string())
          })?;
        match loop_state.iterations.get_mut(&scope.index) {
          Some(status @ ForEachIterationStatus::Started) => {
            *status = ForEachIterationStatus::Succeeded {
              result: data.result.clone(),
            };
          }
          _ => {
            return Err(FoldError::InvalidHistory(format!(
              "For-each iteration {:?}:{} succeeded without one active start.",
              data.for_each_id, scope.index
            )));
          }
        }
      }
      RunEventPayload::ForEachIterationFailed(data) => {
        require_running_or_cancelling(&projection)?;
        let scope = event.iteration.as_ref().expect("validated iteration scope");
        let loop_state = projection
          .for_each
          .get_mut(&data.for_each_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("For-each iteration failed before opening.".to_string())
          })?;
        match loop_state.iterations.get_mut(&scope.index) {
          Some(status @ ForEachIterationStatus::Started) => {
            *status = ForEachIterationStatus::Failed {
              failed_node_id: data.failed_node_id.clone(),
            };
            if let Some(execution) = loop_state.executions.get_mut(&scope.index) {
              execution.pending_retries.clear();
            }
          }
          _ => {
            return Err(FoldError::InvalidHistory(format!(
              "For-each iteration {:?}:{} failed without one active start.",
              data.for_each_id, scope.index
            )));
          }
        }
      }
      RunEventPayload::ForEachIterationSkipped(data) => {
        require_running_or_cancelling(&projection)?;
        let scope = event.iteration.as_ref().expect("validated iteration scope");
        let loop_state = projection
          .for_each
          .get_mut(&data.for_each_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("For-each iteration skipped before opening.".to_string())
          })?;
        if loop_state.status != ForEachStatus::Open
          || scope.index >= loop_state.total
          || loop_state.iterations.contains_key(&scope.index)
        {
          return Err(FoldError::InvalidHistory(format!(
            "For-each iteration {:?}:{} cannot be skipped in its current state.",
            data.for_each_id, scope.index
          )));
        }
        loop_state
          .iterations
          .insert(scope.index, ForEachIterationStatus::Skipped);
      }
      RunEventPayload::ForEachSucceeded(data)
      | RunEventPayload::ForEachFailed(data)
      | RunEventPayload::ForEachCancelled(data) => {
        require_running_or_cancelling(&projection)?;
        let is_success = matches!(&event.payload, RunEventPayload::ForEachSucceeded(_));
        let is_cancelled = matches!(&event.payload, RunEventPayload::ForEachCancelled(_));
        let loop_state = projection
          .for_each
          .get_mut(&data.for_each_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("For-each settled before opening.".to_string())
          })?;
        let succeeded = loop_state
          .iterations
          .values()
          .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
          .count() as u32;
        let failed = loop_state
          .iterations
          .values()
          .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
          .count() as u32;
        let skipped = loop_state
          .iterations
          .values()
          .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
          .count() as u32;
        let pending_retry_count = loop_state
          .executions
          .values()
          .map(|execution| execution.pending_retries.len())
          .sum::<usize>();
        let active_attempt_count = loop_state
          .executions
          .values()
          .flat_map(|execution| &execution.attempts)
          .filter(|attempt| attempt.status == AttemptStatus::Started)
          .count();
        let active_attempts = loop_state
          .executions
          .iter()
          .flat_map(|(index, execution)| {
            execution
              .attempts
              .iter()
              .filter(|attempt| attempt.status == AttemptStatus::Started)
              .map(move |attempt| {
                format!(
                  "{}:{}:{}:{}",
                  index,
                  attempt.identity.node_id,
                  attempt.identity.attempt,
                  attempt.identity.invocation_id
                )
              })
          })
          .collect::<Vec<_>>();
        let active_operation_count = loop_state
          .executions
          .values()
          .flat_map(|execution| execution.operations.values())
          .filter(|operation| operation.status == OperationStatus::Started)
          .count();
        let active_lifecycle_count = loop_state
          .executions
          .values()
          .flat_map(|execution| execution.lifecycle_hooks.values())
          .filter(|hook| {
            matches!(
              hook.status,
              LifecycleHookStatus::Requested | LifecycleHookStatus::Running
            )
          })
          .count();
        let active_reusable_count = loop_state
          .executions
          .values()
          .flat_map(|execution| execution.reusable_lifecycle_hooks.values())
          .filter(|hook| {
            matches!(
              hook.status,
              ReusableLifecycleStatus::Requested | ReusableLifecycleStatus::Running
            )
          })
          .count();
        let active_parallel_count = loop_state
          .executions
          .values()
          .flat_map(|execution| execution.parallel_groups.values())
          .filter(|group| group.status == ParallelGroupStatus::Started)
          .count();
        let owns_active_work = pending_retry_count != 0
          || active_attempt_count != 0
          || active_operation_count != 0
          || active_lifecycle_count != 0
          || active_reusable_count != 0
          || active_parallel_count != 0;
        if loop_state.status != ForEachStatus::Open
          || data.total != loop_state.total
          || (data.succeeded, data.failed, data.skipped) != (succeeded, failed, skipped)
          || succeeded + failed + skipped != loop_state.total
          || (is_success && (failed != 0 || skipped != 0))
          || owns_active_work
        {
          return Err(FoldError::InvalidHistory(format!(
            "For-each {:?} settlement does not match its folded iterations: event=({},{},{}), folded=({succeeded},{failed},{skipped}), total={}, activeWork={owns_active_work} (retries={pending_retry_count}, attempts={active_attempt_count} {active_attempts:?}, operations={active_operation_count}, lifecycle={active_lifecycle_count}, reusable={active_reusable_count}, parallel={active_parallel_count}).",
            data.for_each_id, data.succeeded, data.failed, data.skipped, loop_state.total
          )));
        }
        loop_state.status = if is_success {
          ForEachStatus::Succeeded
        } else if is_cancelled {
          ForEachStatus::Cancelled
        } else {
          ForEachStatus::Failed
        };
        if is_success {
          let ordered_results = (0..loop_state.total)
            .filter_map(|index| match loop_state.iterations.get(&index) {
              Some(ForEachIterationStatus::Succeeded {
                result: Some(result),
              }) => Some(result.clone()),
              _ => None,
            })
            .collect::<Vec<_>>();
          let mut output = Map::new();
          output.insert("total".to_string(), Value::from(loop_state.total));
          output.insert("succeeded".to_string(), Value::from(succeeded));
          if !ordered_results.is_empty() {
            output.insert("results".to_string(), Value::Array(ordered_results));
          }
          let output = Value::Object(output);
          if projection_digest(&output)? != data.aggregate_digest {
            return Err(FoldError::InvalidHistory(format!(
              "For-each {:?} aggregate digest does not match its public output.",
              data.for_each_id
            )));
          }
          projection
            .context
            .steps
            .insert(data.for_each_id.clone(), output);
        }
      }
      RunEventPayload::ParallelGroupStarted(data) => {
        require_running(&projection)?;
        if projection.parallel_groups.contains_key(&data.parallel_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Parallel group {:?} was started more than once.",
            data.parallel_id
          )));
        }
        projection.parallel_groups.insert(
          data.parallel_id.clone(),
          ParallelGroupProjection {
            parallel_id: data.parallel_id.clone(),
            fork_context: projection.context.clone(),
            status: ParallelGroupStatus::Started,
          },
        );
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        require_running_or_cancelling(&projection)?;
        let group = projection
          .parallel_groups
          .get_mut(&data.parallel_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(format!(
              "Parallel group {:?} completed before it started.",
              data.parallel_id
            ))
          })?;
        if !matches!(group.status, ParallelGroupStatus::Started) {
          return Err(FoldError::InvalidHistory(format!(
            "Parallel group {:?} was completed more than once.",
            data.parallel_id
          )));
        }
        group.status = ParallelGroupStatus::Completed {
          outcome: data.outcome,
          failed_node_ids: data.failed_node_ids.clone(),
          cancelled_node_ids: data.cancelled_node_ids.clone(),
        };
      }
      RunEventPayload::ApprovalRequested(data) => {
        require_running(&projection)?;
        if projection.approval_requests.contains_key(&data.approval_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Approval {:?} was requested more than once.",
            data.approval_id
          )));
        }
        if projection
          .approval_requests
          .values()
          .any(|request| request.request_id == data.request_id)
        {
          return Err(FoldError::InvalidHistory(format!(
            "Approval request ID {:?} appears more than once.",
            data.request_id
          )));
        }
        projection.approval_requests.insert(
          data.approval_id.clone(),
          ApprovalRequestProjection {
            approval_id: data.approval_id.clone(),
            request_id: data.request_id.clone(),
            requested_at: event.occurred_at,
            expires_at: data.expires_at,
            on_timeout: data.on_timeout,
            status: ApprovalRequestStatus::Waiting,
          },
        );
        projection.status = RunStatus::Waiting;
      }
      RunEventPayload::NotificationDeliveryRequested(data) => {
        if projection.status != RunStatus::Waiting {
          return Err(FoldError::InvalidHistory(
            "notification_delivery_requested requires a waiting approval.".to_string(),
          ));
        }
        let request = projection
          .approval_requests
          .get(&data.approval_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(
              "Notification delivery references an unknown approval.".to_string(),
            )
          })?;
        if request.request_id != data.request_id
          || !matches!(request.status, ApprovalRequestStatus::Waiting)
          || projection
            .notification_deliveries
            .contains_key(&data.delivery_id)
        {
          return Err(FoldError::InvalidHistory(
            "Notification delivery intent does not match the active approval request.".to_string(),
          ));
        }
        projection.notification_deliveries.insert(
          data.delivery_id.clone(),
          NotificationDeliveryProjection {
            approval_id: data.approval_id.clone(),
            request_id: data.request_id.clone(),
            delivery_id: data.delivery_id.clone(),
            provider: data.provider.clone(),
            destination: data.destination.clone(),
            status: NotificationDeliveryStatus::Requested,
          },
        );
      }
      RunEventPayload::NotificationDeliveryAttemptStarted(data) => {
        if projection.status != RunStatus::Waiting {
          return Err(FoldError::InvalidHistory(
            "Notification delivery attempts require a waiting approval.".to_string(),
          ));
        }
        let delivery = projection
          .notification_deliveries
          .get_mut(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(
              "Notification attempt started before its durable intent.".to_string(),
            )
          })?;
        if delivery.approval_id != data.approval_id || delivery.request_id != data.request_id {
          return Err(FoldError::InvalidHistory(
            "Notification attempt identity does not match its intent.".to_string(),
          ));
        }
        let expected_attempt = match &delivery.status {
          NotificationDeliveryStatus::Requested => 1,
          NotificationDeliveryStatus::Failed {
            attempt,
            final_: false,
            failure,
            ..
          } if failure.retryable => attempt + 1,
          _ => 0,
        };
        if data.attempt != expected_attempt {
          return Err(FoldError::InvalidHistory(
            "Notification attempt is duplicated or out of order.".to_string(),
          ));
        }
        delivery.status = NotificationDeliveryStatus::AttemptStarted {
          attempt: data.attempt,
          attempt_id: data.attempt_id.clone(),
          idempotency_key: data.idempotency_key.clone(),
        };
      }
      RunEventPayload::NotificationDeliverySucceeded(data) => {
        let delivery = projection
          .notification_deliveries
          .get_mut(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Notification success has no durable intent.".to_string())
          })?;
        if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, .. } if *attempt == data.attempt && attempt_id == &data.attempt_id)
        {
          return Err(FoldError::InvalidHistory(
            "Notification success does not close its active attempt.".to_string(),
          ));
        }
        delivery.status = NotificationDeliveryStatus::Succeeded {
          attempt: data.attempt,
          attempt_id: data.attempt_id.clone(),
          provider_message: data.provider_message.clone(),
        };
      }
      RunEventPayload::NotificationDeliveryFailed(data) => {
        let delivery = projection
          .notification_deliveries
          .get_mut(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Notification failure has no durable intent.".to_string())
          })?;
        if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, .. } if *attempt == data.attempt && attempt_id == &data.attempt_id)
        {
          return Err(FoldError::InvalidHistory(
            "Notification failure does not close its active attempt.".to_string(),
          ));
        }
        delivery.status = NotificationDeliveryStatus::Failed {
          attempt: data.attempt,
          attempt_id: data.attempt_id.clone(),
          final_: data.final_,
          failure: data.failure.clone(),
          failed_at: event.occurred_at,
        };
      }
      RunEventPayload::NotificationDecisionAccepted(data) => {
        if projection.status != RunStatus::Waiting || !projection.notification_decisions.is_empty()
        {
          return Err(FoldError::InvalidHistory(
            "Only one notification decision may be accepted while waiting.".to_string(),
          ));
        }
        let delivery = projection
          .notification_deliveries
          .get(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(
              "Notification decision references an unknown delivery.".to_string(),
            )
          })?;
        if delivery.approval_id != data.approval_id
          || delivery.request_id != data.request_id
          || !matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
        {
          return Err(FoldError::InvalidHistory(
            "Notification decision requires a successful delivery for the active request."
              .to_string(),
          ));
        }
        projection.notification_decisions.push(data.clone());
      }
      RunEventPayload::ApprovalResolved(data) => {
        if projection.status != RunStatus::Waiting {
          return Err(FoldError::InvalidHistory(
            "approval_resolved requires a waiting run.".to_string(),
          ));
        }
        let request = projection
          .approval_requests
          .get_mut(&data.approval_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(format!(
              "Approval {:?} resolved before it was requested.",
              data.approval_id
            ))
          })?;
        if request.request_id != data.request_id {
          return Err(FoldError::InvalidHistory(format!(
            "Approval {:?} resolved with a mismatched request ID.",
            data.approval_id
          )));
        }
        if !matches!(request.status, ApprovalRequestStatus::Waiting) {
          return Err(FoldError::InvalidHistory(format!(
            "Approval {:?} was resolved more than once.",
            data.approval_id
          )));
        }
        match &data.resolution {
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          } if request
            .expires_at
            .is_some_and(|deadline| event.occurred_at >= deadline) =>
          {
            return Err(FoldError::InvalidHistory(format!(
              "Human decision for approval {:?} occurred at or after its deadline.",
              data.approval_id
            )));
          }
          ApprovalResolution::Decision {
            decision: ApprovalDecision::Rejected,
            source: ApprovalDecisionSource::Timeout,
          } if request.on_timeout == ApprovalTimeoutPolicy::Reject
            && request
              .expires_at
              .is_some_and(|deadline| event.occurred_at >= deadline) => {}
          ApprovalResolution::TimeoutFailure
            if request.on_timeout == ApprovalTimeoutPolicy::Fail
              && request
                .expires_at
                .is_some_and(|deadline| event.occurred_at >= deadline) => {}
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          } => {}
          _ => {
            return Err(FoldError::InvalidHistory(format!(
              "Approval {:?} resolution does not match its deadline and timeout policy.",
              data.approval_id
            )));
          }
        }
        if let ApprovalResolution::Decision { decision, source } = data.resolution {
          projection.context.steps.insert(
            data.approval_id.clone(),
            json!({
              "decision": match decision {
                ApprovalDecision::Approved => "approved",
                ApprovalDecision::Rejected => "rejected",
              },
              "source": match source {
                ApprovalDecisionSource::Human => "human",
                ApprovalDecisionSource::Timeout => "timeout",
              },
              "decidedAt": event
                .occurred_at
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            }),
          );
        }
        request.status = ApprovalRequestStatus::Resolved {
          resolution: data.resolution.clone(),
          resolved_at: event.occurred_at,
        };
        projection.status = RunStatus::Running;
      }
      RunEventPayload::NotificationMessageUpdateRequested(data) => {
        if projection.status != RunStatus::Running {
          return Err(FoldError::InvalidHistory(
            "Message update work requires a resolved approval.".to_string(),
          ));
        }
        let delivery = projection
          .notification_deliveries
          .get(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Message update references an unknown delivery.".to_string())
          })?;
        if delivery.approval_id != data.approval_id
          || delivery.request_id != data.request_id
          || !matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
          || projection
            .notification_updates
            .contains_key(&data.delivery_id)
        {
          return Err(FoldError::InvalidHistory(
            "Message update intent does not match a successful delivery.".to_string(),
          ));
        }
        projection.notification_updates.insert(
          data.delivery_id.clone(),
          NotificationMessageUpdateProjection {
            approval_id: data.approval_id.clone(),
            request_id: data.request_id.clone(),
            delivery_id: data.delivery_id.clone(),
            update_id: data.update_id.clone(),
            resolution: data.resolution,
            status: NotificationMessageUpdateStatus::Requested,
          },
        );
      }
      RunEventPayload::NotificationMessageUpdateAttemptStarted(data) => {
        let update = projection
          .notification_updates
          .get_mut(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(
              "Message update attempt started before its intent.".to_string(),
            )
          })?;
        let expected = match &update.status {
          NotificationMessageUpdateStatus::Requested => 1,
          NotificationMessageUpdateStatus::Failed {
            attempt,
            final_: false,
            failure,
            ..
          } if failure.retryable => attempt + 1,
          _ => 0,
        };
        if update.update_id != data.update_id || data.attempt != expected {
          return Err(FoldError::InvalidHistory(
            "Message update attempt is duplicated or out of order.".to_string(),
          ));
        }
        update.status = NotificationMessageUpdateStatus::AttemptStarted {
          attempt: data.attempt,
          attempt_id: data.attempt_id.clone(),
        };
      }
      RunEventPayload::NotificationMessageUpdated(data) => {
        let update = projection
          .notification_updates
          .get_mut(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Message update success has no intent.".to_string())
          })?;
        if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == data.attempt && attempt_id == &data.attempt_id)
        {
          return Err(FoldError::InvalidHistory(
            "Message update success does not close its attempt.".to_string(),
          ));
        }
        update.status = NotificationMessageUpdateStatus::Updated {
          attempt: data.attempt,
          attempt_id: data.attempt_id.clone(),
        };
      }
      RunEventPayload::NotificationMessageUpdateFailed(data) => {
        let update = projection
          .notification_updates
          .get_mut(&data.delivery_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Message update failure has no intent.".to_string())
          })?;
        if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == data.attempt && attempt_id == &data.attempt_id)
        {
          return Err(FoldError::InvalidHistory(
            "Message update failure does not close its attempt.".to_string(),
          ));
        }
        update.status = NotificationMessageUpdateStatus::Failed {
          attempt: data.attempt,
          attempt_id: data.attempt_id.clone(),
          final_: data.final_,
          failure: data.failure.clone(),
          failed_at: event.occurred_at,
        };
      }
      RunEventPayload::OperationStarted(data) => {
        require_operation_runtime(&projection)?;
        let attempt_active = projection.attempts.iter().any(|attempt| {
          attempt.identity.node_id == data.node_id
            && attempt.identity.attempt == data.attempt_number
            && attempt.identity.invocation_id == data.invocation_id
            && attempt.status == AttemptStatus::Started
        }) || projection.lifecycle_hooks.values().any(|hook| {
          hook.actions.get(&data.node_id).is_some_and(|action| {
            action.attempt == data.attempt_number && action.status == LifecycleActionStatus::Started
          })
        });
        if !attempt_active {
          return Err(FoldError::InvalidHistory(
            "operation_started requires its matching active step or lifecycle action attempt."
              .to_string(),
          ));
        }
        let key = OperationIdentity {
          invocation_id: data.invocation_id.clone(),
          call_id: data.call_id.clone(),
        };
        if projection.operations.contains_key(&key) {
          return Err(FoldError::InvalidHistory(format!(
            "Operation call {:?} was started more than once in invocation {:?}.",
            data.call_id, data.invocation_id
          )));
        }
        if projection.operations.values().any(|operation| {
          operation.identity.invocation_id == data.invocation_id
            && operation.operation_key == data.operation_key
        }) {
          return Err(FoldError::InvalidHistory(format!(
            "Logical operation key {:?} was reused within invocation {:?}.",
            data.operation_key, data.invocation_id
          )));
        }
        projection.operations.insert(
          key.clone(),
          OperationProjection {
            identity: key,
            node_id: data.node_id.clone(),
            attempt_number: data.attempt_number,
            operation_key: data.operation_key.clone(),
            capability: data.capability.clone(),
            operation: data.operation.clone(),
            execution_mode: data.execution_mode,
            metadata: data.metadata.clone(),
            status: OperationStatus::Started,
          },
        );
      }
      RunEventPayload::OperationSucceeded(data) => {
        require_operation_runtime(&projection)?;
        require_active_operation_attempt(
          &projection,
          &data.node_id,
          data.attempt_number,
          &data.invocation_id,
        )?;
        let key = OperationIdentity {
          invocation_id: data.invocation_id.clone(),
          call_id: data.call_id.clone(),
        };
        let operation = projection.operations.get_mut(&key).ok_or_else(|| {
          FoldError::InvalidHistory("operation_succeeded has no matching start.".to_string())
        })?;
        validate_operation_terminal_identity(
          operation,
          &data.node_id,
          data.attempt_number,
          &data.operation_key,
          &data.capability,
          &data.operation,
          data.execution_mode,
        )?;
        operation.metadata = data.metadata.clone();
        operation.status = OperationStatus::Succeeded {
          duration_ms: data.duration_ms,
          result_bytes: data.result_bytes,
          result_digest: data.result_digest.clone(),
        };
      }
      RunEventPayload::OperationFailed(data) => {
        require_operation_settlement(&projection)?;
        require_active_operation_attempt(
          &projection,
          &data.node_id,
          data.attempt_number,
          &data.invocation_id,
        )?;
        let key = OperationIdentity {
          invocation_id: data.invocation_id.clone(),
          call_id: data.call_id.clone(),
        };
        let operation = projection.operations.get_mut(&key).ok_or_else(|| {
          FoldError::InvalidHistory("operation_failed has no matching start.".to_string())
        })?;
        validate_operation_terminal_identity(
          operation,
          &data.node_id,
          data.attempt_number,
          &data.operation_key,
          &data.capability,
          &data.operation,
          data.execution_mode,
        )?;
        operation.metadata = data.metadata.clone();
        operation.status = OperationStatus::Failed {
          duration_ms: data.duration_ms,
          failure: data.failure.clone(),
        };
      }
      RunEventPayload::RunCancellationRequested(data) => {
        if !matches!(
          projection.status,
          RunStatus::Queued | RunStatus::Running | RunStatus::Waiting
        ) {
          return Err(FoldError::InvalidHistory(
            "Cancellation may only be requested before business outcome is decided.".to_string(),
          ));
        }
        if projection.cancellation_request_id.is_some() {
          return Err(FoldError::InvalidHistory(
            "A run may contain only one durable cancellation request.".to_string(),
          ));
        }
        projection.cancellation_request_id = Some(data.request_id.clone());
        projection.status = RunStatus::Cancelling;
      }
      RunEventPayload::LifecycleHookRequested(data) => {
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
      }
      RunEventPayload::LifecycleActionAttemptStarted(data) => {
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
            "A lifecycle hook may have only one active, uniquely identified action attempt."
              .to_string(),
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
      }
      RunEventPayload::LifecycleActionSucceeded(data) => {
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
      }
      RunEventPayload::LifecycleActionFailed(data) => {
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
      }
      RunEventPayload::LifecycleHookCompleted(data) => {
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
      }
      RunEventPayload::ReusableLifecycleRequested(data) => {
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
      }
      RunEventPayload::ReusableLifecycleActionStarted(data) => {
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
      }
      RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
        let key = format!("{}:{:?}", data.invocation_id, data.hook);
        let hook = projection
          .reusable_lifecycle_hooks
          .get_mut(&key)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Unknown reusable lifecycle hook.".to_string())
          })?;
        if hook.definition_digest != data.definition_digest
          || hook.active_action_id.as_deref() != Some(data.action_id.as_str())
        {
          return Err(FoldError::InvalidHistory(
            "Reusable lifecycle success does not close its active action.".to_string(),
          ));
        }
        hook.active_action_id = None;
        hook.completed_action_ids.push(data.action_id.clone());
      }
      RunEventPayload::ReusableLifecycleActionFailed(data) => {
        let key = format!("{}:{:?}", data.invocation_id, data.hook);
        let hook = projection
          .reusable_lifecycle_hooks
          .get_mut(&key)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Unknown reusable lifecycle hook.".to_string())
          })?;
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
      }
      RunEventPayload::ReusableLifecycleCompleted(data) => {
        let key = format!("{}:{:?}", data.invocation_id, data.hook);
        let hook = projection
          .reusable_lifecycle_hooks
          .get_mut(&key)
          .ok_or_else(|| {
            FoldError::InvalidHistory("Unknown reusable lifecycle hook.".to_string())
          })?;
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
      }
      RunEventPayload::RunOutcomeDecided(data) => {
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
            if projection.cancellation_request_id.as_deref()
              != Some(cancellation_request_id.as_str())
            {
              return Err(FoldError::InvalidHistory(
                "Cancelled outcome does not match the durable cancellation request.".to_string(),
              ));
            }
          }
        }
        projection.business_outcome = Some(data.outcome());
        projection.lifecycle_status = LifecycleStatus::Finalizing;
        projection.status = RunStatus::Finalizing;
      }
      RunEventPayload::RunFinalized(data) => {
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
            "run_finalized requires the decided outcome and no unfinished lifecycle hook."
              .to_string(),
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
      }
      RunEventPayload::RunSucceeded(data) => {
        require_running(&projection)?;
        if projection.operations.values().any(|operation| {
          operation.execution_mode == OperationExecutionMode::Managed
            && operation.status == OperationStatus::Started
        }) {
          return Err(FoldError::InvalidHistory(
            "A run cannot succeed while a managed operation remains active.".to_string(),
          ));
        }
        if !projection.pending_retries.is_empty() {
          return Err(FoldError::InvalidHistory(
            "A run cannot succeed while a retry remains pending.".to_string(),
          ));
        }
        let terminal_output_exists = projection
          .context
          .steps
          .contains_key(&data.terminal_node_id)
          || data
            .terminal_node_id
            .strip_prefix("__woml_approval__")
            .and_then(|value| value.strip_suffix("__join"))
            .is_some_and(|approval_id| projection.context.steps.contains_key(approval_id));
        if !terminal_output_exists {
          return Err(FoldError::InvalidHistory(format!(
            "Terminal node {:?} has not published a successful output.",
            data.terminal_node_id
          )));
        }
        projection.status = RunStatus::Succeeded;
        projection.business_outcome = Some(BusinessOutcome::Succeeded);
        projection.lifecycle_status = LifecycleStatus::Completed;
        projection.terminal_node_id = Some(data.terminal_node_id.clone());
        projection.result = Some(data.result.clone());
      }
      RunEventPayload::RunFailed(data) => {
        if projection.operations.values().any(|operation| {
          operation.execution_mode == OperationExecutionMode::Managed
            && operation.status == OperationStatus::Started
        }) {
          return Err(FoldError::InvalidHistory(
            "A run cannot fail while a managed operation remains active.".to_string(),
          ));
        }
        if matches!(data, RunFailedData::V5(_)) {
          if projection.status != RunStatus::Waiting {
            return Err(FoldError::InvalidHistory(
              "Notification-scoped run_failed requires a waiting approval.".to_string(),
            ));
          }
        } else {
          require_running(&projection)?;
        }
        let failure = match data {
          RunFailedData::V5(RunFailedDataV5::Notification {
            approval_id,
            request_id,
            failed_delivery_ids,
            failure,
          }) => {
            let request = projection
              .approval_requests
              .get(approval_id)
              .ok_or_else(|| {
                FoldError::InvalidHistory(
                  "Notification run failure references an unknown approval.".to_string(),
                )
              })?;
            let actual_failed = projection
              .notification_deliveries
              .values()
              .filter(|delivery| {
                delivery.approval_id == *approval_id && delivery.request_id == *request_id
              })
              .filter_map(|delivery| {
                matches!(
                  delivery.status,
                  NotificationDeliveryStatus::Failed { final_: true, .. }
                )
                .then_some(delivery.delivery_id.clone())
              })
              .collect::<Vec<_>>();
            let any_succeeded = projection.notification_deliveries.values().any(|delivery| {
              delivery.approval_id == *approval_id
                && delivery.request_id == *request_id
                && matches!(
                  delivery.status,
                  NotificationDeliveryStatus::Succeeded { .. }
                )
            });
            if request.request_id != *request_id
              || any_succeeded
              || actual_failed != *failed_delivery_ids
            {
              return Err(FoldError::InvalidHistory(
                "Notification run failure does not match final delivery outcomes.".to_string(),
              ));
            }
            RunFailure::Notification {
              approval_id: approval_id.clone(),
              request_id: request_id.clone(),
              failed_delivery_ids: failed_delivery_ids.clone(),
              failure: failure.clone(),
            }
          }
          RunFailedData::V1(data) => {
            if let (Some(node_id), Some(attempt), Some(invocation_id)) =
              (&data.node_id, data.attempt, &data.invocation_id)
            {
              require_failed_attempt(
                &projection,
                &attempt_indexes,
                node_id,
                attempt,
                invocation_id,
              )?;
            }
            RunFailure::Attempt(data.failure.clone())
          }
          RunFailedData::V2(RunFailedDataV2::Attempt {
            node_id,
            attempt,
            invocation_id,
            failure,
          }) => {
            require_failed_attempt(
              &projection,
              &attempt_indexes,
              node_id,
              *attempt,
              invocation_id,
            )?;
            RunFailure::Attempt(failure.clone())
          }
          RunFailedData::V2(RunFailedDataV2::Branch { failure, .. }) => {
            RunFailure::Branch(failure.clone())
          }
          RunFailedData::V3(RunFailedDataV3::Parallel {
            parallel_id,
            policy,
            primary_node_id,
            failed_node_ids,
            cancelled_node_ids,
            failure,
          }) => {
            let group = projection.parallel_groups.get(parallel_id).ok_or_else(|| {
              FoldError::InvalidHistory(format!(
                "run_failed references parallel group {parallel_id:?} before it started."
              ))
            })?;
            match &group.status {
              ParallelGroupStatus::Completed {
                outcome: ParallelGroupOutcome::Failed,
                failed_node_ids: completed_failed,
                cancelled_node_ids: completed_cancelled,
              } if completed_failed == failed_node_ids
                && completed_cancelled == cancelled_node_ids => {}
              _ => {
                return Err(FoldError::InvalidHistory(format!(
                  "run_failed does not match the failed completion of parallel group {parallel_id:?}."
                )));
              }
            }
            RunFailure::Parallel {
              parallel_id: parallel_id.clone(),
              policy: *policy,
              primary_node_id: primary_node_id.clone(),
              failed_node_ids: failed_node_ids.clone(),
              cancelled_node_ids: cancelled_node_ids.clone(),
              failure: failure.clone(),
            }
          }
          RunFailedData::V4(RunFailedDataV4::Approval {
            approval_id,
            request_id,
            failure,
          }) => {
            let request = projection
              .approval_requests
              .get(approval_id)
              .ok_or_else(|| {
                FoldError::InvalidHistory(format!(
                  "run_failed references unknown approval {approval_id:?}."
                ))
              })?;
            if request.request_id != *request_id
              || !matches!(
                request.status,
                ApprovalRequestStatus::Resolved {
                  resolution: ApprovalResolution::TimeoutFailure,
                  ..
                }
              )
            {
              return Err(FoldError::InvalidHistory(format!(
                "run_failed does not match the timeout failure of approval {approval_id:?}."
              )));
            }
            RunFailure::Approval {
              approval_id: approval_id.clone(),
              request_id: request_id.clone(),
              failure: failure.clone(),
            }
          }
        };
        projection.pending_retries.clear();
        projection.status = RunStatus::Failed;
        projection.business_outcome = Some(BusinessOutcome::Failed);
        projection.lifecycle_status = LifecycleStatus::Completed;
        projection.failure = Some(failure);
      }
    }
    projection.last_sequence = event.sequence;
  }
  Ok(projection)
}

fn projection_digest(value: &Value) -> Result<String, FoldError> {
  let bytes = serde_json_canonicalizer::to_vec(value).map_err(|error| {
    FoldError::InvalidHistory(format!("A durable JSON value is not canonical: {error}"))
  })?;
  Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn require_failed_attempt(
  projection: &RunProjection,
  attempt_indexes: &HashMap<AttemptIdentity, usize>,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Result<(), FoldError> {
  let key = identity(node_id, attempt, invocation_id);
  let attempt_index = attempt_indexes.get(&key).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "run_failed references unknown attempt {invocation_id:?}."
    ))
  })?;
  if !matches!(
    projection.attempts[*attempt_index].status,
    AttemptStatus::Failed { .. }
  ) {
    return Err(FoldError::InvalidHistory(format!(
      "run_failed references attempt {invocation_id:?} before it failed."
    )));
  }
  Ok(())
}

fn require_running(projection: &RunProjection) -> Result<(), FoldError> {
  if projection.status != RunStatus::Running {
    return Err(FoldError::InvalidHistory(
      "A run must be started and nonterminal before attempt or terminal events.".to_string(),
    ));
  }
  Ok(())
}

fn require_running_or_cancelling(projection: &RunProjection) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Running | RunStatus::Cancelling
  ) {
    return Err(FoldError::InvalidHistory(
      "An attempt may only settle while its run is running or cancelling.".to_string(),
    ));
  }
  Ok(())
}

fn require_operation_runtime(projection: &RunProjection) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Running | RunStatus::Finalizing
  ) {
    return Err(FoldError::InvalidHistory(
      "An operation requires a running or lifecycle-finalizing run.".to_string(),
    ));
  }
  Ok(())
}

fn require_operation_settlement(projection: &RunProjection) -> Result<(), FoldError> {
  if !matches!(
    projection.status,
    RunStatus::Running | RunStatus::Cancelling | RunStatus::Finalizing
  ) {
    return Err(FoldError::InvalidHistory(
      "An operation may only settle for an active or finalizing run.".to_string(),
    ));
  }
  Ok(())
}
