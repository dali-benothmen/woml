//! The language-neutral WOML execution core.
//!
//! This crate is intentionally isolated from the legacy Cronflow SDK execution
//! paths and from N-API. It consumes the versioned Compiled Workflow Model and
//! derives all run state by folding versioned events.

pub mod durable;
pub mod engine;
pub mod event;
pub mod host;
pub mod model;
pub mod notification_host;
pub mod notification_protocol;
pub mod notification_runtime;
pub mod projection;
pub mod protocol;
pub mod runtime;
pub mod store;
pub mod webhook;

pub use durable::{
  ApprovalDecisionOutcome, ApprovalDecisionOutcomeStatus, ApprovalTimeoutSettlement,
  ApprovalTimeoutSettlementStatus, ApprovalTokenBinding, DurableDagEngine, DurableEngineError,
  DurableEventStore, DurableStoreError, IssuedApprovalToken, NotificationDeliveryWork,
  NotificationDispatchReport, NotificationProviderAdapter, NotificationProviderDeliveryResult,
  NotificationProviderUpdateResult, NotificationUpdateWork, RecoveryReport, RunDefinitionBinding,
  StepFailureCommit, StepFailureDisposition, TriggerAdmissionOutcome, TriggerAdmissionRequest,
  TriggerOccurrence, TriggerRecoveryWork, DURABLE_STORE_SCHEMA_VERSION,
};
pub use engine::{step_effect_idempotency_key, EngineError, InMemoryDagEngine};
pub use event::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalRequestedData,
  ApprovalResolution, ApprovalResolvedData, ApprovalTimeoutPolicy, AttemptFailure,
  AttemptFailureKind, BranchFailure, BranchSelectedData, FailureSizeDetails, JsonValueType,
  NotificationDecisionAcceptedData, NotificationDeliveryAttemptStartedData,
  NotificationDeliveryFailedData, NotificationDeliveryRequestedData,
  NotificationDeliverySucceededData, NotificationMessageUpdateAttemptStartedData,
  NotificationMessageUpdateFailedData, NotificationMessageUpdateRequestedData,
  NotificationMessageUpdatedData, NotificationResolution, NotificationRunFailure,
  NotificationSafeFailure, ParallelFailure, ParallelFailurePolicy, ParallelGroupCompletedData,
  ParallelGroupOutcome, ParallelGroupStartedData, ProviderMessageIdentity, RunEvent,
  RunEventPayload, RunFailedData, RunFailedDataV1, RunFailedDataV2, RunFailedDataV3,
  RunFailedDataV4, RunFailedDataV5, RunStartedData, StepRetryScheduledData,
};
pub use host::{ScriptHostClient, ScriptHostClientError, ScriptHostProcessOptions};
pub use model::{
  CompiledWorkflowDefinition, ModelIssue, ModelIssueCode, ModelValidationError,
  NotificationDefinition,
};
pub use notification_host::{
  NotificationHostClient, NotificationHostClientError, NotificationHostProcessOptions,
};
pub use notification_protocol::{
  NotificationApprovalMessage, NotificationCompletedMessage, NotificationCredentials,
  NotificationDeliverMessage, NotificationHostOutcome, NotificationInteractionMessage,
  NotificationReadyMessage, NotificationSecretReference, NotificationUpdateMessage,
  NOTIFICATION_PROVIDER_MAX_FRAME_BYTES, NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};
pub use notification_runtime::{
  run_notification_provider_journey, NotificationDeliveryDiagnostic,
  NotificationJourneyDiagnostics, NotificationJourneyError, NotificationJourneyResult,
  NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION,
};
pub use projection::{
  fold_events, ApprovalRequestProjection, ApprovalRequestStatus, FoldError,
  NotificationDeliveryProjection, NotificationDeliveryStatus, NotificationMessageUpdateProjection,
  NotificationMessageUpdateStatus, ParallelGroupProjection, ParallelGroupStatus,
  RetryScheduleProjection, RunFailure, RunProjection, RunStatus, WorkflowContext,
};
pub use runtime::{
  execute_admitted_trigger_run_durable, execute_workflow, execute_workflow_durable,
  execute_workflow_durable_outcome, recover_durable_runs, resolve_human_approval_durable,
  resume_workflow_durable, resume_workflow_durable_outcome, settle_approval_timeout_durable,
  BranchFailureSite, EngineClock, ExecutionProgress, ExecutionProgressReporter,
  FailedApprovalDetails, FailedNotificationDetails, FailedParallelDetails, FixedEngineClock,
  RuntimeExecutionError, RuntimeExecutionOptions, SystemEngineClock, WaitingWorkflowApproval,
  WorkflowExecutionResult, WorkflowRuntimeOutcome, EXECUTION_PROGRESS_CONTRACT,
  EXECUTION_PROGRESS_VERSION, RUNTIME_OUTCOME_CONTRACT, RUNTIME_OUTCOME_VERSION,
};
pub use store::{EventStoreError, InMemoryEventStore};
pub use webhook::{
  ExternalTriggerAdmissionCommand, ExternalTriggerAdmissionReceiver, TriggerProgress,
  TriggerProgressReporter, WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer,
  WomlWebhookServerConfig, TRIGGER_PROGRESS_CONTRACT, TRIGGER_PROGRESS_CONTRACT_VERSION,
  WEBHOOK_MAX_BODY_BYTES,
};

pub const COMPILED_MODEL_SCHEMA_VERSION_V1: u32 = 1;
pub const COMPILED_MODEL_SCHEMA_VERSION_V2: u32 = 2;
pub const COMPILED_MODEL_SCHEMA_VERSION_V3: u32 = 3;
pub const COMPILED_MODEL_SCHEMA_VERSION_V4: u32 = 4;
pub const COMPILED_MODEL_SCHEMA_VERSION_V5: u32 = 5;
pub const COMPILED_MODEL_SCHEMA_VERSION_V6: u32 = 6;
pub const COMPILED_MODEL_SCHEMA_VERSION_V7: u32 = 7;
pub const COMPILED_MODEL_SCHEMA_VERSION: u32 = COMPILED_MODEL_SCHEMA_VERSION_V7;
pub const RUN_EVENT_SCHEMA_VERSION_V1: u32 = 1;
pub const RUN_EVENT_SCHEMA_VERSION_V2: u32 = 2;
pub const RUN_EVENT_SCHEMA_VERSION_V3: u32 = 3;
pub const RUN_EVENT_SCHEMA_VERSION_V4: u32 = 4;
pub const RUN_EVENT_SCHEMA_VERSION_V5: u32 = 5;
pub const RUN_EVENT_SCHEMA_VERSION_V6: u32 = 6;
pub const RUN_EVENT_SCHEMA_VERSION_V7: u32 = 7;
pub const RUN_EVENT_SCHEMA_VERSION: u32 = RUN_EVENT_SCHEMA_VERSION_V7;

pub const fn run_event_schema_version_for_model(model_schema_version: u32) -> u32 {
  if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V7 {
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
