//! Typed durable API records, grouped by the authority or read model they describe.
//!
//! A record is not necessarily one SQL row or a rebuildable projection. These
//! types retain their original public paths, Serde attributes, and field shapes.

mod admission;
mod approvals;
mod correction;
mod infrastructure_policy;
mod inspection;
mod locks;
mod notifications;
mod ownership;
mod policy;
mod recovery;
mod run_control;
mod waits;

pub use admission::{
  InternalEventAdmissionOutcome, InternalEventAdmissionRequest, IntervalCursor,
  IntervalCursorRegistration, IntervalCursorRegistrationOutcome, RunDefinitionBinding,
  ScheduleCursor, ScheduleCursorRegistration, ScheduleCursorRegistrationOutcome,
  TriggerAdmissionOutcome, TriggerAdmissionRequest, TriggerOccurrence, TriggerRecoveryWork,
};
pub use approvals::{
  ApprovalDecisionOutcome, ApprovalDecisionOutcomeStatus, ApprovalTimeoutSettlement,
  ApprovalTimeoutSettlementStatus, ApprovalTokenBinding, IssuedApprovalToken,
  RichApprovalDecisionAdmission, RichApprovalDecisionInspection, RichApprovalDecisionOutcome,
  RichApprovalDecisionStatus, RichApprovalIdentityDecision, RichApprovalInspectionV1,
  RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
};
pub(in crate::durable) use approvals::{RichQuorumState, RichReviewRow};
pub use correction::{
  CorrectionInspectionV1, CorrectionObservationV1, CorrectionRecoveryActionV1,
  CorrectionRecoveryCandidateV1, CorrectionRecoveryRequestV1, CorrectionRecoveryResultV1,
  CorrectionRecoveryStatusV1, CorrectionSagaInspectionV1, UnresolvedCorrectionActionV1,
  CORRECTION_INSPECTION_PROFILE, CORRECTION_OBSERVATION_PROFILE, CORRECTION_RECOVERY_BATCH_LIMIT,
  CORRECTION_RECOVERY_REQUEST_PROFILE, CORRECTION_RECOVERY_RESULT_PROFILE,
  CORRECTION_UNRESOLVED_STORE_LIMIT,
};
pub use infrastructure_policy::InfrastructurePolicyRunBindingV1;
pub(crate) use infrastructure_policy::{
  InfrastructurePolicyApprovalEvidenceV1, InfrastructurePolicyArtifactEvidenceV1,
  InfrastructurePolicyEvidenceSnapshotV1, InfrastructurePolicyLockEvidenceV1,
};
pub use inspection::{
  InspectedForEachStatusV6, InspectedForkJoinModeV4, InspectedForkStatusV4,
  InspectedReusableKindV5, InspectedReusableLifecycleStatusV5, InspectedReusableStatusV5,
  PolicyWaitingFor, PublicRunStatus, RunInspectionCancellationV2, RunInspectionForEachCountsV6,
  RunInspectionForEachItemV6, RunInspectionForEachV6, RunInspectionForkCountsV4,
  RunInspectionForkJoinV4, RunInspectionForkV4, RunInspectionForksV4, RunInspectionHookV2,
  RunInspectionPolicyV3, RunInspectionReusableCountsV5, RunInspectionReusableDefinitionsV5,
  RunInspectionReusableItemV5, RunInspectionRichApprovalV11, RunInspectionRichApprovalsV11,
  RunInspectionV11, RunInspectionV2, RunInspectionV3, RunInspectionV4, RunInspectionV5,
  RunInspectionV6, RunListV1, RunListV2, RunSummaryV1, RunSummaryV2, RuntimeObservationV1,
  RUN_INSPECTION_RICH_APPROVAL_LIMIT,
};
pub use locks::{
  InspectedLockStatusV1, LockInspectionV1, LockLeaseMaintenance, LockObservationV1,
  LockRecoveryCandidateV1, LockRecoveryKindV1, ProgrammaticLockAcquire,
  ProgrammaticLockAcquireState, ProgrammaticLockLease, LOCK_RECOVERY_BATCH_LIMIT,
  LOCK_WAITERS_PER_KEY_LIMIT, LOCK_WAITERS_PER_STORE_LIMIT,
};
pub use notifications::{
  NotificationDeliveryWork, NotificationDispatchReport, NotificationProviderAdapter,
  NotificationProviderDeliveryResult, NotificationProviderUpdateResult, NotificationUpdateWork,
};
pub use ownership::RuntimeOwnerLease;
pub use policy::{PolicyClaimWaitReason, PolicyExecutionClaimResult, SchedulerClaimV1};
pub use recovery::{ForkRecoveryBranchV1, ForkRecoveryWorkV1, RecoveryReport};
pub use run_control::{
  InspectedBusinessOutcome, RunCancellationCode, RunCancellationResult, RunCancellationStatus,
  RunTimeoutSettlement,
};
pub use waits::{
  DurableWaitRecordV1, ExternalSignalAdmissionOutcome, ExternalSignalAdmissionRequest,
  ExternalSignalAdmissionStatus, ExternalSignalResolution, InspectedWaitStatusV1, InspectedWaitV1,
  SignalWaitTimeoutSettlement, TimerWaitSettlement, WaitInspectionV1,
  EXTERNAL_SIGNAL_ADMISSION_PROFILE, EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN,
  EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES, EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH,
  EXTERNAL_SIGNAL_RESULT_PROFILE,
};
