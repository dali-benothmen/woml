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
  DurableDagEngine, DurableEngineError, DurableEventStore, DurableStoreError, RecoveryReport,
  RunDefinitionBinding,
};
pub use engine::{EngineError, InMemoryDagEngine};
pub use event::{
  AttemptFailure, AttemptFailureKind, FailureSizeDetails, RunEvent, RunEventPayload,
};
pub use host::{ScriptHostClient, ScriptHostClientError, ScriptHostProcessOptions};
pub use model::{CompiledWorkflowDefinition, ModelIssue, ModelIssueCode, ModelValidationError};
pub use projection::{fold_events, FoldError, RunProjection, RunStatus, WorkflowContext};
pub use runtime::{
  execute_workflow, execute_workflow_durable, recover_durable_runs, RuntimeExecutionError,
  RuntimeExecutionOptions, WorkflowExecutionResult,
};
pub use store::{EventStoreError, InMemoryEventStore};

pub const COMPILED_MODEL_SCHEMA_VERSION_V1: u32 = 1;
pub const COMPILED_MODEL_SCHEMA_VERSION_V2: u32 = 2;
pub const COMPILED_MODEL_SCHEMA_VERSION: u32 = COMPILED_MODEL_SCHEMA_VERSION_V2;
pub const RUN_EVENT_SCHEMA_VERSION: u32 = 1;
