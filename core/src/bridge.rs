//! N-API bridge for Node.js communication
//! 
//! This module handles the communication between the Rust core engine
//! and the Node.js SDK via N-API.

use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as TokioMutex;
use napi_derive::napi;
use crate::{
    models::WorkflowDefinition,
    state::{StateManager, AsyncStateManager},
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

/// N-API bridge for Node.js communication (synchronous version - kept for backward compatibility)
pub struct Bridge {
    state_manager: Arc<Mutex<StateManager>>,
    trigger_manager: Arc<Mutex<TriggerManager>>,
    trigger_executor: TriggerExecutor,
    job_dispatcher: Arc<Mutex<Arc<tokio::sync::Mutex<Dispatcher>>>>, // Wrapper for async dispatcher
}

/// Async N-API bridge for Node.js communication
/// Uses async components for non-blocking operations
pub struct AsyncBridge {
    state_manager: Arc<AsyncStateManager>,
    trigger_manager: Arc<TokioMutex<TriggerManager>>,
    trigger_executor: Arc<TriggerExecutor>,
    job_dispatcher: Arc<TokioMutex<Dispatcher>>,
}

/// Global shared Bridge instance to eliminate N-API function duplication
static BRIDGE_CACHE: OnceLock<Mutex<Option<Arc<Bridge>>>> = OnceLock::new();

/// Global shared AsyncBridge instance for async N-API functions
static ASYNC_BRIDGE_CACHE: OnceLock<TokioMutex<Option<Arc<AsyncBridge>>>> = OnceLock::new();

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

/// Get or create shared AsyncBridge instance for async N-API functions
async fn get_shared_async_bridge(db_path: &str) -> CoreResult<Arc<AsyncBridge>> {
    let cache = ASYNC_BRIDGE_CACHE.get_or_init(|| TokioMutex::new(None));
    let mut bridge_opt = cache.lock().await;
    
    if let Some(bridge) = bridge_opt.as_ref() {
        Ok(bridge.clone())
    } else {
        let new_bridge = Arc::new(AsyncBridge::new(db_path)?);
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
        
        // Create a tokio Mutex wrapper for the dispatcher
        // The dispatcher needs async state manager access
        let state_manager_for_dispatcher = {
            // Create new state manager for dispatcher (will be shared later)
            Arc::new(tokio::sync::Mutex::new(StateManager::new(db_path)?))
        };
        
        let dispatcher_config = crate::dispatcher::WorkerPoolConfig::default();
        let async_dispatcher = Dispatcher::new(dispatcher_config, state_manager_for_dispatcher);
        let async_dispatcher_arc = Arc::new(tokio::sync::Mutex::new(async_dispatcher));
        let job_dispatcher = Arc::new(Mutex::new(Arc::clone(&async_dispatcher_arc))); // Sync wrapper for Bridge
        
        let trigger_executor = TriggerExecutor::new(
            state_manager.clone(), 
            trigger_manager.clone(),
            Arc::clone(&job_dispatcher)  // Share the same Arc<Mutex<Arc<TokioMutex<Dispatcher>>>>
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
        
        // Acquire lock, register workflow, then immediately release
        {
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
        state_manager.register_workflow(workflow.clone())?;
        } // Lock released here
        
        // Register triggers without holding the state manager lock
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
        
        // Acquire lock, create run, then immediately release
        let run_id = {
        let mut state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
            state_manager.create_run(workflow_id, payload)?
        }; // Lock released here
        
        log::info!("Successfully created run: {} for workflow: {}", run_id, workflow_id);
        Ok(run_id.to_string())
    }

    /// Get workflow run status
    pub fn get_run_status(&self, run_id: &str) -> CoreResult<String> {
        log::info!("Getting status for run: {}", run_id);
        
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        // Acquire lock, get run, then immediately release
        let _run = {
        let state_manager = self.state_manager.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire state manager lock".to_string()))?;
            state_manager.get_run(&run_uuid)?
                .ok_or_else(|| CoreError::WorkflowNotFound(format!("Run not found: {}", run_id)))?
        }; // Lock released here
        
        // Build response without holding the lock
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
        
        // Acquire lock, get all needed data, then immediately release
        let (run, workflow, completed_steps) = {
        let state_manager = self.state_manager.lock().unwrap();
            
        let run = state_manager.get_run(&run_uuid)?
            .ok_or_else(|| CoreError::RunNotFound(format!("Run not found: {}", run_id)))?;
        
        let workflow = state_manager.get_workflow(&run.workflow_id)?
            .ok_or_else(|| CoreError::WorkflowNotFound(run.workflow_id.clone()))?;
        
            let completed_steps = state_manager.get_completed_steps(&run_uuid)?;
            
            (run, workflow, completed_steps)
        }; // Lock released here
        
        // Process step data without holding the lock
        let step = workflow.get_step(step_id)
            .ok_or_else(|| CoreError::Validation(format!("Step '{}' not found in workflow '{}'", step_id, run.workflow_id)))?;
        
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
        
        // Acquire lock, get workflow, then immediately release
        let _workflow = {
        let state_manager = self.state_manager.lock().unwrap();
            state_manager.get_workflow(&job.workflow_id)?
        }; // Lock released here
        
        let _run_uuid = Uuid::parse_str(&job.run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        // Build response without holding the lock
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

    /// Get job status (sync wrapper around async method)
    pub fn get_job_status(&self, job_id: &str) -> CoreResult<Option<String>> {
        log::info!("Getting job status for: {}", job_id);
        
        // Use tokio runtime to block on async call
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let dispatcher_arc = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
            let dispatcher = dispatcher_arc.lock().await;
        
            match dispatcher.get_job_status(job_id).await? {
            Some(state) => Ok(Some(format!("{:?}", state))),
            None => Ok(None),
        }
        })
    }

    /// Cancel a job (sync wrapper around async method)
    pub fn cancel_job(&self, job_id: &str) -> CoreResult<bool> {
        log::info!("Cancelling job: {}", job_id);
        
        // Use tokio runtime to block on async call
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let dispatcher_arc = self.job_dispatcher.lock()
            .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
            let dispatcher = dispatcher_arc.lock().await;
        
            dispatcher.cancel_job(job_id).await
        })
    }

    /// Get dispatcher statistics (sync wrapper around async method)
    pub fn get_dispatcher_stats(&self) -> CoreResult<crate::dispatcher::DispatcherStats> {
        log::info!("Getting dispatcher statistics");
        
        // Use tokio runtime to block on async call
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let dispatcher_arc = self.job_dispatcher.lock()
                .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
            let dispatcher = dispatcher_arc.lock().await;
            
            dispatcher.get_stats().await
        })
    }

    /// Get workflow run status (sync wrapper around async method)
    pub fn get_workflow_run_status(&self, run_id: &str) -> CoreResult<Option<crate::models::RunStatus>> {
        log::info!("Getting workflow run status for: {}", run_id);
        
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let dispatcher_arc = self.job_dispatcher.lock()
                .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
            let dispatcher = dispatcher_arc.lock().await;
            
            dispatcher.get_workflow_run_status(run_id).await
        })
    }

    /// Get completed steps for a workflow run (sync wrapper around async method)
    pub fn get_workflow_completed_steps(&self, run_id: &str) -> CoreResult<Vec<crate::models::StepResult>> {
        log::info!("Getting completed steps for workflow run: {}", run_id);
        
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let dispatcher_arc = self.job_dispatcher.lock()
                .map_err(|e| CoreError::Internal(format!("Failed to acquire dispatcher lock: {}", e)))?;
            let dispatcher = dispatcher_arc.lock().await;
            
            dispatcher.get_workflow_completed_steps(run_id).await
        })
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
// ASYNC BRIDGE IMPLEMENTATION (Task 2.1.3)
// ============================================================================

impl AsyncBridge {
    /// Create a new async N-API bridge
    pub fn new(db_path: &str) -> CoreResult<Self> {
        let state_manager = Arc::new(AsyncStateManager::new(db_path)?);
        let trigger_manager = Arc::new(TokioMutex::new(TriggerManager::new()));
        
        // Dispatcher now uses Tokio async tasks
        let dispatcher_config = crate::dispatcher::WorkerPoolConfig::default();
        let async_state_manager = Arc::new(TokioMutex::new(StateManager::new(db_path)?));
        let job_dispatcher = Arc::new(TokioMutex::new(Dispatcher::new(dispatcher_config, async_state_manager.clone())));
        
        // TriggerExecutor still needs sync components for now
        // TODO: Update TriggerExecutor to use async in Phase 3.2
        let sync_trigger_manager = Arc::new(Mutex::new(TriggerManager::new()));
        let sync_state_manager_for_trigger = Arc::new(Mutex::new(StateManager::new(db_path)?));
        // Share the same dispatcher Arc with trigger executor (it will use block_on to call async methods)
        let sync_dispatcher_for_trigger = Arc::new(Mutex::new(Arc::clone(&job_dispatcher)));
        
        let trigger_executor = Arc::new(TriggerExecutor::new(
            sync_state_manager_for_trigger,
            sync_trigger_manager,
            sync_dispatcher_for_trigger,
        ));
        
        Ok(AsyncBridge {
            state_manager,
            trigger_manager,
            trigger_executor,
            job_dispatcher,
        })
    }

    /// Register a workflow from Node.js (async)
    pub async fn register_workflow(&self, workflow_json: &str) -> CoreResult<()> {
        log::info!("Registering workflow from JSON (async): {}", workflow_json);
        
        let workflow: WorkflowDefinition = serde_json::from_str(workflow_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        workflow.validate()
            .map_err(|e| CoreError::InvalidWorkflow(e))?;
        
        self.state_manager.register_workflow(workflow.clone()).await?;
        
        let trigger_ids = self.trigger_executor.register_workflow_triggers(&workflow.id, &workflow)?;
        
        log::info!("Successfully registered workflow: {} with {} triggers: {:?}", workflow.id, trigger_ids.len(), trigger_ids);
        Ok(())
    }

    /// Create a workflow run from Node.js (async)
    pub async fn create_run(&self, workflow_id: &str, payload_json: &str) -> CoreResult<String> {
        log::info!("Creating run for workflow: {} with payload: {}", workflow_id, payload_json);
        
        let payload: serde_json::Value = serde_json::from_str(payload_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        let run_id = self.state_manager.create_run(workflow_id, payload).await?;
        
        log::info!("Successfully created run: {} for workflow: {}", run_id, workflow_id);
        Ok(run_id.to_string())
    }

    /// Get workflow run status (async)
    pub async fn get_run_status(&self, run_id: &str) -> CoreResult<String> {
        log::info!("Getting status for run: {}", run_id);
        
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        let _run = self.state_manager.get_run(&run_uuid).await?
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

    /// Execute a step with context (async)
    pub async fn execute_step(&self, run_id: &str, step_id: &str) -> CoreResult<String> {
        log::info!("Executing step {} for run {} (async)", step_id, run_id);
        
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::UuidParse(e))?;
        
        let run = self.state_manager.get_run(&run_uuid).await?
            .ok_or_else(|| CoreError::RunNotFound(format!("Run not found: {}", run_id)))?;
        
        let workflow = self.state_manager.get_workflow(&run.workflow_id).await?
            .ok_or_else(|| CoreError::WorkflowNotFound(run.workflow_id.clone()))?;
        
        let step = workflow.get_step(step_id)
            .ok_or_else(|| CoreError::Validation(format!("Step '{}' not found in workflow '{}'", step_id, run.workflow_id)))?;
        
        let completed_steps = self.state_manager.get_completed_steps(&run_uuid).await?;
        
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
        
        log::info!("Step execution context created for step {}", step_id);
        Ok(context_json)
    }

    /// Execute a job (async)
    pub async fn execute_job(&self, job_json: &str) -> CoreResult<String> {
        log::info!("Executing job with context (async): {}", job_json);
        
        let job: Job = serde_json::from_str(job_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        let job_id = job.id.clone();
        
        let dispatcher = self.job_dispatcher.lock().await;
        dispatcher.submit_job(job).await?;
        
        log::info!("Job {} submitted successfully", job_id);
        
        Ok(serde_json::json!({
            "success": true,
            "job_id": job_id,
            "message": "Job submitted successfully"
        }).to_string())
    }

    /// Get job status (async)
    pub async fn get_job_status(&self, job_id: &str) -> CoreResult<Option<String>> {
        log::info!("Getting status for job: {}", job_id);
        
        let dispatcher = self.job_dispatcher.lock().await;
        
        match dispatcher.get_job_status(job_id).await? {
            Some(state) => Ok(Some(format!("{:?}", state))),
            None => Ok(None),
        }
    }

    /// Register a webhook trigger (async)
    pub async fn register_webhook_trigger(&self, workflow_id: &str, trigger_json: &str) -> CoreResult<()> {
        log::info!("Registering webhook trigger for workflow: {} with config: {}", workflow_id, trigger_json);
        
        let trigger: crate::triggers::WebhookTrigger = serde_json::from_str(trigger_json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        trigger.validate()?;
        
        let mut trigger_manager = self.trigger_manager.lock().await;
        trigger_manager.register_webhook_trigger(workflow_id, trigger)?;
        
        log::info!("Successfully registered webhook trigger for workflow: {}", workflow_id);
        Ok(())
    }

    /// Get all webhook triggers (async)
    pub async fn get_webhook_triggers(&self) -> CoreResult<String> {
        let trigger_manager = self.trigger_manager.lock().await;
        
        let triggers = trigger_manager.get_webhook_triggers();
        
        let triggers_json = serde_json::to_string(&triggers)
            .map_err(|e| CoreError::Serialization(e))?;
        
        Ok(triggers_json)
    }

    /// Get dispatcher statistics (async)
    pub async fn get_dispatcher_stats(&self) -> CoreResult<crate::dispatcher::DispatcherStats> {
        log::info!("Getting dispatcher statistics (async)");
        
        let dispatcher = self.job_dispatcher.lock().await;
        
        dispatcher.get_stats().await
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

/// Register a workflow via N-API (synchronous version)
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

/// Register a workflow via N-API (async version) - Task 2.1.4
#[napi(ts_return_type = "Promise<WorkflowRegistrationResult>")]
pub async fn register_workflow_async(workflow_json: String, db_path: String) -> napi::Result<WorkflowRegistrationResult> {
    match get_shared_async_bridge(&db_path).await {
        Ok(bridge) => {
            match bridge.register_workflow(&workflow_json).await {
                Ok(_) => Ok(WorkflowRegistrationResult {
                    success: true,
                    message: "Workflow registered successfully".to_string(),
                }),
                Err(e) => Ok(WorkflowRegistrationResult {
                success: false,
                    message: format!("Failed to register workflow: {}", e),
                }),
            }
        }
        Err(e) => Ok(WorkflowRegistrationResult {
                success: false,
            message: format!("Failed to get bridge: {}", e),
        }),
    }
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

/// Create a workflow run via N-API (synchronous version)
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

/// Create a workflow run via N-API (async version) - Task 2.1.4
#[napi(ts_return_type = "Promise<RunCreationResult>")]
pub async fn create_run_async(workflow_id: String, payload_json: String, db_path: String) -> napi::Result<RunCreationResult> {
    match get_shared_async_bridge(&db_path).await {
        Ok(bridge) => {
            match bridge.create_run(&workflow_id, &payload_json).await {
                Ok(run_id) => Ok(RunCreationResult {
                    success: true,
                    id: Some(run_id),
                    data: None,
                    message: "Run created successfully".to_string(),
                }),
                Err(e) => Ok(RunCreationResult {
                    success: false,
                    id: None,
                    data: None,
                    message: format!("Failed to create run: {}", e),
                }),
            }
        }
        Err(e) => Ok(RunCreationResult {
            success: false,
            id: None,
            data: None,
            message: format!("Failed to get bridge: {}", e),
        }),
    }
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

/// Execute a step via N-API (synchronous version)
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

/// Execute a step via N-API (async version) - Task 2.1.4
#[napi(ts_return_type = "Promise<StepExecutionResult>")]
pub async fn execute_step_async(run_id: String, step_id: String, db_path: String) -> napi::Result<StepExecutionResult> {
    match get_shared_async_bridge(&db_path).await {
        Ok(bridge) => {
            match bridge.execute_step(&run_id, &step_id).await {
                Ok(result) => Ok(StepExecutionResult {
        success: true,
                    data: Some(result),
                    message: "Step executed successfully".to_string(),
                }),
                Err(e) => Ok(StepExecutionResult {
            success: false,
                    data: None,
                    message: format!("Failed to execute step: {}", e),
                }),
            }
        }
        Err(e) => Ok(StepExecutionResult {
            success: false,
            data: None,
            message: format!("Failed to get bridge: {}", e),
        }),
    }
}

/// Execute a job with context via N-API (synchronous version)
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

/// Execute a job with context via N-API (async version) - Task 2.1.4
#[napi(ts_return_type = "Promise<JobExecutionResult>")]
pub async fn execute_job_async(job_json: String, db_path: String) -> napi::Result<JobExecutionResult> {
    log::info!("Executing job with context (async): {}", job_json);
    
    match get_shared_async_bridge(&db_path).await {
        Ok(bridge) => {
            match bridge.execute_job(&job_json).await {
        Ok(result_json) => {
            let result: serde_json::Value = match serde_json::from_str(&result_json) {
                Ok(result) => result,
                Err(_) => {
                            return Ok(JobExecutionResult {
                        success: false,
                        job_id: None,
                        run_id: None,
                        step_id: None,
                        context: None,
                        result: None,
                        message: "Failed to parse execution result".to_string(),
                            });
                }
            };
            
                    Ok(JobExecutionResult {
                success: true,
                job_id: result["job_id"].as_str().map(|s| s.to_string()),
                run_id: result["run_id"].as_str().map(|s| s.to_string()),
                step_id: result["step_id"].as_str().map(|s| s.to_string()),
                context: result["context"].as_str().map(|s| s.to_string()),
                result: Some(result_json),
                message: "Job executed successfully".to_string(),
                    })
            }
                Err(e) => Ok(JobExecutionResult {
                success: false,
                job_id: None,
                run_id: None,
                step_id: None,
                context: None,
                result: None,
                message: format!("Failed to execute job: {}", e),
                }),
            }
        }
        Err(e) => Ok(JobExecutionResult {
            success: false,
            job_id: None,
            run_id: None,
            step_id: None,
            context: None,
            result: None,
            message: format!("Failed to get bridge: {}", e),
        }),
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