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
pub mod projection;
pub mod protocol;
pub mod runtime;
pub mod store;

pub use durable::{
  ApprovalTokenBinding, DurableDagEngine, DurableEngineError, DurableEventStore, DurableStoreError,
  IssuedApprovalToken, RecoveryReport, RunDefinitionBinding, DURABLE_STORE_SCHEMA_VERSION,
};
pub use engine::{EngineError, InMemoryDagEngine};
pub use event::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalRequestedData,
  ApprovalResolution, ApprovalResolvedData, ApprovalTimeoutPolicy, AttemptFailure,
  AttemptFailureKind, BranchFailure, BranchSelectedData, FailureSizeDetails, JsonValueType,
  ParallelFailure, ParallelFailurePolicy, ParallelGroupCompletedData, ParallelGroupOutcome,
  ParallelGroupStartedData, RunEvent, RunEventPayload, RunFailedData, RunFailedDataV1,
  RunFailedDataV2, RunFailedDataV3, RunFailedDataV4,
};
pub use host::{ScriptHostClient, ScriptHostClientError, ScriptHostProcessOptions};
pub use model::{CompiledWorkflowDefinition, ModelIssue, ModelIssueCode, ModelValidationError};
pub use projection::{
  fold_events, ApprovalRequestProjection, ApprovalRequestStatus, FoldError,
  ParallelGroupProjection, ParallelGroupStatus, RunFailure, RunProjection, RunStatus,
  WorkflowContext,
};
pub use runtime::{
  execute_workflow, execute_workflow_durable, execute_workflow_durable_outcome,
  recover_durable_runs, resume_workflow_durable, resume_workflow_durable_outcome,
  BranchFailureSite, FailedApprovalDetails, FailedParallelDetails, RuntimeExecutionError,
  RuntimeExecutionOptions, WaitingWorkflowApproval, WorkflowExecutionResult,
  WorkflowRuntimeOutcome, RUNTIME_OUTCOME_CONTRACT, RUNTIME_OUTCOME_VERSION,
};
pub use store::{EventStoreError, InMemoryEventStore};

pub const COMPILED_MODEL_SCHEMA_VERSION_V1: u32 = 1;
pub const COMPILED_MODEL_SCHEMA_VERSION_V2: u32 = 2;
pub const COMPILED_MODEL_SCHEMA_VERSION_V3: u32 = 3;
pub const COMPILED_MODEL_SCHEMA_VERSION_V4: u32 = 4;
pub const COMPILED_MODEL_SCHEMA_VERSION: u32 = COMPILED_MODEL_SCHEMA_VERSION_V4;
pub const RUN_EVENT_SCHEMA_VERSION_V1: u32 = 1;
pub const RUN_EVENT_SCHEMA_VERSION_V2: u32 = 2;
pub const RUN_EVENT_SCHEMA_VERSION_V3: u32 = 3;
pub const RUN_EVENT_SCHEMA_VERSION_V4: u32 = 4;
pub const RUN_EVENT_SCHEMA_VERSION: u32 = RUN_EVENT_SCHEMA_VERSION_V4;

pub const fn run_event_schema_version_for_model(model_schema_version: u32) -> u32 {
  if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V4 {
    RUN_EVENT_SCHEMA_VERSION_V4
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V3 {
    RUN_EVENT_SCHEMA_VERSION_V3
  } else if model_schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V2 {
    RUN_EVENT_SCHEMA_VERSION_V2
  } else {
    RUN_EVENT_SCHEMA_VERSION_V1
  }
}
