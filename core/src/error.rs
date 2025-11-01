//! Error types for the Node-Cronflow Core Engine

use thiserror::Error;

/// Core engine error types
#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Invalid workflow definition: {0}")]
    InvalidWorkflow(String),

    #[error("Workflow not found: {0}")]
    WorkflowNotFound(String),

    #[error("Run not found: {0}")]
    RunNotFound(String),

    #[error("Step not found: {0}")]
    StepNotFound(String),

    #[error("Step execution failed: {0}")]
    StepExecution(String),

    #[error("State management error: {0}")]
    State(String),

    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    #[error("Invalid workflow state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid trigger configuration: {0}")]
    InvalidTrigger(String),

    #[error("Trigger not found: {0}")]
    TriggerNotFound(String),

    #[error("Date parsing error: {0}")]
    DateParse(#[from] chrono::ParseError),

    #[error("UUID parsing error: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for core operations
pub type CoreResult<T> = Result<T, CoreError>; 