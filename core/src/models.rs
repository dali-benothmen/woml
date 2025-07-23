//! Data models for the Node-Cronflow Core Engine

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Workflow definition structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDefinition>,
    pub triggers: Vec<TriggerDefinition>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowDefinition {
    /// Validate the workflow definition
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("Workflow ID cannot be empty".to_string());
        }
        
        if self.name.is_empty() {
            return Err("Workflow name cannot be empty".to_string());
        }
        
        if self.steps.is_empty() {
            return Err("Workflow must have at least one step".to_string());
        }
        
        // Validate step IDs are unique
        let step_ids: Vec<&String> = self.steps.iter().map(|s| &s.id).collect();
        let unique_ids: Vec<&String> = step_ids.iter().map(|&&ref id| id).collect();
        if step_ids.len() != unique_ids.len() {
            return Err("Step IDs must be unique".to_string());
        }
        
        // Validate each step
        for step in &self.steps {
            step.validate()?;
        }
        
        Ok(())
    }
    
    /// Get a step by ID
    pub fn get_step(&self, step_id: &str) -> Option<&StepDefinition> {
        self.steps.iter().find(|s| s.id == step_id)
    }
    
    /// Check if workflow has a specific trigger type
    pub fn has_trigger_type(&self, trigger_type: &str) -> bool {
        self.triggers.iter().any(|t| match t {
            TriggerDefinition::Webhook { .. } => trigger_type == "webhook",
            TriggerDefinition::Schedule { .. } => trigger_type == "schedule",
            TriggerDefinition::Manual => trigger_type == "manual",
        })
    }
}

/// Step definition structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefinition {
    pub id: String,
    pub name: String,
    pub action: String,
    pub timeout: Option<u64>,
    pub retry: Option<RetryConfig>,
    pub depends_on: Vec<String>,
}

impl StepDefinition {
    /// Validate the step definition
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("Step ID cannot be empty".to_string());
        }
        
        if self.name.is_empty() {
            return Err("Step name cannot be empty".to_string());
        }
        
        if self.action.is_empty() {
            return Err("Step action cannot be empty".to_string());
        }
        
        // Validate retry configuration if present
        if let Some(retry) = &self.retry {
            retry.validate()?;
        }
        
        Ok(())
    }
    
    /// Check if step has dependencies
    pub fn has_dependencies(&self) -> bool {
        !self.depends_on.is_empty()
    }
    
    /// Get timeout in milliseconds
    pub fn get_timeout_ms(&self) -> Option<u64> {
        self.timeout
    }
    
    /// Check if step can be retried (placeholder - will be implemented with retry count tracking)
    pub fn can_retry(&self) -> bool {
        // For now, always return true if retry config exists
        // In the future, this will check against the actual retry count
        self.retry.is_some()
    }
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff_ms: u64,
}

impl RetryConfig {
    /// Validate retry configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.max_attempts == 0 {
            return Err("Max attempts must be greater than 0".to_string());
        }
        
        if self.backoff_ms == 0 {
            return Err("Backoff must be greater than 0".to_string());
        }
        
        Ok(())
    }
    
    /// Get total retry time in milliseconds
    pub fn get_total_retry_time_ms(&self) -> u64 {
        self.max_attempts as u64 * self.backoff_ms
    }
}

/// Trigger definition structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerDefinition {
    Webhook {
        path: String,
        method: String,
    },
    Schedule {
        cron_expression: String,
    },
    Manual,
}

impl TriggerDefinition {
    /// Validate the trigger definition
    pub fn validate(&self) -> Result<(), String> {
        match self {
            TriggerDefinition::Webhook { path, method } => {
                if path.is_empty() {
                    return Err("Webhook path cannot be empty".to_string());
                }
                if method.is_empty() {
                    return Err("Webhook method cannot be empty".to_string());
                }
                // Validate HTTP method
                let valid_methods = ["GET", "POST", "PUT", "DELETE", "PATCH"];
                if !valid_methods.contains(&method.to_uppercase().as_str()) {
                    return Err(format!("Invalid HTTP method: {}", method));
                }
                Ok(())
            }
            TriggerDefinition::Schedule { cron_expression } => {
                if cron_expression.is_empty() {
                    return Err("Cron expression cannot be empty".to_string());
                }
                // Basic cron validation (could be enhanced with a cron parser)
                if cron_expression.split_whitespace().count() != 5 {
                    return Err("Invalid cron expression format".to_string());
                }
                Ok(())
            }
            TriggerDefinition::Manual => Ok(()),
        }
    }
    
    /// Get trigger type as string
    pub fn get_type(&self) -> &'static str {
        match self {
            TriggerDefinition::Webhook { .. } => "webhook",
            TriggerDefinition::Schedule { .. } => "schedule",
            TriggerDefinition::Manual => "manual",
        }
    }
}

/// Workflow run state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub workflow_id: String,
    pub status: RunStatus,
    pub payload: serde_json::Value,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl WorkflowRun {
    /// Validate the workflow run
    pub fn validate(&self) -> Result<(), String> {
        if self.workflow_id.is_empty() {
            return Err("Workflow ID cannot be empty".to_string());
        }
        
        // Check that completed runs have a completed_at timestamp
        if matches!(self.status, RunStatus::Completed | RunStatus::Failed) && self.completed_at.is_none() {
            return Err("Completed runs must have a completed_at timestamp".to_string());
        }
        
        // Check that failed runs have an error message
        if matches!(self.status, RunStatus::Failed) && self.error.is_none() {
            return Err("Failed runs must have an error message".to_string());
        }
        
        Ok(())
    }
    
    /// Check if run is active (pending or running)
    pub fn is_active(&self) -> bool {
        matches!(self.status, RunStatus::Pending | RunStatus::Running)
    }
    
    /// Check if run is completed (success or failure)
    pub fn is_completed(&self) -> bool {
        matches!(self.status, RunStatus::Completed | RunStatus::Failed)
    }
    
    /// Get run duration in milliseconds
    pub fn get_duration_ms(&self) -> Option<u64> {
        self.completed_at.map(|completed| {
            (completed - self.started_at).num_milliseconds() as u64
        })
    }
}

/// Run status enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    /// Check if status is terminal (no further transitions possible)
    pub fn is_terminal(&self) -> bool {
        matches!(self, RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled)
    }
    
    /// Get status as string
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }
}

/// Step execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub status: StepStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
}

impl StepResult {
    /// Validate the step result
    pub fn validate(&self) -> Result<(), String> {
        if self.step_id.is_empty() {
            return Err("Step ID cannot be empty".to_string());
        }
        
        // Check that completed steps have a completed_at timestamp
        if matches!(self.status, StepStatus::Completed | StepStatus::Failed) && self.completed_at.is_none() {
            return Err("Completed steps must have a completed_at timestamp".to_string());
        }
        
        // Check that failed steps have an error message
        if matches!(self.status, StepStatus::Failed) && self.error.is_none() {
            return Err("Failed steps must have an error message".to_string());
        }
        
        Ok(())
    }
    
    /// Check if step is active (pending or running)
    pub fn is_active(&self) -> bool {
        matches!(self.status, StepStatus::Pending | StepStatus::Running)
    }
    
    /// Check if step is completed (success or failure)
    pub fn is_completed(&self) -> bool {
        matches!(self.status, StepStatus::Completed | StepStatus::Failed)
    }
    
    /// Get step duration in milliseconds
    pub fn get_duration_ms(&self) -> Option<u64> {
        self.duration_ms.or_else(|| {
            self.completed_at.map(|completed| {
                (completed - self.started_at).num_milliseconds() as u64
            })
        })
    }
}

/// Step status enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl StepStatus {
    /// Check if status is terminal (no further transitions possible)
    pub fn is_terminal(&self) -> bool {
        matches!(self, StepStatus::Completed | StepStatus::Failed | StepStatus::Skipped)
    }
    
    /// Get status as string
    pub fn as_str(&self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Completed => "completed",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
        }
    }
} 

/// Workflow completion context for hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCompletionContext {
    /// Run ID
    pub run_id: String,
    /// Workflow ID
    pub workflow_id: String,
    /// Final run status
    pub status: RunStatus,
    /// All completed step results
    pub completed_steps: Vec<StepResult>,
    /// Error message if workflow failed
    pub error: Option<String>,
    /// Total execution duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Workflow start time
    pub started_at: chrono::DateTime<Utc>,
    /// Workflow completion time
    pub completed_at: chrono::DateTime<Utc>,
    /// Original payload that triggered the workflow
    pub payload: serde_json::Value,
    /// Final workflow output (last step result)
    pub final_output: Option<serde_json::Value>,
}

impl WorkflowCompletionContext {
    /// Create a new completion context
    pub fn new(
        run_id: String,
        workflow_id: String,
        status: RunStatus,
        completed_steps: Vec<StepResult>,
        error: Option<String>,
        started_at: chrono::DateTime<Utc>,
        completed_at: chrono::DateTime<Utc>,
        payload: serde_json::Value,
    ) -> Self {
        let duration_ms = Some((completed_at - started_at).num_milliseconds() as u64);
        let final_output = completed_steps.last().and_then(|step| step.output.clone());
        
        Self {
            run_id,
            workflow_id,
            status,
            completed_steps,
            error,
            duration_ms,
            started_at,
            completed_at,
            payload,
            final_output,
        }
    }
    
    /// Check if workflow completed successfully
    pub fn is_success(&self) -> bool {
        matches!(self.status, RunStatus::Completed)
    }
    
    /// Check if workflow failed
    pub fn is_failure(&self) -> bool {
        matches!(self.status, RunStatus::Failed)
    }
    
    /// Get number of completed steps
    pub fn completed_step_count(&self) -> usize {
        self.completed_steps.len()
    }
    
    /// Get number of failed steps
    pub fn failed_step_count(&self) -> usize {
        self.completed_steps.iter()
            .filter(|step| matches!(step.status, StepStatus::Failed))
            .count()
    }
} 