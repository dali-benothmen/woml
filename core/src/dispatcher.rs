use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};

use crate::error::CoreError;
use crate::job::{Job, JobQueue, JobState};
use crate::models::{StepResult, StepStatus};
use serde_json;

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
        Self {
            min_workers: 2,
            max_workers: 10,
            worker_timeout_ms: 30000, // 30 seconds
            queue_size: 1000,
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
#[derive(Debug, Clone, Default)]
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
}

impl Dispatcher {
    /// Create a new job dispatcher
    pub fn new(config: WorkerPoolConfig) -> Self {
        Self {
            job_queue: Arc::new(Mutex::new(JobQueue::new())),
            workers: Arc::new(Mutex::new(HashMap::new())),
            config,
            stats: Arc::new(Mutex::new(DispatcherStats::default())),
            completed_jobs: Arc::new(Mutex::new(Vec::new())),
            running_jobs: Arc::new(Mutex::new(HashMap::new())),
            shutdown_flag: Arc::new(Mutex::new(false)),
        }
    }

    /// Start the dispatcher with worker pool
    pub fn start(&mut self) -> Result<(), CoreError> {
        log::info!("Starting job dispatcher with {} workers", self.config.min_workers);
        
        // Start worker pool
        for i in 0..self.config.min_workers {
            let worker_id = format!("worker-{}", i);
            let shutdown_flag = Arc::clone(&self.shutdown_flag);
            self.start_worker(worker_id, shutdown_flag)?;
        }
        
        // Start timeout monitor
        let shutdown_flag = Arc::clone(&self.shutdown_flag);
        self.start_timeout_monitor(shutdown_flag)?;
        
        log::info!("Job dispatcher started successfully");
        Ok(())
    }

    /// Stop the dispatcher
    pub fn stop(&mut self) -> Result<(), CoreError> {
        log::info!("Stopping job dispatcher");
        
        // Set shutdown flag
        {
            let mut flag = self.shutdown_flag.lock()
                .map_err(|_| CoreError::Internal("Failed to acquire shutdown flag lock".to_string()))?;
            *flag = true;
        }
        
        // Wait for workers to stop
        let mut workers = self.workers.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire workers lock".to_string()))?;
        
        for worker in workers.values_mut() {
            worker.status = WorkerStatus::Stopping;
        }
        
        log::info!("Job dispatcher stopped");
        Ok(())
    }

    /// Submit a job for execution
    pub fn submit_job(&self, job: Job) -> Result<(), CoreError> {
        let job_id = job.id.clone();
        log::info!("Submitting job {} for execution", job_id);
        
        let mut queue = self.job_queue.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire queue lock".to_string()))?;
        
        queue.enqueue(job)?;
        
        // Update stats
        let mut stats = self.stats.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire stats lock".to_string()))?;
        stats.queue_depth = queue.get_jobs().len();
        
        log::info!("Job {} submitted successfully", job_id);
        Ok(())
    }

    /// Get dispatcher statistics
    pub fn get_stats(&self) -> Result<DispatcherStats, CoreError> {
        let stats = self.stats.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire stats lock".to_string()))?;
        
        let queue = self.job_queue.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire queue lock".to_string()))?;
        
        let workers = self.workers.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire workers lock".to_string()))?;
        
        let mut result = stats.clone();
        result.queue_depth = queue.get_jobs().len();
        result.active_workers = workers.values().filter(|w| w.is_busy()).count();
        result.idle_workers = workers.values().filter(|w| w.is_idle()).count();
        
        Ok(result)
    }

    /// Get job status
    pub fn get_job_status(&self, job_id: &str) -> Result<Option<JobState>, CoreError> {
        let queue = self.job_queue.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire queue lock".to_string()))?;
        
        if let Some(job) = queue.get_job(job_id) {
            Ok(Some(job.state.clone()))
        } else {
            // Check if job is completed
            let completed = self.completed_jobs.lock()
                .map_err(|_| CoreError::Internal("Failed to acquire completed jobs lock".to_string()))?;
            
            if completed.contains(&job_id.to_string()) {
                Ok(Some(JobState::Completed))
            } else {
                Ok(None)
            }
        }
    }

    /// Cancel a job
    pub fn cancel_job(&self, job_id: &str) -> Result<bool, CoreError> {
        log::info!("Cancelling job {}", job_id);
        
        let mut queue = self.job_queue.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire queue lock".to_string()))?;
        
        if let Some(job) = queue.get_job_mut(job_id) {
            job.cancel()?;
            log::info!("Job {} cancelled successfully", job_id);
            Ok(true)
        } else {
            log::warn!("Job {} not found for cancellation", job_id);
            Ok(false)
        }
    }

    /// Start a worker thread
    fn start_worker(&self, worker_id: String, shutdown_flag: Arc<Mutex<bool>>) -> Result<(), CoreError> {
        let job_queue = Arc::clone(&self.job_queue);
        let workers = Arc::clone(&self.workers);
        let stats = Arc::clone(&self.stats);
        let completed_jobs = Arc::clone(&self.completed_jobs);
        let running_jobs = Arc::clone(&self.running_jobs);
        
        // Add worker to pool
        {
            let mut workers_guard = workers.lock()
                .map_err(|_| CoreError::Internal("Failed to acquire workers lock".to_string()))?;
            workers_guard.insert(worker_id.clone(), Worker::new(worker_id.clone()));
        }
        
        // Start worker thread
        thread::spawn(move || {
            log::info!("Worker {} started", worker_id);
            
            loop {
                // Check for shutdown signal
                {
                    let flag = shutdown_flag.lock().unwrap();
                    if *flag {
                        log::info!("Worker {} received shutdown signal", worker_id);
                        break;
                    }
                }
                
                // Try to get a job
                let job = {
                    let mut queue = job_queue.lock().unwrap();
                    let completed = completed_jobs.lock().unwrap();
                    queue.dequeue(&completed)
                };
                
                if let Some(mut job) = job {
                    // Update worker status
                    {
                        let mut workers_guard = workers.lock().unwrap();
                        if let Some(worker) = workers_guard.get_mut(&worker_id) {
                            worker.start_job(job.id.clone());
                        }
                    }
                    
                    // Track running job
                    {
                        let mut running = running_jobs.lock().unwrap();
                        running.insert(job.id.clone(), Utc::now());
                    }
                    
                    log::info!("Worker {} processing job {}", worker_id, job.id);
                    
                    // Process the job
                    let start_time = Instant::now();
                    let result = Self::process_job(&mut job);
                    let processing_time = start_time.elapsed().as_millis() as u64;
                    
                    // Update job with result
                    let success = result.is_ok();
                    if let Ok(step_result) = result {
                        let _ = job.complete(step_result);
                    } else {
                        let error = result.err().unwrap().to_string();
                        let _ = job.fail(error);
                    }
                    
                    // Update worker status
                    {
                        let mut workers_guard = workers.lock().unwrap();
                        if let Some(worker) = workers_guard.get_mut(&worker_id) {
                            worker.finish_job(processing_time);
                        }
                    }
                    
                    // Update completed jobs
                    {
                        let mut completed = completed_jobs.lock().unwrap();
                        completed.push(job.id.clone());
                    }
                    
                    // Remove from running jobs
                    {
                        let mut running = running_jobs.lock().unwrap();
                        running.remove(&job.id);
                    }
                    
                    // Update stats
                    {
                        let mut stats_guard = stats.lock().unwrap();
                        stats_guard.total_jobs_processed += 1;
                        if success {
                            stats_guard.successful_jobs += 1;
                        } else {
                            stats_guard.failed_jobs += 1;
                        }
                        
                        // Update average processing time
                        let total_time = stats_guard.average_processing_time_ms * (stats_guard.total_jobs_processed - 1) + processing_time;
                        stats_guard.average_processing_time_ms = total_time / stats_guard.total_jobs_processed;
                    }
                    
                    log::info!("Worker {} completed job {} in {}ms", worker_id, job.id, processing_time);
                } else {
                    // No job available, sleep briefly
                    thread::sleep(Duration::from_millis(100));
                }
            }
            
            log::info!("Worker {} stopped", worker_id);
        });
        
        Ok(())
    }

    /// Start timeout monitor
    fn start_timeout_monitor(&self, shutdown_flag: Arc<Mutex<bool>>) -> Result<(), CoreError> {
        let job_queue = Arc::clone(&self.job_queue);
        let running_jobs = Arc::clone(&self.running_jobs);
        let stats = Arc::clone(&self.stats);
        let config = self.config.clone();
        
        thread::spawn(move || {
            log::info!("Timeout monitor started");
            
            loop {
                // Check for shutdown signal
                {
                    let flag = shutdown_flag.lock().unwrap();
                    if *flag {
                        log::info!("Timeout monitor received shutdown signal");
                        break;
                    }
                }
                
                // Check for timed out jobs
                let timed_out_jobs = {
                    let queue = job_queue.lock().unwrap();
                    let running = running_jobs.lock().unwrap();
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
                };
                
                // Handle timed out jobs
                for job_id in timed_out_jobs {
                    log::warn!("Job {} timed out", job_id);
                    
                    let mut queue = job_queue.lock().unwrap();
                    if let Some(job) = queue.get_job_mut(&job_id) {
                        let _ = job.fail("Job timed out".to_string());
                    }
                    
                    // Update stats
                    {
                        let mut stats_guard = stats.lock().unwrap();
                        stats_guard.timed_out_jobs += 1;
                    }
                    
                    // Remove from running jobs
                    {
                        let mut running = running_jobs.lock().unwrap();
                        running.remove(&job_id);
                    }
                }
                
                // Sleep before next check
                thread::sleep(Duration::from_millis(1000));
            }
            
            log::info!("Timeout monitor stopped");
        });
        
        Ok(())
    }

    /// Process a job (simplified version without bridge dependency)
    fn process_job(job: &mut Job) -> Result<StepResult, CoreError> {
        log::info!("Processing job: {} for step: {}", job.id, job.step_name);
        
        // For now, we'll simulate job processing
        // In a real implementation, this would call the Bun.js step execution
        
        // Simulate processing time
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        // Create a simulated step result
        let step_result = StepResult {
            step_id: job.step_name.clone(),
            status: StepStatus::Completed,
            output: Some(serde_json::json!({
                "job_id": job.id,
                "step_name": job.step_name,
                "workflow_id": job.workflow_id,
                "run_id": job.run_id,
                "status": "completed",
                "message": "Job processed successfully",
                "timestamp": chrono::Utc::now().to_rfc3339(),
            })),
            error: None,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(100),
        };
        
        log::info!("Job {} processed successfully", job.id);
        Ok(step_result)
    }

    /// Scale worker pool based on queue depth
    pub fn scale_workers(&self) -> Result<(), CoreError> {
        let queue = self.job_queue.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire queue lock".to_string()))?;
        
        let workers = self.workers.lock()
            .map_err(|_| CoreError::Internal("Failed to acquire workers lock".to_string()))?;
        
        let queue_depth = queue.get_jobs().len();
        let active_workers = workers.values().filter(|w| w.is_busy()).count();
        let total_workers = workers.len();
        
        // Scale up if queue is deep and we have capacity
        if queue_depth > active_workers * 2 && total_workers < self.config.max_workers {
            log::info!("Scaling up workers: queue_depth={}, active_workers={}", queue_depth, active_workers);
            // TODO: Implement worker scaling
        }
        
        // Scale down if queue is empty and we have excess workers
        if queue_depth == 0 && total_workers > self.config.min_workers {
            log::info!("Scaling down workers: queue_depth={}, total_workers={}", queue_depth, total_workers);
            // TODO: Implement worker scaling
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
        let dispatcher = Dispatcher::new(config);
        
        assert_eq!(dispatcher.config.min_workers, 2);
        assert_eq!(dispatcher.config.max_workers, 10);
    }

    #[test]
    fn test_job_submission() {
        let config = WorkerPoolConfig::default();
        let dispatcher = Dispatcher::new(config);
        
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
        let dispatcher = Dispatcher::new(config);
        
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
} 