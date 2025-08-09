//! Data models for the Node-Cronflow Core Engine

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Control flow condition types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConditionType {
    If,
    ElseIf,
    Else,
    EndIf,
}

impl ConditionType {
    /// Check if this is a conditional step that requires evaluation
    pub fn is_conditional(&self) -> bool {
        matches!(self, ConditionType::If | ConditionType::ElseIf)
    }
    
    /// Check if this is a control flow boundary step
    pub fn is_boundary(&self) -> bool {
        matches!(self, ConditionType::If | ConditionType::EndIf)
    }
    
    /// Get the condition type as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            ConditionType::If => "if",
            ConditionType::ElseIf => "elseif",
            ConditionType::Else => "else",
            ConditionType::EndIf => "endif",
        }
    }
}

/// Control flow block for managing conditional execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlFlowBlock {
    /// Unique identifier for this control flow block
    pub block_id: String,
    /// Type of condition (if/elseif/else/endif)
    pub condition_type: ConditionType,
    /// Whether the condition was met (for if/elseif)
    pub condition_met: bool,
    /// Whether this block has been executed
    pub executed: bool,
    /// Start step ID for this block
    pub start_step: String,
    /// End step ID for this block (if known)
    pub end_step: Option<String>,
    /// Nested control flow blocks
    pub nested_blocks: Vec<ControlFlowBlock>,
    /// Parent block ID (for nested conditions)
    pub parent_block_id: Option<String>,
}

impl ControlFlowBlock {
    /// Create a new control flow block
    pub fn new(block_id: String, condition_type: ConditionType, start_step: String) -> Self {
        Self {
            block_id,
            condition_type,
            condition_met: false,
            executed: false,
            start_step,
            end_step: None,
            nested_blocks: Vec::new(),
            parent_block_id: None,
        }
    }
    
    /// Mark the condition as met
    pub fn mark_condition_met(&mut self) {
        self.condition_met = true;
    }
    
    /// Mark the block as executed
    pub fn mark_executed(&mut self) {
        self.executed = true;
    }
    
    /// Check if this block should be executed
    pub fn should_execute(&self) -> bool {
        match self.condition_type {
            ConditionType::If => self.condition_met,
            ConditionType::ElseIf => self.condition_met && !self.executed,
            ConditionType::Else => !self.executed, // Else executes if no previous condition was met
            ConditionType::EndIf => true, // EndIf always executes
        }
    }
    
    /// Add a nested block
    pub fn add_nested_block(&mut self, block: ControlFlowBlock) {
        self.nested_blocks.push(block);
    }
}

/// Condition evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionResult {
    /// Whether the condition was met
    pub met: bool,
    /// Error message if condition evaluation failed
    pub error: Option<String>,
    /// Additional metadata about the evaluation
    pub metadata: serde_json::Value,
}

impl ConditionResult {
    /// Create a successful condition result
    pub fn success(met: bool) -> Self {
        Self {
            met,
            error: None,
            metadata: serde_json::json!({}),
        }
    }
    
    /// Create a failed condition result
    pub fn failure(error: String) -> Self {
        Self {
            met: false,
            error: Some(error),
            metadata: serde_json::json!({}),
        }
    }
}

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
        
        let step_ids: Vec<&String> = self.steps.iter().map(|s| &s.id).collect();
        let unique_ids: Vec<&String> = step_ids.iter().map(|&&ref id| id).collect();
        if step_ids.len() != unique_ids.len() {
            return Err("Step IDs must be unique".to_string());
        }
        
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
            TriggerDefinition::Manual => trigger_type == "manual",
        })
    }
}

/// Step definition structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefinition {
    pub id: String,
    pub name: String,
    pub title: Option<String>, // Human-readable title for the step
    pub description: Option<String>, // Optional description of what the step does
    pub action: String,
    pub timeout: Option<u64>,
    pub retry: Option<RetryConfig>,
    pub depends_on: Vec<String>,
    /// Control flow condition type (if/elseif/else/endif)
    pub condition_type: Option<ConditionType>,
    /// Serialized condition expression for evaluation
    pub condition_expression: Option<String>,
    /// Control flow block identifier
    pub control_flow_block: Option<String>,
    /// Whether this step is part of a control flow structure
    pub is_control_flow: bool,
    /// Whether this step should be executed in parallel
    pub parallel: Option<bool>,
    /// Parallel group identifier for grouping parallel steps
    pub parallel_group_id: Option<String>,
    /// Number of steps in the parallel group
    pub parallel_step_count: Option<usize>,
    /// Whether this is a race condition step
    pub race: Option<bool>,
    /// Whether this is a forEach loop step
    pub for_each: Option<bool>,
    /// Whether this step should pause workflow execution
    pub pause: Option<bool>,
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
        
        if let Some(retry) = &self.retry {
            retry.validate()?;
        }
        
        self.validate_control_flow()?;
        
        self.validate_parallel_execution()?;
        
        Ok(())
    }
    
    /// Validate control flow configuration
    fn validate_control_flow(&self) -> Result<(), String> {
        // If this is a control flow step, ensure it has proper configuration
        if self.is_control_flow {
            if let Some(condition_type) = &self.condition_type {
                match condition_type {
                    ConditionType::If | ConditionType::ElseIf => {
                        if self.condition_expression.is_none() {
                            return Err(format!("{} step must have a condition expression", condition_type.as_str()));
                        }
                        if self.control_flow_block.is_none() {
                            return Err(format!("{} step must have a control flow block identifier", condition_type.as_str()));
                        }
                    },
                    ConditionType::Else | ConditionType::EndIf => {
                        if self.control_flow_block.is_none() {
                            return Err(format!("{} step must have a control flow block identifier", condition_type.as_str()));
                        }
                    }
                }
            } else {
                return Err("Control flow step must have a condition type".to_string());
            }
        }
        
        Ok(())
    }
    
    /// Validate parallel execution configuration
    fn validate_parallel_execution(&self) -> Result<(), String> {
        if self.parallel.is_some() {
            if self.parallel_group_id.is_none() {
                return Err("Parallel step must have a parallel group ID".to_string());
            }
            if self.parallel_step_count.is_none() {
                return Err("Parallel step must have a parallel step count".to_string());
            }
            if self.parallel_step_count.unwrap() == 0 {
                return Err("Parallel step count must be greater than 0".to_string());
            }
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
        // In the future, this will check against the actual retry count
        self.retry.is_some()
    }
    
    /// Check if this step is a control flow step
    pub fn is_control_flow_step(&self) -> bool {
        self.is_control_flow && self.condition_type.is_some()
    }
    
    /// Check if this step requires condition evaluation
    pub fn requires_condition_evaluation(&self) -> bool {
        if let Some(condition_type) = &self.condition_type {
            condition_type.is_conditional()
        } else {
            false
        }
    }
    
    /// Check if this step is a control flow boundary
    pub fn is_control_flow_boundary(&self) -> bool {
        if let Some(condition_type) = &self.condition_type {
            condition_type.is_boundary()
        } else {
            false
        }
    }
    
    /// Get the control flow block ID
    pub fn get_control_flow_block_id(&self) -> Option<&String> {
        self.control_flow_block.as_ref()
    }
    
    /// Get the condition expression for evaluation
    pub fn get_condition_expression(&self) -> Option<&String> {
        self.condition_expression.as_ref()
    }
    
    /// Check if this step should be executed in parallel
    pub fn is_parallel(&self) -> bool {
        self.parallel.unwrap_or(false)
    }
    
    /// Check if this step is part of a race condition
    pub fn is_race(&self) -> bool {
        self.race.unwrap_or(false)
    }
    
    /// Check if this step is a forEach loop
    pub fn is_for_each(&self) -> bool {
        self.for_each.unwrap_or(false)
    }
    
    /// Get the parallel group ID if this step is parallel
    pub fn get_parallel_group_id(&self) -> Option<&String> {
        self.parallel_group_id.as_ref()
    }
    
    /// Get the number of steps in the parallel group
    pub fn get_parallel_step_count(&self) -> Option<usize> {
        self.parallel_step_count
    }
    
    /// Check if this step is a parallel execution step (parallel, race, or forEach)
    pub fn is_parallel_execution(&self) -> bool {
        self.is_parallel() || self.is_race() || self.is_for_each()
    }
    
    /// Check if this step should pause workflow execution
    pub fn is_pause_step(&self) -> bool {
        self.pause.unwrap_or(false)
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
                let valid_methods = ["GET", "POST", "PUT", "DELETE", "PATCH"];
                if !valid_methods.contains(&method.to_uppercase().as_str()) {
                    return Err(format!("Invalid HTTP method: {}", method));
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
        
        if matches!(self.status, RunStatus::Completed | RunStatus::Failed) && self.completed_at.is_none() {
            return Err("Completed runs must have a completed_at timestamp".to_string());
        }
        
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
        
        if matches!(self.status, StepStatus::Completed | StepStatus::Failed) && self.completed_at.is_none() {
            return Err("Completed steps must have a completed_at timestamp".to_string());
        }
        
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

/// Parallel step group for managing concurrent execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelStepGroup {
    /// Unique identifier for this parallel group
    pub group_id: String,
    /// Step IDs that are part of this parallel group
    pub step_ids: Vec<String>,
    /// Current status of the parallel group
    pub status: ParallelGroupStatus,
    /// Results from individual parallel steps
    pub results: std::collections::HashMap<String, StepResult>,
    /// When the parallel group started execution
    pub started_at: DateTime<Utc>,
    /// When the parallel group completed execution
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message if the parallel group failed
    pub error: Option<String>,
    /// Whether to fail fast on first error
    pub fail_fast: bool,
    /// Maximum timeout for the entire parallel group
    pub timeout_ms: Option<u64>,
}

impl ParallelStepGroup {
    /// Create a new parallel step group
    pub fn new(group_id: String, step_ids: Vec<String>) -> Self {
        Self {
            group_id,
            step_ids,
            status: ParallelGroupStatus::Pending,
            results: std::collections::HashMap::new(),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
            fail_fast: true, // Default to fail fast
            timeout_ms: None,
        }
    }
    
    /// Mark the group as running
    pub fn mark_running(&mut self) {
        self.status = ParallelGroupStatus::Running;
        self.started_at = Utc::now();
    }
    
    /// Mark the group as completed
    pub fn mark_completed(&mut self) {
        self.status = ParallelGroupStatus::Completed;
        self.completed_at = Some(Utc::now());
    }
    
    /// Mark the group as failed
    pub fn mark_failed(&mut self, error: String) {
        self.status = ParallelGroupStatus::Failed;
        self.completed_at = Some(Utc::now());
        self.error = Some(error);
    }
    
    /// Mark the group as partially failed
    pub fn mark_partially_failed(&mut self, error: String) {
        self.status = ParallelGroupStatus::PartiallyFailed;
        self.completed_at = Some(Utc::now());
        self.error = Some(error);
    }
    
    /// Add a step result to the group
    pub fn add_step_result(&mut self, step_id: String, result: StepResult) {
        self.results.insert(step_id, result);
    }
    
    /// Check if all steps in the group are completed
    pub fn is_completed(&self) -> bool {
        self.step_ids.iter().all(|step_id| {
            self.results.contains_key(step_id.as_str()) && 
            matches!(self.results[step_id.as_str()].status, StepStatus::Completed)
        })
    }
    
    /// Check if any step in the group failed
    pub fn has_failures(&self) -> bool {
        self.step_ids.iter().any(|step_id| {
            self.results.contains_key(step_id.as_str()) && 
            matches!(self.results[step_id.as_str()].status, StepStatus::Failed)
        })
    }
    
    /// Get the number of completed steps
    pub fn completed_count(&self) -> usize {
        self.step_ids.iter().filter(|step_id| {
            self.results.contains_key(step_id.as_str()) && 
            matches!(self.results[step_id.as_str()].status, StepStatus::Completed)
        }).count()
    }
    
    /// Get the number of failed steps
    pub fn failed_count(&self) -> usize {
        self.step_ids.iter().filter(|step_id| {
            self.results.contains_key(step_id.as_str()) && 
            matches!(self.results[step_id.as_str()].status, StepStatus::Failed)
        }).count()
    }
    
    /// Get the duration of the parallel group execution
    pub fn get_duration_ms(&self) -> Option<u64> {
        self.completed_at.map(|completed| {
            (completed - self.started_at).num_milliseconds() as u64
        })
    }
}

/// Parallel execution status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ParallelGroupStatus {
    /// Group is pending execution
    Pending,
    /// Group is currently running
    Running,
    /// All steps in the group completed successfully
    Completed,
    /// All steps in the group failed
    Failed,
    /// Some steps succeeded, some failed
    PartiallyFailed,
    /// Group execution timed out
    TimedOut,
}

impl ParallelGroupStatus {
    /// Check if this is a terminal status
    pub fn is_terminal(&self) -> bool {
        matches!(self, 
            ParallelGroupStatus::Completed | 
            ParallelGroupStatus::Failed | 
            ParallelGroupStatus::PartiallyFailed |
            ParallelGroupStatus::TimedOut
        )
    }
    
    /// Get the status as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            ParallelGroupStatus::Pending => "pending",
            ParallelGroupStatus::Running => "running",
            ParallelGroupStatus::Completed => "completed",
            ParallelGroupStatus::Failed => "failed",
            ParallelGroupStatus::PartiallyFailed => "partially_failed",
            ParallelGroupStatus::TimedOut => "timed_out",
        }
    }
} 