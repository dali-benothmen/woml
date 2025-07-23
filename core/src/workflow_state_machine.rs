//! Workflow Execution State Machine
//! 
//! This module provides a state machine for orchestrating workflow execution,
//! including step tracking, dependency management, and control flow logic.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use crate::error::{CoreError, CoreResult};
use crate::state::StateManager;
use crate::models::{WorkflowDefinition, WorkflowRun, StepDefinition, StepResult, StepStatus, RunStatus};

/// Workflow execution state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkflowExecutionState {
    /// Workflow is pending execution
    Pending,
    /// Workflow is currently running
    Running,
    /// Workflow execution is paused
    Paused,
    /// Workflow completed successfully
    Completed,
    /// Workflow failed
    Failed,
    /// Workflow was cancelled
    Cancelled,
}

/// Step execution state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecutionState {
    /// Step definition
    pub step: StepDefinition,
    /// Current status
    pub status: StepStatus,
    /// Dependencies that need to be completed
    pub pending_dependencies: HashSet<String>,
    /// Whether this step is ready for execution
    pub ready: bool,
    /// Step result if completed
    pub result: Option<StepResult>,
    /// Retry count
    pub retry_count: u32,
    /// Last error if failed
    pub last_error: Option<String>,
}

impl StepExecutionState {
    /// Create a new step execution state
    pub fn new(step: StepDefinition) -> Self {
        let pending_dependencies: HashSet<String> = step.depends_on.iter().cloned().collect();
        let ready = pending_dependencies.is_empty();
        
        Self {
            step,
            status: StepStatus::Pending,
            pending_dependencies,
            ready,
            result: None,
            retry_count: 0,
            last_error: None,
        }
    }
    
    /// Mark a dependency as completed
    pub fn mark_dependency_completed(&mut self, step_id: &str) {
        self.pending_dependencies.remove(step_id);
        self.ready = self.pending_dependencies.is_empty();
    }
    
    /// Mark step as running
    pub fn mark_running(&mut self) {
        self.status = StepStatus::Running;
    }
    
    /// Mark step as completed
    pub fn mark_completed(&mut self, result: StepResult) {
        self.status = StepStatus::Completed;
        self.result = Some(result);
    }
    
    /// Mark step as failed
    pub fn mark_failed(&mut self, error: String) {
        self.status = StepStatus::Failed;
        self.last_error = Some(error);
        self.retry_count += 1;
    }
    
    /// Check if step can be retried
    pub fn can_retry(&self) -> bool {
        if let Some(retry_config) = &self.step.retry {
            self.retry_count < retry_config.max_attempts
        } else {
            false
        }
    }
    
    /// Reset step for retry
    pub fn reset_for_retry(&mut self) {
        self.status = StepStatus::Pending;
        self.last_error = None;
    }
}

/// Workflow execution statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionStats {
    /// Total number of steps
    pub total_steps: usize,
    /// Number of completed steps
    pub completed_steps: usize,
    /// Number of failed steps
    pub failed_steps: usize,
    /// Number of pending steps
    pub pending_steps: usize,
    /// Number of running steps
    pub running_steps: usize,
    /// Number of skipped steps
    pub skipped_steps: usize,
    /// Workflow start time
    pub started_at: chrono::DateTime<Utc>,
    /// Workflow completion time
    pub completed_at: Option<chrono::DateTime<Utc>>,
    /// Total execution time in milliseconds
    pub total_duration_ms: Option<u64>,
}

impl WorkflowExecutionStats {
    /// Create new stats
    pub fn new(total_steps: usize) -> Self {
        Self {
            total_steps,
            completed_steps: 0,
            failed_steps: 0,
            pending_steps: total_steps,
            running_steps: 0,
            skipped_steps: 0,
            started_at: Utc::now(),
            completed_at: None,
            total_duration_ms: None,
        }
    }
    
    /// Update stats based on step states
    pub fn update_from_states(&mut self, step_states: &HashMap<String, StepExecutionState>) {
        self.completed_steps = 0;
        self.failed_steps = 0;
        self.pending_steps = 0;
        self.running_steps = 0;
        self.skipped_steps = 0;
        
        for state in step_states.values() {
            match state.status {
                StepStatus::Completed => self.completed_steps += 1,
                StepStatus::Failed => self.failed_steps += 1,
                StepStatus::Pending => self.pending_steps += 1,
                StepStatus::Running => self.running_steps += 1,
                StepStatus::Skipped => self.skipped_steps += 1,
            }
        }
    }
    
    /// Mark workflow as completed
    pub fn mark_completed(&mut self) {
        self.completed_at = Some(Utc::now());
        self.total_duration_ms = self.completed_at
            .map(|completed| (completed - self.started_at).num_milliseconds() as u64);
    }
    
    /// Get completion percentage
    pub fn completion_percentage(&self) -> f64 {
        if self.total_steps == 0 {
            0.0
        } else {
            (self.completed_steps as f64 / self.total_steps as f64) * 100.0
        }
    }
    
    /// Check if workflow is complete
    pub fn is_complete(&self) -> bool {
        self.completed_steps + self.failed_steps + self.skipped_steps == self.total_steps
    }
}

/// Workflow execution state machine
pub struct WorkflowStateMachine {
    /// State manager for persistence
    state_manager: Arc<Mutex<StateManager>>,
    /// Workflow ID
    workflow_id: String,
    /// Run ID
    run_id: Uuid,
    /// Current execution state
    execution_state: WorkflowExecutionState,
    /// Step execution states
    step_states: HashMap<String, StepExecutionState>,
    /// Completed step results
    completed_steps: Vec<StepResult>,
    /// Current step index
    current_step_index: usize,
    /// Total number of steps
    total_steps: usize,
    /// Execution statistics
    stats: WorkflowExecutionStats,
    /// Workflow definition
    workflow_definition: Option<WorkflowDefinition>,
    /// Workflow run
    workflow_run: Option<WorkflowRun>,
}

impl WorkflowStateMachine {
    /// Create a new workflow state machine
    pub fn new(
        state_manager: Arc<Mutex<StateManager>>,
        workflow_id: String,
        run_id: Uuid,
    ) -> Self {
        Self {
            state_manager,
            workflow_id,
            run_id,
            execution_state: WorkflowExecutionState::Pending,
            step_states: HashMap::new(),
            completed_steps: Vec::new(),
            current_step_index: 0,
            total_steps: 0,
            stats: WorkflowExecutionStats::new(0),
            workflow_definition: None,
            workflow_run: None,
        }
    }
    
    /// Initialize the state machine with workflow definition
    pub fn initialize(&mut self) -> CoreResult<()> {
        log::info!("Initializing workflow state machine for run: {}", self.run_id);
        
        // Get workflow definition and run
        let workflow = {
            let state_manager = self.state_manager.lock()
                .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
            
            state_manager.get_workflow(&self.workflow_id)?
                .ok_or_else(|| CoreError::WorkflowNotFound(self.workflow_id.clone()))?
        };
        
        let run = {
            let state_manager = self.state_manager.lock()
                .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
            
            state_manager.get_run(&self.run_id)?
                .ok_or_else(|| CoreError::RunNotFound(format!("Run not found: {}", self.run_id)))?
        };
        
        // Store workflow and run
        self.workflow_definition = Some(workflow.clone());
        self.workflow_run = Some(run.clone());
        
        // Initialize step states
        self.initialize_step_states(&workflow)?;
        
        // Update statistics
        self.total_steps = workflow.steps.len();
        self.stats = WorkflowExecutionStats::new(self.total_steps);
        
        // Set initial state
        self.execution_state = WorkflowExecutionState::Running;
        
        log::info!("Workflow state machine initialized with {} steps", self.total_steps);
        Ok(())
    }
    
    /// Initialize step states from workflow definition
    fn initialize_step_states(&mut self, workflow: &WorkflowDefinition) -> CoreResult<()> {
        self.step_states.clear();
        
        // Create step states
        for step in &workflow.steps {
            let step_state = StepExecutionState::new(step.clone());
            self.step_states.insert(step.id.clone(), step_state);
        }
        
        // Validate dependencies
        self.validate_dependencies()?;
        
        log::debug!("Initialized {} step states", self.step_states.len());
        Ok(())
    }
    
    /// Validate step dependencies
    fn validate_dependencies(&self) -> CoreResult<()> {
        for (step_id, step_state) in &self.step_states {
            for dependency in &step_state.step.depends_on {
                if !self.step_states.contains_key(dependency) {
                    return Err(CoreError::Validation(format!(
                        "Step {} depends on non-existent step {}", step_id, dependency
                    )));
                }
            }
        }
        Ok(())
    }
    
    /// Get steps that are ready for execution
    pub fn get_ready_steps(&self) -> Vec<String> {
        self.step_states
            .iter()
            .filter(|(_, state)| state.ready && state.status == StepStatus::Pending)
            .map(|(step_id, _)| step_id.clone())
            .collect()
    }
    
    /// Mark a step as running
    pub fn mark_step_running(&mut self, step_id: &str) -> CoreResult<()> {
        if let Some(step_state) = self.step_states.get_mut(step_id) {
            step_state.mark_running();
            self.update_stats();
            log::debug!("Marked step {} as running", step_id);
            Ok(())
        } else {
            Err(CoreError::StepNotFound(format!("Step not found: {}", step_id)))
        }
    }
    
    /// Mark a step as completed
    pub fn mark_step_completed(&mut self, step_id: &str, output: serde_json::Value) -> CoreResult<()> {
        if let Some(step_state) = self.step_states.get_mut(step_id) {
            // Create step result
            let result = StepResult {
                step_id: step_id.to_string(),
                status: StepStatus::Completed,
                output: Some(output),
                error: None,
                started_at: Utc::now(), // This should be updated with actual start time
                completed_at: Some(Utc::now()),
                duration_ms: None, // This should be calculated from actual start time
            };
            
            step_state.mark_completed(result.clone());
            self.completed_steps.push(result);
            
            // Update dependencies for other steps
            self.update_dependencies(step_id);
            
            // Update statistics
            self.update_stats();
            
            log::debug!("Marked step {} as completed", step_id);
            Ok(())
        } else {
            Err(CoreError::StepNotFound(format!("Step not found: {}", step_id)))
        }
    }
    
    /// Mark a step as failed
    pub fn mark_step_failed(&mut self, step_id: &str, error: String) -> CoreResult<()> {
        if let Some(step_state) = self.step_states.get_mut(step_id) {
            step_state.mark_failed(error.clone());
            
            // Create step result
            let result = StepResult {
                step_id: step_id.to_string(),
                status: StepStatus::Failed,
                output: None,
                error: Some(error),
                started_at: Utc::now(), // This should be updated with actual start time
                completed_at: Some(Utc::now()),
                duration_ms: None, // This should be calculated from actual start time
            };
            
            self.completed_steps.push(result);
            
            // Update statistics
            self.update_stats();
            
            log::debug!("Marked step {} as failed", step_id);
            Ok(())
        } else {
            Err(CoreError::StepNotFound(format!("Step not found: {}", step_id)))
        }
    }
    
    /// Update dependencies when a step is completed
    fn update_dependencies(&mut self, completed_step_id: &str) {
        for step_state in self.step_states.values_mut() {
            step_state.mark_dependency_completed(completed_step_id);
        }
    }
    
    /// Update execution statistics
    fn update_stats(&mut self) {
        self.stats.update_from_states(&self.step_states);
    }
    
    /// Check if workflow is complete
    pub fn check_workflow_completion(&mut self) -> CoreResult<bool> {
        if self.stats.is_complete() {
            // Determine final state
            if self.stats.failed_steps > 0 {
                self.execution_state = WorkflowExecutionState::Failed;
            } else {
                self.execution_state = WorkflowExecutionState::Completed;
            }
            
            self.stats.mark_completed();
            
            log::info!("Workflow completed with state: {:?}", self.execution_state);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    /// Get current execution state
    pub fn get_execution_state(&self) -> &WorkflowExecutionState {
        &self.execution_state
    }
    
    /// Get execution statistics
    pub fn get_stats(&self) -> &WorkflowExecutionStats {
        &self.stats
    }
    
    /// Get step state
    pub fn get_step_state(&self, step_id: &str) -> Option<&StepExecutionState> {
        self.step_states.get(step_id)
    }
    
    /// Get all step states
    pub fn get_step_states(&self) -> &HashMap<String, StepExecutionState> {
        &self.step_states
    }
    
    /// Get completed steps
    pub fn get_completed_steps(&self) -> &[StepResult] {
        &self.completed_steps
    }
    
    /// Pause workflow execution
    pub fn pause(&mut self) -> CoreResult<()> {
        if self.execution_state == WorkflowExecutionState::Running {
            self.execution_state = WorkflowExecutionState::Paused;
            log::info!("Workflow execution paused");
            Ok(())
        } else {
            Err(CoreError::InvalidState(format!(
                "Cannot pause workflow in state: {:?}", self.execution_state
            )))
        }
    }
    
    /// Resume workflow execution
    pub fn resume(&mut self) -> CoreResult<()> {
        if self.execution_state == WorkflowExecutionState::Paused {
            self.execution_state = WorkflowExecutionState::Running;
            log::info!("Workflow execution resumed");
            Ok(())
        } else {
            Err(CoreError::InvalidState(format!(
                "Cannot resume workflow in state: {:?}", self.execution_state
            )))
        }
    }
    
    /// Cancel workflow execution
    pub fn cancel(&mut self, reason: Option<String>) -> CoreResult<()> {
        if !self.execution_state.is_terminal() {
            self.execution_state = WorkflowExecutionState::Cancelled;
            log::info!("Workflow execution cancelled: {}", reason.unwrap_or_else(|| "No reason provided".to_string()));
            Ok(())
        } else {
            Err(CoreError::InvalidState(format!(
                "Cannot cancel workflow in state: {:?}", self.execution_state
            )))
        }
    }
    
    /// Get workflow definition
    pub fn get_workflow_definition(&self) -> Option<&WorkflowDefinition> {
        self.workflow_definition.as_ref()
    }
    
    /// Get workflow run
    pub fn get_workflow_run(&self) -> Option<&WorkflowRun> {
        self.workflow_run.as_ref()
    }
    
    /// Save state to database
    pub fn save_state(&self) -> CoreResult<()> {
        let mut state_manager = self.state_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
        
        // Update run status
        let run_status = match self.execution_state {
            WorkflowExecutionState::Pending | WorkflowExecutionState::Running => RunStatus::Running,
            WorkflowExecutionState::Paused => RunStatus::Running, // Keep as running when paused
            WorkflowExecutionState::Completed => RunStatus::Completed,
            WorkflowExecutionState::Failed => RunStatus::Failed,
            WorkflowExecutionState::Cancelled => RunStatus::Cancelled,
        };
        
        state_manager.update_run_status(&self.run_id, run_status)?;
        
        // Save completed steps
        for step_result in &self.completed_steps {
            state_manager.save_step_result(&self.run_id, step_result.clone())?;
        }
        
        log::debug!("Saved workflow state to database");
        Ok(())
    }
}

impl WorkflowExecutionState {
    /// Check if state is terminal (no further transitions possible)
    pub fn is_terminal(&self) -> bool {
        matches!(self, 
            WorkflowExecutionState::Completed | 
            WorkflowExecutionState::Failed | 
            WorkflowExecutionState::Cancelled
        )
    }
    
    /// Get state as string
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowExecutionState::Pending => "pending",
            WorkflowExecutionState::Running => "running",
            WorkflowExecutionState::Paused => "paused",
            WorkflowExecutionState::Completed => "completed",
            WorkflowExecutionState::Failed => "failed",
            WorkflowExecutionState::Cancelled => "cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{WorkflowDefinition, StepDefinition, TriggerDefinition, RunStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn create_test_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            id: "test-workflow".to_string(),
            name: "Test Workflow".to_string(),
            description: None,
            steps: vec![
                StepDefinition {
                    id: "step-1".to_string(),
                    name: "Step 1".to_string(),
                    action: "test_action_1".to_string(),
                    timeout: None,
                    retry: None,
                    depends_on: vec![],
                },
                StepDefinition {
                    id: "step-2".to_string(),
                    name: "Step 2".to_string(),
                    action: "test_action_2".to_string(),
                    timeout: None,
                    retry: None,
                    depends_on: vec!["step-1".to_string()],
                },
                StepDefinition {
                    id: "step-3".to_string(),
                    name: "Step 3".to_string(),
                    action: "test_action_3".to_string(),
                    timeout: None,
                    retry: None,
                    depends_on: vec!["step-1".to_string()],
                },
            ],
            triggers: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn create_test_run() -> WorkflowRun {
        WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "test-workflow".to_string(),
            status: RunStatus::Pending,
            payload: serde_json::json!({"test": "data"}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    #[test]
    fn test_state_machine_creation() {
        let state_manager = Arc::new(Mutex::new(crate::state::StateManager::new(":memory:").unwrap()));
        let run_id = Uuid::new_v4();
        let state_machine = WorkflowStateMachine::new(
            state_manager,
            "test-workflow".to_string(),
            run_id,
        );
        
        assert_eq!(state_machine.workflow_id, "test-workflow");
        assert_eq!(state_machine.run_id, run_id);
        assert_eq!(state_machine.execution_state, WorkflowExecutionState::Pending);
    }

    #[test]
    fn test_step_state_creation() {
        let step = StepDefinition {
            id: "test-step".to_string(),
            name: "Test Step".to_string(),
            action: "test_action".to_string(),
            timeout: None,
            retry: None,
            depends_on: vec!["dependency-1".to_string(), "dependency-2".to_string()],
        };
        
        let step_state = StepExecutionState::new(step);
        
        assert_eq!(step_state.status, StepStatus::Pending);
        assert_eq!(step_state.pending_dependencies.len(), 2);
        assert!(!step_state.ready);
        assert_eq!(step_state.retry_count, 0);
    }

    #[test]
    fn test_dependency_management() {
        let step = StepDefinition {
            id: "test-step".to_string(),
            name: "Test Step".to_string(),
            action: "test_action".to_string(),
            timeout: None,
            retry: None,
            depends_on: vec!["dependency-1".to_string(), "dependency-2".to_string()],
        };
        
        let mut step_state = StepExecutionState::new(step);
        
        // Initially not ready
        assert!(!step_state.ready);
        assert_eq!(step_state.pending_dependencies.len(), 2);
        
        // Mark first dependency as completed
        step_state.mark_dependency_completed("dependency-1");
        assert!(!step_state.ready);
        assert_eq!(step_state.pending_dependencies.len(), 1);
        
        // Mark second dependency as completed
        step_state.mark_dependency_completed("dependency-2");
        assert!(step_state.ready);
        assert_eq!(step_state.pending_dependencies.len(), 0);
    }

    #[test]
    fn test_workflow_stats() {
        let mut stats = WorkflowExecutionStats::new(5);
        
        assert_eq!(stats.total_steps, 5);
        assert_eq!(stats.completed_steps, 0);
        assert_eq!(stats.pending_steps, 5);
        assert_eq!(stats.completion_percentage(), 0.0);
        assert!(!stats.is_complete());
        
        // Update stats
        stats.completed_steps = 3;
        stats.failed_steps = 1;
        stats.pending_steps = 1;
        
        assert_eq!(stats.completion_percentage(), 60.0);
        assert!(!stats.is_complete());
        
        // Complete all steps
        stats.completed_steps = 4;
        stats.failed_steps = 1;
        stats.pending_steps = 0;
        
        assert!(stats.is_complete());
    }

    #[test]
    fn test_execution_state_transitions() {
        let state = WorkflowExecutionState::Pending;
        assert!(!state.is_terminal());
        assert_eq!(state.as_str(), "pending");
        
        let state = WorkflowExecutionState::Running;
        assert!(!state.is_terminal());
        assert_eq!(state.as_str(), "running");
        
        let state = WorkflowExecutionState::Completed;
        assert!(state.is_terminal());
        assert_eq!(state.as_str(), "completed");
        
        let state = WorkflowExecutionState::Failed;
        assert!(state.is_terminal());
        assert_eq!(state.as_str(), "failed");
        
        let state = WorkflowExecutionState::Cancelled;
        assert!(state.is_terminal());
        assert_eq!(state.as_str(), "cancelled");
    }
} 