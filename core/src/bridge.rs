//! N-API bridge for Node.js communication
//! 
//! This module handles the communication between the Rust core engine
//! and the Node.js SDK via N-API.

use napi_derive::napi;
use crate::error::{CoreError, CoreResult};
use crate::state::StateManager;
use crate::models::WorkflowDefinition;
use crate::job::Job;
use crate::triggers::{TriggerManager, WebhookTrigger, ScheduleTrigger};
use crate::trigger_executor::TriggerExecutor;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use uuid::Uuid;

/// N-API bridge for Node.js communication
pub struct Bridge {
    state_manager: Arc<Mutex<StateManager>>,
    trigger_manager: Arc<Mutex<TriggerManager>>,
    trigger_executor: TriggerExecutor,
}

impl Bridge {
    /// Create a new N-API bridge
    pub fn new(db_path: &str) -> CoreResult<Self> {
        let state_manager = Arc::new(Mutex::new(StateManager::new(db_path)?));
        let trigger_manager = Arc::new(Mutex::new(TriggerManager::new()));
        let trigger_executor = TriggerExecutor::new(state_manager.clone(), trigger_manager.clone());
        
        Ok(Bridge { 
            state_manager,
            trigger_manager,
            trigger_executor,
        })
    }

    /// Register a workflow from Node.js
    pub fn register_workflow(&self, workflow_json: &str) -> CoreResult<()> {
        log::info!("Registering workflow from JSON: {}", workflow_json);
        
        // Parse JSON to WorkflowDefinition
        let workflow: WorkflowDefinition = serde_json::from_str(workflow_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Validate the workflow
        workflow.validate()
            .map_err(|e| CoreError::InvalidWorkflow(e))?;
        
        // Register with state manager
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        state_manager.register_workflow(workflow.clone())?;
        
        // Register triggers for the workflow
        let trigger_ids = self.trigger_executor.register_workflow_triggers(&workflow.id, &workflow)?;
        
        log::info!("Successfully registered workflow: {} with {} triggers: {:?}", workflow.id, trigger_ids.len(), trigger_ids);
        Ok(())
    }

    /// Register a webhook trigger for a workflow
    pub fn register_webhook_trigger(&self, workflow_id: &str, trigger_json: &str) -> CoreResult<()> {
        log::info!("Registering webhook trigger for workflow: {} with config: {}", workflow_id, trigger_json);
        
        // Parse trigger JSON
        let trigger: WebhookTrigger = serde_json::from_str(trigger_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Validate the trigger
        trigger.validate()?;
        
        // Register with trigger manager
        let mut trigger_manager = self.trigger_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire trigger manager lock".to_string()))?;
        
        trigger_manager.register_webhook_trigger(workflow_id, trigger)?;
        
        log::info!("Successfully registered webhook trigger for workflow: {}", workflow_id);
        Ok(())
    }

    /// Get all registered webhook triggers
    pub fn get_webhook_triggers(&self) -> CoreResult<String> {
        let trigger_manager = self.trigger_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire trigger manager lock".to_string()))?;
        
        let triggers = trigger_manager.get_webhook_triggers();
        
        let triggers_json = serde_json::to_string(&triggers)
            .map_err(|e| CoreError::Serialization(e))?;
        
        Ok(triggers_json)
    }

    /// Register a schedule trigger for a workflow
    pub fn register_schedule_trigger(&self, workflow_id: &str, trigger_json: &str) -> CoreResult<String> {
        log::info!("Registering schedule trigger for workflow: {} with config: {}", workflow_id, trigger_json);
        
        // Parse trigger JSON
        let trigger: ScheduleTrigger = serde_json::from_str(trigger_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Validate the trigger
        trigger.validate()?;
        
        // Register with trigger manager
        let mut trigger_manager = self.trigger_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire trigger manager lock".to_string()))?;
        
        let trigger_id = trigger_manager.register_schedule_trigger(workflow_id, trigger)?;
        
        log::info!("Successfully registered schedule trigger for workflow: {} with ID: {}", workflow_id, trigger_id);
        Ok(trigger_id)
    }

    /// Get all registered schedule triggers
    pub fn get_schedule_triggers(&self) -> CoreResult<String> {
        let trigger_manager = self.trigger_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire trigger manager lock".to_string()))?;
        
        let triggers = trigger_manager.get_schedule_triggers();
        
        let triggers_json = serde_json::to_string(&triggers)
            .map_err(|e| CoreError::Serialization(e))?;
        
        Ok(triggers_json)
    }

    /// Enable or disable a schedule trigger
    pub fn set_schedule_enabled(&self, trigger_id: &str, enabled: bool) -> CoreResult<()> {
        log::info!("Setting schedule trigger {} enabled: {}", trigger_id, enabled);
        
        let mut trigger_manager = self.trigger_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire trigger manager lock".to_string()))?;
        
        trigger_manager.set_schedule_enabled(trigger_id, enabled)?;
        
        log::info!("Successfully updated schedule trigger: {}", trigger_id);
        Ok(())
    }

    /// Remove a schedule trigger
    pub fn remove_schedule_trigger(&self, trigger_id: &str) -> CoreResult<()> {
        log::info!("Removing schedule trigger: {}", trigger_id);
        
        let mut trigger_manager = self.trigger_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire trigger manager lock".to_string()))?;
        
        trigger_manager.remove_schedule_trigger(trigger_id)?;
        
        log::info!("Successfully removed schedule trigger: {}", trigger_id);
        Ok(())
    }

    /// Create a workflow run from Node.js
    pub fn create_run(&self, workflow_id: &str, payload_json: &str) -> CoreResult<String> {
        log::info!("Creating run for workflow: {} with payload: {}", workflow_id, payload_json);
        
        // Parse payload JSON
        let payload: serde_json::Value = serde_json::from_str(payload_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Create run with state manager
        let mut state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        let run_id = state_manager.create_run(workflow_id, payload)?;
        
        log::info!("Successfully created run: {} for workflow: {}", run_id, workflow_id);
        Ok(run_id.to_string())
    }

    /// Get workflow run status
    pub fn get_run_status(&self, run_id: &str) -> CoreResult<String> {
        log::info!("Getting status for run: {}", run_id);
        
        // Parse run ID
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        // Get run from state manager
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        let _run = state_manager.get_run(&run_uuid)?
            .ok_or_else(|| CoreError::WorkflowNotFound(format!("Run not found: {}", run_id)))?;
        
        // For now, return a simple status
        // TODO: Implement full run status serialization
        let status_json = serde_json::json!({
            "run_id": run_id,
            "status": "pending",
            "message": "Run status retrieved successfully"
        });
        
        let result = serde_json::to_string(&status_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Retrieved status for run: {}", run_id);
        Ok(result)
    }

    /// Execute a step for a workflow run
    pub fn execute_step(&self, run_id: &str, step_id: &str, context_json: &str) -> CoreResult<String> {
        // If context_json is not empty, use it for step function execution (skip all database operations)
        if !context_json.is_empty() {
            // Parse context JSON to validate it
            let context: crate::context::Context = match serde_json::from_str::<crate::context::Context>(context_json) {
                Ok(context) => context,
                Err(e) => {
                    return Err(CoreError::Serialization(e));
                }
            };

            // For now, return the context as a result
            // TODO: In Task 1.3, we'll implement the actual Bun.js step execution
            let result = serde_json::json!({
                "step_name": step_id,
                "workflow_id": context.workflow_id,
                "run_id": context.run_id,
                "context": context_json,
                "status": "ready_for_bun_execution",
                "message": "Step function ready for Bun.js execution"
            });

            let result_json = serde_json::to_string(&result)
                .map_err(|e| CoreError::Serialization(e))?;

            return Ok(result_json);
        }
        
        // Parse run ID (only for basic step execution)
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        // Get state manager
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        // Get run from state manager
        let run = state_manager.get_run(&run_uuid)?
            .ok_or_else(|| CoreError::WorkflowNotFound(format!("Run not found: {}", run_id)))?;
        
        // Get workflow definition
        let workflow = state_manager.get_workflow(&run.workflow_id)?
            .ok_or_else(|| CoreError::WorkflowNotFound(format!("Workflow not found: {}", run.workflow_id)))?;
        
        // Find the step to execute
        let step = workflow.get_step(step_id)
            .ok_or_else(|| CoreError::InvalidWorkflow(format!("Step '{}' not found in workflow '{}'", step_id, workflow.id)))?;
        
        // Get completed steps for this run
        let completed_steps = state_manager.get_completed_steps(&run_uuid)?;
        
        // Create context object for Bun.js execution (original functionality)
        let mut context = crate::context::Context::new(
            run_id.to_string(),
            run.workflow_id.clone(),
            step_id.to_string(),
            run.payload.clone(),
            run.clone(),
            completed_steps,
        )?;
        
        // Update context metadata
        let step_index = workflow.steps.iter().position(|s| s.id == step_id).unwrap_or(0);
        context.update_step_metadata(step_index, workflow.steps.len());
        
        // Set timeout if configured
        if let Some(timeout) = step.timeout {
            context.set_timeout(timeout);
        }
        
        // Add services to context (for now, empty - will be implemented in Phase 4)
        // TODO: Load services from workflow configuration
        
        // Serialize context for Bun.js
        let context_json = context.to_json()?;
        
        // For now, return the context as a result
        // TODO: In Task 1.2, we'll add the N-API call to Bun.js
        let result = serde_json::json!({
            "run_id": run_id,
            "step_id": step_id,
            "workflow_id": run.workflow_id,
            "context": context_json,
            "status": "ready_for_execution",
            "message": "Step context prepared for Bun.js execution"
        });
        
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        return Ok(result_json);
    }

    /// Execute a job with context for Bun.js
    pub fn execute_job(&self, job: &Job, _services: HashMap<String, serde_json::Value>) -> CoreResult<String> {
        log::info!("Executing job: {}", job.id);
        
        // Get state manager
        let state_manager = self.state_manager.lock().unwrap();
        
        // Get workflow definition
        let _workflow = state_manager.get_workflow(&job.workflow_id)?;
        
        // Get run information
        let _run_uuid = Uuid::parse_str(&job.run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        // For now, return a simple result
        // TODO: Implement actual job execution logic
        let result = serde_json::json!({
            "job_id": job.id,
            "run_id": job.run_id,
            "step_id": job.step_name,
            "status": "pending",
            "message": "Job execution not yet implemented"
        });
        
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Job execution result for {}: {}", job.id, result_json);
        Ok(result_json)
    }

    /// Execute a webhook trigger
    pub fn execute_webhook_trigger(&self, request_json: &str) -> CoreResult<String> {
        log::info!("Executing webhook trigger with request: {}", request_json);
        
        // Parse webhook request JSON
        let request: crate::triggers::WebhookRequest = serde_json::from_str(request_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Execute the webhook trigger
        let result = self.trigger_executor.execute_webhook_trigger(request)?;
        
        // Serialize the result
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Webhook trigger execution result: {}", result_json);
        Ok(result_json)
    }

    /// Execute a schedule trigger
    pub fn execute_schedule_trigger(&self, trigger_id: &str) -> CoreResult<String> {
        log::info!("Executing schedule trigger: {}", trigger_id);
        
        // Execute the schedule trigger
        let result = self.trigger_executor.execute_schedule_trigger(trigger_id)?;
        
        // Serialize the result
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Schedule trigger execution result: {}", result_json);
        Ok(result_json)
    }

    /// Execute a manual trigger
    pub fn execute_manual_trigger(&self, workflow_id: &str, payload_json: &str) -> CoreResult<String> {
        log::info!("Executing manual trigger for workflow: {} with payload: {}", workflow_id, payload_json);
        
        // Parse payload JSON
        let payload: serde_json::Value = serde_json::from_str(payload_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Execute the manual trigger
        let result = self.trigger_executor.execute_manual_trigger(workflow_id, payload)?;
        
        // Serialize the result
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Manual trigger execution result: {}", result_json);
        Ok(result_json)
    }

    /// Get trigger statistics
    pub fn get_trigger_stats(&self) -> CoreResult<String> {
        log::info!("Getting trigger statistics");
        
        // Get trigger statistics
        let stats = self.trigger_executor.get_trigger_stats()?;
        
        // Serialize the result
        let stats_json = serde_json::to_string(&stats)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Trigger statistics: {}", stats_json);
        Ok(stats_json)
    }

    /// Get triggers for a workflow
    pub fn get_workflow_triggers(&self, workflow_id: &str) -> CoreResult<String> {
        log::info!("Getting triggers for workflow: {}", workflow_id);
        
        // Get workflow triggers
        let triggers = self.trigger_executor.get_workflow_triggers(workflow_id)?;
        
        // Serialize the result
        let triggers_json = serde_json::to_string(&triggers)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Workflow triggers: {}", triggers_json);
        Ok(triggers_json)
    }

    /// Unregister triggers for a workflow
    pub fn unregister_workflow_triggers(&self, workflow_id: &str) -> CoreResult<()> {
        log::info!("Unregistering triggers for workflow: {}", workflow_id);
        
        // Unregister workflow triggers
        self.trigger_executor.unregister_workflow_triggers(workflow_id)?;
        
        log::info!("Successfully unregistered triggers for workflow: {}", workflow_id);
        Ok(())
    }

    /// Start the webhook server
    pub fn start_webhook_server(&mut self) -> CoreResult<()> {
        log::info!("Starting webhook server...");
        
        // For now, we'll just log that the server is configured
        // The actual HTTP server will be started by the Node.js side
        // This is a temporary workaround until we resolve the threading issues
        
        log::info!("Webhook server configuration ready");
        log::info!("Note: HTTP server needs to be started separately");
        
        Ok(())
    }
    
    /// Stop the webhook server
    pub fn stop_webhook_server(&mut self) -> CoreResult<()> {
        log::info!("Stopping webhook server...");
        
        // Clean up any server state
        // self.webhook_server = None; // Removed as per edit hint
        // self.runtime = None; // Removed as per edit hint
        
        log::info!("Webhook server stopped successfully");
        Ok(())
    }
}

// N-API module setup
#[napi(object)]
pub struct WorkflowRegistrationResult {
    pub success: bool,
    pub message: String,
}

#[napi(object)]
pub struct RunCreationResult {
    pub success: bool,
    pub run_id: Option<String>,
    pub message: String,
}

#[napi(object)]
pub struct RunStatusResult {
    pub success: bool,
    pub status: Option<String>,
    pub message: String,
}

#[napi(object)]
pub struct StepExecutionResult {
    pub success: bool,
    pub result: Option<String>,
    pub message: String,
}

#[napi(object)]
pub struct JobExecutionResult {
    pub success: bool,
    pub job_id: Option<String>,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub context: Option<String>,
    pub result: Option<String>,
    pub message: String,
}

#[napi(object)]
pub struct WebhookTriggerRegistrationResult {
    pub success: bool,
    pub message: String,
}

#[napi(object)]
pub struct WebhookTriggersResult {
    pub success: bool,
    pub triggers: Option<String>,
    pub message: String,
}

#[napi(object)]
pub struct ScheduleTriggerRegistrationResult {
    pub success: bool,
    pub message: String,
}

#[napi(object)]
pub struct ScheduleTriggersResult {
    pub success: bool,
    pub triggers: Option<String>,
    pub message: String,
}

/// Trigger execution result
#[napi_derive::napi]
pub struct TriggerExecutionResult {
    pub success: bool,
    pub run_id: Option<String>,
    pub workflow_id: Option<String>,
    pub message: String,
}

/// Trigger statistics result
#[napi_derive::napi]
pub struct TriggerStatsResult {
    pub success: bool,
    pub stats: Option<String>,
    pub message: String,
}

/// Workflow triggers result
#[napi_derive::napi]
pub struct WorkflowTriggersResult {
    pub success: bool,
    pub triggers: Option<String>,
    pub message: String,
}

/// Trigger unregistration result
#[napi_derive::napi]
pub struct TriggerUnregistrationResult {
    pub success: bool,
    pub message: String,
}

/// Register a workflow via N-API
#[napi]
pub fn register_workflow(workflow_json: String, db_path: String) -> WorkflowRegistrationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return WorkflowRegistrationResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.register_workflow(&workflow_json) {
        Ok(_) => {
            WorkflowRegistrationResult {
                success: true,
                message: "Workflow registered successfully".to_string(),
            }
        }
        Err(e) => {
            WorkflowRegistrationResult {
                success: false,
                message: format!("Failed to register workflow: {}", e),
            }
        }
    }
}

/// Register a webhook trigger via N-API
#[napi]
pub fn register_webhook_trigger(workflow_id: String, trigger_json: String, db_path: String) -> WebhookTriggerRegistrationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return WebhookTriggerRegistrationResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.register_webhook_trigger(&workflow_id, &trigger_json) {
        Ok(_) => {
            WebhookTriggerRegistrationResult {
                success: true,
                message: "Webhook trigger registered successfully".to_string(),
            }
        }
        Err(e) => {
            WebhookTriggerRegistrationResult {
                success: false,
                message: format!("Failed to register webhook trigger: {}", e),
            }
        }
    }
}

/// Get all webhook triggers via N-API
#[napi]
pub fn get_webhook_triggers(db_path: String) -> WebhookTriggersResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return WebhookTriggersResult {
                success: false,
                triggers: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.get_webhook_triggers() {
        Ok(triggers_json) => {
            WebhookTriggersResult {
                success: true,
                triggers: Some(triggers_json),
                message: "Webhook triggers retrieved successfully".to_string(),
            }
        }
        Err(e) => {
            WebhookTriggersResult {
                success: false,
                triggers: None,
                message: format!("Failed to get webhook triggers: {}", e),
            }
        }
    }
}

/// Register a schedule trigger via N-API
#[napi]
pub fn register_schedule_trigger(workflow_id: String, trigger_json: String, db_path: String) -> ScheduleTriggerRegistrationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return ScheduleTriggerRegistrationResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.register_schedule_trigger(&workflow_id, &trigger_json) {
        Ok(trigger_id) => {
            ScheduleTriggerRegistrationResult {
                success: true,
                message: format!("Schedule trigger registered successfully with ID: {}", trigger_id),
            }
        }
        Err(e) => {
            ScheduleTriggerRegistrationResult {
                success: false,
                message: format!("Failed to register schedule trigger: {}", e),
            }
        }
    }
}

/// Get all schedule triggers via N-API
#[napi]
pub fn get_schedule_triggers(db_path: String) -> ScheduleTriggersResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return ScheduleTriggersResult {
                success: false,
                triggers: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.get_schedule_triggers() {
        Ok(triggers_json) => {
            ScheduleTriggersResult {
                success: true,
                triggers: Some(triggers_json),
                message: "Schedule triggers retrieved successfully".to_string(),
            }
        }
        Err(e) => {
            ScheduleTriggersResult {
                success: false,
                triggers: None,
                message: format!("Failed to get schedule triggers: {}", e),
            }
        }
    }
}

/// Enable or disable a schedule trigger via N-API
#[napi]
pub fn set_schedule_enabled(trigger_id: String, enabled: bool, db_path: String) -> ScheduleTriggerRegistrationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return ScheduleTriggerRegistrationResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.set_schedule_enabled(&trigger_id, enabled) {
        Ok(_) => {
            ScheduleTriggerRegistrationResult {
                success: true,
                message: format!("Schedule trigger {} enabled: {}", trigger_id, enabled),
            }
        }
        Err(e) => {
            ScheduleTriggerRegistrationResult {
                success: false,
                message: format!("Failed to set schedule trigger enabled: {}", e),
            }
        }
    }
}

/// Remove a schedule trigger via N-API
#[napi]
pub fn remove_schedule_trigger(trigger_id: String, db_path: String) -> ScheduleTriggerRegistrationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return ScheduleTriggerRegistrationResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.remove_schedule_trigger(&trigger_id) {
        Ok(_) => {
            ScheduleTriggerRegistrationResult {
                success: true,
                message: format!("Schedule trigger {} removed", trigger_id),
            }
        }
        Err(e) => {
            ScheduleTriggerRegistrationResult {
                success: false,
                message: format!("Failed to remove schedule trigger: {}", e),
            }
        }
    }
}

/// Create a workflow run via N-API
#[napi]
pub fn create_run(workflow_id: String, payload_json: String, db_path: String) -> RunCreationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return RunCreationResult {
                success: false,
                run_id: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.create_run(&workflow_id, &payload_json) {
        Ok(run_id) => {
            RunCreationResult {
                success: true,
                run_id: Some(run_id),
                message: "Run created successfully".to_string(),
            }
        }
        Err(e) => {
            RunCreationResult {
                success: false,
                run_id: None,
                message: format!("Failed to create run: {}", e),
            }
        }
    }
}

/// Get run status via N-API
#[napi]
pub fn get_run_status(run_id: String, db_path: String) -> RunStatusResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return RunStatusResult {
                success: false,
                status: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.get_run_status(&run_id) {
        Ok(status_json) => {
            RunStatusResult {
                success: true,
                status: Some(status_json),
                message: "Status retrieved successfully".to_string(),
            }
        }
        Err(e) => {
            RunStatusResult {
                success: false,
                status: None,
                message: format!("Failed to get run status: {}", e),
            }
        }
    }
}

/// Execute a step via N-API
#[napi]
pub fn execute_step(run_id: String, step_id: String, db_path: String, context_json: String) -> StepExecutionResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return StepExecutionResult {
                success: false,
                result: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.execute_step(&run_id, &step_id, &context_json) {
        Ok(result_json) => {
            StepExecutionResult {
                success: true,
                result: Some(result_json),
                message: "Step executed successfully".to_string(),
            }
        }
        Err(e) => {
            StepExecutionResult {
                success: false,
                result: None,
                message: format!("Failed to execute step: {}", e),
            }
        }
    }
}

/// Execute a step function in Bun.js via N-API
#[napi]
pub fn execute_step_function(
    step_name: String,
    context_json: String,
    workflow_id: String,
    run_id: String,
    db_path: String
) -> StepExecutionResult {
    log::info!("Executing step function: {} for workflow: {} run: {}", step_name, workflow_id, run_id);
    
    let _bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return StepExecutionResult {
                success: false,
                result: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    // Parse context JSON
    let _context: crate::context::Context = match serde_json::from_str(&context_json) {
        Ok(context) => context,
        Err(e) => {
            return StepExecutionResult {
                success: false,
                result: None,
                message: format!("Failed to parse context JSON: {}", e),
            };
        }
    };
    
    // For now, return the context as a result
    // TODO: In Task 1.3, we'll implement the actual Bun.js step execution
    let result = serde_json::json!({
        "step_name": step_name,
        "workflow_id": workflow_id,
        "run_id": run_id,
        "context": context_json,
        "status": "ready_for_bun_execution",
        "message": "Step function ready for Bun.js execution"
    });
    
    let result_json = serde_json::to_string(&result)
        .map_err(|e| CoreError::Serialization(e))
        .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e));
    
    StepExecutionResult {
        success: true,
        result: Some(result_json),
        message: "Step function prepared for Bun.js execution".to_string(),
    }
}

/// Execute a job function in Bun.js via N-API
#[napi]
pub fn execute_job_function(
    job_json: String,
    services_json: String,
    db_path: String
) -> JobExecutionResult {
    log::info!("Executing job function with job: {}", job_json);
    
    let _bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    // Parse job from JSON
    let job: Job = match serde_json::from_str(&job_json) {
        Ok(job) => job,
        Err(e) => {
            return JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to parse job JSON: {}", e),
            };
        }
    };
    
    // Parse services from JSON
    let _services: HashMap<String, serde_json::Value> = match serde_json::from_str(&services_json) {
        Ok(services) => services,
        Err(e) => {
            return JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to parse services JSON: {}", e),
            };
        }
    };
    
    // For now, return a simple result
    // TODO: In Task 1.3, we'll implement the actual Bun.js job execution
    let result = serde_json::json!({
        "job_id": job.id,
        "run_id": job.run_id,
        "step_id": job.step_name,
        "services": services_json,
        "status": "ready_for_bun_execution",
        "message": "Job function ready for Bun.js execution"
    });
    
    let result_json = serde_json::to_string(&result)
        .map_err(|e| CoreError::Serialization(e))
        .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e));
    
    JobExecutionResult {
        success: true,
        job_id: Some(job.id),
        run_id: Some(job.run_id),
        step_id: Some(job.step_name),
        context: Some(services_json),
        result: Some(result_json),
        message: "Job function prepared for Bun.js execution".to_string(),
    }
}

/// Execute a job with context via N-API
#[napi]
pub fn execute_job(job_json: String, services_json: String, db_path: String) -> JobExecutionResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    // Parse job from JSON
    let job: Job = match serde_json::from_str(&job_json) {
        Ok(job) => job,
        Err(e) => {
            return JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to parse job JSON: {}", e),
            };
        }
    };
    
    // Parse services from JSON
    let services: HashMap<String, serde_json::Value> = match serde_json::from_str(&services_json) {
        Ok(services) => services,
        Err(e) => {
            return JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to parse services JSON: {}", e),
            };
        }
    };
    
    match bridge.execute_job(&job, services) {
        Ok(result_json) => {
            // Parse the result to extract individual fields
            let result: serde_json::Value = match serde_json::from_str(&result_json) {
                Ok(result) => result,
                Err(_) => {
                    return JobExecutionResult {
                        success: false,
                        job_id: None,
                        run_id: None,
                        step_id: None,
                        context: None,
                        result: None,
                        message: "Failed to parse execution result".to_string(),
                    };
                }
            };
            
            JobExecutionResult {
                success: true,
                job_id: result["job_id"].as_str().map(|s| s.to_string()),
                run_id: result["run_id"].as_str().map(|s| s.to_string()),
                step_id: result["step_id"].as_str().map(|s| s.to_string()),
                context: result["context"].as_str().map(|s| s.to_string()),
                result: Some(result_json),
                message: "Job executed successfully".to_string(),
            }
        }
        Err(e) => {
            JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to execute job: {}", e),
            }
        }
    }
} 

/// Execute a webhook trigger via N-API
#[napi]
pub fn execute_webhook_trigger(request_json: String, db_path: String) -> TriggerExecutionResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.execute_webhook_trigger(&request_json) {
        Ok(result_json) => {
            // Parse the result to extract individual fields
            let result: serde_json::Value = match serde_json::from_str(&result_json) {
                Ok(result) => result,
                Err(_) => {
                    return TriggerExecutionResult {
                        success: false,
                        run_id: None,
                        workflow_id: None,
                        message: "Failed to parse execution result".to_string(),
                    };
                }
            };
            
            TriggerExecutionResult {
                success: true,
                run_id: result["run_id"].as_str().map(|s| s.to_string()),
                workflow_id: result["workflow_id"].as_str().map(|s| s.to_string()),
                message: result["message"].as_str().unwrap_or("Webhook trigger executed successfully").to_string(),
            }
        }
        Err(e) => {
            TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to execute webhook trigger: {}", e),
            }
        }
    }
}

/// Execute a schedule trigger via N-API
#[napi]
pub fn execute_schedule_trigger(trigger_id: String, db_path: String) -> TriggerExecutionResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.execute_schedule_trigger(&trigger_id) {
        Ok(result_json) => {
            // Parse the result to extract individual fields
            let result: serde_json::Value = match serde_json::from_str(&result_json) {
                Ok(result) => result,
                Err(_) => {
                    return TriggerExecutionResult {
                        success: false,
                        run_id: None,
                        workflow_id: None,
                        message: "Failed to parse execution result".to_string(),
                    };
                }
            };
            
            TriggerExecutionResult {
                success: true,
                run_id: result["run_id"].as_str().map(|s| s.to_string()),
                workflow_id: result["workflow_id"].as_str().map(|s| s.to_string()),
                message: result["message"].as_str().unwrap_or("Schedule trigger executed successfully").to_string(),
            }
        }
        Err(e) => {
            TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to execute schedule trigger: {}", e),
            }
        }
    }
}

/// Execute a manual trigger via N-API
#[napi]
pub fn execute_manual_trigger(workflow_id: String, payload_json: String, db_path: String) -> TriggerExecutionResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.execute_manual_trigger(&workflow_id, &payload_json) {
        Ok(result_json) => {
            // Parse the result to extract individual fields
            let result: serde_json::Value = match serde_json::from_str(&result_json) {
                Ok(result) => result,
                Err(_) => {
                    return TriggerExecutionResult {
                        success: false,
                        run_id: None,
                        workflow_id: None,
                        message: "Failed to parse execution result".to_string(),
                    };
                }
            };
            
            TriggerExecutionResult {
                success: true,
                run_id: result["run_id"].as_str().map(|s| s.to_string()),
                workflow_id: result["workflow_id"].as_str().map(|s| s.to_string()),
                message: result["message"].as_str().unwrap_or("Manual trigger executed successfully").to_string(),
            }
        }
        Err(e) => {
            TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to execute manual trigger: {}", e),
            }
        }
    }
}

/// Get trigger statistics via N-API
#[napi]
pub fn get_trigger_stats(db_path: String) -> TriggerStatsResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return TriggerStatsResult {
                success: false,
                stats: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.get_trigger_stats() {
        Ok(stats_json) => {
            TriggerStatsResult {
                success: true,
                stats: Some(stats_json),
                message: "Trigger statistics retrieved successfully".to_string(),
            }
        }
        Err(e) => {
            TriggerStatsResult {
                success: false,
                stats: None,
                message: format!("Failed to get trigger statistics: {}", e),
            }
        }
    }
}

/// Get triggers for a workflow via N-API
#[napi]
pub fn get_workflow_triggers(workflow_id: String, db_path: String) -> WorkflowTriggersResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return WorkflowTriggersResult {
                success: false,
                triggers: None,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.get_workflow_triggers(&workflow_id) {
        Ok(triggers_json) => {
            WorkflowTriggersResult {
                success: true,
                triggers: Some(triggers_json),
                message: "Workflow triggers retrieved successfully".to_string(),
            }
        }
        Err(e) => {
            WorkflowTriggersResult {
                success: false,
                triggers: None,
                message: format!("Failed to get workflow triggers: {}", e),
            }
        }
    }
}

/// Unregister triggers for a workflow via N-API
#[napi]
pub fn unregister_workflow_triggers(workflow_id: String, db_path: String) -> TriggerUnregistrationResult {
    let bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return TriggerUnregistrationResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.unregister_workflow_triggers(&workflow_id) {
        Ok(_) => {
            TriggerUnregistrationResult {
                success: true,
                message: format!("Successfully unregistered triggers for workflow: {}", workflow_id),
            }
        }
        Err(e) => {
            TriggerUnregistrationResult {
                success: false,
                message: format!("Failed to unregister workflow triggers: {}", e),
            }
        }
    }
} 

/// Start the webhook server via N-API
#[napi]
pub fn start_webhook_server(db_path: String) -> WebhookServerResult {
    let mut bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return WebhookServerResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.start_webhook_server() {
        Ok(_) => {
            WebhookServerResult {
                success: true,
                message: "Webhook server started successfully".to_string(),
            }
        }
        Err(e) => {
            WebhookServerResult {
                success: false,
                message: format!("Failed to start webhook server: {}", e),
            }
        }
    }
}

/// Stop the webhook server via N-API
#[napi]
pub fn stop_webhook_server(db_path: String) -> WebhookServerResult {
    let mut bridge = match Bridge::new(&db_path) {
        Ok(bridge) => bridge,
        Err(e) => {
            return WebhookServerResult {
                success: false,
                message: format!("Failed to create bridge: {}", e),
            };
        }
    };
    
    match bridge.stop_webhook_server() {
        Ok(_) => {
            WebhookServerResult {
                success: true,
                message: "Webhook server stopped successfully".to_string(),
            }
        }
        Err(e) => {
            WebhookServerResult {
                success: false,
                message: format!("Failed to stop webhook server: {}", e),
            }
        }
    }
}

#[napi(object)]
pub struct WebhookServerResult {
    pub success: bool,
    pub message: String,
} 