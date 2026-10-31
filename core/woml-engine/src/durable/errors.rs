//! Stable durable-store errors and their existing conversion/display contracts.

use crate::FoldError;
use crate::ModelValidationError;
use crate::ReviewerDirectoryError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DurableStoreError {
  #[error(transparent)]
  Sqlite(#[from] rusqlite::Error),
  #[error(transparent)]
  Json(#[from] serde_json::Error),
  #[error(transparent)]
  Fold(#[from] FoldError),
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error("unsupported WOML event-store schema version {0:?}")]
  UnsupportedStoreVersion(String),
  #[error("compiled definition {0:?} is not registered")]
  DefinitionNotFound(String),
  #[error("definition hash {0:?} is already bound to a different compiled model")]
  DefinitionConflict(String),
  #[error("run {0:?} does not exist")]
  RunNotFound(String),
  #[error("run {0:?} already exists and cannot be rebound")]
  RunAlreadyExists(String),
  #[error("trigger {trigger_id:?} is not registered for workflow {workflow_id:?}")]
  TriggerNotFound {
    workflow_id: String,
    trigger_id: String,
  },
  #[error("trigger admission does not match its registered workflow definition")]
  TriggerDefinitionMismatch,
  #[error("trigger admission handler does not match the compiled trigger")]
  TriggerHandlerMismatch,
  #[error("the trigger source identity is already bound to a different payload")]
  TriggerIdempotencyConflict,
  #[error("workflow {0:?} has an active run bound to a different definition or runtime policy")]
  RuntimePolicyDefinitionConflict(String),
  #[error("the durable runtime-policy queue reached its local safety ceiling")]
  RuntimePolicyQueueFull,
  #[error("run {0:?} already has a live scheduler claim")]
  SchedulerClaimConflict(String),
  #[error("run {0:?} requires fail-closed recovery before it can be scheduled")]
  SchedulerRecoveryRequired(String),
  #[error("the internal event publication identity is already bound to different data")]
  InternalEventIdempotencyConflict,
  #[error("the internal event would repeat a subscriber already present in its lineage")]
  InternalEventCycle,
  #[error("the internal event exceeds the maximum lineage depth")]
  InternalEventDepthExceeded,
  #[error("the workflow call identity is already bound to different input")]
  WorkflowCallIdempotencyConflict,
  #[error("the workflow call exceeds the maximum lineage depth")]
  WorkflowCallDepthExceeded,
  #[error("the workflow call would create a recursive call cycle")]
  WorkflowCallCycle,
  #[error("the workflow call target does not match its registered definition")]
  WorkflowCallDefinitionMismatch,
  #[error("stored workflow call history is contradictory: {0}")]
  WorkflowCallHistoryInvalid(String),
  #[error("workflow ID {0:?} already has a live local owner")]
  WorkflowRuntimeDuplicateOwner(String),
  #[error("deployment {deployment_id:?} is already owned by runtime {runtime_instance_id:?}")]
  DeploymentRuntimeOwned {
    deployment_id: String,
    runtime_instance_id: String,
  },
  #[error("the durable schedule cursor changed before this operation could commit")]
  ScheduleCursorConflict,
  #[error("the durable interval cursor changed before this operation could commit")]
  IntervalCursorConflict,
  #[error("stored trigger occurrence history is contradictory: {0}")]
  TriggerHistoryInvalid(String),
  #[error("stored event is invalid: {0}")]
  InvalidStoredEvent(String),
  #[error("approval token is invalid")]
  InvalidApprovalToken,
  #[error("approval token has expired")]
  ExpiredApprovalToken,
  #[error("approval deadline has already been reached")]
  ApprovalExpired,
  #[error("a different human approval decision is already durable")]
  ApprovalDecisionConflict,
  #[error("the rich approval request was not found")]
  RichApprovalNotFound,
  #[error("the rich approval binding digest does not match")]
  RichApprovalBindingMismatch,
  #[error("the canonical reviewer already recorded a conflicting decision")]
  RichApprovalReviewerConflict,
  #[error(transparent)]
  ReviewerDirectory(#[from] ReviewerDirectoryError),
  #[error("the lock acquisition identity is already bound to different input")]
  LockIdempotencyConflict,
  #[error("the durable lock wait queue reached its safety ceiling")]
  LockWaitLimit,
  #[error("the durable lock fencing-token space is exhausted")]
  LockFencingExhausted,
  #[error("stored lock history is contradictory: {0}")]
  LockHistoryInvalid(String),
  #[error("approval timeout settlement requires a request with a deadline")]
  ApprovalHasNoDeadline,
  #[error("{0}")]
  Contract(String),
}
