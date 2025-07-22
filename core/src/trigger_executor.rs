//! Trigger execution system for the Node-Cronflow Core Engine
//! 
//! This module handles the execution of triggers and their connection to workflow runs.

use crate::error::{CoreError, CoreResult};
use crate::state::StateManager;
use crate::triggers::{TriggerManager, WebhookRequest, ScheduleTrigger};
use crate::models::WorkflowDefinition;
use crate::step_orchestrator::StepOrchestrator;
use crate::dispatcher::Dispatcher;
use crate::job::Job;
use chrono::Utc;
use log;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use serde::Serialize;

/// Trigger execution result
#[derive(Debug, Clone, Serialize)]
pub struct TriggerExecutionResult {
    pub success: bool,
    pub run_id: Option<Uuid>,
    pub workflow_id: Option<String>,
    pub message: String,
}

impl TriggerExecutionResult {
    /// Create a successful execution result
    pub fn success(run_id: Uuid, workflow_id: String) -> Self {
        Self {
            success: true,
            run_id: Some(run_id),
            workflow_id: Some(workflow_id),
            message: format!("Trigger executed successfully, created run: {}", run_id),
        }
    }

    /// Create a failed execution result
    pub fn failure(message: String) -> Self {
        Self {
            success: false,
            run_id: None,
            workflow_id: None,
            message,
        }
    }
}

/// Trigger executor for handling trigger-to-workflow connections
pub struct TriggerExecutor {
    state_manager: Arc<Mutex<StateManager>>,
    trigger_manager: Arc<Mutex<TriggerManager>>,
    step_orchestrator: StepOrchestrator,
    job_dispatcher: Arc<Mutex<Dispatcher>>,
}

impl TriggerExecutor {
    /// Create a new trigger executor
    pub fn new(
        state_manager: Arc<Mutex<StateManager>>, 
        trigger_manager: Arc<Mutex<TriggerManager>>,
        job_dispatcher: Arc<Mutex<Dispatcher>>
    ) -> Self {
        let step_orchestrator = StepOrchestrator::new(state_manager.clone());
        Self {
            state_manager,
            trigger_manager,
            step_orchestrator,
            job_dispatcher,
        }
    }

    /// Execute a webhook trigger
    pub fn execute_webhook_trigger(&self, request: WebhookRequest) -> CoreResult<TriggerExecutionResult> {
        log::info!("Executing webhook trigger: {} {}", request.method, request.path);
        
        // Get trigger manager
        let trigger_manager = self.trigger_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
        
        // Handle webhook request
        let (workflow_id, payload) = trigger_manager.handle_webhook_request(request)?;
        
        // Execute the workflow
        let result = self.execute_workflow(&workflow_id, payload)?;
        
        log::info!("Webhook trigger executed successfully for workflow: {}", workflow_id);
        Ok(result)
    }

    /// Execute a schedule trigger
    pub fn execute_schedule_trigger(&self, trigger_id: &str) -> CoreResult<TriggerExecutionResult> {
        log::info!("Executing schedule trigger: {}", trigger_id);
        
        // Get trigger manager
        let trigger_manager = self.trigger_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
        
        // Get workflow ID for this trigger
        let workflow_id = trigger_manager.get_workflow_id_for_schedule(trigger_id)
            .ok_or_else(|| CoreError::TriggerNotFound(format!("Schedule trigger not found: {}", trigger_id)))?
            .clone();
        
        // Create payload for scheduled execution
        let payload = serde_json::json!({
            "trigger_type": "schedule",
            "trigger_id": trigger_id,
            "scheduled_at": Utc::now().to_rfc3339(),
        });
        
        // Execute the workflow
        let result = self.execute_workflow(&workflow_id, payload)?;
        
        log::info!("Schedule trigger executed successfully for workflow: {}", workflow_id);
        Ok(result)
    }

    /// Execute a manual trigger
    pub fn execute_manual_trigger(&self, workflow_id: &str, payload: serde_json::Value) -> CoreResult<TriggerExecutionResult> {
        log::info!("Executing manual trigger for workflow: {}", workflow_id);
        
        // Execute the workflow
        let result = self.execute_workflow(workflow_id, payload)?;
        
        log::info!("Manual trigger executed successfully for workflow: {}", workflow_id);
        Ok(result)
    }

    /// Execute a workflow run
    fn execute_workflow(&self, workflow_id: &str, payload: serde_json::Value) -> CoreResult<TriggerExecutionResult> {
        // Get state manager
        let mut state_manager = self.state_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
        
        // Verify workflow exists
        let workflow = state_manager.get_workflow(workflow_id)?
            .ok_or_else(|| CoreError::WorkflowNotFound(format!("Workflow not found: {}", workflow_id)))?;
        
        // Validate workflow
        workflow.validate()
            .map_err(|e| CoreError::InvalidWorkflow(e))?;
        
        // Create workflow run
        let run_id = state_manager.create_run(workflow_id, payload.clone())?;
        
        log::info!("Created workflow run: {} for workflow: {}", run_id, workflow_id);
        
        // Create and submit jobs for workflow steps
        match self.create_and_submit_jobs(&workflow, &run_id, &payload) {
            Ok(job_count) => {
                log::info!("Successfully submitted {} jobs for workflow run: {}", job_count, run_id);
            }
            Err(error) => {
                log::error!("Failed to submit jobs for workflow run {}: {}", run_id, error);
                // Note: We still return success for the trigger execution
                // The job submission failure will be handled separately
            }
        }
        
        Ok(TriggerExecutionResult::success(run_id, workflow_id.to_string()))
    }

    /// Create and submit jobs for workflow steps
    fn create_and_submit_jobs(&self, workflow: &WorkflowDefinition, run_id: &Uuid, payload: &serde_json::Value) -> CoreResult<usize> {
        log::info!("Creating jobs for workflow: {} run: {}", workflow.id, run_id);
        
        // Create a workflow run object for job creation
        let run = crate::models::WorkflowRun {
            id: *run_id,
            workflow_id: workflow.id.clone(),
            status: crate::models::RunStatus::Running,
            payload: payload.clone(),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };
        
        // Create all jobs for the workflow using the new method
        let jobs = Job::create_workflow_jobs(workflow, &run, payload.clone())?;
        
        // Submit all jobs to the dispatcher
        let dispatcher = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
        
        let job_count = jobs.len();
        for job in jobs {
            let job_id = job.id.clone();
            let step_name = job.step_name.clone();
            dispatcher.submit_job(job)?;
            log::debug!("Submitted job: {} for step: {}", job_id, step_name);
        }
        
        log::info!("Successfully created and submitted {} jobs for workflow: {}", job_count, workflow.id);
        Ok(job_count)
    }

    /// Get all active triggers for a workflow
    pub fn get_workflow_triggers(&self, workflow_id: &str) -> CoreResult<Vec<String>> {
        let trigger_manager = self.trigger_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
        
        let mut triggers = Vec::new();
        
        // Get webhook triggers
        for (path, (_, wf_id)) in &trigger_manager.webhook_triggers {
            if wf_id == workflow_id {
                triggers.push(format!("webhook:{}", path));
            }
        }
        
        // Get schedule triggers
        for (trigger_id, (_, wf_id)) in &trigger_manager.schedule_triggers {
            if wf_id == workflow_id {
                triggers.push(format!("schedule:{}", trigger_id));
            }
        }
        
        Ok(triggers)
    }

    /// Register triggers for a workflow
    pub fn register_workflow_triggers(&self, workflow_id: &str, workflow: &WorkflowDefinition) -> CoreResult<Vec<String>> {
        log::info!("Registering triggers for workflow: {}", workflow_id);
        
        let mut trigger_ids = Vec::new();
        let mut trigger_manager = self.trigger_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
        
        for trigger_def in &workflow.triggers {
            match trigger_def {
                crate::models::TriggerDefinition::Webhook { path, method } => {
                    // Create webhook trigger
                    let webhook_trigger = crate::triggers::WebhookTrigger::new(path.clone(), method.clone());
                    
                    // Register webhook trigger
                    trigger_manager.register_webhook_trigger(workflow_id, webhook_trigger)?;
                    trigger_ids.push(format!("webhook:{}", path));
                    
                    log::info!("Registered webhook trigger: {} {} for workflow: {}", method, path, workflow_id);
                }
                
                crate::models::TriggerDefinition::Schedule { cron_expression } => {
                    // Create schedule trigger
                    let schedule_trigger = ScheduleTrigger::new(cron_expression.clone());
                    
                    // Register schedule trigger
                    let trigger_id = trigger_manager.register_schedule_trigger(workflow_id, schedule_trigger)?;
                    trigger_ids.push(format!("schedule:{}", trigger_id));
                    
                    log::info!("Registered schedule trigger: {} for workflow: {}", cron_expression, workflow_id);
                }
                
                crate::models::TriggerDefinition::Manual => {
                    // Manual triggers don't need registration
                    trigger_ids.push("manual".to_string());
                    log::info!("Registered manual trigger for workflow: {}", workflow_id);
                }
            }
        }
        
        log::info!("Successfully registered {} triggers for workflow: {}", trigger_ids.len(), workflow_id);
        Ok(trigger_ids)
    }

    /// Unregister all triggers for a workflow
    pub fn unregister_workflow_triggers(&self, workflow_id: &str) -> CoreResult<()> {
        log::info!("Unregistering triggers for workflow: {}", workflow_id);
        
        let mut trigger_manager = self.trigger_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
        
        // Remove webhook triggers
        let webhook_paths: Vec<String> = trigger_manager.webhook_triggers
            .iter()
            .filter(|(_, (_, wf_id))| wf_id == workflow_id)
            .map(|(path, _)| path.clone())
            .collect();
        
        for path in webhook_paths {
            trigger_manager.webhook_triggers.remove(&path);
            log::info!("Removed webhook trigger: {} for workflow: {}", path, workflow_id);
        }
        
        // Remove schedule triggers
        let schedule_ids: Vec<String> = trigger_manager.schedule_triggers
            .iter()
            .filter(|(_, (_, wf_id))| wf_id == workflow_id)
            .map(|(trigger_id, _)| trigger_id.clone())
            .collect();
        
        for trigger_id in schedule_ids {
            trigger_manager.remove_schedule_trigger(&trigger_id)?;
            log::info!("Removed schedule trigger: {} for workflow: {}", trigger_id, workflow_id);
        }
        
        log::info!("Successfully unregistered all triggers for workflow: {}", workflow_id);
        Ok(())
    }

    /// Get trigger statistics
    pub fn get_trigger_stats(&self) -> CoreResult<TriggerStats> {
        let trigger_manager = self.trigger_manager.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
        
        let webhook_count = trigger_manager.webhook_triggers.len();
        let schedule_count = trigger_manager.schedule_triggers.len();
        let total_triggers = webhook_count + schedule_count;
        
        Ok(TriggerStats {
            total_triggers,
            webhook_triggers: webhook_count,
            schedule_triggers: schedule_count,
        })
    }
}

/// Statistics about triggers
#[derive(Debug, Clone, Serialize)]
pub struct TriggerStats {
    pub total_triggers: usize,
    pub webhook_triggers: usize,
    pub schedule_triggers: usize,
}

impl TriggerStats {
    /// Create new trigger stats
    pub fn new() -> Self {
        Self {
            total_triggers: 0,
            webhook_triggers: 0,
            schedule_triggers: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{WorkflowDefinition, TriggerDefinition, StepDefinition};
    use chrono::Utc;

    #[test]
    fn test_trigger_execution_result() {
        let run_id = Uuid::new_v4();
        let workflow_id = "test-workflow".to_string();
        
        let success_result = TriggerExecutionResult::success(run_id, workflow_id.clone());
        assert!(success_result.success);
        assert_eq!(success_result.workflow_id, Some(workflow_id));
        assert_eq!(success_result.run_id, Some(run_id));
        
        let failure_result = TriggerExecutionResult::failure("Test error".to_string());
        assert!(!failure_result.success);
        assert_eq!(failure_result.message, "Test error");
    }

    #[test]
    fn test_trigger_stats() {
        let stats = TriggerStats::new();
        assert_eq!(stats.total_triggers, 0);
        assert_eq!(stats.webhook_triggers, 0);
        assert_eq!(stats.schedule_triggers, 0);
    }
} 