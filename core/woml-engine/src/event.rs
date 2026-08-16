use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
  capability::{validate_safe_metadata, CapabilityFailure},
  RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V10, RUN_EVENT_SCHEMA_VERSION_V11,
  RUN_EVENT_SCHEMA_VERSION_V12, RUN_EVENT_SCHEMA_VERSION_V13, RUN_EVENT_SCHEMA_VERSION_V14,
  RUN_EVENT_SCHEMA_VERSION_V2, RUN_EVENT_SCHEMA_VERSION_V3, RUN_EVENT_SCHEMA_VERSION_V4,
  RUN_EVENT_SCHEMA_VERSION_V5, RUN_EVENT_SCHEMA_VERSION_V6, RUN_EVENT_SCHEMA_VERSION_V7,
  RUN_EVENT_SCHEMA_VERSION_V8, RUN_EVENT_SCHEMA_VERSION_V9,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEvent {
  pub event_schema_version: u32,
  pub event_id: String,
  pub run_id: String,
  pub sequence: u64,
  pub occurred_at: DateTime<Utc>,
  #[serde(flatten)]
  pub payload: RunEventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RunEventPayload {
  RunAdmitted(RunAdmittedData),
  RunExecutionStarted(RunExecutionStartedData),
  RunTimeoutReached(RunTimeoutReachedData),
  RunStarted(RunStartedData),
  StepAttemptStarted(StepAttemptStartedData),
  StepAttemptSucceeded(StepAttemptSucceededData),
  StepAttemptFailed(StepAttemptFailedData),
  StepRetryScheduled(StepRetryScheduledData),
  BranchSelected(BranchSelectedData),
  ChoiceSelected(ChoiceSelectedData),
  ParallelGroupStarted(ParallelGroupStartedData),
  ParallelGroupCompleted(ParallelGroupCompletedData),
  ApprovalRequested(ApprovalRequestedData),
  ApprovalResolved(ApprovalResolvedData),
  NotificationDeliveryRequested(NotificationDeliveryRequestedData),
  NotificationDeliveryAttemptStarted(NotificationDeliveryAttemptStartedData),
  NotificationDeliverySucceeded(NotificationDeliverySucceededData),
  NotificationDeliveryFailed(NotificationDeliveryFailedData),
  NotificationDecisionAccepted(NotificationDecisionAcceptedData),
  NotificationMessageUpdateRequested(NotificationMessageUpdateRequestedData),
  NotificationMessageUpdateAttemptStarted(NotificationMessageUpdateAttemptStartedData),
  NotificationMessageUpdated(NotificationMessageUpdatedData),
  NotificationMessageUpdateFailed(NotificationMessageUpdateFailedData),
  OperationStarted(OperationStartedData),
  OperationSucceeded(OperationSucceededData),
  OperationFailed(OperationFailedData),
  ForkOpened(ForkOpenedData),
  ForkBranchSettled(ForkBranchSettledData),
  ForkJoinSettled(ForkJoinSettledData),
  RunCancellationRequested(RunCancellationRequestedData),
  LifecycleHookRequested(LifecycleHookRequestedData),
  LifecycleActionAttemptStarted(LifecycleActionIdentityData),
  LifecycleActionSucceeded(LifecycleActionIdentityData),
  LifecycleActionFailed(LifecycleActionFailedData),
  LifecycleHookCompleted(LifecycleHookCompletedData),
  ReusableLifecycleRequested(ReusableLifecycleRequestedData),
  ReusableLifecycleActionStarted(ReusableLifecycleActionStartedData),
  ReusableLifecycleActionSucceeded(ReusableLifecycleActionSucceededData),
  ReusableLifecycleActionFailed(ReusableLifecycleActionFailedData),
  ReusableLifecycleCompleted(ReusableLifecycleCompletedData),
  RunOutcomeDecided(RunOutcomeDecidedData),
  RunFinalized(RunFinalizedData),
  RunSucceeded(RunSucceededData),
  RunFailed(RunFailedData),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAdmissionTrigger {
  pub id: String,
  pub handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAdmissionQueue {
  pub name: String,
  pub discipline: crate::model::QueueDiscipline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAdmittedData {
  pub definition_hash: String,
  pub policy_hash: String,
  pub trigger: RunAdmissionTrigger,
  pub payload: Map<String, Value>,
  pub queue: RunAdmissionQueue,
  pub admitted_at: DateTime<Utc>,
  pub occurrence_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunExecutionStartedData {
  pub started_at: DateTime<Utc>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunTimeoutReachedData {
  pub deadline_at: DateTime<Utc>,
  pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunCancellationRequestedData {
  pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSubjectKind {
  Workflow,
  Step,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleSubject {
  pub kind: LifecycleSubjectKind,
  pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleHookRequestedData {
  pub hook_invocation_id: String,
  pub hook_id: String,
  pub event: crate::model::LifecycleEventName,
  pub subject: LifecycleSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleActionIdentityData {
  pub hook_invocation_id: String,
  pub action_id: String,
  pub attempt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleFailureKind {
  ScriptThrew,
  TimedOut,
  NonJson,
  WorkerCrashed,
  HostCrashed,
  Interrupted,
  SizeLimitExceeded,
  Cancelled,
  ProviderFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFailure {
  pub kind: LifecycleFailureKind,
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleActionFailedData {
  pub hook_invocation_id: String,
  pub action_id: String,
  pub attempt: u32,
  pub failure: LifecycleFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookCompletionStatus {
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleHookCompletedData {
  pub hook_invocation_id: String,
  pub status: LifecycleHookCompletionStatus,
  pub failed_actions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReusableLifecycleHook {
  OnSuccess,
  OnError,
  OnComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReusableLifecycleOutcome {
  Succeeded,
  Failed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleRequestedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleActionStartedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub action_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleActionSucceededData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub action_id: String,
  pub outcome: ReusableLifecycleOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleActionFailedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub action_id: String,
  pub outcome: ReusableLifecycleOutcome,
  pub warning_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleCompletedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub outcome: ReusableLifecycleOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessOutcome {
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunOutcomeDecidedData {
  Succeeded {
    result: Value,
  },
  Failed {
    failure: LifecycleFailure,
  },
  Cancelled {
    #[serde(rename = "cancellationRequestId")]
    cancellation_request_id: String,
  },
}

impl RunOutcomeDecidedData {
  pub const fn outcome(&self) -> BusinessOutcome {
    match self {
      Self::Succeeded { .. } => BusinessOutcome::Succeeded,
      Self::Failed { .. } => BusinessOutcome::Failed,
      Self::Cancelled { .. } => BusinessOutcome::Cancelled,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalLifecycleStatus {
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleWarning {
  pub hook_id: String,
  pub action_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub step_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub provider: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub destination: Option<String>,
  pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunFinalizedData {
  pub outcome: BusinessOutcome,
  pub lifecycle_status: FinalLifecycleStatus,
  pub warnings: Vec<LifecycleWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationExecutionMode {
  Observed,
  Managed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationStartedData {
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationSucceededData {
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
  pub duration_ms: f64,
  pub result_bytes: u64,
  pub result_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFailedData {
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub capability: String,
  pub operation: String,
  pub execution_mode: OperationExecutionMode,
  pub metadata: Map<String, Value>,
  pub duration_ms: f64,
  pub failure: CapabilityFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunStartedData {
  pub workflow_id: String,
  pub definition_hash: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub trigger_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub trigger_handler: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub trigger_occurrence_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ingress: Option<RunIngress>,
  pub trigger: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunIngress {
  WorkflowCall { call_key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptStartedData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub handler: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptSucceededData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepAttemptFailedData {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub failure: AttemptFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepRetryScheduledData {
  pub node_id: String,
  pub failed_attempt: u32,
  pub next_attempt: u32,
  pub scheduled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchSelectedData {
  pub branch_id: String,
  pub arm_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChoiceSelectedData {
  pub choice_id: String,
  pub arm_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkOpenedData {
  pub fork_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkBranchOutcome {
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkBranchSettledData {
  pub fork_id: String,
  pub branch_id: String,
  pub terminal_node_id: String,
  pub outcome: ForkBranchOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkJoinOutcome {
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkJoinSettledData {
  pub fork_id: String,
  pub outcome: ForkJoinOutcome,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blocking_branch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParallelGroupStartedData {
  pub parallel_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelGroupOutcome {
  Succeeded,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParallelGroupCompletedData {
  pub parallel_id: String,
  pub outcome: ParallelGroupOutcome,
  pub failed_node_ids: Vec<String>,
  pub cancelled_node_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutPolicy {
  Reject,
  Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalRequestedData {
  pub approval_id: String,
  pub request_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<DateTime<Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
  Approved,
  Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionSource {
  Human,
  Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApprovalResolution {
  Decision {
    decision: ApprovalDecision,
    source: ApprovalDecisionSource,
  },
  TimeoutFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalResolvedData {
  pub approval_id: String,
  pub request_id: String,
  pub resolution: ApprovalResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliveryRequestedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliveryAttemptStartedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProviderMessageIdentity {
  Slack(SlackProviderMessageIdentity),
  Communication(CommunicationProviderMessageIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlackProviderMessageIdentity {
  pub workspace_id: String,
  pub channel_id: String,
  pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunicationProviderMessageIdentity {
  pub provider: String,
  pub account_id: String,
  pub conversation_id: String,
  pub message_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliverySucceededData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  pub provider_message: ProviderMessageIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationSafeFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
  pub retryable: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDeliveryFailedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  #[serde(rename = "final")]
  pub final_: bool,
  pub failure: NotificationSafeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDecisionAcceptedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub provider_actor_id: String,
  pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationResolution {
  Approved,
  Rejected,
  TimeoutFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationMessageUpdateRequestedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub resolution: NotificationResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationMessageUpdateAttemptStartedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub attempt: u32,
  pub attempt_id: String,
}

pub type NotificationMessageUpdatedData = NotificationMessageUpdateAttemptStartedData;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationMessageUpdateFailedData {
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub update_id: String,
  pub attempt: u32,
  pub attempt_id: String,
  #[serde(rename = "final")]
  pub final_: bool,
  pub failure: NotificationSafeFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSucceededData {
  pub terminal_node_id: String,
  pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunFailedDataV1 {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub node_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub invocation_id: Option<String>,
  pub failure: AttemptFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV2 {
  Attempt {
    #[serde(rename = "nodeId")]
    node_id: String,
    attempt: u32,
    #[serde(rename = "invocationId")]
    invocation_id: String,
    failure: AttemptFailure,
  },
  Branch {
    #[serde(rename = "branchId")]
    branch_id: String,
    #[serde(rename = "armId", default, skip_serializing_if = "Option::is_none")]
    arm_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<Vec<String>>,
    failure: BranchFailure,
  },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParallelFailurePolicy {
  #[serde(rename = "fail-fast")]
  FailFast,
  #[serde(rename = "wait-all")]
  WaitAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
}

impl ParallelFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if self.kind != "parallel_child_failed" || self.code != "WOML_PARALLEL_CHILD_FAILED" {
      return Err(EventValidationError::Invalid(
        "Parallel failure requires kind parallel_child_failed and code WOML_PARALLEL_CHILD_FAILED."
          .to_string(),
      ));
    }
    if self.message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Parallel failure message must not be empty.".to_string(),
      ));
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV3 {
  Parallel {
    #[serde(rename = "parallelId")]
    parallel_id: String,
    policy: ParallelFailurePolicy,
    #[serde(rename = "primaryNodeId")]
    primary_node_id: String,
    #[serde(rename = "failedNodeIds")]
    failed_node_ids: Vec<String>,
    #[serde(rename = "cancelledNodeIds")]
    cancelled_node_ids: Vec<String>,
    failure: ParallelFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
}

impl ApprovalFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if self.kind != "approval_timeout" || self.code != "WOML_APPROVAL_TIMEOUT" {
      return Err(EventValidationError::Invalid(
        "Approval failure requires kind approval_timeout and code WOML_APPROVAL_TIMEOUT."
          .to_string(),
      ));
    }
    if self.message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Approval failure message must not be empty.".to_string(),
      ));
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV4 {
  Approval {
    #[serde(rename = "approvalId")]
    approval_id: String,
    #[serde(rename = "requestId")]
    request_id: String,
    failure: ApprovalFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationRunFailure {
  pub kind: String,
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failureScope", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunFailedDataV5 {
  Notification {
    #[serde(rename = "approvalId")]
    approval_id: String,
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "failedDeliveryIds")]
    failed_delivery_ids: Vec<String>,
    failure: NotificationRunFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunFailedData {
  V5(RunFailedDataV5),
  V4(RunFailedDataV4),
  V3(RunFailedDataV3),
  V2(RunFailedDataV2),
  V1(RunFailedDataV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptFailureKind {
  ScriptThrew,
  ScriptTimedOut,
  InvalidScriptResult,
  ContextTooLarge,
  ResultTooLarge,
  WorkerCrashed,
  HostCrashed,
  InvocationCancelled,
  Interrupted,
  ServiceFailed,
}

impl AttemptFailureKind {
  pub const fn code(self) -> &'static str {
    match self {
      Self::ScriptThrew => "WOML_SCRIPT_THROWN",
      Self::ScriptTimedOut => "WOML_SCRIPT_TIMEOUT",
      Self::InvalidScriptResult => "WOML_SCRIPT_NON_JSON_RESULT",
      Self::ContextTooLarge => "WOML_SCRIPT_CONTEXT_TOO_LARGE",
      Self::ResultTooLarge => "WOML_SCRIPT_RESULT_TOO_LARGE",
      Self::WorkerCrashed => "WOML_SCRIPT_WORKER_CRASHED",
      Self::HostCrashed => "WOML_SCRIPT_HOST_CRASHED",
      Self::InvocationCancelled => "WOML_SCRIPT_CANCELLED",
      Self::Interrupted => "WOML_STEP_INTERRUPTED",
      Self::ServiceFailed => "WOML_SERVICE_FAILED",
    }
  }

  pub const fn accepts_size_details(self) -> bool {
    matches!(self, Self::ContextTooLarge | Self::ResultTooLarge)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureSizeDetails {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub actual_bytes: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptFailure {
  pub kind: AttemptFailureKind,
  pub code: String,
  pub message: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<FailureSizeDetails>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub capability: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub operation: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub call_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retryable: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ambiguous: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cause: Option<CapabilityFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonValueType {
  Null,
  Number,
  String,
  Array,
  Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BranchFailure {
  BranchTestNotBoolean {
    code: String,
    message: String,
    #[serde(rename = "actualType")]
    actual_type: JsonValueType,
  },
  ReferenceNotAvailable {
    code: String,
    message: String,
  },
  BranchSelectionInvalid {
    code: String,
    message: String,
  },
}

impl BranchFailure {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::BranchTestNotBoolean { .. } => "WOML_BRANCH_TEST_NOT_BOOLEAN",
      Self::ReferenceNotAvailable { .. } => "WOML_REFERENCE_NOT_AVAILABLE",
      Self::BranchSelectionInvalid { .. } => "WOML_BRANCH_SELECTION_INVALID",
    }
  }

  pub fn validate(&self) -> Result<(), EventValidationError> {
    let (code, message) = match self {
      Self::BranchTestNotBoolean { code, message, .. }
      | Self::ReferenceNotAvailable { code, message }
      | Self::BranchSelectionInvalid { code, message } => (code, message),
    };
    if code != self.code() {
      return Err(EventValidationError::Invalid(format!(
        "Branch failure requires code {:?}, received {:?}.",
        self.code(),
        code
      )));
    }
    if message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Branch failure message must not be empty.".to_string(),
      ));
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventValidationError {
  #[error("unsupported run event schema version {0}")]
  UnsupportedSchemaVersion(u32),
  #[error("{0}")]
  Invalid(String),
}

fn valid_id(value: &str) -> bool {
  !value.is_empty() && value.chars().count() <= 256
}

fn valid_public_structural_id(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
    && characters.all(|character| character.is_ascii_alphanumeric())
    && value.chars().count() <= 256
}

fn valid_prefixed_id(value: &str, prefix: &str, minimum: usize) -> bool {
  value.len() >= minimum
    && value.len() <= 256
    && value.strip_prefix(prefix).is_some_and(|suffix| {
      !suffix.is_empty()
        && suffix
          .bytes()
          .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

pub fn is_notification_delivery_id(value: &str, approval_id: &str) -> bool {
  let Some(suffix) = value.strip_prefix(&format!("{approval_id}:notify:")) else {
    return false;
  };
  let Some((tag, destination)) = suffix
    .split_once(":channel:")
    .or_else(|| suffix.split_once(":chat:"))
  else {
    return false;
  };
  [tag, destination].iter().all(|part| {
    !part.is_empty()
      && part.bytes().all(|byte| byte.is_ascii_digit())
      && (*part == "0" || !part.starts_with('0'))
  })
}

fn valid_sha256(value: &str) -> bool {
  value.strip_prefix("sha256:").is_some_and(|digest| {
    digest.len() == 64
      && digest
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
  })
}

fn valid_lifecycle_failure(failure: &LifecycleFailure) -> bool {
  !failure.message.is_empty()
    && failure.message.len() <= 1024
    && failure.code.len() <= 128
    && failure.code.starts_with("WOML_")
    && failure
      .code
      .bytes()
      .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn valid_lifecycle_action_identity(
  hook_invocation_id: &str,
  action_id: &str,
  attempt: u32,
) -> bool {
  valid_sha256(hook_invocation_id) && valid_id(action_id) && attempt == 1
}

fn valid_capability_name(value: &str) -> bool {
  value.len() <= 128
    && value.split(['.', '_', '-']).all(|segment| {
      let mut characters = segment.chars();
      matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

fn validate_operation_base(
  event_schema_version: u32,
  node_id: &str,
  attempt_number: u32,
  invocation_id: &str,
  call_id: &str,
  operation_key: &str,
  capability: &str,
  operation: &str,
  metadata: &Map<String, Value>,
) -> Result<(), EventValidationError> {
  if !matches!(
    event_schema_version,
    RUN_EVENT_SCHEMA_VERSION_V8
      | RUN_EVENT_SCHEMA_VERSION_V9
      | RUN_EVENT_SCHEMA_VERSION_V10
      | RUN_EVENT_SCHEMA_VERSION_V11
  ) {
    return Err(EventValidationError::Invalid(
      "Operation events are available only in run-event schema v8 or later.".to_string(),
    ));
  }
  validate_identity(node_id, attempt_number, invocation_id)?;
  if attempt_number > 10
    || !valid_id(call_id)
    || !valid_sha256(operation_key)
    || !valid_capability_name(capability)
    || !valid_capability_name(operation)
  {
    return Err(EventValidationError::Invalid(
      "Operation event contains an invalid correlation or logical operation identity.".to_string(),
    ));
  }
  validate_safe_metadata(metadata).map_err(EventValidationError::Invalid)
}

fn valid_trigger_handler(value: &str) -> bool {
  matches!(
    value,
    "trigger.manual"
      | "trigger.webhook"
      | "trigger.slack"
      | "trigger.telegram"
      | "trigger.discord"
      | "trigger.schedule"
      | "trigger.interval"
      | "trigger.event"
  )
}

pub(crate) fn valid_provider_message(message: &ProviderMessageIdentity) -> bool {
  match message {
    ProviderMessageIdentity::Slack(message) => {
      valid_prefixed_id(&message.workspace_id, "T", 9)
        && matches!(
          message.channel_id.as_bytes().first(),
          Some(b'C' | b'G' | b'D')
        )
        && message.channel_id.len() >= 9
        && message.channel_id.len() <= 32
        && message.channel_id[1..]
          .bytes()
          .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && message
          .message_id
          .split_once('.')
          .is_some_and(|(seconds, micros)| {
            seconds.len() >= 10
              && seconds.bytes().all(|byte| byte.is_ascii_digit())
              && micros.len() == 6
              && micros.bytes().all(|byte| byte.is_ascii_digit())
          })
    }
    ProviderMessageIdentity::Communication(message) => {
      matches!(
        message.provider.as_str(),
        "telegram" | "discord" | "whatsapp"
      ) && valid_id(&message.account_id)
        && valid_id(&message.conversation_id)
        && valid_id(&message.message_id)
        && message.thread_id.as_deref().is_none_or(valid_id)
    }
  }
}

impl NotificationSafeFailure {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    const KINDS: &[&str] = &[
      "secret_not_found",
      "provider_auth_failed",
      "destination_invalid",
      "rate_limited",
      "provider_unavailable",
      "delivery_ambiguous",
      "host_crashed",
      "size_limit_exceeded",
      "request_invalid",
      "update_failed",
    ];
    if !KINDS.contains(&self.kind.as_str())
      || !self.code.starts_with("WOML_")
      || !self
        .code
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
      || self.message.is_empty()
      || self.message.len() > 1024
      || self.retry_after_ms.is_some_and(|value| value > 86_400_000)
    {
      return Err(EventValidationError::Invalid(
        "Notification failure contains an invalid safe failure contract.".to_string(),
      ));
    }
    Ok(())
  }
}

pub fn is_approval_request_id(value: &str) -> bool {
  let Some(suffix) = value.strip_prefix("aprreq_") else {
    return false;
  };
  value.len() >= 10
    && value.len() <= 256
    && !suffix.is_empty()
    && suffix
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

impl ApprovalResolution {
  pub fn validate(&self) -> Result<(), EventValidationError> {
    if matches!(
      self,
      Self::Decision {
        decision: ApprovalDecision::Approved,
        source: ApprovalDecisionSource::Timeout,
      }
    ) {
      return Err(EventValidationError::Invalid(
        "A timeout approval resolution may reject only.".to_string(),
      ));
    }
    Ok(())
  }
}

fn valid_branch_arm_id(branch_id: &str, arm_id: &str) -> bool {
  let Some(suffix) = arm_id.strip_prefix(&format!("{branch_id}:")) else {
    return false;
  };
  if suffix == "otherwise" {
    return true;
  }
  let Some(index) = suffix.strip_prefix("when:") else {
    return false;
  };
  !index.is_empty()
    && index.bytes().all(|byte| byte.is_ascii_digit())
    && (index == "0" || !index.starts_with('0'))
    && arm_id.chars().count() <= 256
}

fn valid_ordered_id_lists(first: &[String], second: &[String]) -> bool {
  let mut seen = std::collections::HashSet::new();
  first
    .iter()
    .chain(second)
    .all(|node_id| valid_public_structural_id(node_id) && seen.insert(node_id))
}

fn validate_identity(
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Result<(), EventValidationError> {
  if !valid_id(node_id) || !valid_id(invocation_id) || attempt == 0 {
    return Err(EventValidationError::Invalid(
      "Attempt identity requires valid nodeId, invocationId, and attempt >= 1.".to_string(),
    ));
  }
  Ok(())
}

fn validate_notification_identity(
  approval_id: &str,
  request_id: &str,
  delivery_id: &str,
) -> Result<(), EventValidationError> {
  if !valid_public_structural_id(approval_id)
    || !is_approval_request_id(request_id)
    || !is_notification_delivery_id(delivery_id, approval_id)
  {
    return Err(EventValidationError::Invalid(
      "Notification event contains invalid approval, request, or delivery identity.".to_string(),
    ));
  }
  Ok(())
}

impl AttemptFailure {
  pub fn legacy_defaults() -> Self {
    Self {
      kind: AttemptFailureKind::Interrupted,
      code: AttemptFailureKind::Interrupted.code().to_string(),
      message: "legacy failure defaults".to_string(),
      details: None,
      capability: None,
      operation: None,
      call_id: None,
      retryable: None,
      ambiguous: None,
      cause: None,
    }
  }

  pub fn validate(&self) -> Result<(), EventValidationError> {
    let service_fields = (
      self.capability.as_deref(),
      self.operation.as_deref(),
      self.call_id.as_deref(),
      self.retryable,
      self.ambiguous,
      self.cause.as_ref(),
    );
    if self.kind == AttemptFailureKind::ServiceFailed {
      let valid = matches!(service_fields,
        (Some(capability), Some(operation), Some(call_id), Some(retryable), Some(ambiguous), Some(cause))
          if valid_capability_name(capability)
            && valid_capability_name(operation)
            && valid_id(call_id)
            && retryable == cause.retryable
            && ambiguous == cause.ambiguous
            && self.code == cause.code
            && cause.validate().is_ok()
      );
      if !valid || self.details.is_some() {
        return Err(EventValidationError::Invalid(
          "service_failed requires capability, operation, callId, retryable, ambiguous, and a matching safe cause.".to_string(),
        ));
      }
    } else if self.code != self.kind.code() {
      return Err(EventValidationError::Invalid(format!(
        "Failure kind {:?} requires code {:?}, received {:?}.",
        self.kind,
        self.kind.code(),
        self.code
      )));
    }
    if self.message.is_empty() {
      return Err(EventValidationError::Invalid(
        "Failure message must not be empty.".to_string(),
      ));
    }
    if self.kind != AttemptFailureKind::ServiceFailed
      && [
        self.capability.is_some(),
        self.operation.is_some(),
        self.call_id.is_some(),
        self.retryable.is_some(),
        self.ambiguous.is_some(),
        self.cause.is_some(),
      ]
      .into_iter()
      .any(|present| present)
    {
      return Err(EventValidationError::Invalid(
        "Only service_failed may contain service failure fields.".to_string(),
      ));
    }
    match (&self.details, self.kind.accepts_size_details()) {
      (Some(_), false) => Err(EventValidationError::Invalid(
        "Only size-limit failures may contain details.".to_string(),
      )),
      (Some(details), true) if details.actual_bytes.is_none() && details.limit_bytes.is_none() => {
        Err(EventValidationError::Invalid(
          "Failure size details must contain actualBytes or limitBytes.".to_string(),
        ))
      }
      _ => Ok(()),
    }
  }
}

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
        data.resolution.validate()?;
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
          "slack" | "telegram" | "discord" | "custom"
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

pub fn is_definition_hash(value: &str) -> bool {
  let Some(hex) = value.strip_prefix("sha256:") else {
    return false;
  };
  hex.len() == 64
    && hex
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
