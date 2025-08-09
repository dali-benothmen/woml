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
use crate::models::{WorkflowDefinition, WorkflowRun, StepDefinition, StepResult, StepStatus, RunStatus, ControlFlowBlock, ConditionType, ConditionResult, ParallelStepGroup, ParallelGroupStatus};
use crate::condition_evaluator::ConditionEvaluator;
use crate::context::Context;

/// Parallel execution configuration
#[derive(Debug, Clone)]
pub struct ParallelExecutionConfig {
    /// Maximum number of parallel steps that can run simultaneously
    pub max_concurrent_steps: usize,
    /// Whether to fail fast on first parallel step failure
    pub fail_fast: bool,
    /// Default timeout for parallel groups in milliseconds
    pub default_timeout_ms: Option<u64>,
    /// Whether to enable parallel execution
    pub enabled: bool,
}

impl Default for ParallelExecutionConfig {
    fn default() -> Self {
        // Use centralized configuration
        let core_config = crate::config::CoreConfig::default();
        Self {
            max_concurrent_steps: core_config.execution.max_concurrent_steps,
            fail_fast: core_config.execution.fail_fast,
            default_timeout_ms: core_config.execution.default_timeout_ms,
            enabled: true, // Always enabled by default
        }
    }
}

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
    /// Control flow blocks for conditional execution
    control_flow_blocks: HashMap<String, ControlFlowBlock>,
    /// Active control flow block stack (for nested conditions)
    control_flow_stack: Vec<String>,
    /// Steps that should be skipped due to control flow
    skipped_steps: HashSet<String>,
    /// Current condition evaluation context
    condition_context: Option<Context>,
    /// Parallel step groups for concurrent execution
    parallel_groups: HashMap<String, ParallelStepGroup>,
    /// Currently running parallel groups
    running_parallel_groups: HashSet<String>,
    /// Parallel execution configuration
    parallel_config: ParallelExecutionConfig,
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
            control_flow_blocks: HashMap::new(),
            control_flow_stack: Vec::new(),
            skipped_steps: HashSet::new(),
            condition_context: None,
            parallel_groups: HashMap::new(),
            running_parallel_groups: HashSet::new(),
            parallel_config: ParallelExecutionConfig::default(),
        }
    }
    
    /// Initialize the state machine with workflow definition
    pub fn initialize(&mut self) -> CoreResult<()> {
        log::info!("Initializing workflow state machine for run: {}", self.run_id);
        
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
        
        self.initialize_step_states(&workflow)?;
        
        self.initialize_control_flow_blocks(&workflow)?;
        
        self.validate_control_flow_structure(&workflow)?;
        
        self.initialize_parallel_groups(&workflow)?;
        
        self.create_condition_context(&run)?;
        
        self.total_steps = workflow.steps.len();
        self.stats = WorkflowExecutionStats::new(self.total_steps);
        
        self.execution_state = WorkflowExecutionState::Running;
        
        log::info!("Workflow state machine initialized with {} steps", self.total_steps);
        Ok(())
    }
    
    /// Initialize step states from workflow definition
    fn initialize_step_states(&mut self, workflow: &WorkflowDefinition) -> CoreResult<()> {
        self.step_states.clear();
        
        for step in &workflow.steps {
            let step_state = StepExecutionState::new(step.clone());
            self.step_states.insert(step.id.clone(), step_state);
        }
        
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
    
    /// Initialize control flow blocks from workflow definition
    fn initialize_control_flow_blocks(&mut self, workflow: &WorkflowDefinition) -> CoreResult<()> {
        self.control_flow_blocks.clear();
        self.control_flow_stack.clear();
        self.skipped_steps.clear();
        
        // Build control flow blocks from step definitions
        let mut current_block_id: Option<String> = None;
        let mut block_start_step: Option<String> = None;
        
        for step in &workflow.steps {
            if step.is_control_flow_step() {
                if let Some(condition_type) = &step.condition_type {
                    match condition_type {
                        ConditionType::If => {
                            // Start a new control flow block
                            if let Some(block_id) = &step.control_flow_block {
                                current_block_id = Some(block_id.clone());
                                block_start_step = Some(step.id.clone());
                                
                                let block = ControlFlowBlock::new(
                                    block_id.clone(),
                                    condition_type.clone(),
                                    step.id.clone(),
                                );
                                self.control_flow_blocks.insert(block_id.clone(), block);
                                self.control_flow_stack.push(block_id.clone());
                            }
                        },
                        ConditionType::ElseIf => {
                            // Continue the current block
                            if let Some(block_id) = &step.control_flow_block {
                                if let Some(block) = self.control_flow_blocks.get_mut(block_id) {
                                }
                            }
                        },
                        ConditionType::Else => {
                            // Continue the current block
                            if let Some(block_id) = &step.control_flow_block {
                                if let Some(block) = self.control_flow_blocks.get_mut(block_id) {
                                }
                            }
                        },
                        ConditionType::EndIf => {
                            // End the current control flow block
                            if let Some(block_id) = &step.control_flow_block {
                                if let Some(block) = self.control_flow_blocks.get_mut(block_id) {
                                    block.end_step = Some(step.id.clone());
                                }
                                self.control_flow_stack.pop();
                                current_block_id = None;
                                block_start_step = None;
                            }
                        }
                    }
                }
            }
        }
        
        log::debug!("Initialized {} control flow blocks", self.control_flow_blocks.len());
        Ok(())
    }
    
    /// Validate control flow structure
    fn validate_control_flow_structure(&self, workflow: &WorkflowDefinition) -> CoreResult<()> {
        let mut if_count = 0;
        let mut endif_count = 0;
        let mut current_block_stack = Vec::new();
        
        for step in &workflow.steps {
            if step.is_control_flow_step() {
                if let Some(condition_type) = &step.condition_type {
                    match condition_type {
                        ConditionType::If => {
                            if_count += 1;
                            if let Some(block_id) = &step.control_flow_block {
                                current_block_stack.push(block_id.clone());
                            }
                        },
                        ConditionType::ElseIf => {
                            if current_block_stack.is_empty() {
                                return Err(CoreError::Validation(
                                    "elseIf found without matching if".to_string()
                                ));
                            }
                        },
                        ConditionType::Else => {
                            if current_block_stack.is_empty() {
                                return Err(CoreError::Validation(
                                    "else found without matching if".to_string()
                                ));
                            }
                        },
                        ConditionType::EndIf => {
                            endif_count += 1;
                            if let Some(block_id) = &step.control_flow_block {
                                if let Some(expected_block_id) = current_block_stack.pop() {
                                    if block_id != &expected_block_id {
                                        return Err(CoreError::Validation(
                                            format!("Mismatched endIf for block {}", block_id)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        if if_count != endif_count {
            return Err(CoreError::Validation(
                format!("Unbalanced if/endif: {} if, {} endif", if_count, endif_count)
            ));
        }
        
        if !current_block_stack.is_empty() {
            return Err(CoreError::Validation(
                format!("Unclosed control flow blocks: {:?}", current_block_stack)
            ));
        }
        
        Ok(())
    }
    
    /// Initialize parallel execution groups from workflow definition
    fn initialize_parallel_groups(&mut self, workflow: &WorkflowDefinition) -> CoreResult<()> {
        self.parallel_groups.clear();
        self.running_parallel_groups.clear();

        for step in &workflow.steps {
            if step.is_parallel() {
                let group_id = step.get_parallel_group_id().ok_or_else(|| CoreError::Validation(
                    "Parallel step without a group ID".to_string()
                ))?;

                let group = self.parallel_groups.entry(group_id.clone()).or_insert_with(|| {
                    ParallelStepGroup::new(group_id.clone(), Vec::new())
                });

                group.step_ids.push(step.id.clone());
            }
        }

        log::debug!("Initialized {} parallel groups", self.parallel_groups.len());
        Ok(())
    }

    /// Create condition evaluation context from workflow run
    fn create_condition_context(&mut self, run: &WorkflowRun) -> CoreResult<()> {
        let workflow = self.workflow_definition.as_ref()
            .ok_or_else(|| CoreError::Internal("Workflow definition not found".to_string()))?;
        
        let context = Context::new(
            self.run_id.to_string(),
            self.workflow_id.clone(),
            "condition-eval".to_string(),
            run.payload.clone(),
            run.clone(),
            self.completed_steps.clone(),
        )?;
        
        self.condition_context = Some(context);
        Ok(())
    }
    
    /// Get steps that are ready for execution
    pub fn get_ready_steps(&self) -> Vec<String> {
        self.step_states
            .iter()
            .filter(|(_, state)| state.ready && state.status == StepStatus::Pending)
            .filter(|(step_id, _)| !self.skipped_steps.contains(*step_id))
            .map(|(step_id, _)| step_id.clone())
            .collect()
    }
    
    /// Evaluate condition for a step
    pub fn evaluate_step_condition(&mut self, step_id: &str) -> CoreResult<ConditionResult> {
        let step_state = self.step_states.get(step_id)
            .ok_or_else(|| CoreError::StepNotFound(format!("Step not found: {}", step_id)))?;
        
        if !step_state.step.requires_condition_evaluation() {
            return Ok(ConditionResult::success(true));
        }
        
        let condition_expr = step_state.step.get_condition_expression()
            .ok_or_else(|| CoreError::Validation(format!("Step {} requires condition but has no expression", step_id)))?;
        
        let context = self.condition_context.as_ref()
            .ok_or_else(|| CoreError::Internal("Condition context not available".to_string()))?;
        
        let evaluator = ConditionEvaluator::new(context.clone(), self.completed_steps.clone());
        evaluator.evaluate_condition(condition_expr)
    }
    
    /// Handle control flow step execution
    pub fn handle_control_flow_step(&mut self, step_id: &str) -> CoreResult<bool> {
        let step_state = self.step_states.get(step_id)
            .ok_or_else(|| CoreError::StepNotFound(format!("Step not found: {}", step_id)))?;
        
        if !step_state.step.is_control_flow_step() {
            return Ok(true); // Not a control flow step, execute normally
        }
        
        // Clone the condition type and block ID to avoid borrow checker issues
        let condition_type = step_state.step.condition_type.clone();
        let block_id = step_state.step.get_control_flow_block_id().cloned();
        
        if let Some(condition_type) = &condition_type {
            match condition_type {
                ConditionType::If => {
                    let condition_result = self.evaluate_step_condition(step_id)?;
                    if condition_result.met {
                        // Condition is true, continue execution
                        if let Some(block_id) = &block_id {
                            if let Some(block) = self.control_flow_blocks.get_mut(block_id) {
                                block.mark_condition_met();
                            }
                        }
                        Ok(true)
                    } else {
                        // Condition is false, skip to else/elseif/endif
                        self.skip_until_control_flow_end(step_id)?;
                        Ok(false)
                    }
                },
                ConditionType::ElseIf => {
                    if let Some(block_id) = &block_id {
                        if let Some(block) = self.control_flow_blocks.get(block_id) {
                            if block.condition_met {
                                // Previous condition was met, skip this elseif
                                self.skip_until_control_flow_end(step_id)?;
                                return Ok(false);
                            }
                        }
                    }
                    
                    // Evaluate this elseif condition
                    let condition_result = self.evaluate_step_condition(step_id)?;
                    if condition_result.met {
                        if let Some(block_id) = &block_id {
                            if let Some(block) = self.control_flow_blocks.get_mut(block_id) {
                                block.mark_condition_met();
                            }
                        }
                        Ok(true)
                    } else {
                        // Continue to next elseif/else/endif
                        Ok(true)
                    }
                },
                ConditionType::Else => {
                    if let Some(block_id) = &block_id {
                        if let Some(block) = self.control_flow_blocks.get(block_id) {
                            if block.condition_met {
                                // Previous condition was met, skip else
                                self.skip_until_control_flow_end(step_id)?;
                                return Ok(false);
                            }
                        }
                    }
                    
                    // No previous condition was met, execute else
                    Ok(true)
                },
                ConditionType::EndIf => {
                    // End of control flow block, always execute
                    Ok(true)
                }
            }
        } else {
            Ok(true)
        }
    }
    
    /// Skip steps until the end of the current control flow block
    fn skip_until_control_flow_end(&mut self, current_step_id: &str) -> CoreResult<()> {
        let workflow = self.workflow_definition.as_ref()
            .ok_or_else(|| CoreError::Internal("Workflow definition not found".to_string()))?;
        
        let current_step = workflow.get_step(current_step_id)
            .ok_or_else(|| CoreError::StepNotFound(format!("Step not found: {}", current_step_id)))?;
        
        let block_id = current_step.get_control_flow_block_id()
            .ok_or_else(|| CoreError::Validation("Control flow step without block ID".to_string()))?;
        
        for step in &workflow.steps {
            if let Some(step_block_id) = step.get_control_flow_block_id() {
                if step_block_id == block_id {
                    self.skipped_steps.insert(step.id.clone());
                }
            }
        }
        
        Ok(())
    }
    
    /// Update control flow state after step completion
    pub fn update_control_flow_state(&mut self, step_id: &str) -> CoreResult<()> {
        let step_state = self.step_states.get(step_id)
            .ok_or_else(|| CoreError::StepNotFound(format!("Step not found: {}", step_id)))?;
        
        if step_state.step.is_control_flow_step() {
            if let Some(block_id) = step_state.step.get_control_flow_block_id() {
                if let Some(block) = self.control_flow_blocks.get_mut(block_id) {
                    block.mark_executed();
                }
            }
        }
        
        Ok(())
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
            
            self.update_control_flow_state(step_id)?;
            
            self.update_dependencies(step_id);
            
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

    /// Execute completion hooks (onSuccess or onFailure)
    pub fn execute_completion_hooks(&self, context: &crate::models::WorkflowCompletionContext) -> CoreResult<()> {
        log::info!("Executing completion hooks for workflow: {} run: {}", context.workflow_id, context.run_id);
        
        // Determine which hook to execute based on final status
        let hook_type = if context.is_success() {
            "onSuccess"
        } else {
            "onFailure"
        };
        
        log::info!("Executing {} hook for workflow: {}", hook_type, context.workflow_id);
        
        // In the next phase, this will call the N-API function to execute hooks in Bun.js
        match hook_type {
            "onSuccess" => {
                log::info!("✅ Workflow {} completed successfully in {}ms", 
                    context.workflow_id, 
                    context.duration_ms.unwrap_or(0)
                );
                log::info!("   - Completed steps: {}", context.completed_step_count());
                log::info!("   - Final output: {:?}", context.final_output);
            },
            "onFailure" => {
                log::error!("❌ Workflow {} failed after {}ms", 
                    context.workflow_id, 
                    context.duration_ms.unwrap_or(0)
                );
                log::error!("   - Completed steps: {}", context.completed_step_count());
                log::error!("   - Failed steps: {}", context.failed_step_count());
                if let Some(error) = &context.error {
                    log::error!("   - Error: {}", error);
                }
            },
            _ => {
                log::warn!("Unknown hook type: {}", hook_type);
            }
        }
        
        Ok(())
    }
    
    /// Create completion context for hook execution
    pub fn create_completion_context(&self, final_status: RunStatus, error_message: Option<String>) -> CoreResult<crate::models::WorkflowCompletionContext> {
        let workflow = self.workflow_definition.as_ref()
            .ok_or_else(|| CoreError::Internal("Workflow definition not found".to_string()))?;
        
        let run = self.workflow_run.as_ref()
            .ok_or_else(|| CoreError::Internal("Workflow run not found".to_string()))?;
        
        let completed_at = chrono::Utc::now();
        
        let context = crate::models::WorkflowCompletionContext::new(
            self.run_id.to_string(),
            self.workflow_id.clone(),
            final_status,
            self.completed_steps.clone(),
            error_message,
            run.started_at,
            completed_at,
            run.payload.clone(),
        );
        
        Ok(context)
    }
    
    /// Finalize workflow completion with hooks and cleanup
    pub fn finalize_completion(&mut self, error_message: Option<String>) -> CoreResult<()> {
        log::info!("Finalizing workflow completion for: {} run: {}", self.workflow_id, self.run_id);
        
        // Determine final status
        let final_status = if self.stats.failed_steps > 0 {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        };
        
        self.execution_state = match final_status {
            RunStatus::Completed => WorkflowExecutionState::Completed,
            RunStatus::Failed => WorkflowExecutionState::Failed,
            _ => WorkflowExecutionState::Failed, // Default to failed for unexpected status
        };
        
        // Mark stats as completed
        self.stats.mark_completed();
        
        let completion_context = self.create_completion_context(final_status.clone(), error_message.clone())?;
        
        // Execute hooks
        self.execute_completion_hooks(&completion_context)?;
        
        // Save final state to database
        self.save_state()?;
        
        log::info!("Workflow {} finalized with status: {:?}", self.workflow_id, final_status);
        Ok(())
    }

    /// Check if this step is a control flow boundary
    pub fn is_control_flow_boundary(&self, step_id: &str) -> bool {
        if let Some(step_state) = self.step_states.get(step_id) {
            step_state.step.is_control_flow_boundary()
        } else {
            false
        }
    }
    
    /// Detect parallel step groups in the workflow
    pub fn detect_parallel_groups(&self) -> Vec<ParallelStepGroup> {
        let mut groups = Vec::new();
        let mut current_group: Option<ParallelStepGroup> = None;
        
        if let Some(workflow) = &self.workflow_definition {
            for step in &workflow.steps {
                if step.is_parallel() {
                    // Start or continue a parallel group
                    if let Some(ref mut group) = current_group {
                        group.step_ids.push(step.id.clone());
                    } else {
                        // Start a new parallel group
                        let group_id = format!("parallel_group_{}", step.id);
                        let mut group = ParallelStepGroup::new(group_id, vec![step.id.clone()]);
                        group.fail_fast = self.parallel_config.fail_fast;
                        group.timeout_ms = self.parallel_config.default_timeout_ms;
                        current_group = Some(group);
                    }
                } else {
                    // End current parallel group if exists
                    if let Some(group) = current_group.take() {
                        groups.push(group);
                    }
                }
            }
            
            // Don't forget the last group
            if let Some(group) = current_group {
                groups.push(group);
            }
        }
        
        groups
    }
    
    /// Execute a parallel step group
    pub fn execute_parallel_group(&mut self, group: &ParallelStepGroup) -> CoreResult<Vec<StepResult>> {
        log::info!("Executing parallel group: {} with {} steps", group.group_id, group.step_ids.len());
        
        // Mark group as running
        let mut group = group.clone();
        group.mark_running();
        
        // Store the group in our tracking
        self.parallel_groups.insert(group.group_id.clone(), group.clone());
        self.running_parallel_groups.insert(group.group_id.clone());
        
        // In a real implementation, this would use the job dispatcher for concurrent execution
        let mut results = Vec::new();
        
        for step_id in &group.step_ids {
            // Simulate step execution first (before any mutable borrows)
            let result = self.simulate_parallel_step_execution(step_id)?;
            let result_clone = result.clone();
            results.push(result);
            
            if let Some(step_state) = self.step_states.get_mut(step_id) {
                // Mark step as running
                step_state.mark_running();
                
                // Mark step as completed
                step_state.mark_completed(result_clone.clone());
            }
            
            if let Some(group) = self.parallel_groups.get_mut(&group.group_id) {
                group.add_step_result(step_id.clone(), result_clone);
            }
        }
        
        // Mark group as completed
        if let Some(group) = self.parallel_groups.get_mut(&group.group_id) {
            if group.has_failures() {
                group.mark_partially_failed("Some steps in parallel group failed".to_string());
            } else {
                group.mark_completed();
            }
        }
        
        self.running_parallel_groups.remove(&group.group_id);
        
        log::info!("Parallel group {} completed with {} results", group.group_id, results.len());
        Ok(results)
    }
    
    /// Aggregate results from parallel steps
    pub fn aggregate_parallel_results(&self, results: Vec<StepResult>) -> CoreResult<serde_json::Value> {
        let mut aggregated = serde_json::Map::new();
        let mut success_count = 0;
        let mut failure_count = 0;
        
        for result in results {
            let step_id = result.step_id.clone();
            
            if matches!(result.status, StepStatus::Completed) {
                success_count += 1;
                if let Some(output) = result.output {
                    aggregated.insert(step_id, output);
                }
            } else {
                failure_count += 1;
                if let Some(error) = result.error {
                    aggregated.insert(format!("{}_error", step_id), serde_json::Value::String(error));
                }
            }
        }
        
        aggregated.insert("success_count".to_string(), serde_json::Value::Number(success_count.into()));
        aggregated.insert("failure_count".to_string(), serde_json::Value::Number(failure_count.into()));
        aggregated.insert("total_count".to_string(), serde_json::Value::Number((success_count + failure_count).into()));
        
        Ok(serde_json::Value::Object(aggregated))
    }
    
    /// Handle parallel execution failures
    pub fn handle_parallel_failures(&mut self, group: &ParallelStepGroup, failures: Vec<String>) -> CoreResult<()> {
        log::warn!("Handling {} failures in parallel group: {}", failures.len(), group.group_id);
        
        if self.parallel_config.fail_fast {
            // Fail fast - mark the entire group as failed
            if let Some(group) = self.parallel_groups.get_mut(&group.group_id) {
                group.mark_failed(format!("Parallel group failed: {}", failures.join(", ")));
            }
            
            // Mark all steps in the group as failed
            for step_id in &group.step_ids {
                if let Some(step_state) = self.step_states.get_mut(step_id) {
                    step_state.mark_failed("Parallel group failed".to_string());
                }
            }
        } else {
            // Continue on error - mark only failed steps as failed
            if let Some(group) = self.parallel_groups.get_mut(&group.group_id) {
                group.mark_partially_failed(format!("Some steps failed: {}", failures.join(", ")));
            }
        }
        
        Ok(())
    }
    
    /// Simulate parallel step execution (placeholder for actual execution)
    fn simulate_parallel_step_execution(&self, step_id: &str) -> CoreResult<StepResult> {
        log::debug!("Simulating parallel step execution: {}", step_id);
        
        let start_time = Utc::now();
        
        // Simulate some processing time
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        let end_time = Utc::now();
        let duration_ms = (end_time - start_time).num_milliseconds() as u64;
        
        let output = serde_json::json!({
            "step_id": step_id,
            "status": "completed",
            "message": "Parallel step executed successfully (simulated)",
            "timestamp": end_time.to_rfc3339(),
            "duration_ms": duration_ms,
        });
        
        Ok(StepResult {
            step_id: step_id.to_string(),
            status: StepStatus::Completed,
            output: Some(output),
            error: None,
            started_at: start_time,
            completed_at: Some(end_time),
            duration_ms: Some(duration_ms),
        })
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
            control_flow_blocks: vec![
                ControlFlowBlock {
                    id: "block-1".to_string(),
                    conditions: vec![
                        ConditionType::If(ConditionResult::True),
                    ],
                },
                ControlFlowBlock {
                    id: "block-2".to_string(),
                    conditions: vec![
                        ConditionType::If(ConditionResult::False),
                    ],
                },
            ],
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