//! N-API bridge for Node.js communication
//! 
//! This module handles the communication between the Rust core engine
//! and the Node.js SDK via N-API.

use std::sync::{Arc, Mutex, OnceLock};
use napi_derive::napi;
use crate::{
    models::WorkflowDefinition,
    state::StateManager,
    trigger_executor::TriggerExecutor,
    dispatcher::Dispatcher,
    triggers::TriggerManager,
    error::CoreError,
    job::Job,
};
use crate::error::CoreResult;
use uuid::Uuid;
use log;
use serde::Serialize;

/// N-API bridge for Node.js communication
pub struct Bridge {
    state_manager: Arc<Mutex<StateManager>>,
    trigger_manager: Arc<Mutex<TriggerManager>>,
    trigger_executor: TriggerExecutor,
    job_dispatcher: Arc<Mutex<Dispatcher>>,
}

/// Global shared Bridge instance to eliminate N-API function duplication
static BRIDGE_CACHE: OnceLock<Mutex<Option<Arc<Bridge>>>> = OnceLock::new();

/// Get or create shared Bridge instance for N-API functions
fn get_shared_bridge(db_path: &str) -> CoreResult<Arc<Bridge>> {
    let cache = BRIDGE_CACHE.get_or_init(|| Mutex::new(None));
    let mut bridge_opt = cache.lock()
        .map_err(|e| CoreError::Internal(format!("Failed to acquire bridge cache lock: {}", e)))?;
    
    if let Some(bridge) = bridge_opt.as_ref() {
        Ok(bridge.clone())
    } else {
        let new_bridge = Arc::new(Bridge::new(db_path)?);
        *bridge_opt = Some(new_bridge.clone());
        Ok(new_bridge)
    }
}

/// Helper function for consistent N-API error handling
fn handle_bridge_error<T: Default>(error: CoreError) -> T {
    log::error!("Bridge operation failed: {}", error);
    T::default()
}

/// Macro for standardized N-API function patterns with shared bridge
macro_rules! with_shared_bridge {
    ($db_path:expr, $success_result:expr, $failure_result:expr, $operation:expr) => {
        match get_shared_bridge($db_path) {
            Ok(bridge) => {
                match $operation(bridge) {
                    Ok(result) => $success_result(result),
                    Err(e) => $failure_result(format!("Operation failed: {}", e)),
                }
            }
            Err(e) => $failure_result(format!("Failed to get bridge: {}", e)),
        }
    };
}

impl Bridge {
    /// Create a new N-API bridge
    pub fn new(db_path: &str) -> CoreResult<Self> {
        let state_manager = Arc::new(Mutex::new(StateManager::new(db_path)?));
        let trigger_manager = Arc::new(Mutex::new(TriggerManager::new()));
        
        let dispatcher_config = crate::dispatcher::WorkerPoolConfig::default();
        let job_dispatcher = Arc::new(Mutex::new(Dispatcher::new(dispatcher_config, state_manager.clone())));
        
        let trigger_executor = TriggerExecutor::new(
            state_manager.clone(), 
            trigger_manager.clone(),
            job_dispatcher.clone()
        );
        
        Ok(Bridge { 
            state_manager,
            trigger_manager,
            trigger_executor,
            job_dispatcher,
        })
    }

    /// Register a workflow from Node.js
    pub fn register_workflow(&self, workflow_json: &str) -> CoreResult<()> {
        log::info!("Registering workflow from JSON: {}", workflow_json);
        
        let workflow: WorkflowDefinition = serde_json::from_str(workflow_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        workflow.validate()
            .map_err(|e| CoreError::InvalidWorkflow(e))?;
        
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        state_manager.register_workflow(workflow.clone())?;
        
        let trigger_ids = self.trigger_executor.register_workflow_triggers(&workflow.id, &workflow)?;
        
        log::info!("Successfully registered workflow: {} with {} triggers: {:?}", workflow.id, trigger_ids.len(), trigger_ids);
        Ok(())
    }

    /// Register a webhook trigger for a workflow
    pub fn register_webhook_trigger(&self, workflow_id: &str, trigger_json: &str) -> CoreResult<()> {
        log::info!("Registering webhook trigger for workflow: {} with config: {}", workflow_id, trigger_json);
        
        let trigger: crate::triggers::WebhookTrigger = serde_json::from_str(trigger_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        trigger.validate()?;
        
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

    /// Create a workflow run from Node.js
    pub fn create_run(&self, workflow_id: &str, payload_json: &str) -> CoreResult<String> {
        log::info!("Creating run for workflow: {} with payload: {}", workflow_id, payload_json);
        
        let payload: serde_json::Value = serde_json::from_str(payload_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        let mut state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        let run_id = state_manager.create_run(workflow_id, payload)?;
        
        log::info!("Successfully created run: {} for workflow: {}", run_id, workflow_id);
        Ok(run_id.to_string())
    }

    /// Get workflow run status
    pub fn get_run_status(&self, run_id: &str) -> CoreResult<String> {
        log::info!("Getting status for run: {}", run_id);
        
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        
        let _run = state_manager.get_run(&run_uuid)?
            .ok_or_else(|| CoreError::WorkflowNotFound(format!("Run not found: {}", run_id)))?;
        
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

    /// Execute a step with context for Bun.js
    pub fn execute_step(&self, run_id: &str, step_id: &str) -> CoreResult<String> {
        log::info!("Executing step {} for run {}", step_id, run_id);
        
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        let state_manager = self.state_manager.lock().unwrap();
        let run = state_manager.get_run(&run_uuid)?
            .ok_or_else(|| CoreError::RunNotFound(format!("Run not found: {}", run_id)))?;
        
        let workflow = state_manager.get_workflow(&run.workflow_id)?
            .ok_or_else(|| CoreError::WorkflowNotFound(run.workflow_id.clone()))?;
        
        let step = workflow.get_step(step_id)
            .ok_or_else(|| CoreError::Validation(format!("Step '{}' not found in workflow '{}'", step_id, run.workflow_id)))?;
        
        let completed_steps = state_manager.get_completed_steps(&run_uuid)?;
        
        let mut context = crate::context::Context::new(
            run_id.to_string(),
            run.workflow_id.clone(),
            step_id.to_string(),
            run.payload.clone(),
            run.clone(),
            completed_steps,
        )?;
        
        if let Some(timeout) = step.timeout {
            context.set_timeout(timeout);
        }
        
        // Serialize context for Bun.js
        let context_json = context.to_json()?;
        
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
    pub fn execute_job(&self, job: &Job) -> CoreResult<String> {
        log::info!("Executing job: {}", job.id);
        
        let state_manager = self.state_manager.lock().unwrap();
        
        let _workflow = state_manager.get_workflow(&job.workflow_id)?;
        
        let _run_uuid = Uuid::parse_str(&job.run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        let result = serde_json::json!({
            "job_id": job.id,
            "run_id": job.run_id,
            "step_id": job.step_name,
            "status": "pending",
            "message": "Job execution not yet implemented"
        });
        
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        Ok(result_json)
    }

    /// Execute a webhook trigger
    pub fn execute_webhook_trigger(&self, request_json: &str) -> CoreResult<String> {
        log::info!("Executing webhook trigger with request: {}", request_json);
        
        let request: crate::triggers::WebhookRequest = serde_json::from_str(request_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Execute the webhook trigger
        let result = self.trigger_executor.execute_webhook_trigger(request)?;
        
        let result_json = serde_json::to_string(&result)
            .map_err(|e| CoreError::Serialization(e))?;
        
        log::info!("Webhook trigger execution result: {}", result_json);
        Ok(result_json)
    }

    /// Execute a manual trigger
    pub fn execute_manual_trigger(&self, workflow_id: &str, payload_json: &str) -> CoreResult<String> {
        log::info!("Executing manual trigger for workflow: {} with payload: {}", workflow_id, payload_json);
        
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

    /// Start the webhook server with proper async support
    pub async fn start_webhook_server_async(&mut self) -> CoreResult<()> {
        log::info!("Starting webhook server with async support...");
        
        let config = crate::webhook_server::WebhookServerConfig::default();
        let mut webhook_server = crate::webhook_server::WebhookServer::new(
            config,
            self.trigger_manager.clone(),
            self.state_manager.clone(),
        );
        
        webhook_server.start().await?;
        log::info!("Webhook server started successfully");
        Ok(())
    }
    
    /// Start the webhook server (legacy sync method)
    pub fn start_webhook_server(&self) -> CoreResult<()> {
        log::info!("Starting webhook server (legacy mode)...");
        log::info!("Note: Use start_webhook_server_async() for full async support");
        log::info!("Webhook server configuration ready");
        Ok(())
    }
    
    /// Stop the webhook server
    pub fn stop_webhook_server(&self) -> CoreResult<()> {
        log::info!("Stopping webhook server");
        // Note: For async server, use the WebhookServer instance directly
        Ok(())
    }

    /// Get job status
    pub fn get_job_status(&self, job_id: &str) -> CoreResult<Option<String>> {
        log::info!("Getting job status for: {}", job_id);
        
        let dispatcher = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
        
        match dispatcher.get_job_status(job_id)? {
            Some(state) => Ok(Some(format!("{:?}", state))),
            None => Ok(None),
        }
    }

    /// Cancel a job
    pub fn cancel_job(&self, job_id: &str) -> CoreResult<bool> {
        log::info!("Cancelling job: {}", job_id);
        
        let dispatcher = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
        
        dispatcher.cancel_job(job_id)
    }

    /// Get dispatcher statistics
    pub fn get_dispatcher_stats(&self) -> CoreResult<crate::dispatcher::DispatcherStats> {
        log::info!("Getting dispatcher statistics");
        
        let dispatcher = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
        
        dispatcher.get_stats()
    }

    /// Get workflow run status
    pub fn get_workflow_run_status(&self, run_id: &str) -> CoreResult<Option<crate::models::RunStatus>> {
        log::info!("Getting workflow run status for: {}", run_id);
        
        let dispatcher = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
        
        dispatcher.get_workflow_run_status(run_id)
    }

    /// Get completed steps for a workflow run
    pub fn get_workflow_completed_steps(&self, run_id: &str) -> CoreResult<Vec<crate::models::StepResult>> {
        log::info!("Getting completed steps for workflow run: {}", run_id);
        
        let dispatcher = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
        
        dispatcher.get_workflow_completed_steps(run_id)
    }

    /// Execute workflow steps using step orchestrator and state machine
    pub fn execute_workflow_steps(&self, run_id: &str, workflow_id: &str) -> CoreResult<String> {
        log::info!("Executing workflow steps for run: {} workflow: {}", run_id, workflow_id);
        
        let run_uuid = Uuid::parse_str(run_id)
            .map_err(|e| CoreError::Validation(format!("Invalid run ID: {}", e)))?;
        
        let step_orchestrator = crate::step_orchestrator::StepOrchestrator::new(self.state_manager.clone());
        
        // Start step execution using the orchestrator
        match step_orchestrator.start_step_execution(&run_uuid, workflow_id) {
            Ok(()) => {
                log::info!("Successfully executed workflow steps for run: {}", run_id);
                Ok(serde_json::json!({
                    "success": true,
                    "run_id": run_id,
                    "workflow_id": workflow_id,
                    "message": "Workflow steps executed successfully"
                }).to_string())
            }
            Err(error) => {
                log::error!("Failed to execute workflow steps for run {}: {}", run_id, error);
                Err(error)
            }
        }
    }

    /// Execute workflow hook (onSuccess or onFailure)
    pub fn execute_workflow_hook(&self, hook_type: &str, context_json: &str, workflow_id: &str) -> CoreResult<String> {
        log::info!("Executing {} hook for workflow: {}", hook_type, workflow_id);
        
        if hook_type != "onSuccess" && hook_type != "onFailure" {
            return Err(CoreError::Validation(format!("Invalid hook type: {}", hook_type)));
        }
        
        // In the next phase, this will call the Bun.js hook execution
        let result = serde_json::json!({
            "success": true,
            "hook_type": hook_type,
            "workflow_id": workflow_id,
            "message": format!("{} hook executed successfully", hook_type),
            "context": serde_json::from_str::<serde_json::Value>(context_json).unwrap_or(serde_json::Value::Null)
        });
        
        Ok(result.to_string())
    }
}

// ============================================================================
// CONSOLIDATED N-API RESULT TYPES (Task 1.5)
// ============================================================================

/// Simple result with just success + message
#[derive(Debug, Clone, Serialize)]
#[napi(object)]
pub struct SimpleResult {
    pub success: bool,
    pub message: String,
}

/// Result with optional data payload (JSON string)
#[derive(Debug, Clone, Serialize)]
#[napi(object)]
pub struct DataResult {
    pub success: bool,
    pub data: Option<String>,
    pub message: String,
}

/// Result with optional ID and data
#[derive(Debug, Clone, Serialize)]
#[napi(object)]
pub struct IdDataResult {
    pub success: bool,
    pub id: Option<String>,
    pub data: Option<String>,
    pub message: String,
}

// ============================================================================
// SPECIALIZED RESULT TYPES (kept for complex structures)
// ============================================================================

/// Result for job execution (complex, multiple fields)
#[derive(Debug, Clone, Serialize)]
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

/// Result for job cancellation (has boolean flag)
#[derive(Debug, Clone, Serialize)]
#[napi(object)]
pub struct JobCancellationResult {
    pub success: bool,
    pub job_id: Option<String>,
    pub cancelled: bool,
    pub message: String,
}

/// Result for trigger execution (two IDs)
#[derive(Debug, Clone, Serialize)]
#[napi(object)]
pub struct TriggerExecutionResult {
    pub success: bool,
    pub run_id: Option<String>,
    pub workflow_id: Option<String>,
    pub message: String,
}

/// Result for hook execution
#[derive(Debug, Clone, Serialize)]
#[napi(object)]
pub struct HookExecutionResult {
    pub success: bool,
    pub hook_type: Option<String>,
    pub workflow_id: Option<String>,
    pub result: Option<String>,
    pub message: String,
}

// Type aliases for backward compatibility and clarity
pub type WorkflowRegistrationResult = SimpleResult;
pub type WebhookTriggerRegistrationResult = SimpleResult;
pub type TriggerUnregistrationResult = SimpleResult;
pub type WebhookServerResult = SimpleResult;

pub type RunCreationResult = IdDataResult;
pub type RunStatusResult = DataResult;
pub type StepExecutionResult = DataResult;
pub type WebhookTriggersResult = DataResult;
pub type DispatcherStatsResult = DataResult;
pub type TriggerStatsResult = DataResult;
pub type WorkflowTriggersResult = DataResult;

pub type JobStatusResult = IdDataResult;
pub type WorkflowRunStatusResult = IdDataResult;
pub type WorkflowStepsResult = IdDataResult;

/// Register a workflow via N-API
#[napi]
pub fn register_workflow(workflow_json: String, db_path: String) -> WorkflowRegistrationResult {
    with_shared_bridge!(
        &db_path,
        |_| WorkflowRegistrationResult {
            success: true,
            message: "Workflow registered successfully".to_string(),
        },
        |msg: String| WorkflowRegistrationResult {
            success: false,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.register_workflow(&workflow_json)
    )
}

/// Register a webhook trigger via N-API
#[napi]
pub fn register_webhook_trigger(workflow_id: String, trigger_json: String, db_path: String) -> WebhookTriggerRegistrationResult {
    with_shared_bridge!(
        &db_path,
        |_| WebhookTriggerRegistrationResult {
            success: true,
            message: "Webhook trigger registered successfully".to_string(),
        },
        |msg: String| WebhookTriggerRegistrationResult {
            success: false,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.register_webhook_trigger(&workflow_id, &trigger_json)
    )
}

/// Get all webhook triggers via N-API
#[napi]
pub fn get_webhook_triggers(db_path: String) -> WebhookTriggersResult {
    with_shared_bridge!(
        &db_path,
        |triggers_json: String| WebhookTriggersResult {
            success: true,
            data: Some(triggers_json),
            message: "Webhook triggers retrieved successfully".to_string(),
        },
        |msg: String| WebhookTriggersResult {
            success: false,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.get_webhook_triggers()
    )
}

/// Create a workflow run via N-API
#[napi]
pub fn create_run(workflow_id: String, payload_json: String, db_path: String) -> RunCreationResult {
    with_shared_bridge!(
        &db_path,
        |run_id: String| RunCreationResult {
            success: true,
            id: Some(run_id),
            data: None,
            message: "Run created successfully".to_string(),
        },
        |msg: String| RunCreationResult {
            success: false,
            id: None,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.create_run(&workflow_id, &payload_json)
    )
}

/// Get run status via N-API
#[napi]
pub fn get_run_status(run_id: String, db_path: String) -> RunStatusResult {
    with_shared_bridge!(
        &db_path,
        |status_json: String| RunStatusResult {
            success: true,
            data: Some(status_json),
            message: "Status retrieved successfully".to_string(),
        },
        |msg: String| RunStatusResult {
            success: false,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.get_run_status(&run_id)
    )
}

/// Execute a step via N-API
#[napi]
pub fn execute_step(run_id: String, step_id: String, db_path: String) -> StepExecutionResult {
    with_shared_bridge!(
        &db_path,
        |result: String| StepExecutionResult {
            success: true,
            data: Some(result),
            message: "Step executed successfully".to_string(),
        },
        |msg: String| StepExecutionResult {
            success: false,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.execute_step(&run_id, &step_id)
    )
}

/// Execute a job with context via N-API
#[napi]
pub fn execute_job(job_json: String, db_path: String) -> JobExecutionResult {
    log::info!("Executing job with context: {}", job_json);
    
    // Parse job JSON first
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
    
    // Use shared bridge instance
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.execute_job(&job) {
                Ok(result_json) => {
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
        Err(e) => {
            JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Get job status via N-API
#[napi]
pub fn get_job_status(job_id: String, db_path: String) -> JobStatusResult {
    log::info!("Getting job status for: {}", job_id);
    
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.get_job_status(&job_id) {
                Ok(status) => {
                    let status_str = match status {
                        Some(s) => format!("{:?}", s),
                        None => "not_found".to_string(),
                    };
                    
                    JobStatusResult {
                        success: true,
                        id: Some(job_id),
                        data: Some(status_str),
                        message: "Job status retrieved successfully".to_string(),
                    }
                }
                Err(e) => {
                    JobStatusResult {
                        success: false,
                        id: None,
                        data: None,
                        message: format!("Failed to get job status: {}", e),
                    }
                }
            }
        }
        Err(e) => {
            JobStatusResult {
                success: false,
                id: None,
                data: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Cancel a job via N-API
#[napi]
pub fn cancel_job(job_id: String, db_path: String) -> JobCancellationResult {
    log::info!("Cancelling job: {}", job_id);
    
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.cancel_job(&job_id) {
                Ok(cancelled) => {
                    JobCancellationResult {
                        success: true,
                        job_id: Some(job_id),
                        cancelled,
                        message: if cancelled {
                            "Job cancelled successfully".to_string()
                        } else {
                            "Job not found or already completed".to_string()
                        },
                    }
                }
                Err(e) => {
                    JobCancellationResult {
                        success: false,
                        job_id: None,
                        cancelled: false,
                        message: format!("Failed to cancel job: {}", e),
                    }
                }
            }
        }
        Err(e) => {
            JobCancellationResult {
                success: false,
                job_id: None,
                cancelled: false,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Get dispatcher statistics via N-API
#[napi]
pub fn get_dispatcher_stats(db_path: String) -> DispatcherStatsResult {
    log::info!("Getting dispatcher statistics");
    
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.get_dispatcher_stats() {
                Ok(stats) => {
                    let stats_json = serde_json::to_string(&stats)
                        .unwrap_or_else(|_| "{}".to_string());
                    
                    DispatcherStatsResult {
                        success: true,
                        data: Some(stats_json),
                        message: "Dispatcher statistics retrieved successfully".to_string(),
                    }
                }
                Err(e) => {
                    DispatcherStatsResult {
                        success: false,
                        data: None,
                        message: format!("Failed to get dispatcher stats: {}", e),
                    }
                }
            }
        }
        Err(e) => {
            DispatcherStatsResult {
                success: false,
                data: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Get workflow run status via N-API
#[napi]
pub fn get_workflow_run_status(run_id: String, db_path: String) -> WorkflowRunStatusResult {
    log::info!("Getting workflow run status for: {}", run_id);
    
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.get_workflow_run_status(&run_id) {
                Ok(status) => {
                    let status_str = match status {
                        Some(s) => format!("{:?}", s),
                        None => "not_found".to_string(),
                    };
                    
                    WorkflowRunStatusResult {
                        success: true,
                        id: Some(run_id),
                        data: Some(status_str),
                        message: "Workflow run status retrieved successfully".to_string(),
                    }
                }
                Err(e) => {
                    WorkflowRunStatusResult {
                        success: false,
                        id: None,
                        data: None,
                        message: format!("Failed to get workflow run status: {}", e),
                    }
                }
            }
        }
        Err(e) => {
            WorkflowRunStatusResult {
                success: false,
                id: None,
                data: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Get completed steps for a workflow run via N-API
#[napi]
pub fn get_workflow_completed_steps(run_id: String, db_path: String) -> WorkflowStepsResult {
    log::info!("Getting completed steps for workflow run: {}", run_id);
    
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.get_workflow_completed_steps(&run_id) {
                Ok(steps) => {
                    let steps_json = serde_json::to_string(&steps)
                        .unwrap_or_else(|_| "[]".to_string());
                    
                    WorkflowStepsResult {
                        success: true,
                        id: Some(run_id),
                        data: Some(steps_json),
                        message: "Workflow completed steps retrieved successfully".to_string(),
                    }
                }
                Err(e) => {
                    WorkflowStepsResult {
                        success: false,
                        id: None,
                        data: None,
                        message: format!("Failed to get workflow completed steps: {}", e),
                    }
                }
            }
        }
        Err(e) => {
            WorkflowStepsResult {
                success: false,
                id: None,
                data: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Execute a webhook trigger via N-API
#[napi]
pub fn execute_webhook_trigger(request_json: String, db_path: String) -> TriggerExecutionResult {
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.execute_webhook_trigger(&request_json) {
                Ok(result_json) => {
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
        Err(e) => {
            TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Execute a manual trigger via N-API
#[napi]
pub fn execute_manual_trigger(workflow_id: String, payload_json: String, db_path: String) -> TriggerExecutionResult {
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.execute_manual_trigger(&workflow_id, &payload_json) {
                Ok(result_json) => {
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
        Err(e) => {
            TriggerExecutionResult {
                success: false,
                run_id: None,
                workflow_id: None,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Get trigger statistics via N-API
#[napi]
pub fn get_trigger_stats(db_path: String) -> TriggerStatsResult {
    with_shared_bridge!(
        &db_path,
        |stats_json: String| TriggerStatsResult {
            success: true,
            data: Some(stats_json),
            message: "Trigger statistics retrieved successfully".to_string(),
        },
        |msg: String| TriggerStatsResult {
            success: false,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.get_trigger_stats()
    )
}

/// Get triggers for a workflow via N-API
#[napi]
pub fn get_workflow_triggers(workflow_id: String, db_path: String) -> WorkflowTriggersResult {
    with_shared_bridge!(
        &db_path,
        |triggers_json: String| WorkflowTriggersResult {
            success: true,
            data: Some(triggers_json),
            message: "Workflow triggers retrieved successfully".to_string(),
        },
        |msg: String| WorkflowTriggersResult {
            success: false,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.get_workflow_triggers(&workflow_id)
    )
}

/// Unregister triggers for a workflow via N-API
#[napi]
pub fn unregister_workflow_triggers(workflow_id: String, db_path: String) -> TriggerUnregistrationResult {
    with_shared_bridge!(
        &db_path,
        |_| TriggerUnregistrationResult {
            success: true,
            message: format!("Successfully unregistered triggers for workflow: {}", workflow_id),
        },
        |msg: String| TriggerUnregistrationResult {
            success: false,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.unregister_workflow_triggers(&workflow_id)
    )
} 

/// Start the webhook server via N-API
#[napi]
pub fn start_webhook_server(db_path: String) -> WebhookServerResult {
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            // Note: start_webhook_server doesn't actually mutate the bridge
            // The Arc allows interior mutability where needed
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
        Err(e) => {
            WebhookServerResult {
                success: false,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

/// Stop the webhook server via N-API
#[napi]
pub fn stop_webhook_server(db_path: String) -> WebhookServerResult {
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            // Note: stop_webhook_server doesn't actually mutate the bridge
            // The Arc allows interior mutability where needed
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
        Err(e) => {
            WebhookServerResult {
                success: false,
                message: format!("Failed to get bridge: {}", e),
            }
        }
    }
}

#[napi]
pub fn execute_workflow_steps(run_id: String, workflow_id: String, db_path: String) -> StepExecutionResult {
    with_shared_bridge!(
        &db_path,
        |result: String| StepExecutionResult {
            success: true,
            data: Some(result),
            message: "Workflow steps executed successfully".to_string(),
        },
        |msg: String| StepExecutionResult {
            success: false,
            data: None,
            message: msg,
        },
        |bridge: Arc<Bridge>| bridge.execute_workflow_steps(&run_id, &workflow_id)
    )
} 

#[napi]
pub fn execute_workflow_hook(hook_type: String, context_json: String, workflow_id: String, db_path: String) -> HookExecutionResult {
    match get_shared_bridge(&db_path) {
        Ok(bridge) => {
            match bridge.execute_workflow_hook(&hook_type, &context_json, &workflow_id) {
                Ok(result) => {
                    HookExecutionResult {
                        success: true,
                        hook_type: Some(hook_type),
                        workflow_id: Some(workflow_id),
                        result: Some(result),
                        message: "Hook executed successfully".to_string(),
                    }
                }
                Err(error) => {
                    HookExecutionResult {
                        success: false,
                        hook_type: Some(hook_type),
                        workflow_id: Some(workflow_id),
                        result: None,
                        message: format!("Failed to execute hook: {}", error),
                    }
                }
            }
        }
        Err(error) => {
            HookExecutionResult {
                success: false,
                hook_type: Some(hook_type),
                workflow_id: Some(workflow_id),
                result: None,
                message: format!("Failed to get bridge: {}", error),
            }
        }
    }
} 

// Note: pause_workflow and resume_workflow removed (Task 1.4)
// These were placeholder functions that didn't actually pause/resume workflows.
// When workflow state machine is integrated (Phase 2, Task 2.2), 
// these functions can be re-implemented with actual functionality. 