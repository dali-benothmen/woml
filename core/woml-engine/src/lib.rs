//! The language-neutral WOML execution core.
//!
//! This crate is intentionally isolated from retired authoring runtimes and
//! from N-API. It consumes the versioned Compiled Workflow Model and
//! derives all run state by folding versioned events.

pub mod artifacts;
pub mod backup;
pub mod cache;
pub mod capability;
pub mod custom_notification_host;
pub mod custom_notification_provider_protocol;
pub mod database;
mod database_postgres;
pub mod discord;
pub mod durable;
pub mod durable_state;
pub mod engine;
pub mod event;
pub mod events_service;
pub mod host;
pub mod http;
pub mod infrastructure_policy;
pub mod infrastructure_policy_runtime;
pub mod interval;
pub mod locks;
pub mod model;
pub mod notification_host;
pub mod notification_protocol;
pub mod notification_runtime;
pub mod operational_output;
#[doc(hidden)]
pub mod performance;
pub mod presentation;
pub mod process;
pub mod projection;
pub mod protocol;
pub mod remote_coordinator;
pub mod remote_coordinator_server;
pub mod remote_dispatch;
pub mod remote_execution;
pub mod remote_protocol;
pub mod remote_transport;
pub mod remote_worker;
pub mod remote_worker_journal;
pub mod retention;
pub mod rich_approval;
pub mod runtime;
pub mod schedule;
pub mod storage;
pub mod store;
pub mod target_registry;
pub mod telegram;
pub mod telemetry;
pub mod telemetry_propagation;
pub mod webhook;
pub mod whatsapp;
pub mod workflow_calls;

pub use remote_coordinator::{
  remote_transport_profile, RemoteCoordinatorError, RemoteSessionAuthorityV1,
  RemoteSessionRecordV1, RemoteWorkerHealthStatusV1, RemoteWorkerHealthV1, TrustedWorkerIdentityV1,
  REMOTE_MAX_REPLAY_IDS, REMOTE_MAX_SESSIONS, REMOTE_MESSAGE_CLOCK_SKEW_SECONDS,
  REMOTE_SESSION_LIFETIME_SECONDS,
};
pub use remote_coordinator_server::{
  RemoteCoordinatorServerError, RemoteCoordinatorServerV1, RemoteCoordinatorTlsOptionsV1,
  REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS, REMOTE_COORDINATOR_MAX_CONNECTIONS,
  REMOTE_COORDINATOR_MAX_HEADER_BYTES,
};
pub use remote_dispatch::{
  derive_remote_dispatch_id, RemoteDispatchError, RemoteDispatchInspectionV1,
  RemoteDispatchRecordV1, RemoteDispatchRequestV1, RemoteDispatchResolutionV1,
  RemoteDispatchStateV1, RemoteOutputBindingV1, REMOTE_DISPATCH_HEARTBEAT_MAX_BYTES,
  REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT,
};
pub use remote_execution::{
  ingest_remote_output_batch, RemoteExecutionError, RemoteOutputIngestReportV1,
};
pub use remote_protocol::{
  decode_remote_dispatch_package, decode_worker_dispatch_fact, decode_worker_hello,
  decode_worker_output_batch, decode_worker_poll, decode_worker_shutdown, decode_worker_terminal,
  RemoteArtifactGrantV1, RemoteArtifactOperationV1, RemoteCoordinatorAuthorityV1,
  RemoteDispatchPackageV1, RemoteEffectiveAuthorityV1, RemoteOperationalRecordV1,
  RemoteProtocolError, WorkerArchitectureV1, WorkerCapabilityV1, WorkerDispatchFactV1,
  WorkerHelloV1, WorkerOperatingSystemV1, WorkerOutputBatchV1, WorkerPlatformV1,
  WorkerPollResultV1, WorkerPollV1, WorkerSessionV1, WorkerShutdownV1, WorkerTerminalFailureV1,
  WorkerTerminalOutcomeV1, WorkerTerminalV1, REMOTE_DISPATCH_PACKAGE_PROFILE,
  REMOTE_DISPATCH_PACKAGE_VERSION, REMOTE_OUTPUT_BATCH_MAX_RECORDS,
  REMOTE_TERMINAL_RESULT_MAX_BYTES, WORKER_PROTOCOL_MAX_CAPABILITIES, WORKER_PROTOCOL_MAX_CAPACITY,
  WORKER_PROTOCOL_MAX_MESSAGE_BYTES, WORKER_PROTOCOL_MAX_TARGETS, WORKER_PROTOCOL_PROFILE,
  WORKER_PROTOCOL_VERSION,
};
pub use remote_transport::{
  RemoteDispatchOfferV1, RemoteDispatchTerminalReceiptV1, RemoteDispatchTransportV1,
  RemoteTransportError, REMOTE_OFFER_LIFETIME_SECONDS, REMOTE_TRANSPORT_MAX_PENDING,
};
pub use remote_worker::{
  parse_worker_configuration, prepare_worker_configuration, PreparedWorkerConfigurationV1,
  RemoteWorkerConfigurationError, WorkerConfigurationV1, WorkerSecretProviderV1,
  WorkerTlsReferencesV1, WORKER_CONFIGURATION_MAX_BYTES, WORKER_CONFIGURATION_PROFILE,
  WORKER_CONFIGURATION_VERSION,
};
pub use remote_worker_journal::{
  prepare_remote_dispatch, verify_remote_artifact_transfer, EnvironmentRemoteSecretProvider,
  PreparedRemoteDispatchV1, RemoteSecretProvider, RemoteWorkerAttemptRuntimeV1,
  RemoteWorkerAuthorityV1, RemoteWorkerCancellationTokenV1, RemoteWorkerError,
  RemoteWorkerTerminalV1, ResolvedRemoteSecretProvider, WorkerAttemptRecordV1,
  WorkerAttemptStateV1, WorkerJournalV1, REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS,
  WORKER_JOURNAL_RECOVERY_LIMIT, WORKER_JOURNAL_SCHEMA_VERSION,
};
pub use target_registry::{
  parse_target_registry, prepare_target_registry, PreparedTargetRegistryV1, TargetRegistryEntryV1,
  TargetRegistryError, TargetRegistryV1, TARGET_REGISTRY_MAX_BYTES, TARGET_REGISTRY_MAX_TARGETS,
  TARGET_REGISTRY_PROFILE, TARGET_REGISTRY_VERSION,
};

pub use artifacts::{
  export_artifact, export_artifact_backup, inspect_artifact, inspect_run_artifacts,
  maintain_artifacts, observe_artifacts, restore_artifact_backup, verify_artifact_backup,
  ArtifactBackendPolicy, ArtifactBackupInventoryV1, ArtifactExportV1, ArtifactInspectionV1,
  ArtifactLimits, ArtifactMaintenanceV1, ArtifactMaterializationPolicy, ArtifactObservationV1,
  ArtifactPolicyError, ArtifactPolicyModes, ArtifactPolicyV1, ArtifactProvenanceV1,
  ArtifactReferenceV1, ArtifactRetentionPolicy, ArtifactRunInspectionV1, ArtifactRuntimeMode,
  ArtifactSecretScanningPolicy, KnownSecretAction, ManagedArtifactPutHandler, ManagedArtifactStore,
  ARTIFACTS_CONTRACT, ARTIFACTS_CONTRACT_VERSION, ARTIFACT_POLICY_CONTRACT,
  ARTIFACT_POLICY_CONTRACT_VERSION,
};
pub use backup::{
  create_online_backup, inspect_backup_store, prepare_restored_store, record_verified_backup,
  BackupError, BackupStoreInspection,
};
pub use cache::{
  CacheClock, CacheLimits, FixedCacheClock, ManagedCacheHandler, ManagedCacheStore,
  SystemCacheClock, CACHE_CONTRACT, CACHE_CONTRACT_VERSION, DEFAULT_CACHE_MAX_BYTES,
  DEFAULT_CACHE_MAX_ENTRIES, DEFAULT_CACHE_TTL_MS, MAX_CACHE_KEY_BYTES, MAX_CACHE_TTL_MS,
  MAX_CACHE_VALUE_BYTES,
};
pub use capability::{
  capability_transport_failure, derive_operation_key, CapabilityCallIdentity, CapabilityCallLimits,
  CapabilityCallRequest, CapabilityCallResult, CapabilityCancellationToken, CapabilityDescriptor,
  CapabilityEffect, CapabilityFailure, CapabilityFailureKind, CapabilityHandler,
  CapabilityRegistry, CapabilityRegistryError, DurableCapabilityAuthority,
  DurableCapabilityAuthorityError, NativeFetchFailure, NativeFetchFailureKind,
  NativeFetchInvocationIdentity, NativeFetchObservation, TestCapabilityHandler,
  CAPABILITY_CALL_CONTRACT, CAPABILITY_CALL_CONTRACT_VERSION, DEFAULT_CAPABILITY_FRAME_BYTES,
  DEFAULT_CAPABILITY_INPUT_BYTES, DEFAULT_CAPABILITY_RESULT_BYTES, DEFAULT_CAPABILITY_TIMEOUT_MS,
  NATIVE_FETCH_OBSERVATION_CONTRACT, NATIVE_FETCH_OBSERVATION_CONTRACT_VERSION,
};
pub use custom_notification_host::{
  CustomNotificationHostClient, CustomNotificationHostClientError,
  CustomNotificationHostProcessOptions, CustomProviderScriptArtifact,
};
pub use custom_notification_provider_protocol::{
  decode_custom_provider_frame, encode_custom_provider_frame, CustomNotificationAction,
  CustomNotificationActions, CustomNotificationKind, CustomNotificationRequest,
  CustomProviderAttempt, CustomProviderCancelMessage, CustomProviderCompletedMessage,
  CustomProviderExecuteMessage, CustomProviderFailure, CustomProviderFailureKind,
  CustomProviderLimits, CustomProviderOutcome, CustomProviderReadyMessage, CustomProviderReceipt,
  CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES, CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
pub use database::{ManagedDatabaseHandler, ManagedDatabasePool};
pub use discord::ManagedDiscordHandler;
pub use durable::{
  derive_lifecycle_hook_invocation_id, ApprovalDecisionOutcome, ApprovalDecisionOutcomeStatus,
  ApprovalTimeoutSettlement, ApprovalTimeoutSettlementStatus, ApprovalTokenBinding,
  CorrectionInspectionV1, CorrectionObservationV1, CorrectionRecoveryActionV1,
  CorrectionRecoveryCandidateV1, CorrectionRecoveryRequestV1, CorrectionRecoveryResultV1,
  CorrectionRecoveryStatusV1, CorrectionSagaInspectionV1, DurableDagEngine, DurableEngineError,
  DurableEventStore, DurableStoreError, DurableWaitRecordV1, ExternalSignalAdmissionOutcome,
  ExternalSignalAdmissionRequest, ExternalSignalAdmissionStatus, ExternalSignalResolution,
  ForkRecoveryBranchV1, ForkRecoveryWorkV1, InfrastructurePolicyRunBindingV1,
  InspectedBusinessOutcome, InspectedForEachStatusV6, InspectedForkJoinModeV4,
  InspectedForkStatusV4, InspectedLockStatusV1, InspectedReusableKindV5,
  InspectedReusableLifecycleStatusV5, InspectedReusableStatusV5, InspectedWaitStatusV1,
  InspectedWaitV1, InternalEventAdmissionOutcome, InternalEventAdmissionRequest, IntervalCursor,
  IntervalCursorRegistration, IntervalCursorRegistrationOutcome, IssuedApprovalToken,
  LockInspectionV1, LockLeaseMaintenance, LockObservationV1, LockRecoveryCandidateV1,
  LockRecoveryKindV1, NotificationDeliveryWork, NotificationDispatchReport,
  NotificationProviderAdapter, NotificationProviderDeliveryResult,
  NotificationProviderUpdateResult, NotificationUpdateWork, PolicyClaimWaitReason,
  PolicyExecutionClaimResult, PolicyWaitingFor, ProgrammaticLockAcquire,
  ProgrammaticLockAcquireState, ProgrammaticLockLease, PublicRunStatus, RecoveryReport,
  RichApprovalDecisionAdmission, RichApprovalDecisionInspection, RichApprovalDecisionOutcome,
  RichApprovalDecisionStatus, RichApprovalIdentityDecision, RichApprovalInspectionV1,
  RunCancellationCode, RunCancellationResult, RunCancellationStatus, RunDefinitionBinding,
  RunInspectionCancellationV2, RunInspectionForEachCountsV6, RunInspectionForEachItemV6,
  RunInspectionForEachV6, RunInspectionForkCountsV4, RunInspectionForkJoinV4, RunInspectionForkV4,
  RunInspectionForksV4, RunInspectionHookV2, RunInspectionPolicyV3, RunInspectionReusableCountsV5,
  RunInspectionReusableDefinitionsV5, RunInspectionReusableItemV5, RunInspectionRichApprovalV11,
  RunInspectionRichApprovalsV11, RunInspectionV11, RunInspectionV2, RunInspectionV3,
  RunInspectionV4, RunInspectionV5, RunInspectionV6, RunListV1, RunListV2, RunSummaryV1,
  RunSummaryV2, RunTimeoutSettlement, RuntimeObservationV1, RuntimeOwnerLease, ScheduleCursor,
  ScheduleCursorRegistration, ScheduleCursorRegistrationOutcome, SchedulerClaimV1,
  SignalWaitTimeoutSettlement, StepFailureCommit, StepFailureDisposition, TimerWaitSettlement,
  TriggerAdmissionOutcome, TriggerAdmissionRequest, TriggerOccurrence, TriggerRecoveryWork,
  UnresolvedCorrectionActionV1, WaitInspectionV1, CORRECTION_INSPECTION_PROFILE,
  CORRECTION_OBSERVATION_PROFILE, CORRECTION_RECOVERY_BATCH_LIMIT,
  CORRECTION_RECOVERY_REQUEST_PROFILE, CORRECTION_RECOVERY_RESULT_PROFILE,
  CORRECTION_UNRESOLVED_STORE_LIMIT, DURABLE_STORE_SCHEMA_VERSION,
  EXTERNAL_SIGNAL_ADMISSION_PROFILE, EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN,
  EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES, EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH,
  EXTERNAL_SIGNAL_RESULT_PROFILE, LOCK_RECOVERY_BATCH_LIMIT, LOCK_WAITERS_PER_KEY_LIMIT,
  LOCK_WAITERS_PER_STORE_LIMIT, RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
  RUNTIME_POLICY_QUEUE_CEILING, RUN_INSPECTION_RICH_APPROVAL_LIMIT,
};
pub use durable_state::{
  DurableStateError, DurableStateExecution, DurableStateLimits, DurableStateStore, FixedStateClock,
  ManagedDurableStateHandler, ManagedDurableStateStore, StateClock, SystemStateClock,
  DEFAULT_STATE_MAX_BYTES, DEFAULT_STATE_MAX_KEYS, MAX_STATE_KEY_BYTES, MAX_STATE_SAFE_INTEGER,
  MAX_STATE_VALUE_BYTES, STATE_BUSY_TIMEOUT_MS, STATE_CONTRACT, STATE_CONTRACT_VERSION,
};
pub use engine::{step_effect_idempotency_key, EngineError, InMemoryDagEngine};
pub use event::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalInvalidatedData,
  ApprovalQuorumProgressedData, ApprovalRequestedData, ApprovalResolution, ApprovalResolvedData,
  ApprovalReviewRequestedData, ApprovalReviewerDecisionRecordedData, ApprovalTimeoutPolicy,
  AttemptFailure, AttemptFailureKind, BranchFailure, BranchSelectedData, BusinessOutcome,
  ChoiceSelectedData, CommunicationProviderMessageIdentity, CompensationRegisteredData,
  CorrectionAttemptFailedData, CorrectionAttemptIdentityData, CorrectionInterruptedData,
  CorrectionRetryScheduledData, CorrectiveActionKind, DurableWaitKind, DurableWaitTimeoutBehavior,
  DurableWaitWinner, ExternalSignalReceivedData, FailureSizeDetails, FinalLifecycleStatus,
  ForkBranchOutcome, ForkBranchSettledData, ForkJoinOutcome, ForkJoinSettledData, ForkOpenedData,
  InfrastructurePolicyDecisionRecordedData, JsonValueType, LifecycleActionFailedData,
  LifecycleActionIdentityData, LifecycleFailure, LifecycleFailureKind, LifecycleHookCompletedData,
  LifecycleHookCompletionStatus, LifecycleHookRequestedData, LifecycleSubject,
  LifecycleSubjectKind, LifecycleWarning, LockAcquireTimedOutData, LockAcquiredData,
  LockAcquisitionCancelledData, LockLeaseLostData, LockReleasedData, LockWaitRegisteredData,
  ManualRecoveryRequiredData, NotificationDecisionAcceptedData,
  NotificationDeliveryAttemptStartedData, NotificationDeliveryFailedData,
  NotificationDeliveryRequestedData, NotificationDeliverySucceededData,
  NotificationMessageUpdateAttemptStartedData, NotificationMessageUpdateFailedData,
  NotificationMessageUpdateRequestedData, NotificationMessageUpdatedData, NotificationResolution,
  NotificationRunFailure, NotificationSafeFailure, OperationExecutionMode, OperationFailedData,
  OperationStartedData, OperationSucceededData, ParallelFailure, ParallelFailurePolicy,
  ParallelGroupCompletedData, ParallelGroupOutcome, ParallelGroupStartedData,
  ProviderMessageIdentity, RemoteDispatchTransitionedData, ReusableLifecycleActionFailedData,
  ReusableLifecycleActionStartedData, ReusableLifecycleActionSucceededData,
  ReusableLifecycleCompletedData, ReusableLifecycleHook, ReusableLifecycleOutcome,
  ReusableLifecycleRequestedData, RichApprovalResolutionReason, RunAdmissionQueue,
  RunAdmissionTrigger, RunAdmittedData, RunCancellationRequestedData, RunEvent, RunEventPayload,
  RunExecutionStartedData, RunFailedData, RunFailedDataV1, RunFailedDataV2, RunFailedDataV3,
  RunFailedDataV4, RunFailedDataV5, RunFinalizedData, RunIngress, RunOutcomeDecidedData,
  RunStartedData, RunSucceededData, RunTimeoutReachedData, SagaCorrectionReason,
  SagaCorrectionStartedData, SagaIdentityData, SlackProviderMessageIdentity, StepAttemptFailedData,
  StepAttemptStartedData, StepAttemptSucceededData, StepRetryScheduledData, WaitCancelledData,
  WaitRegisteredData, WaitSatisfiedData, WaitTimedOutData,
};
pub use events_service::{
  EventServiceAcceptedRun, EventServiceRunDispatcher, EventServiceSubscriber, ManagedEventsHandler,
  EVENTS_SERVICE_CONTRACT, EVENTS_SERVICE_CONTRACT_VERSION, MAX_INTERNAL_EVENT_DEPTH,
};
pub use host::{
  ScriptHostClient, ScriptHostClientError, ScriptHostModuleArtifact, ScriptHostProcessOptions,
};
pub use http::ManagedHttpHandler;
pub use infrastructure_policy::{
  parse_infrastructure_policy, prepare_emergency_deny_overlay, prepare_infrastructure_policy,
  InfrastructurePolicyBundleV1, InfrastructurePolicyDecisionV1, InfrastructurePolicyDefaultV1,
  InfrastructurePolicyEffectV1, InfrastructurePolicyError, InfrastructurePolicyExecutionModeV1,
  InfrastructurePolicyRequirementsV1, InfrastructurePolicyRuleMatchV1, InfrastructurePolicyRuleV1,
  InfrastructurePolicySignatureStatusV1, InfrastructurePolicySignatureV1,
  InfrastructurePolicyTrustV1, PreparedInfrastructurePolicyV1, INFRASTRUCTURE_POLICY_CONTRACT,
  INFRASTRUCTURE_POLICY_CONTRACT_VERSION, INFRASTRUCTURE_POLICY_MAX_BYTES,
  INFRASTRUCTURE_POLICY_MAX_RULES,
};
pub use infrastructure_policy_runtime::{
  apply_emergency_deny_overlay, InfrastructurePolicyDecisionOutcomeV1,
  InfrastructurePolicyDecisionRecordV1, InfrastructurePolicyDescriptorContextV1,
  InfrastructurePolicyEmergencyDenyV1, InfrastructurePolicyEvaluationBudgetV1,
  InfrastructurePolicyEvaluatorCacheV1, InfrastructurePolicyEvidenceAuthorityV1,
  InfrastructurePolicyEvidenceKindV1, InfrastructurePolicyEvidenceReferenceV1,
  InfrastructurePolicyOperationDescriptorV1, InfrastructurePolicyOperationMetadataV1,
  InfrastructurePolicyPreparedEvaluatorV1, InfrastructurePolicyReasonCodeV1,
  InfrastructurePolicyRequirementResultV1, InfrastructurePolicyRequirementStatusV1,
  INFRASTRUCTURE_POLICY_DECISION_CONTRACT, INFRASTRUCTURE_POLICY_DESCRIPTOR_CONTRACT,
  INFRASTRUCTURE_POLICY_EVIDENCE_CONTRACT,
};
pub use interval::{
  IntervalError, IntervalProgress, IntervalProgressReason, IntervalProgressReporter, WomlInterval,
  INTERVAL_PROGRESS_CONTRACT, INTERVAL_PROGRESS_CONTRACT_VERSION, MAX_INTERVAL_MS, MIN_INTERVAL_MS,
};
pub use locks::{
  ManagedLocksHandler, ManagedLocksService, LOCKS_CAPABILITY_CONTRACT,
  LOCKS_CAPABILITY_CONTRACT_VERSION,
};
pub use model::{
  CompiledCorrectionIdentity, CompiledCorrectiveAction, CompiledLifecycleAction,
  CompiledLifecycleDefinition, CompiledLifecycleHook, CompiledLock, CompiledModuleBinding,
  CompiledModuleRuntime, CompiledQueuePolicy, CompiledRateLimitPolicy, CompiledReusableInvocation,
  CompiledRuntimePolicy, CompiledSaga, CompiledWait, CompiledWaitSignal,
  CompiledWorkflowDefinition, CompiledWorkflowNode, LifecycleEventName, ModelIssue, ModelIssueCode,
  ModelValidationError, NotificationDefinition, QueueDiscipline, RateLimitAlgorithm,
  ScriptRuntimeBindings, ValueExpression,
};
pub use notification_host::{
  NotificationHostClient, NotificationHostClientError, NotificationHostProcessOptions,
};
pub use notification_protocol::{
  InformationalNotificationDeliverMessage, NotificationApprovalMessage,
  NotificationCompletedMessage, NotificationCredentials, NotificationDeliverMessage,
  NotificationHostOutcome, NotificationInteractionMessage, NotificationReadyMessage,
  NotificationSecretReference, NotificationUpdateMessage, RichApprovalDecisionCapability,
  RichApprovalNotificationAuthority, RichApprovalNotificationDeliverMessage,
  RichApprovalNotificationPresentation, INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  NOTIFICATION_PROVIDER_MAX_FRAME_BYTES, NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION, RICH_APPROVAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
pub use notification_runtime::{
  run_notification_provider_journey, run_notification_provider_journey_with_custom,
  CustomNotificationJourneyOptions, NotificationDeliveryDiagnostic, NotificationJourneyDiagnostics,
  NotificationJourneyError, NotificationJourneyResult, NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION,
};
pub use operational_output::{
  operational_output_store_for_state, LazyOperationalOutputStore, OperationalAppendOutcomeV1,
  OperationalBodyV1, OperationalCloseStatusV1, OperationalExecutionPhaseV1, OperationalGapV1,
  OperationalIdentityV1, OperationalLogLevelV1, OperationalOutputError, OperationalOutputLimitsV1,
  OperationalOutputPrunePlanV1, OperationalOutputPruneResultV1, OperationalOutputReporter,
  OperationalOutputStore, OperationalOutputStoreInspectionV1, OperationalPageV1,
  OperationalProgressSnapshotV1, OperationalProgressStatusV1, OperationalRecordDraftV1,
  OperationalRecordV1, OperationalSourceV1, OperationalStreamEventV1,
  RemoteOperationalAppendOutcomeV1, DEFAULT_OPERATIONAL_INLINE_RUN_BYTES,
  DEFAULT_OPERATIONAL_RETENTION_MS, DEFAULT_OPERATIONAL_STORE_BYTES, MAX_OPERATIONAL_CHUNK_BYTES,
  MAX_OPERATIONAL_FOLLOWERS, MAX_OPERATIONAL_INLINE_RUN_BYTES, MAX_OPERATIONAL_PAGE_BYTES,
  MAX_OPERATIONAL_PAGE_RECORDS, MAX_OPERATIONAL_TEXT_BYTES,
  OPERATIONAL_OUTPUT_STORE_SCHEMA_VERSION, OPERATIONAL_PAGE_PROFILE, OPERATIONAL_RECORD_PROFILE,
  OPERATIONAL_STREAM_PROFILE,
};
pub use presentation::{
  project_run_presentation_v1, recent_run_presentations_from_store_v1,
  run_presentation_from_store_v1, ForEachIterationPresentationV1, ForEachPresentationV1,
  LifecyclePresentationV1, PresentationFailureV1, PresentationLifecycleHook, PresentationRunStatus,
  PresentationStepKind, PresentationStepStatus, PresentationTriggerType, RunPresentationError,
  RunPresentationListV1, RunPresentationSummaryV1, RunPresentationTriggerV1, RunPresentationV1,
  StepPresentationV1, TriggerPresentationV1, WorkflowPresentationV1, RUN_PRESENTATION_LIST_PROFILE,
  RUN_PRESENTATION_MAX_BYTES, RUN_PRESENTATION_MAX_EVENTS, RUN_PRESENTATION_MAX_ITERATIONS,
  RUN_PRESENTATION_MAX_LIFECYCLE, RUN_PRESENTATION_MAX_STEPS, RUN_PRESENTATION_MAX_WARNINGS,
  RUN_PRESENTATION_PROFILE, RUN_PRESENTATION_RECENT_LIMIT,
};
pub use process::{
  ManagedProcessHandler, ProcessPolicyEnvironment, ProcessPolicyError, ProcessPolicyExecutable,
  ProcessPolicyLimits, ProcessPolicyModes, ProcessPolicyV1, ProcessRuntimeMode, PROCESS_CONTRACT,
  PROCESS_CONTRACT_VERSION, PROCESS_POLICY_CONTRACT, PROCESS_POLICY_CONTRACT_VERSION,
};
pub use projection::{
  fold_events, ApprovalRequestProjection, ApprovalRequestStatus, CorrectionActionProjection,
  CorrectionActionStatus, DurableWaitProjection, DurableWaitStatus, FoldError,
  ForkBranchProjection, ForkJoinStatus, ForkProjection, LifecycleActionProjection,
  LifecycleActionStatus, LifecycleHookProjection, LifecycleHookStatus, LifecycleStatus,
  LockContext, LockProjection, LockStatus, NotificationDeliveryProjection,
  NotificationDeliveryStatus, NotificationMessageUpdateProjection, NotificationMessageUpdateStatus,
  OperationIdentity, OperationProjection, OperationStatus, ParallelGroupProjection,
  ParallelGroupStatus, RetryScheduleProjection, ReusableLifecycleProjection,
  ReusableLifecycleStatus, RichApprovalRequestProjection, RichApprovalRequestStatus,
  RichApprovalReviewerDecisionProjection, RunFailure, RunProjection, RunStatus, SagaPhase,
  SagaProjection, WorkflowContext,
};
pub use retention::{
  execute_retention, last_retention_result, plan_retention, RetentionError, RetentionExecutionV1,
  RetentionPlanV1, RetentionPolicyV1, RetentionResultV1, TRIGGER_DEDUPLICATION_SAFETY_DAYS,
};
pub use rich_approval::{
  bind_rich_approval_review, resolve_rich_approval_review, verify_bound_rich_approval_review,
  BoundRichApprovalReview, ReviewChangeActionV1, ReviewChangeV1, ReviewCheckStatusV1,
  ReviewCheckV1, ReviewFactV1, ReviewPayloadV1, ReviewRiskV1, ReviewerDirectoryAuthorityV1,
  ReviewerDirectoryError, ReviewerDirectoryV1, ReviewerIdentityClaimV1, ReviewerIdentityV1,
  ReviewerPrincipalV1, ReviewerProviderV1, RichApprovalArtifactVerifier, RichApprovalBindingInput,
  RichApprovalError, VerifiedReviewerV1, REVIEWER_DIRECTORY_PROFILE, REVIEW_ARTIFACTS_MAX_ITEMS,
  REVIEW_CHANGES_MAX_ITEMS, REVIEW_CHECKS_MAX_ITEMS, REVIEW_FACTS_MAX_ITEMS,
  REVIEW_PAYLOAD_MAX_BYTES, REVIEW_PAYLOAD_MAX_NESTING, REVIEW_PAYLOAD_VERSION,
  REVIEW_SECRET_MIN_BYTES, RICH_APPROVAL_MAX_PRINCIPALS, RICH_APPROVAL_MAX_ROLES,
  RICH_PLAN_APPROVAL_PROFILE,
};
pub use runtime::{
  admit_external_signal_durable, execute_admitted_trigger_run_durable,
  execute_reusable_provider_lifecycle_durable, execute_workflow, execute_workflow_durable,
  execute_workflow_durable_outcome, recover_durable_runs, resolve_human_approval_durable,
  resolve_rich_approval_durable, resume_due_timer_waits_durable, resume_workflow_durable,
  resume_workflow_durable_any_outcome, resume_workflow_durable_outcome,
  settle_approval_timeout_durable, BranchFailureSite, CancelledRunDetails, EngineClock,
  ExecutionProgress, ExecutionProgressReporter, ExternalSignalRuntimeOutcome,
  FailedApprovalDetails, FailedNotificationDetails, FailedParallelDetails, FixedEngineClock,
  LifecycleProgress, LifecycleProgressPhase, LifecycleProgressReporter, RuntimeExecutionError,
  RuntimeExecutionOptions, RuntimeModuleArtifact, RuntimePolicyProgress,
  RuntimePolicyProgressPhase, RuntimePolicyProgressReporter, SystemEngineClock,
  WaitingRichApprovalAuthority, WaitingWorkflowApproval, WaitingWorkflowSignal,
  WaitingWorkflowTimer, WorkflowExecutionResult, WorkflowRuntimeOutcome,
  EXECUTION_PROGRESS_CONTRACT, EXECUTION_PROGRESS_VERSION, LIFECYCLE_PROGRESS_PROFILE,
  RUNTIME_OUTCOME_CONTRACT, RUNTIME_OUTCOME_VERSION, RUNTIME_POLICY_PROGRESS_PROFILE,
};
pub use schedule::{
  ScheduleClock, ScheduleError, ScheduleMisfirePolicy, ScheduleProgress, ScheduleProgressReason,
  ScheduleProgressReporter, SystemScheduleClock, WomlSchedule, SCHEDULE_PROGRESS_CONTRACT,
  SCHEDULE_PROGRESS_CONTRACT_VERSION,
};
pub use storage::{ManagedStorageHandler, ManagedStorageStore, StorageObjectReference};
pub use store::{EventStoreError, InMemoryEventStore};
pub use telegram::ManagedTelegramHandler;
pub use telemetry::{
  bounded_telemetry_reporter, project_telemetry_signals, span_id_v1, trace_id_v1,
  TelemetryAttributeValue, TelemetrySignalContext, TelemetrySignalKind, TelemetrySignalReporter,
  TelemetrySignalStatus, TelemetrySignalV1, TELEMETRY_SIGNAL_PROFILE,
};
pub use telemetry_propagation::{
  parse_w3c_trace_context, woml_trace_sampled, woml_traceparent, InboundTraceContext,
  TelemetryAdmissionLinkReporter, TelemetryAdmissionLinkStatus, TelemetryAdmissionLinkV1,
  W3cTraceContext, MAX_TRACESTATE_BYTES, TELEMETRY_ADMISSION_LINK_PROFILE,
};
pub use webhook::{
  ExternalTriggerAdmissionCommand, ExternalTriggerAdmissionReceiver, TriggerProgress,
  TriggerProgressReporter, WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer,
  WomlWebhookServerConfig, TRIGGER_PROGRESS_CONTRACT, TRIGGER_PROGRESS_CONTRACT_VERSION,
  WEBHOOK_MAX_BODY_BYTES,
};
pub use whatsapp::ManagedWhatsAppHandler;
pub use workflow_calls::{
  derive_workflow_call_key, dispatch_admitted_workflow_call, workflow_routing_credential_hash,
  workflow_routing_session_credential, ManagedWorkflowCallsHandler, WorkflowCallAdmission,
  WorkflowCallAdmissionOutcome, WorkflowCallAdmissionRequest, WorkflowCallIndexState,
  WorkflowCallProgress, WorkflowCallProgressReporter, WorkflowCallRunRelations,
  WorkflowCallRunSummary, WorkflowRoutingAcknowledgement, WorkflowRoutingWakeup,
  WorkflowRuntimeRoute, WorkflowTarget, WorkflowTargetRegistry, WorkflowTargetRegistryError,
  MAX_WORKFLOW_CALL_DEPTH, MAX_WORKFLOW_CALL_INSPECTION_CHILDREN, WORKFLOW_CALL_CONTRACT,
  WORKFLOW_CALL_CONTRACT_VERSION, WORKFLOW_CALL_PROGRESS_CONTRACT,
  WORKFLOW_CALL_PROGRESS_CONTRACT_VERSION, WORKFLOW_ROUTING_CONTRACT,
  WORKFLOW_ROUTING_CONTRACT_VERSION, WORKFLOW_ROUTING_WAKE_PATH,
};

pub const COMPILED_MODEL_SCHEMA_VERSION_V1: u32 = 1;
pub const COMPILED_MODEL_SCHEMA_VERSION_V2: u32 = 2;
pub const COMPILED_MODEL_SCHEMA_VERSION_V3: u32 = 3;
pub const COMPILED_MODEL_SCHEMA_VERSION_V4: u32 = 4;
pub const COMPILED_MODEL_SCHEMA_VERSION_V5: u32 = 5;
pub const COMPILED_MODEL_SCHEMA_VERSION_V6: u32 = 6;
pub const COMPILED_MODEL_SCHEMA_VERSION_V7: u32 = 7;
pub const COMPILED_MODEL_SCHEMA_VERSION_V8: u32 = 8;
pub const COMPILED_MODEL_SCHEMA_VERSION_V9: u32 = 9;
pub const COMPILED_MODEL_SCHEMA_VERSION_V10: u32 = 10;
pub const COMPILED_MODEL_SCHEMA_VERSION_V11: u32 = 11;
pub const COMPILED_MODEL_SCHEMA_VERSION_V12: u32 = 12;
pub const COMPILED_MODEL_SCHEMA_VERSION_V13: u32 = 13;
pub const COMPILED_MODEL_SCHEMA_VERSION_V14: u32 = 14;
pub const COMPILED_MODEL_SCHEMA_VERSION_V15: u32 = 15;
pub const COMPILED_MODEL_SCHEMA_VERSION_V16: u32 = 16;
pub const COMPILED_MODEL_SCHEMA_VERSION_V17: u32 = 17;
pub const COMPILED_MODEL_SCHEMA_VERSION_V18: u32 = 18;
pub const COMPILED_MODEL_SCHEMA_VERSION_V19: u32 = 19;
pub const COMPILED_MODEL_SCHEMA_VERSION_V20: u32 = 20;
pub const COMPILED_MODEL_SCHEMA_VERSION_V21: u32 = 21;
pub const COMPILED_MODEL_SCHEMA_VERSION: u32 = COMPILED_MODEL_SCHEMA_VERSION_V21;
pub const RUN_EVENT_SCHEMA_VERSION_V1: u32 = 1;
pub const RUN_EVENT_SCHEMA_VERSION_V2: u32 = 2;
pub const RUN_EVENT_SCHEMA_VERSION_V3: u32 = 3;
pub const RUN_EVENT_SCHEMA_VERSION_V4: u32 = 4;
pub const RUN_EVENT_SCHEMA_VERSION_V5: u32 = 5;
pub const RUN_EVENT_SCHEMA_VERSION_V6: u32 = 6;
pub const RUN_EVENT_SCHEMA_VERSION_V7: u32 = 7;
pub const RUN_EVENT_SCHEMA_VERSION_V8: u32 = 8;
pub const RUN_EVENT_SCHEMA_VERSION_V9: u32 = 9;
pub const RUN_EVENT_SCHEMA_VERSION_V10: u32 = 10;
pub const RUN_EVENT_SCHEMA_VERSION_V11: u32 = 11;
pub const RUN_EVENT_SCHEMA_VERSION_V12: u32 = 12;
pub const RUN_EVENT_SCHEMA_VERSION_V13: u32 = 13;
pub const RUN_EVENT_SCHEMA_VERSION_V14: u32 = 14;
pub const RUN_EVENT_SCHEMA_VERSION_V15: u32 = 15;
pub const RUN_EVENT_SCHEMA_VERSION_V16: u32 = 16;
pub const RUN_EVENT_SCHEMA_VERSION_V17: u32 = 17;
pub const RUN_EVENT_SCHEMA_VERSION_V18: u32 = 18;
pub const RUN_EVENT_SCHEMA_VERSION_V19: u32 = 19;
pub const RUN_EVENT_SCHEMA_VERSION_V20: u32 = 20;
pub const RUN_EVENT_SCHEMA_VERSION: u32 = RUN_EVENT_SCHEMA_VERSION_V20;

pub const fn run_event_schema_version_for_model(model_schema_version: u32) -> u32 {
  if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V21 {
    RUN_EVENT_SCHEMA_VERSION_V20
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V20 {
    RUN_EVENT_SCHEMA_VERSION_V19
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V19 {
    RUN_EVENT_SCHEMA_VERSION_V18
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V18 {
    RUN_EVENT_SCHEMA_VERSION_V17
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V17 {
    RUN_EVENT_SCHEMA_VERSION_V16
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V16 {
    RUN_EVENT_SCHEMA_VERSION_V15
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V15 {
    RUN_EVENT_SCHEMA_VERSION_V14
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V14 {
    RUN_EVENT_SCHEMA_VERSION_V13
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V13 {
    RUN_EVENT_SCHEMA_VERSION_V12
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V12 {
    RUN_EVENT_SCHEMA_VERSION_V11
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V11 {
    RUN_EVENT_SCHEMA_VERSION_V10
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V10 {
    RUN_EVENT_SCHEMA_VERSION_V9
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V8 {
    RUN_EVENT_SCHEMA_VERSION_V8
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V7 {
    RUN_EVENT_SCHEMA_VERSION_V7
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V6 {
    RUN_EVENT_SCHEMA_VERSION_V6
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V5 {
    RUN_EVENT_SCHEMA_VERSION_V5
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V4 {
    RUN_EVENT_SCHEMA_VERSION_V4
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V3 {
    RUN_EVENT_SCHEMA_VERSION_V3
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V2 {
    RUN_EVENT_SCHEMA_VERSION_V2
  } else {
    RUN_EVENT_SCHEMA_VERSION_V1
  }
}
