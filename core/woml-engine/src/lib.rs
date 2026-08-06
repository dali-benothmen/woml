//! The language-neutral WOML execution core.
//!
//! This crate is intentionally isolated from the legacy Cronflow SDK execution
//! paths and from N-API. It consumes the versioned Compiled Workflow Model and
//! derives all run state by folding versioned events.

pub mod engine;
pub mod event;
pub mod model;
pub mod projection;
pub mod store;

pub use engine::{EngineError, InMemoryDagEngine};
pub use event::{
  AttemptFailure, AttemptFailureKind, FailureSizeDetails, RunEvent, RunEventPayload,
};
pub use model::{CompiledWorkflowDefinition, ModelIssue, ModelIssueCode, ModelValidationError};
pub use projection::{fold_events, FoldError, RunProjection, RunStatus, WorkflowContext};
pub use store::{EventStoreError, InMemoryEventStore};

pub const COMPILED_MODEL_SCHEMA_VERSION: u32 = 1;
pub const RUN_EVENT_SCHEMA_VERSION: u32 = 1;
