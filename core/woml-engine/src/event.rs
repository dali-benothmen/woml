//! Stable event contract facade. Private modules separate shapes and validation.

mod admission;
mod approvals;
mod correction;
mod errors;
mod execution;
mod failures;
mod forks;
mod identities;
mod iterations;
mod lifecycle;
mod locks;
mod notifications;
mod operations;
mod parallel;
mod policy;
mod remote;
mod validation;
mod waits;

pub use admission::{
  RunAdmissionQueue, RunAdmissionTrigger, RunAdmittedData, RunCancellationRequestedData,
  RunExecutionStartedData, RunTimeoutReachedData,
};
pub use approvals::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalInvalidatedData,
  ApprovalQuorumProgressedData, ApprovalRequestedData, ApprovalResolution, ApprovalResolvedData,
  ApprovalReviewRequestedData, ApprovalReviewerDecisionRecordedData, ApprovalTimeoutPolicy,
  RichApprovalResolutionReason,
};
pub use correction::{
  CompensationRegisteredData, CorrectionAttemptFailedData, CorrectionAttemptIdentityData,
  CorrectionInterruptedData, CorrectionRetryScheduledData, CorrectiveActionKind,
  ManualRecoveryRequiredData, SagaCorrectionReason, SagaCorrectionStartedData, SagaIdentityData,
};
pub use errors::EventValidationError;
pub use execution::{
  BranchSelectedData, ChoiceSelectedData, RunIngress, RunStartedData, RunSucceededData,
  StepAttemptFailedData, StepAttemptStartedData, StepAttemptSucceededData, StepRetryScheduledData,
};
pub use failures::{
  AttemptFailure, AttemptFailureKind, BranchFailure, FailureSizeDetails, JsonValueType,
  ParallelFailure, ParallelFailurePolicy, RunFailedData, RunFailedDataV1, RunFailedDataV2,
  RunFailedDataV3, RunFailedDataV4, RunFailedDataV5,
};
pub use forks::{
  ForkBranchOutcome, ForkBranchSettledData, ForkJoinOutcome, ForkJoinSettledData, ForkOpenedData,
};
pub(crate) use identities::valid_provider_message;
pub use identities::{is_approval_request_id, is_definition_hash, is_notification_delivery_id};
use identities::{
  valid_branch_arm_id, valid_capability_name, valid_fencing_token, valid_id,
  valid_lifecycle_action_identity, valid_lifecycle_failure, valid_lock_key, valid_ordered_id_lists,
  valid_prefixed_id, valid_public_structural_id, valid_remote_public_id,
  valid_rich_approval_identity, valid_rich_approval_roles, valid_sha256, valid_signal_name,
  valid_trigger_handler, validate_for_each_iteration_event, validate_identity,
  validate_notification_identity, validate_operation_base,
};
pub use iterations::{
  ForEachIterationFailedData, ForEachIterationFailureKind, ForEachIterationIdentityData,
  ForEachIterationScope, ForEachIterationSkippedData, ForEachIterationSucceededData,
  ForEachOpenedData, ForEachSettledData, ForEachSkipReason,
};
pub use lifecycle::{
  BusinessOutcome, FinalLifecycleStatus, LifecycleActionFailedData, LifecycleActionIdentityData,
  LifecycleFailure, LifecycleFailureKind, LifecycleHookCompletedData,
  LifecycleHookCompletionStatus, LifecycleHookRequestedData, LifecycleSubject,
  LifecycleSubjectKind, LifecycleWarning, ReusableLifecycleActionFailedData,
  ReusableLifecycleActionStartedData, ReusableLifecycleActionSucceededData,
  ReusableLifecycleCompletedData, ReusableLifecycleHook, ReusableLifecycleOutcome,
  ReusableLifecycleRequestedData, RunFinalizedData, RunOutcomeDecidedData,
};
pub use locks::{
  LockAcquireTimedOutData, LockAcquiredData, LockAcquisitionCancelledData, LockLeaseLostData,
  LockReleasedData, LockWaitRegisteredData,
};
pub use notifications::{
  CommunicationProviderMessageIdentity, NotificationDecisionAcceptedData,
  NotificationDeliveryAttemptStartedData, NotificationDeliveryFailedData,
  NotificationDeliveryRequestedData, NotificationDeliverySucceededData,
  NotificationMessageUpdateAttemptStartedData, NotificationMessageUpdateFailedData,
  NotificationMessageUpdateRequestedData, NotificationMessageUpdatedData, NotificationResolution,
  NotificationRunFailure, NotificationSafeFailure, ProviderMessageIdentity,
  SlackProviderMessageIdentity,
};
pub use operations::{
  OperationExecutionMode, OperationFailedData, OperationStartedData, OperationSucceededData,
};
pub use parallel::{ParallelGroupCompletedData, ParallelGroupOutcome, ParallelGroupStartedData};
pub use policy::InfrastructurePolicyDecisionRecordedData;
pub use remote::RemoteDispatchTransitionedData;
pub use waits::{
  DurableWaitKind, DurableWaitTimeoutBehavior, DurableWaitWinner, ExternalSignalReceivedData,
  WaitCancelledData, WaitRegisteredData, WaitSatisfiedData, WaitTimedOutData,
};

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEvent {
  pub event_schema_version: u32,
  pub event_id: String,
  pub run_id: String,
  pub sequence: u64,
  pub occurred_at: DateTime<Utc>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub iteration: Option<ForEachIterationScope>,
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
  ApprovalReviewRequested(ApprovalReviewRequestedData),
  ApprovalReviewerDecisionRecorded(ApprovalReviewerDecisionRecordedData),
  ApprovalQuorumProgressed(ApprovalQuorumProgressedData),
  ApprovalResolved(ApprovalResolvedData),
  ApprovalInvalidated(ApprovalInvalidatedData),
  NotificationDeliveryRequested(NotificationDeliveryRequestedData),
  NotificationDeliveryAttemptStarted(NotificationDeliveryAttemptStartedData),
  NotificationDeliverySucceeded(NotificationDeliverySucceededData),
  NotificationDeliveryFailed(NotificationDeliveryFailedData),
  NotificationDecisionAccepted(NotificationDecisionAcceptedData),
  NotificationMessageUpdateRequested(NotificationMessageUpdateRequestedData),
  NotificationMessageUpdateAttemptStarted(NotificationMessageUpdateAttemptStartedData),
  NotificationMessageUpdated(NotificationMessageUpdatedData),
  NotificationMessageUpdateFailed(NotificationMessageUpdateFailedData),
  InfrastructurePolicyDecisionRecorded(InfrastructurePolicyDecisionRecordedData),
  RemoteDispatchTransitioned(RemoteDispatchTransitionedData),
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
  ForEachOpened(ForEachOpenedData),
  ForEachIterationStarted(ForEachIterationIdentityData),
  ForEachIterationSucceeded(ForEachIterationSucceededData),
  ForEachIterationFailed(ForEachIterationFailedData),
  ForEachIterationSkipped(ForEachIterationSkippedData),
  ForEachSucceeded(ForEachSettledData),
  ForEachFailed(ForEachSettledData),
  ForEachCancelled(ForEachSettledData),
  WaitRegistered(WaitRegisteredData),
  ExternalSignalReceived(ExternalSignalReceivedData),
  WaitSatisfied(WaitSatisfiedData),
  WaitTimedOut(WaitTimedOutData),
  WaitCancelled(WaitCancelledData),
  LockWaitRegistered(LockWaitRegisteredData),
  LockAcquired(LockAcquiredData),
  LockReleased(LockReleasedData),
  LockAcquireTimedOut(LockAcquireTimedOutData),
  LockAcquisitionCancelled(LockAcquisitionCancelledData),
  LockLeaseLost(LockLeaseLostData),
  SagaStarted(SagaIdentityData),
  CompensationRegistered(CompensationRegisteredData),
  SagaForwardSucceeded(SagaIdentityData),
  SagaCorrectionStarted(SagaCorrectionStartedData),
  CorrectionAttemptStarted(CorrectionAttemptIdentityData),
  CorrectionAttemptSucceeded(CorrectionAttemptIdentityData),
  CorrectionAttemptFailed(CorrectionAttemptFailedData),
  CorrectionRetryScheduled(CorrectionRetryScheduledData),
  CorrectionInterrupted(CorrectionInterruptedData),
  SagaCorrectionCompleted(SagaIdentityData),
  ManualRecoveryRequired(ManualRecoveryRequiredData),
  RunOutcomeDecided(RunOutcomeDecidedData),
  RunFinalized(RunFinalizedData),
  RunSucceeded(RunSucceededData),
  RunFailed(RunFailedData),
}
