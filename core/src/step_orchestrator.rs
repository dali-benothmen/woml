//! Step execution orchestration for the Node-Cronflow Core Engine
//! 
//! This module handles the orchestration of workflow step execution,
//! including step sequencing, dependency management, and execution flow.

use crate::error::{CoreError, CoreResult};
use crate::state::StateManager;
use crate::models::{WorkflowDefinition, WorkflowRun, StepResult, StepStatus};
use crate::context::Context;
use crate::workflow_state_machine::{WorkflowStateMachine, WorkflowExecutionState};
use chrono::Utc;
use log;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use serde_json;

/// Step execution orchestrator
pub struct StepOrchestrator {
    state_manager: Arc<Mutex<StateManager>>,
}

impl StepOrchestrator {
    /// Create a new step orchestrator
    pub fn new(state_manager: Arc<Mutex<StateManager>>) -> Self {
        Self {
            state_manager,
        }
    }

    /// Start step execution for a workflow run
    pub fn start_step_execution(&self, run_id: &Uuid, workflow_id: &str) -> CoreResult<()> {
        log::info!("Starting step execution for run: {} workflow: {}", run_id, workflow_id);
        
        // Create workflow state machine
        let mut state_machine = WorkflowStateMachine::new(
            self.state_manager.clone(),
            workflow_id.to_string(),
            *run_id,
        );
        
        // Initialize the state machine
        state_machine.initialize()?;
        
        // Execute steps using the state machine
        self.execute_steps_with_state_machine(state_machine)?;
        
        log::info!("Step execution completed for run: {}", run_id);
        Ok(())
    }

    /// Execute steps using the workflow state machine
    fn execute_steps_with_state_machine(&self, mut state_machine: WorkflowStateMachine) -> CoreResult<()> {
        log::info!("Executing steps using state machine for workflow: {}", state_machine.get_workflow_definition().unwrap().id);
        
        // Get workflow definition and run - clone them to avoid borrow checker issues
        let workflow = state_machine.get_workflow_definition()
            .ok_or_else(|| CoreError::Internal("Workflow definition not found in state machine".to_string()))?
            .clone();
        
        let run = state_machine.get_workflow_run()
            .ok_or_else(|| CoreError::Internal("Workflow run not found in state machine".to_string()))?
            .clone();
        
        // Execute steps until completion
        while !state_machine.check_workflow_completion()? {
            // Get ready steps
            let ready_steps = state_machine.get_ready_steps();
            
            if ready_steps.is_empty() {
                log::warn!("No ready steps found, but workflow is not complete");
                break;
            }
            
            // Check for parallel execution groups
            let parallel_groups = state_machine.detect_parallel_groups();
            
            if !parallel_groups.is_empty() {
                // Execute parallel groups
                for group in parallel_groups {
                    log::info!("Executing parallel group: {} with {} steps", group.group_id, group.step_ids.len());
                    
                    // Execute the parallel group
                    let parallel_results = state_machine.execute_parallel_group(&group)?;
                    
                    // Aggregate the results
                    let aggregated_result = state_machine.aggregate_parallel_results(parallel_results)?;
                    
                    // Mark the parallel group as a single completed step
                    let group_step_id = format!("parallel_group_{}", group.group_id);
                    state_machine.mark_step_completed(&group_step_id, aggregated_result)?;
                    
                    log::info!("Parallel group {} completed successfully", group.group_id);
                }
            } else {
                // Execute each ready step sequentially
                for step_id in ready_steps {
                    log::info!("Executing ready step: {}", step_id);
                    
                    // Mark step as running
                    state_machine.mark_step_running(&step_id)?;
                    
                    // Get step definition
                    let step_def = workflow.get_step(&step_id)
                        .ok_or_else(|| CoreError::StepNotFound(format!("Step not found: {}", step_id)))?
                        .clone();
                    
                    // Check if this is a pause step
                    if step_def.is_pause_step() {
                        log::info!("Pause step detected: {}", step_id);
                        
                        // Pause the workflow state machine
                        state_machine.pause()?;
                        
                        // Mark the pause step as completed with pause information
                        let pause_output = serde_json::json!({
                            "paused": true,
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "step_id": step_id,
                            "message": "Workflow paused for manual intervention"
                        });
                        
                        state_machine.mark_step_completed(&step_id, pause_output)?;
                        
                        // Save state to database
                        state_machine.save_state()?;
                        
                        log::info!("Workflow paused at step: {}", step_id);
                        
                        // Return early - workflow is now paused
                        return Ok(());
                    }
                    
                    // Get completed steps for context
                    let completed_steps = state_machine.get_completed_steps().to_vec();
                    
                    // Execute the step using the state machine context
                    match self.execute_step_with_state_machine(&workflow, &run, &step_def, &completed_steps, 0) {
                        Ok(output) => {
                            // Mark step as completed in state machine
                            state_machine.mark_step_completed(&step_id, output)?;
                            log::info!("Step {} completed successfully", step_id);
                        }
                        Err(error) => {
                            // Mark step as failed in state machine
                            state_machine.mark_step_failed(&step_id, error.to_string())?;
                            log::error!("Step {} failed: {}", step_id, error);
                            
                            // Check if we should continue or stop
                            if !step_def.can_retry() {
                                log::error!("Step {} cannot be retried, stopping workflow", step_id);
                                break;
                            }
                        }
                    }
                    
                    // Save state to database
                    state_machine.save_state()?;
                }
            }
        }
        
        // Check final completion and finalize
        let is_complete = state_machine.check_workflow_completion()?;
        if is_complete {
            // Determine error message if any steps failed
            let error_message = if state_machine.get_stats().failed_steps > 0 {
                let failed_steps: Vec<_> = state_machine.get_completed_steps()
                    .iter()
                    .filter(|step| matches!(step.status, crate::models::StepStatus::Failed))
                    .map(|step| format!("{}: {}", step.step_id, step.error.as_deref().unwrap_or("Unknown error")))
                    .collect();
                Some(format!("Workflow failed: {}", failed_steps.join(", ")))
            } else {
                None
            };
            
            // Finalize workflow completion with hooks and cleanup
            state_machine.finalize_completion(error_message)?;
        }
        
        let final_state = state_machine.get_execution_state();
        let stats = state_machine.get_stats();
        
        log::info!("Workflow execution completed with state: {:?}, stats: {:?}", final_state, stats);
        Ok(())
    }

    /// Execute all steps in a workflow (legacy method - kept for backward compatibility)
    fn execute_steps(&self, workflow: WorkflowDefinition, run: WorkflowRun) -> CoreResult<()> {
        log::info!("Executing {} steps for workflow: {}", workflow.steps.len(), workflow.id);
        
        let mut completed_steps: Vec<StepResult> = Vec::new();
        let mut step_index = 0;
        
        for step_def in &workflow.steps {
            log::info!("Executing step: {} ({}/{})", step_def.id, step_index + 1, workflow.steps.len());
            
            // Check if this is a pause step
            if step_def.is_pause_step() {
                log::info!("Pause step detected: {}", step_def.id);
                
                // Create pause step result
                let pause_result = StepResult {
                    step_id: step_def.id.clone(),
                    status: StepStatus::Completed,
                    output: Some(serde_json::json!({
                        "paused": true,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "step_id": step_def.id,
                        "message": "Workflow paused for manual intervention"
                    })),
                    error: None,
                    started_at: Utc::now(),
                    completed_at: Some(Utc::now()),
                    duration_ms: Some(0),
                };
                
                // Save pause step result
                completed_steps.push(pause_result);
                
                // Update run status in database
                self.update_run_status(&run.id, &completed_steps)?;
                
                log::info!("Workflow paused at step: {}", step_def.id);
                
                // Return early - workflow is now paused
                return Ok(());
            }
            
            // Create step result
            let mut step_result = StepResult {
                step_id: step_def.id.clone(),
                status: StepStatus::Running,
                output: None,
                error: None,
                started_at: Utc::now(),
                completed_at: None,
                duration_ms: None,
            };
            
            // Execute the step
            match self.execute_step_with_state_machine(&workflow, &run, step_def, &completed_steps, step_index) {
                Ok(output) => {
                    // Step completed successfully
                    step_result.status = StepStatus::Completed;
                    step_result.output = Some(output);
                    step_result.completed_at = Some(Utc::now());
                    step_result.duration_ms = step_result.completed_at
                        .map(|completed| (completed - step_result.started_at).num_milliseconds() as u64);
                    
                    log::info!("Step {} completed successfully", step_def.id);
                }
                Err(error) => {
                    // Step failed
                    step_result.status = StepStatus::Failed;
                    step_result.error = Some(error.to_string());
                    step_result.completed_at = Some(Utc::now());
                    step_result.duration_ms = step_result.completed_at
                        .map(|completed| (completed - step_result.started_at).num_milliseconds() as u64);
                    
                    log::error!("Step {} failed: {}", step_def.id, error);
                    
                    // For now, we'll continue with the next step
                    // In the future, we might want to implement retry logic or stop execution
                }
            }
            
            // Save step result
            completed_steps.push(step_result.clone());
            
            // Update run status in database
            self.update_run_status(&run.id, &completed_steps)?;
            
            step_index += 1;
        }
        
        // Mark run as completed
        self.complete_run(&run.id, &completed_steps)?;
        
        log::info!("All steps executed for workflow: {}", workflow.id);
        Ok(())
    }

    /// Execute a single step with state machine integration
    fn execute_step_with_state_machine(
        &self,
        workflow: &WorkflowDefinition,
        run: &WorkflowRun,
        step_def: &crate::models::StepDefinition,
        completed_steps: &[StepResult],
        step_index: usize,
    ) -> CoreResult<serde_json::Value> {
        log::debug!("Executing step with state machine: {} for run: {}", step_def.id, run.id);
        
        // Create context for step execution
        let context = self.create_step_context(workflow, run, step_def, completed_steps, step_index)?;
        
        // Convert context to JSON for Bun.js execution
        let context_json = context.to_json()
            .map_err(|e| CoreError::Internal(format!("Failed to serialize context: {}", e)))?;
        
        // Try to execute via Bun.js if available, otherwise simulate
        let output = match self.execute_via_bun(&step_def.id, &context_json, &workflow.id, &run.id.to_string()) {
            Ok(result) => {
                log::info!("Step {} executed via Bun.js", step_def.id);
                result
            }
            Err(error) => {
                log::warn!("Bun.js execution failed for step {}, falling back to simulation: {}", step_def.id, error);
                self.simulate_step_execution(&step_def.id, &context)?
            }
        };
        
        log::info!("Step {} executed successfully", step_def.id);
        Ok(output)
    }

    /// Execute step via Bun.js (placeholder for N-API integration)
    fn execute_via_bun(&self, step_name: &str, context_json: &str, workflow_id: &str, run_id: &str) -> CoreResult<serde_json::Value> {
        log::debug!("Attempting Bun.js execution for step: {}", step_name);
        
        // In a real implementation, this would:
        // 1. Call the N-API function to trigger Bun.js step execution
        // 2. Wait for the Bun.js step execution to complete
        // 3. Return the actual step result
        
        // For now, we'll simulate a successful Bun.js execution
        // This simulates what would happen when the Bun.js step handler executes
        let simulated_result = serde_json::json!({
            "step_name": step_name,
            "workflow_id": workflow_id,
            "run_id": run_id,
            "status": "completed",
            "output": {
                "message": format!("Step {} executed successfully in Bun.js", step_name),
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "execution_type": "bun_step_handler",
                "step_id": step_name,
                "workflow_id": workflow_id,
                "run_id": run_id,
                "context_data": {
                    "step_name": step_name,
                    "workflow_id": workflow_id,
                    "run_id": run_id,
                }
            },
            "duration_ms": 150, // Simulated execution time
            "message": "Step executed successfully in Bun.js"
        });
        
        log::info!("Step {} executed successfully via Bun.js simulation", step_name);
        Ok(simulated_result)
    }

    /// Create context for step execution
    fn create_step_context(
        &self,
        workflow: &WorkflowDefinition,
        run: &WorkflowRun,
        step_def: &crate::models::StepDefinition,
        completed_steps: &[StepResult],
        step_index: usize,
    ) -> CoreResult<Context> {
        // Convert completed steps to HashMap
        let mut steps_map = std::collections::HashMap::new();
        for step_result in completed_steps {
            steps_map.insert(step_result.step_id.clone(), step_result.clone());
        }
        
        // Create context
        let context = Context::new(
            run.id.to_string(),
            workflow.id.clone(),
            step_def.id.clone(),
            run.payload.clone(),
            run.clone(),
            completed_steps.to_vec(),
        )?;
        
        // Update metadata
        let mut context = context;
        context.metadata.step_index = step_index;
        context.metadata.total_steps = workflow.steps.len();
        
        Ok(context)
    }

    /// Simulate step execution (placeholder for Bun.js integration)
    fn simulate_step_execution(&self, step_id: &str, context: &Context) -> CoreResult<serde_json::Value> {
        log::debug!("Simulating step execution: {}", step_id);
        
        // For now, return a simple success result
        // In Task 1.5, this will call the Bun.js step execution
        let output = serde_json::json!({
            "step_id": step_id,
            "status": "completed",
            "message": "Step executed successfully (simulated)",
            "timestamp": Utc::now().to_rfc3339(),
            "context": {
                "run_id": context.run_id,
                "workflow_id": context.workflow_id,
                "step_name": context.step_name,
            }
        });
        
        Ok(output)
    }

    /// Update run status with completed steps
    fn update_run_status(&self, run_id: &Uuid, completed_steps: &[StepResult]) -> CoreResult<()> {
        let mut state_manager = self.state_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
        
        // Update run with step results
        state_manager.update_run_with_steps(run_id, completed_steps)?;
        
        Ok(())
    }

    /// Mark run as completed
    fn complete_run(&self, run_id: &Uuid, completed_steps: &[StepResult]) -> CoreResult<()> {
        let mut state_manager = self.state_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
        
        // Check if any steps failed
        let has_failures = completed_steps.iter().any(|step| step.status == StepStatus::Failed);
        
        // Update run status
        let status = if has_failures {
            crate::models::RunStatus::Failed
        } else {
            crate::models::RunStatus::Completed
        };
        
        state_manager.complete_run(run_id, status.clone(), None)?;
        
        log::info!("Run {} marked as {:?}", run_id, status);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{WorkflowDefinition, StepDefinition, TriggerDefinition, RunStatus};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_step_orchestrator_creation() {
        let state_manager = Arc::new(Mutex::new(crate::state::StateManager::new(":memory:").unwrap()));
        let orchestrator = StepOrchestrator::new(state_manager);
        assert!(orchestrator.state_manager.lock().is_ok());
    }

    #[test]
    fn test_context_creation() {
        let state_manager = Arc::new(Mutex::new(crate::state::StateManager::new(":memory:").unwrap()));
        let orchestrator = StepOrchestrator::new(state_manager);
        
        let workflow = WorkflowDefinition {
            id: "test-workflow".to_string(),
            name: "Test Workflow".to_string(),
            description: None,
            steps: vec![
                StepDefinition {
                    id: "step-1".to_string(),
                    name: "Step 1".to_string(),
                    action: "test_action".to_string(),
                    timeout: None,
                    retry: None,
                    depends_on: vec![],
                }
            ],
            triggers: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "test-workflow".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({"test": "data"}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };
        
        let step_def = &workflow.steps[0];
        let completed_steps = vec![];
        
        let context = orchestrator.create_step_context(&workflow, &run, step_def, &completed_steps, 0);
        assert!(context.is_ok());
        
        let context = context.unwrap();
        assert_eq!(context.run_id, run.id.to_string());
        assert_eq!(context.workflow_id, workflow.id);
        assert_eq!(context.step_name, step_def.id);
        assert_eq!(context.metadata.step_index, 0);
        assert_eq!(context.metadata.total_steps, 1);
    }
} 