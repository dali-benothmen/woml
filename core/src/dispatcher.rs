use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::error::CoreError;
use crate::job::{Job, JobQueue, JobState};
use crate::models::{StepResult, StepStatus, WorkflowDefinition, WorkflowRun, RunStatus};
use crate::state::StateManager;
use serde_json;
use serde::Serialize;

/// Worker pool configuration
#[derive(Debug, Clone)]
pub struct WorkerPoolConfig {
    pub min_workers: usize,
    pub max_workers: usize,
    pub worker_timeout_ms: u64,
    pub queue_size: usize,
}

impl Default for WorkerPoolConfig {
    fn default() -> Self {
        // Use centralized configuration
        let core_config = crate::config::CoreConfig::default();
        Self {
            min_workers: core_config.worker_pool.min_workers,
            max_workers: core_config.worker_pool.max_workers,
            worker_timeout_ms: core_config.worker_pool.worker_timeout_ms,
            queue_size: core_config.worker_pool.queue_size,
        }
    }
}

/// Worker status
#[derive(Debug, Clone, PartialEq)]
pub enum WorkerStatus {
    Idle,
    Busy { job_id: String, started_at: DateTime<Utc> },
    Stopping,
    Stopped,
}

/// Worker information
#[derive(Debug, Clone)]
pub struct Worker {
    pub id: String,
    pub status: WorkerStatus,
    pub jobs_processed: u64,
    pub total_processing_time_ms: u64,
    pub last_activity: DateTime<Utc>,
}

impl Worker {
    pub fn new(id: String) -> Self {
        Self {
            id,
            status: WorkerStatus::Idle,
            jobs_processed: 0,
            total_processing_time_ms: 0,
            last_activity: Utc::now(),
        }
    }

    pub fn start_job(&mut self, job_id: String) {
        self.status = WorkerStatus::Busy { 
            job_id, 
            started_at: Utc::now() 
        };
        self.last_activity = Utc::now();
    }

    pub fn finish_job(&mut self, processing_time_ms: u64) {
        self.status = WorkerStatus::Idle;
        self.jobs_processed += 1;
        self.total_processing_time_ms += processing_time_ms;
        self.last_activity = Utc::now();
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.status, WorkerStatus::Idle)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.status, WorkerStatus::Busy { .. })
    }

    pub fn get_current_job_id(&self) -> Option<String> {
        match &self.status {
            WorkerStatus::Busy { job_id, .. } => Some(job_id.clone()),
            _ => None,
        }
    }
}

/// Job execution result
#[derive(Debug, Clone)]
pub struct JobExecutionResult {
    pub job_id: String,
    pub success: bool,
    pub result: Option<StepResult>,
    pub error: Option<String>,
    pub processing_time_ms: u64,
    pub worker_id: String,
}

/// Dispatcher statistics
#[derive(Debug, Clone, Default, Serialize)]
pub struct DispatcherStats {
    pub total_jobs_processed: u64,
    pub successful_jobs: u64,
    pub failed_jobs: u64,
    pub timed_out_jobs: u64,
    pub average_processing_time_ms: u64,
    pub active_workers: usize,
    pub idle_workers: usize,
    pub queue_depth: usize,
}

/// Job dispatcher for managing workflow job execution
pub struct Dispatcher {
    job_queue: Arc<Mutex<JobQueue>>,
    workers: Arc<Mutex<HashMap<String, Worker>>>,
    config: WorkerPoolConfig,
    stats: Arc<Mutex<DispatcherStats>>,
    completed_jobs: Arc<Mutex<Vec<String>>>,
    running_jobs: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    shutdown_flag: Arc<Mutex<bool>>,
    state_manager: Arc<Mutex<StateManager>>, // Added for workflow state updates
    worker_handles: Arc<Mutex<Vec<JoinHandle<()>>>>, // Track tokio task handles
}

impl Dispatcher {
    /// Create a new job dispatcher
    pub fn new(config: WorkerPoolConfig, state_manager: Arc<Mutex<StateManager>>) -> Self {
        Self {
            job_queue: Arc::new(Mutex::new(JobQueue::new())),
            workers: Arc::new(Mutex::new(HashMap::new())),
            config,
            stats: Arc::new(Mutex::new(DispatcherStats::default())),
            completed_jobs: Arc::new(Mutex::new(Vec::new())),
            running_jobs: Arc::new(Mutex::new(HashMap::new())),
            shutdown_flag: Arc::new(Mutex::new(false)),
            state_manager,
            worker_handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start the dispatcher with worker pool
    pub async fn start(&mut self) -> Result<(), CoreError> {
        log::info!("Starting job dispatcher with {} workers", self.config.min_workers);
        
        // Start worker pool
        for i in 0..self.config.min_workers {
            let worker_id = format!("worker-{}", i);
            let shutdown_flag = Arc::clone(&self.shutdown_flag);
            self.start_worker(worker_id, shutdown_flag).await?;
        }
        
        // Start timeout monitor
        let shutdown_flag = Arc::clone(&self.shutdown_flag);
        self.start_timeout_monitor(shutdown_flag).await?;
        
        log::info!("Job dispatcher started successfully");
        Ok(())
    }

    /// Stop the dispatcher
    pub async fn stop(&mut self) -> Result<(), CoreError> {
        log::info!("Stopping job dispatcher");
        
        // Set shutdown flag
        {
            let mut flag = self.shutdown_flag.lock().await;
            *flag = true;
        }
        
        // Mark workers as stopping
        {
            let mut workers = self.workers.lock().await;
            for worker in workers.values_mut() {
                worker.status = WorkerStatus::Stopping;
            }
        }
        
        // Wait for all worker tasks to complete (with timeout)
        let mut handles = self.worker_handles.lock().await;
        let timeout_duration = Duration::from_secs(5);
        
        for handle in handles.drain(..) {
            // Try to wait for task completion with timeout
            match tokio::time::timeout(timeout_duration, handle).await {
                Ok(_) => log::debug!("Worker task completed gracefully"),
                Err(_) => log::warn!("Worker task did not complete within timeout"),
            }
        }
        
        log::info!("Job dispatcher stopped");
        Ok(())
    }

    /// Submit a job for execution
    pub async fn submit_job(&self, job: Job) -> Result<(), CoreError> {
        let job_id = job.id.clone();
        log::info!("Submitting job {} for execution", job_id);
        
        let queue_depth = {
            let mut queue = self.job_queue.lock().await;
            queue.enqueue(job)?;
            queue.get_jobs().len()
        }; // Release lock here
        
        // Update stats without holding queue lock
        {
            let mut stats = self.stats.lock().await;
            stats.queue_depth = queue_depth;
        }
        
        log::info!("Job {} submitted successfully", job_id);
        Ok(())
    }

    /// Get dispatcher statistics
    pub async fn get_stats(&self) -> Result<DispatcherStats, CoreError> {
        // Gather data from each lock scope separately to minimize lock duration
        let stats_clone = {
            let stats = self.stats.lock().await;
            stats.clone()
        };
        
        let queue_depth = {
            let queue = self.job_queue.lock().await;
            queue.get_jobs().len()
        };
        
        let (active_workers, idle_workers) = {
            let workers = self.workers.lock().await;
            let active = workers.values().filter(|w| w.is_busy()).count();
            let idle = workers.values().filter(|w| w.is_idle()).count();
            (active, idle)
        };
        
        let mut result = stats_clone;
        result.queue_depth = queue_depth;
        result.active_workers = active_workers;
        result.idle_workers = idle_workers;
        
        Ok(result)
    }

    /// Get job status
    pub async fn get_job_status(&self, job_id: &str) -> Result<Option<JobState>, CoreError> {
        // Check queue first
        {
            let queue = self.job_queue.lock().await;
            if let Some(job) = queue.get_job(job_id) {
                return Ok(Some(job.state.clone()));
            }
        } // Release queue lock
        
        // Check completed jobs
        let completed = self.completed_jobs.lock().await;
        if completed.contains(&job_id.to_string()) {
            Ok(Some(JobState::Completed))
        } else {
            Ok(None)
        }
    }

    /// Cancel a job
    pub async fn cancel_job(&self, job_id: &str) -> Result<bool, CoreError> {
        log::info!("Cancelling job {}", job_id);
        
        let mut queue = self.job_queue.lock().await;
        
        if let Some(job) = queue.get_job_mut(job_id) {
            job.cancel()?;
            log::info!("Job {} cancelled successfully", job_id);
            Ok(true)
        } else {
            log::warn!("Job {} not found for cancellation", job_id);
            Ok(false)
        }
    }

    /// Start a worker task (async)
    async fn start_worker(&self, worker_id: String, shutdown_flag: Arc<Mutex<bool>>) -> Result<(), CoreError> {
        let job_queue = Arc::clone(&self.job_queue);
        let workers = Arc::clone(&self.workers);
        let stats = Arc::clone(&self.stats);
        let completed_jobs = Arc::clone(&self.completed_jobs);
        let running_jobs = Arc::clone(&self.running_jobs);
        let state_manager = Arc::clone(&self.state_manager);
        let worker_handles = Arc::clone(&self.worker_handles);
        
        // Initialize worker in the workers map
        {
            let mut workers_guard = workers.lock().await;
            workers_guard.insert(worker_id.clone(), Worker::new(worker_id.clone()));
        }
        
        // Spawn async worker task
        let handle = tokio::spawn(async move {
            log::info!("Worker {} started", worker_id);
            
            loop {
                // Check shutdown flag
                {
                    let flag = shutdown_flag.lock().await;
                    if *flag {
                        log::info!("Worker {} received shutdown signal", worker_id);
                        break;
                    }
                } // Lock released here
                
                // Try to get a job (minimize lock duration)
                let job = {
                    let mut queue = job_queue.lock().await;
                    let completed = completed_jobs.lock().await;
                    queue.dequeue(&completed)
                }; // Locks released here
                
                if let Some(mut job) = job {
                    // Update worker status
                    {
                        let mut workers_guard = workers.lock().await;
                        if let Some(worker) = workers_guard.get_mut(&worker_id) {
                            worker.start_job(job.id.clone());
                        }
                    }
                    
                    // Track running job
                    {
                        let mut running = running_jobs.lock().await;
                        running.insert(job.id.clone(), Utc::now());
                    }
                    
                    let job_id_clone = job.id.clone();
                    log::info!("Worker {} processing job {}", worker_id, job_id_clone);
                    
                    // Process the job (use spawn_blocking for potentially CPU-intensive work)
                    let start_time = Instant::now();
                    let state_manager_clone = Arc::clone(&state_manager);
                    
                    let (result, mut job_back) = tokio::task::spawn_blocking(move || {
                        let result = Self::process_job(&mut job);
                        (result, job)
                    }).await.unwrap_or_else(|e| {
                        log::error!("Worker task panicked: {:?}", e);
                        // Create a dummy job for error case
                        let dummy_job = Job {
                            id: job_id_clone.clone(),
                            workflow_id: String::new(),
                            run_id: String::new(),
                            step_name: String::new(),
                            payload: serde_json::Value::Null,
                            state: JobState::Failed,
                            priority: crate::job::JobPriority::Normal,
                            result: None,
                            retry_config: Default::default(),
                            metadata: Default::default(),
                            dependencies: vec![],
                            timeout_ms: None,
                            context: std::collections::HashMap::new(),
                        };
                        (Err(CoreError::Internal("Worker task panicked".to_string())), dummy_job)
                    });
                    
                    let processing_time = start_time.elapsed().as_millis() as u64;
                    let success = result.is_ok();
                    
                    // Clone job_id for logging
                    let job_id_for_logging = job_back.id.clone();
                    let job_id_final = job_back.id.clone();
                    
                    // Process result or handle failure in spawn_blocking to avoid blocking async runtime
                    tokio::task::spawn_blocking(move || {
                        if let Ok(step_result) = result {
                            let _ = job_back.complete(step_result.clone());
                            // Process the job result
                            if let Err(e) = Self::process_job_result_internal(&state_manager_clone, &job_back, &step_result) {
                                log::error!("Failed to process job result for {}: {}", job_id_final, e);
                            }
                        } else {
                            let error = result.err().unwrap().to_string();
                            let _ = job_back.fail(error.clone());
                            // Handle job failure
                            if let Err(e) = Self::handle_job_failure_internal(&state_manager_clone, &mut job_back, &error) {
                                log::error!("Failed to handle job failure for {}: {}", job_id_final, e);
                            }
                        }
                    }).await.unwrap_or_else(|e| {
                        log::error!("Failed to process job result/failure: {:?}", e);
                    });
                    
                    // Update worker status
                    {
                        let mut workers_guard = workers.lock().await;
                        if let Some(worker) = workers_guard.get_mut(&worker_id) {
                            worker.finish_job(processing_time);
                        }
                    }
                    
                    // Mark job as completed
                    {
                        let mut completed = completed_jobs.lock().await;
                        completed.push(job_id_for_logging.clone());
                    }
                    
                    // Remove from running jobs
                    {
                        let mut running = running_jobs.lock().await;
                        running.remove(&job_id_for_logging);
                    }
                    
                    // Update statistics
                    {
                        let mut stats_guard = stats.lock().await;
                        stats_guard.total_jobs_processed += 1;
                        if success {
                            stats_guard.successful_jobs += 1;
                        } else {
                            stats_guard.failed_jobs += 1;
                        }
                        
                        let total_time = stats_guard.average_processing_time_ms * (stats_guard.total_jobs_processed - 1) + processing_time;
                        stats_guard.average_processing_time_ms = total_time / stats_guard.total_jobs_processed;
                    }
                    
                    log::info!("Worker {} completed job {} in {}ms", worker_id, job_id_for_logging, processing_time);
                } else {
                    // No job available, yield and sleep briefly
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            
            log::info!("Worker {} stopped", worker_id);
        });
        
        // Store the task handle
        {
            let mut handles = worker_handles.lock().await;
            handles.push(handle);
        }
        
        Ok(())
    }

    /// Start timeout monitor (async)
    async fn start_timeout_monitor(&self, shutdown_flag: Arc<Mutex<bool>>) -> Result<(), CoreError> {
        let job_queue = Arc::clone(&self.job_queue);
        let running_jobs = Arc::clone(&self.running_jobs);
        let stats = Arc::clone(&self.stats);
        let config = self.config.clone();
        let worker_handles = Arc::clone(&self.worker_handles);
        
        // Spawn async timeout monitor task
        let handle = tokio::spawn(async move {
            log::info!("Timeout monitor started");
            
            // Use tokio::time::interval for more efficient periodic checking
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            
            loop {
                // Check shutdown flag
                {
                    let flag = shutdown_flag.lock().await;
                    if *flag {
                        log::info!("Timeout monitor received shutdown signal");
                        break;
                    }
                }
                
                // Wait for next interval tick
                interval.tick().await;
                
                // Find timed out jobs
                let timed_out_jobs = {
                    let queue = job_queue.lock().await;
                    let running = running_jobs.lock().await;
                    let now = Utc::now();
                    
                    queue.get_jobs()
                        .iter()
                        .filter(|job| {
                            if let Some(started_at) = running.get(&job.id) {
                                let elapsed = now.signed_duration_since(*started_at);
                                elapsed.num_milliseconds() as u64 > job.timeout_ms.unwrap_or(config.worker_timeout_ms)
                            } else {
                                false
                            }
                        })
                        .map(|job| job.id.clone())
                        .collect::<Vec<_>>()
                }; // Locks released here
                
                // Handle timed out jobs
                for job_id in timed_out_jobs {
                    log::warn!("Job {} timed out", job_id);
                    
                    // Fail the job
                    {
                        let mut queue = job_queue.lock().await;
                        if let Some(job) = queue.get_job_mut(&job_id) {
                            let _ = job.fail("Job timed out".to_string());
                        }
                    }
                    
                    // Update stats
                    {
                        let mut stats_guard = stats.lock().await;
                        stats_guard.timed_out_jobs += 1;
                    }
                    
                    // Remove from running jobs
                    {
                        let mut running = running_jobs.lock().await;
                        running.remove(&job_id);
                    }
                }
            }
            
            log::info!("Timeout monitor stopped");
        });
        
        // Store the task handle
        {
            let mut handles = worker_handles.lock().await;
            handles.push(handle);
        }
        
        Ok(())
    }

    /// Process a job (simplified version without bridge dependency)
    fn process_job(job: &mut Job) -> Result<StepResult, CoreError> {
        log::info!("Processing job: {}", job.id);
        
        // Simulate job processing
        let start_time = std::time::Instant::now();
        
        if let Some(should_fail) = job.payload.get("should_fail") {
            if should_fail.as_bool().unwrap_or(false) {
                log::warn!("Test job configured to fail: {}", job.id);
                return Err(CoreError::Internal("Test job failure".to_string()));
            }
        }
        
        // Simulate some processing time
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        let processing_time = start_time.elapsed();
        
        let step_result = StepResult {
            step_id: job.step_name.clone(),
            status: StepStatus::Completed,
            output: Some(serde_json::json!({
                "job_id": job.id,
                "step_name": job.step_name,
                "workflow_id": job.workflow_id,
                "run_id": job.run_id,
                "processing_time_ms": processing_time.as_millis(),
                "message": "Job processed successfully",
                "test_data": job.payload
            })),
            error: None,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(processing_time.as_millis() as u64),
        };
        
        log::info!("Job {} processed successfully in {}ms", job.id, processing_time.as_millis());
        Ok(step_result)
    }

    /// Scale worker pool based on queue depth (async)
    pub async fn scale_workers(&self) -> Result<(), CoreError> {
        let queue = self.job_queue.lock().await;
        
        let workers = self.workers.lock().await;
        
        let queue_depth = queue.get_jobs().len();
        let active_workers = workers.values().filter(|w| w.is_busy()).count();
        let total_workers = workers.len();
        
        // Scale up if queue is deep and we have capacity
        if queue_depth > active_workers * 2 && total_workers < self.config.max_workers {
            log::info!("Scaling up workers: queue_depth={}, active_workers={}", queue_depth, active_workers);
        }
        
        // Scale down if queue is empty and we have excess workers
        if queue_depth == 0 && total_workers > self.config.min_workers {
            log::info!("Scaling down workers: queue_depth={}, total_workers={}", queue_depth, total_workers);
        }
        
        Ok(())
    }

    /// Process completed job results and update workflow state
    pub fn process_job_result(&self, job: &Job, step_result: &StepResult) -> Result<(), CoreError> {
        log::info!("Processing result for job: {} (step: {})", job.id, job.step_name);
        
        let (workflow_id, run_id, step_id) = Job::parse_job_id(&job.id)?;
        let run_uuid = uuid::Uuid::parse_str(&run_id)
            .map_err(|e| CoreError::Validation(format!("Invalid run ID: {}", e)))?;
        
        self.update_workflow_state(&workflow_id, &run_uuid, step_result)?;
        
        self.check_workflow_completion(&workflow_id, &run_uuid)?;
        
        // Determine next steps to execute
        self.determine_next_steps(&workflow_id, &run_uuid)?;
        
        log::info!("Successfully processed result for job: {}", job.id);
        Ok(())
    }

    /// Update workflow state with step result (sync wrapper for spawn_blocking)
    fn update_workflow_state(&self, workflow_id: &str, run_id: &uuid::Uuid, step_result: &StepResult) -> Result<(), CoreError> {
        // Use tokio runtime handle to bridge sync->async
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let mut state_manager = self.state_manager.lock().await;
            
            // Save the step result
            state_manager.save_step_result(run_id, step_result.clone())?;
            
            if let Some(run) = state_manager.get_run(run_id)? {
                if run.status == RunStatus::Pending {
                    state_manager.update_run_status(run_id, RunStatus::Running)?;
                }
            }
            
            log::debug!("Updated workflow state for run: {} step: {}", run_id, step_result.step_id);
            Ok::<(), CoreError>(())
        })
    }

    /// Check if workflow run is complete (sync wrapper for spawn_blocking)
    fn check_workflow_completion(&self, workflow_id: &str, run_id: &Uuid) -> Result<(), CoreError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let mut state_manager = self.state_manager.lock().await;
            
            let workflow = state_manager.get_workflow(workflow_id)?
                .ok_or_else(|| CoreError::WorkflowNotFound(workflow_id.to_string()))?;
            
            let completed_steps = state_manager.get_completed_steps(run_id)?;
            
            let all_steps_completed = workflow.steps.iter().all(|step| {
                completed_steps.iter().any(|result| result.step_id == step.id)
            });
            
            if all_steps_completed {
                let has_failures = completed_steps.iter().any(|result| {
                    matches!(result.status, StepStatus::Failed)
                });
                
                let final_status = if has_failures {
                    RunStatus::Failed
                } else {
                    RunStatus::Completed
                };
                
                let error_message = if has_failures {
                    let failed_steps: Vec<_> = completed_steps.iter()
                        .filter(|result| matches!(result.status, StepStatus::Failed))
                        .map(|result| format!("{}: {}", result.step_id, result.error.as_deref().unwrap_or("Unknown error")))
                        .collect();
                    Some(format!("Workflow failed: {}", failed_steps.join(", ")))
                } else {
                    None
                };
                
                let run = state_manager.get_run(run_id)?
                    .ok_or_else(|| CoreError::Internal("Run not found".to_string()))?;
                
                let completion_context = crate::models::WorkflowCompletionContext::new(
                    run_id.to_string(),
                    workflow_id.to_string(),
                    final_status.clone(),
                    completed_steps.clone(),
                    error_message.clone(),
                    run.started_at,
                    chrono::Utc::now(),
                    run.payload.clone(),
                );
                
                // Execute hooks (for now, just log - will be implemented in Phase 3)
                log::info!("Workflow {} completed with status: {:?}", workflow_id, final_status);
                log::info!("Completion context: {:?}", completion_context);
                
                state_manager.complete_run(run_id, final_status.clone(), error_message)?;
                log::info!("Workflow run {} completed with status: {:?}", run_id, final_status);
            }
            
            Ok::<(), CoreError>(())
        })
    }

    /// Determine next steps to execute based on dependencies (sync wrapper for spawn_blocking)
    fn determine_next_steps(&self, workflow_id: &str, run_id: &uuid::Uuid) -> Result<(), CoreError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            let state_manager = self.state_manager.lock().await;
            
            let workflow = state_manager.get_workflow(workflow_id)?
                .ok_or_else(|| CoreError::WorkflowNotFound(workflow_id.to_string()))?;
            
            let completed_steps = state_manager.get_completed_steps(run_id)?;
            let completed_step_ids: Vec<_> = completed_steps.iter().map(|s| s.step_id.clone()).collect();
            
            let ready_steps: Vec<_> = workflow.steps.iter()
                .filter(|step| {
                    // Skip already completed steps
                    if completed_step_ids.contains(&step.id) {
                        return false;
                    }
                    
                    step.depends_on.iter().all(|dep_id| {
                        completed_step_ids.contains(dep_id)
                    })
                })
                .collect();
            
            if !ready_steps.is_empty() {
                log::info!("Steps ready for execution in workflow {} run {}: {:?}", 
                    workflow_id, run_id, 
                    ready_steps.iter().map(|s| s.id.clone()).collect::<Vec<_>>()
                );
            }
            
            Ok::<(), CoreError>(())
        })
    }

    /// Handle job failure and retry logic (sync wrapper for spawn_blocking)
    pub fn handle_job_failure(&self, job: &mut Job, error: &str) -> Result<(), CoreError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            log::warn!("Handling failure for job: {} - {}", job.id, error);
            
            if job.can_retry() {
                log::info!("Retrying job: {} (attempt {}/{})", 
                    job.id, job.metadata.attempt_count + 1, job.retry_config.max_attempts);
                
                job.retry()?;
                
                // Re-queue the job for retry
                let mut queue = self.job_queue.lock().await;
                
                // Remove from completed jobs if it was there
                let mut completed = self.completed_jobs.lock().await;
                completed.retain(|id| id != &job.id);
                
                // Re-enqueue the job
                queue.enqueue(job.clone())?;
                
                log::info!("Job {} re-queued for retry", job.id);
            } else {
                log::error!("Job {} failed permanently after {} attempts", 
                    job.id, job.metadata.attempt_count);
                
                let step_result = StepResult {
                    step_id: job.step_name.clone(),
                    status: StepStatus::Failed,
                    output: None,
                    error: Some(error.to_string()),
                    started_at: job.metadata.started_at.unwrap_or_else(Utc::now),
                    completed_at: Some(Utc::now()),
                    duration_ms: job.metadata.completed_at.and_then(|completed| {
                        job.metadata.started_at.map(|started| {
                            (completed - started).num_milliseconds() as u64
                        })
                    }),
                };
                
                self.process_job_result(job, &step_result)?;
            }
            
            Ok::<(), CoreError>(())
        })
    }

    /// Get workflow run status (async)
    pub async fn get_workflow_run_status(&self, run_id: &str) -> Result<Option<RunStatus>, CoreError> {
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::Validation(format!("Invalid run ID: {}", e)))?;
        
        let state_manager = self.state_manager.lock().await;
        
        if let Some(run) = state_manager.get_run(&run_uuid)? {
            Ok(Some(run.status))
        } else {
            Ok(None)
        }
    }

    /// Get completed steps for a workflow run
    pub async fn get_workflow_completed_steps(&self, run_id: &str) -> Result<Vec<StepResult>, CoreError> {
        let run_uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|e| CoreError::Validation(format!("Invalid run ID: {}", e)))?;
        
        let state_manager = self.state_manager.lock().await;
        state_manager.get_completed_steps(&run_uuid)
    }

    /// Internal method to process job result (sync wrapper for spawn_blocking)
    fn process_job_result_internal(
        state_manager: &Arc<tokio::sync::Mutex<StateManager>>, 
        job: &Job, 
        step_result: &StepResult
    ) -> Result<(), CoreError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            log::info!("Processing result for job: {} (step: {})", job.id, job.step_name);
            
            let (workflow_id, run_id, _step_id) = Job::parse_job_id(&job.id)?;
            let run_uuid = uuid::Uuid::parse_str(&run_id)
                .map_err(|e| CoreError::Validation(format!("Invalid run ID: {}", e)))?;
            
            let mut state_manager_guard = state_manager.lock().await;
            
            // Save the step result
            state_manager_guard.save_step_result(&run_uuid, step_result.clone())?;
            
            if let Some(run) = state_manager_guard.get_run(&run_uuid)? {
                if run.status == RunStatus::Pending {
                    state_manager_guard.update_run_status(&run_uuid, RunStatus::Running)?;
                }
            }
            
            Self::check_workflow_completion_internal(&mut state_manager_guard, &workflow_id, &run_uuid)?;
            
            log::debug!("Updated workflow state for run: {} step: {}", run_uuid, step_result.step_id);
            Ok::<(), CoreError>(())
        })
    }

    /// Internal method to handle job failure (sync wrapper for spawn_blocking)
    fn handle_job_failure_internal(
        state_manager: &Arc<tokio::sync::Mutex<StateManager>>, 
        job: &mut Job, 
        error: &str
    ) -> Result<(), CoreError> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Internal("No tokio runtime available".to_string()))?;
        
        rt.block_on(async {
            log::warn!("Handling failure for job: {} - {}", job.id, error);
            
            if job.can_retry() {
                log::info!("Retrying job: {} (attempt {}/{})", 
                    job.id, job.metadata.attempt_count + 1, job.retry_config.max_attempts);
                
                job.retry()?;
            } else {
                log::error!("Job {} failed permanently after {} attempts", 
                    job.id, job.metadata.attempt_count);
                
                let step_result = StepResult {
                    step_id: job.step_name.clone(),
                    status: StepStatus::Failed,
                    output: None,
                    error: Some(error.to_string()),
                    started_at: job.metadata.started_at.unwrap_or_else(Utc::now),
                    completed_at: Some(Utc::now()),
                    duration_ms: job.metadata.completed_at.and_then(|completed| {
                        job.metadata.started_at.map(|started| {
                            (completed - started).num_milliseconds() as u64
                        })
                    }),
                };
                
                let mut state_manager_guard = state_manager.lock().await;
                
                let (workflow_id, run_id, _step_id) = Job::parse_job_id(&job.id)?;
                let run_uuid = uuid::Uuid::parse_str(&run_id)
                    .map_err(|e| CoreError::Validation(format!("Invalid run ID: {}", e)))?;
                
                // Save the step result
                state_manager_guard.save_step_result(&run_uuid, step_result.clone())?;
                
                Self::check_workflow_completion_internal(&mut state_manager_guard, &workflow_id, &run_uuid)?;
            }
            
            Ok::<(), CoreError>(())
        })
    }

    /// Internal method to check workflow completion (for worker threads)
    fn check_workflow_completion_internal(
        state_manager: &mut StateManager, 
        workflow_id: &str, 
        run_id: &Uuid
    ) -> Result<(), CoreError> {
        let workflow = state_manager.get_workflow(workflow_id)?
            .ok_or_else(|| CoreError::WorkflowNotFound(workflow_id.to_string()))?;
        
        let completed_steps = state_manager.get_completed_steps(run_id)?;
        
        let all_steps_completed = workflow.steps.iter().all(|step| {
            completed_steps.iter().any(|result| result.step_id == step.id)
        });
        
        if all_steps_completed {
            let has_failures = completed_steps.iter().any(|result| {
                matches!(result.status, StepStatus::Failed)
            });
            
            let final_status = if has_failures {
                RunStatus::Failed
            } else {
                RunStatus::Completed
            };
            
            let error_message = if has_failures {
                let failed_steps: Vec<_> = completed_steps.iter()
                    .filter(|result| matches!(result.status, StepStatus::Failed))
                    .map(|result| format!("{}: {}", result.step_id, result.error.as_deref().unwrap_or("Unknown error")))
                    .collect();
                Some(format!("Workflow failed: {}", failed_steps.join(", ")))
            } else {
                None
            };
            
            state_manager.complete_run(run_id, final_status.clone(), error_message)?;
            log::info!("Workflow run {} completed with status: {:?}", run_id, final_status);
        }
        
        Ok(())
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{Job, JobPriority};
    use serde_json::json;

    #[test]
    fn test_dispatcher_creation() {
        let config = WorkerPoolConfig::default();
        let state_manager = Arc::new(Mutex::new(StateManager::new("test_dispatcher.db").unwrap()));
        let dispatcher = Dispatcher::new(config, state_manager);
        
        assert_eq!(dispatcher.config.min_workers, 2);
        assert_eq!(dispatcher.config.max_workers, 10);
    }

    #[test]
    fn test_job_submission() {
        let config = WorkerPoolConfig::default();
        let state_manager = Arc::new(Mutex::new(StateManager::new("test_dispatcher.db").unwrap()));
        let dispatcher = Dispatcher::new(config, state_manager);
        
        let job = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-1".to_string(),
            json!({"test": "data"}),
            JobPriority::Normal,
        );
        
        assert!(dispatcher.submit_job(job).is_ok());
    }

    #[test]
    fn test_dispatcher_stats() {
        let config = WorkerPoolConfig::default();
        let state_manager = Arc::new(Mutex::new(StateManager::new("test_dispatcher.db").unwrap()));
        let dispatcher = Dispatcher::new(config, state_manager);
        
        let stats = dispatcher.get_stats().unwrap();
        assert_eq!(stats.total_jobs_processed, 0);
        assert_eq!(stats.successful_jobs, 0);
        assert_eq!(stats.failed_jobs, 0);
    }

    #[test]
    fn test_worker_creation() {
        let mut worker = Worker::new("test-worker".to_string());
        
        assert_eq!(worker.id, "test-worker");
        assert!(worker.is_idle());
        assert_eq!(worker.jobs_processed, 0);
        
        worker.start_job("job-1".to_string());
        assert!(worker.is_busy());
        assert_eq!(worker.get_current_job_id(), Some("job-1".to_string()));
        
        worker.finish_job(100);
        assert!(worker.is_idle());
        assert_eq!(worker.jobs_processed, 1);
        assert_eq!(worker.total_processing_time_ms, 100);
    }

    #[test]
    fn test_job_execution_flow() {
        let state_manager = Arc::new(Mutex::new(StateManager::new("test_job_execution_flow.db").unwrap()));
        let config = WorkerPoolConfig::default();
        let mut dispatcher = Dispatcher::new(config, state_manager);
        
        // Start the dispatcher
        dispatcher.start().unwrap();
        
        let job = Job::new(
            "test-workflow".to_string(),
            "test-run".to_string(),
            "test-step".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );
        
        println!("🧪 Test 1: Job submission");
        dispatcher.submit_job(job.clone()).unwrap();
        
        println!("🧪 Test 2: Verify job is in queue");
        let stats = dispatcher.get_stats().unwrap();
        assert_eq!(stats.queue_depth, 1);
        
        println!("🧪 Test 3: Wait for job execution");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        
        println!("🧪 Test 4: Verify job status");
        let job_status = dispatcher.get_job_status(&job.id).unwrap();
        assert!(job_status.is_some());
        
        println!("🧪 Test 5: Check dispatcher stats");
        let final_stats = dispatcher.get_stats().unwrap();
        assert!(final_stats.total_jobs_processed > 0);
        
        // Stop the dispatcher
        dispatcher.stop().unwrap();
        
        println!("✅ Job execution flow test completed successfully");
    }

    #[test]
    fn test_job_result_processing_flow() {
        let state_manager = Arc::new(Mutex::new(StateManager::new("test_job_result_processing_flow.db").unwrap()));
        let config = WorkerPoolConfig::default();
        let mut dispatcher = Dispatcher::new(config, state_manager);
        
        // Start the dispatcher
        dispatcher.start().unwrap();
        
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut job = Job::new(
            "test-workflow".to_string(),
            run_id.clone(),
            "test-step".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );
        
        // Submit the job
        dispatcher.submit_job(job.clone()).unwrap();
        
        // Wait for job to be processed
        std::thread::sleep(std::time::Duration::from_millis(1000));
        
        println!("🧪 Test 1: Verify job was processed");
        let stats = dispatcher.get_stats().unwrap();
        assert!(stats.total_jobs_processed > 0);
        
        println!("🧪 Test 2: Check job status");
        let job_status = dispatcher.get_job_status(&job.id).unwrap();
        assert!(job_status.is_some());
        
        println!("🧪 Test 3: Verify workflow run status");
        let run_status = dispatcher.get_workflow_run_status(&run_id).unwrap();
        assert!(run_status.is_some());
        
        // Stop the dispatcher
        dispatcher.stop().unwrap();
        
        println!("✅ Job result processing flow test completed successfully");
    }

    #[test]
    fn test_job_error_handling_flow() {
        let state_manager = Arc::new(Mutex::new(StateManager::new("test_job_error_handling_flow.db").unwrap()));
        let config = WorkerPoolConfig::default();
        let mut dispatcher = Dispatcher::new(config, state_manager);
        
        // Start the dispatcher
        dispatcher.start().unwrap();
        
        let mut job = Job::new(
            "test-workflow".to_string(),
            "test-run".to_string(),
            "test-step".to_string(),
            serde_json::json!({"test": "data", "should_fail": true}),
            JobPriority::Normal,
        );
        
        job.retry_config.max_attempts = 2;
        
        // Submit the job
        dispatcher.submit_job(job.clone()).unwrap();
        
        // Wait for job to be processed
        std::thread::sleep(std::time::Duration::from_millis(2000));
        
        println!("🧪 Test 1: Verify job failure was handled");
        let stats = dispatcher.get_stats().unwrap();
        assert!(stats.total_jobs_processed > 0);
        
        println!("🧪 Test 2: Check failed jobs count");
        assert!(stats.failed_jobs > 0);
        
        println!("🧪 Test 3: Verify job status after failure");
        let job_status = dispatcher.get_job_status(&job.id).unwrap();
        assert!(job_status.is_some());
        
        // Stop the dispatcher
        dispatcher.stop().unwrap();
        
        println!("✅ Job error handling flow test completed successfully");
    }
} 