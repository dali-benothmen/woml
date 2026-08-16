//! The language-neutral WOML execution core.
//!
//! This crate is intentionally isolated from retired authoring runtimes and
//! from N-API. It consumes the versioned Compiled Workflow Model and
//! derives all run state by folding versioned events.

pub mod backup;
pub mod cache;
pub mod capability;
pub mod custom_notification_host;
pub mod custom_notification_provider_protocol;
pub mod database;
mod database_postgres;
pub mod durable;
pub mod durable_state;
pub mod engine;
pub mod event;
pub mod events_service;
pub mod host;
pub mod http;
pub mod interval;
pub mod model;
pub mod notification_host;
pub mod notification_protocol;
pub mod notification_runtime;
pub mod presentation;
pub mod projection;
pub mod protocol;
pub mod retention;
pub mod runtime;
pub mod schedule;
pub mod storage;
pub mod store;
pub mod telegram;
pub mod webhook;
pub mod workflow_calls;

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
pub use durable::{
  derive_lifecycle_hook_invocation_id, ApprovalDecisionOutcome, ApprovalDecisionOutcomeStatus,
  ApprovalTimeoutSettlement, ApprovalTimeoutSettlementStatus, ApprovalTokenBinding,
  DurableDagEngine, DurableEngineError, DurableEventStore, DurableStoreError, ForkRecoveryBranchV1,
  ForkRecoveryWorkV1, InspectedBusinessOutcome, InspectedForkJoinModeV4, InspectedForkStatusV4,
  InspectedReusableKindV5, InspectedReusableLifecycleStatusV5, InspectedReusableStatusV5,
  InternalEventAdmissionOutcome, InternalEventAdmissionRequest, IntervalCursor,
  IntervalCursorRegistration, IntervalCursorRegistrationOutcome, IssuedApprovalToken,
  NotificationDeliveryWork, NotificationDispatchReport, NotificationProviderAdapter,
  NotificationProviderDeliveryResult, NotificationProviderUpdateResult, NotificationUpdateWork,
  PolicyClaimWaitReason, PolicyExecutionClaimResult, PolicyWaitingFor, PublicRunStatus,
  RecoveryReport, RunCancellationCode, RunCancellationResult, RunCancellationStatus,
  RunDefinitionBinding, RunInspectionCancellationV2, RunInspectionForkCountsV4,
  RunInspectionForkJoinV4, RunInspectionForkV4, RunInspectionForksV4, RunInspectionHookV2,
  RunInspectionPolicyV3, RunInspectionReusableCountsV5, RunInspectionReusableDefinitionsV5,
  RunInspectionReusableItemV5, RunInspectionV2, RunInspectionV3, RunInspectionV4, RunInspectionV5,
  RunListV1, RunListV2, RunSummaryV1, RunSummaryV2, RunTimeoutSettlement, RuntimeObservationV1,
  RuntimeOwnerLease, ScheduleCursor, ScheduleCursorRegistration, ScheduleCursorRegistrationOutcome,
  SchedulerClaimV1, StepFailureCommit, StepFailureDisposition, TriggerAdmissionOutcome,
  TriggerAdmissionRequest, TriggerOccurrence, TriggerRecoveryWork, DURABLE_STORE_SCHEMA_VERSION,
  RUNTIME_POLICY_QUEUE_CEILING,
};
pub use durable_state::{
  DurableStateError, DurableStateExecution, DurableStateLimits, DurableStateStore, FixedStateClock,
  ManagedDurableStateHandler, ManagedDurableStateStore, StateClock, SystemStateClock,
  DEFAULT_STATE_MAX_BYTES, DEFAULT_STATE_MAX_KEYS, MAX_STATE_KEY_BYTES, MAX_STATE_SAFE_INTEGER,
  MAX_STATE_VALUE_BYTES, STATE_BUSY_TIMEOUT_MS, STATE_CONTRACT, STATE_CONTRACT_VERSION,
};
pub use engine::{step_effect_idempotency_key, EngineError, InMemoryDagEngine};
pub use event::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalRequestedData,
  ApprovalResolution, ApprovalResolvedData, ApprovalTimeoutPolicy, AttemptFailure,
  AttemptFailureKind, BranchFailure, BranchSelectedData, BusinessOutcome, ChoiceSelectedData,
  CommunicationProviderMessageIdentity, FailureSizeDetails, FinalLifecycleStatus,
  ForkBranchOutcome, ForkBranchSettledData, ForkJoinOutcome, ForkJoinSettledData, ForkOpenedData,
  JsonValueType, LifecycleActionFailedData, LifecycleActionIdentityData, LifecycleFailure,
  LifecycleFailureKind, LifecycleHookCompletedData, LifecycleHookCompletionStatus,
  LifecycleHookRequestedData, LifecycleSubject, LifecycleSubjectKind, LifecycleWarning,
  NotificationDecisionAcceptedData, NotificationDeliveryAttemptStartedData,
  NotificationDeliveryFailedData, NotificationDeliveryRequestedData,
  NotificationDeliverySucceededData, NotificationMessageUpdateAttemptStartedData,
  NotificationMessageUpdateFailedData, NotificationMessageUpdateRequestedData,
  NotificationMessageUpdatedData, NotificationResolution, NotificationRunFailure,
  NotificationSafeFailure, OperationExecutionMode, OperationFailedData, OperationStartedData,
  OperationSucceededData, ParallelFailure, ParallelFailurePolicy, ParallelGroupCompletedData,
  ParallelGroupOutcome, ParallelGroupStartedData, ProviderMessageIdentity,
  ReusableLifecycleActionFailedData, ReusableLifecycleActionStartedData,
  ReusableLifecycleActionSucceededData, ReusableLifecycleCompletedData, ReusableLifecycleHook,
  ReusableLifecycleOutcome, ReusableLifecycleRequestedData, RunAdmissionQueue, RunAdmissionTrigger,
  RunAdmittedData, RunCancellationRequestedData, RunEvent, RunEventPayload,
  RunExecutionStartedData, RunFailedData, RunFailedDataV1, RunFailedDataV2, RunFailedDataV3,
  RunFailedDataV4, RunFailedDataV5, RunFinalizedData, RunIngress, RunOutcomeDecidedData,
  RunStartedData, RunSucceededData, RunTimeoutReachedData, SlackProviderMessageIdentity,
  StepAttemptFailedData, StepAttemptStartedData, StepAttemptSucceededData, StepRetryScheduledData,
};
pub use events_service::{
  EventServiceAcceptedRun, EventServiceRunDispatcher, EventServiceSubscriber, ManagedEventsHandler,
  EVENTS_SERVICE_CONTRACT, EVENTS_SERVICE_CONTRACT_VERSION, MAX_INTERNAL_EVENT_DEPTH,
};
pub use host::{
  ScriptHostClient, ScriptHostClientError, ScriptHostModuleArtifact, ScriptHostProcessOptions,
};
pub use http::ManagedHttpHandler;
pub use interval::{
  IntervalError, IntervalProgress, IntervalProgressReason, IntervalProgressReporter, WomlInterval,
  INTERVAL_PROGRESS_CONTRACT, INTERVAL_PROGRESS_CONTRACT_VERSION, MAX_INTERVAL_MS, MIN_INTERVAL_MS,
};
pub use model::{
  CompiledLifecycleAction, CompiledLifecycleDefinition, CompiledLifecycleHook,
  CompiledModuleBinding, CompiledModuleRuntime, CompiledQueuePolicy, CompiledRateLimitPolicy,
  CompiledReusableInvocation, CompiledRuntimePolicy, CompiledWorkflowDefinition,
  LifecycleEventName, ModelIssue, ModelIssueCode, ModelValidationError, NotificationDefinition,
  QueueDiscipline, RateLimitAlgorithm, ScriptRuntimeBindings,
};
pub use notification_host::{
  NotificationHostClient, NotificationHostClientError, NotificationHostProcessOptions,
};
pub use notification_protocol::{
  InformationalNotificationDeliverMessage, NotificationApprovalMessage,
  NotificationCompletedMessage, NotificationCredentials, NotificationDeliverMessage,
  NotificationHostOutcome, NotificationInteractionMessage, NotificationReadyMessage,
  NotificationSecretReference, NotificationUpdateMessage,
  INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION, NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  NOTIFICATION_PROVIDER_PROTOCOL, NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
pub use notification_runtime::{
  run_notification_provider_journey, run_notification_provider_journey_with_custom,
  CustomNotificationJourneyOptions, NotificationDeliveryDiagnostic, NotificationJourneyDiagnostics,
  NotificationJourneyError, NotificationJourneyResult, NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION,
};
pub use presentation::{
  project_run_presentation_v1, recent_run_presentations_from_store_v1,
  run_presentation_from_store_v1, LifecyclePresentationV1, PresentationFailureV1,
  PresentationLifecycleHook, PresentationRunStatus, PresentationStepKind, PresentationStepStatus,
  PresentationTriggerType, RunPresentationError, RunPresentationListV1, RunPresentationSummaryV1,
  RunPresentationTriggerV1, RunPresentationV1, StepPresentationV1, TriggerPresentationV1,
  WorkflowPresentationV1, RUN_PRESENTATION_LIST_PROFILE, RUN_PRESENTATION_MAX_BYTES,
  RUN_PRESENTATION_MAX_EVENTS, RUN_PRESENTATION_MAX_LIFECYCLE, RUN_PRESENTATION_MAX_STEPS,
  RUN_PRESENTATION_MAX_WARNINGS, RUN_PRESENTATION_PROFILE, RUN_PRESENTATION_RECENT_LIMIT,
};
pub use projection::{
  fold_events, ApprovalRequestProjection, ApprovalRequestStatus, FoldError, ForkBranchProjection,
  ForkJoinStatus, ForkProjection, LifecycleActionProjection, LifecycleActionStatus,
  LifecycleHookProjection, LifecycleHookStatus, LifecycleStatus, NotificationDeliveryProjection,
  NotificationDeliveryStatus, NotificationMessageUpdateProjection, NotificationMessageUpdateStatus,
  OperationIdentity, OperationProjection, OperationStatus, ParallelGroupProjection,
  ParallelGroupStatus, RetryScheduleProjection, ReusableLifecycleProjection,
  ReusableLifecycleStatus, RunFailure, RunProjection, RunStatus, WorkflowContext,
};
pub use retention::{
  execute_retention, last_retention_result, plan_retention, RetentionError, RetentionExecutionV1,
  RetentionPlanV1, RetentionPolicyV1, RetentionResultV1, TRIGGER_DEDUPLICATION_SAFETY_DAYS,
};
pub use runtime::{
  execute_admitted_trigger_run_durable, execute_reusable_provider_lifecycle_durable,
  execute_workflow, execute_workflow_durable, execute_workflow_durable_outcome,
  recover_durable_runs, resolve_human_approval_durable, resume_workflow_durable,
  resume_workflow_durable_any_outcome, resume_workflow_durable_outcome,
  settle_approval_timeout_durable, BranchFailureSite, CancelledRunDetails, EngineClock,
  ExecutionProgress, ExecutionProgressReporter, FailedApprovalDetails, FailedNotificationDetails,
  FailedParallelDetails, FixedEngineClock, LifecycleProgress, LifecycleProgressPhase,
  LifecycleProgressReporter, RuntimeExecutionError, RuntimeExecutionOptions, RuntimeModuleArtifact,
  RuntimePolicyProgress, RuntimePolicyProgressPhase, RuntimePolicyProgressReporter,
  SystemEngineClock, WaitingWorkflowApproval, WorkflowExecutionResult, WorkflowRuntimeOutcome,
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
pub use webhook::{
  ExternalTriggerAdmissionCommand, ExternalTriggerAdmissionReceiver, TriggerProgress,
  TriggerProgressReporter, WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer,
  WomlWebhookServerConfig, TRIGGER_PROGRESS_CONTRACT, TRIGGER_PROGRESS_CONTRACT_VERSION,
  WEBHOOK_MAX_BODY_BYTES,
};
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
pub const COMPILED_MODEL_SCHEMA_VERSION: u32 = COMPILED_MODEL_SCHEMA_VERSION_V15;
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
pub const RUN_EVENT_SCHEMA_VERSION: u32 = RUN_EVENT_SCHEMA_VERSION_V14;

pub const fn run_event_schema_version_for_model(model_schema_version: u32) -> u32 {
  if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V15 {
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
