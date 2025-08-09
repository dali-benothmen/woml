use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::error::CoreError;
use crate::models::{WorkflowDefinition, WorkflowRun, StepResult, StepDefinition};

/// Job states for tracking execution progress
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Retrying,
}

/// Job priority levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd, Ord, Eq)]
pub enum JobPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

/// Retry configuration for jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        // Use centralized configuration
        let core_config = crate::config::CoreConfig::default();
        Self {
            max_attempts: core_config.execution.retry_attempts,
            backoff_ms: core_config.execution.retry_backoff_ms,
            max_backoff_ms: core_config.execution.max_backoff_ms,
            jitter: core_config.execution.retry_jitter,
        }
    }
}

/// Job metadata for tracking and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMetadata {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub attempt_count: u32,
    pub last_error: Option<String>,
    pub tags: HashMap<String, String>,
}

impl Default for JobMetadata {
    fn default() -> Self {
        Self {
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
            attempt_count: 0,
            last_error: None,
            tags: HashMap::new(),
        }
    }
}

/// Main Job structure for workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub workflow_id: String,
    pub run_id: String,
    pub step_name: String,
    pub state: JobState,
    pub priority: JobPriority,
    pub payload: serde_json::Value,
    pub result: Option<StepResult>,
    pub retry_config: RetryConfig,
    pub metadata: JobMetadata,
    pub dependencies: Vec<String>, // IDs of jobs this job depends on
    pub timeout_ms: Option<u64>,
    pub context: HashMap<String, serde_json::Value>, // Additional context data
}

impl Job {
    /// Create a new job
    pub fn new(
        workflow_id: String,
        run_id: String,
        step_name: String,
        payload: serde_json::Value,
        priority: JobPriority,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            workflow_id,
            run_id,
            step_name,
            state: JobState::Pending,
            priority,
            payload,
            result: None,
            retry_config: RetryConfig::default(),
            metadata: JobMetadata::default(),
            dependencies: Vec::new(),
            timeout_ms: None,
            context: HashMap::new(),
        }
    }

    /// Create a job from a workflow step definition
    pub fn from_workflow_step(
        workflow: &WorkflowDefinition,
        run: &WorkflowRun,
        step_name: &str,
        payload: serde_json::Value,
    ) -> Result<Self, CoreError> {
        let step = workflow
            .steps
            .iter()
            .find(|s| s.id == step_name)
            .ok_or_else(|| {
                CoreError::InvalidWorkflow(format!("Step '{}' not found in workflow", step_name))
            })?;

        step.validate()
            .map_err(|e| CoreError::InvalidWorkflow(e))?;

        let mut job = Self::new(
            workflow.id.clone(),
            run.id.to_string(),
            step_name.to_string(),
            payload,
            Self::determine_priority(step, workflow),
        );

        // Apply step-specific configuration
        Self::apply_step_configuration(&mut job, step, workflow)?;

        Self::setup_dependencies(&mut job, step, workflow)?;

        Self::add_workflow_context(&mut job, workflow, run)?;

        job.validate()?;

        Ok(job)
    }

    /// Create jobs for all steps in a workflow run
    pub fn create_workflow_jobs(
        workflow: &WorkflowDefinition,
        run: &WorkflowRun,
        payload: serde_json::Value,
    ) -> Result<Vec<Self>, CoreError> {
        log::info!("Creating jobs for workflow: {} run: {}", workflow.id, run.id);
        
        let mut jobs = Vec::new();
        
        for step in &workflow.steps {
            let job = Self::from_workflow_step(workflow, run, &step.id, payload.clone())?;
            jobs.push(job);
        }
        
        log::info!("Created {} jobs for workflow: {}", jobs.len(), workflow.id);
        Ok(jobs)
    }

    /// Create jobs for a subset of steps in a workflow run
    pub fn create_step_jobs(
        workflow: &WorkflowDefinition,
        run: &WorkflowRun,
        step_ids: &[String],
        payload: serde_json::Value,
    ) -> Result<Vec<Self>, CoreError> {
        log::info!("Creating jobs for steps: {:?} in workflow: {} run: {}", step_ids, workflow.id, run.id);
        
        let mut jobs = Vec::new();
        
        for step_id in step_ids {
            if !workflow.steps.iter().any(|s| s.id == *step_id) {
                return Err(CoreError::InvalidWorkflow(format!(
                    "Step '{}' not found in workflow '{}'",
                    step_id, workflow.id
                )));
            }
        }
        
        for step_id in step_ids {
            let job = Self::from_workflow_step(workflow, run, step_id, payload.clone())?;
            jobs.push(job);
        }
        
        log::info!("Created {} jobs for specified steps in workflow: {}", jobs.len(), workflow.id);
        Ok(jobs)
    }

    /// Get job ID for a specific step in a workflow run
    pub fn get_job_id(workflow_id: &str, run_id: &str, step_id: &str) -> String {
        format!("{}:{}:{}", workflow_id, run_id, step_id)
    }

    /// Parse job ID to extract workflow, run, and step information
    pub fn parse_job_id(job_id: &str) -> Result<(String, String, String), CoreError> {
        let parts: Vec<&str> = job_id.split(':').collect();
        if parts.len() != 3 {
            return Err(CoreError::Validation(format!(
                "Invalid job ID format: {}. Expected format: workflow_id:run_id:step_id",
                job_id
            )));
        }
        
        Ok((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
    }

    /// Check if this job depends on another job
    pub fn depends_on_job(&self, other_job_id: &str) -> bool {
        self.dependencies.contains(&other_job_id.to_string())
    }

    /// Get all jobs that this job depends on
    pub fn get_dependency_jobs(&self) -> &[String] {
        &self.dependencies
    }

    /// Add a dependency to this job
    pub fn add_dependency(&mut self, dependency_job_id: String) {
        if !self.dependencies.contains(&dependency_job_id) {
            self.dependencies.push(dependency_job_id);
        }
    }

    /// Remove a dependency from this job
    pub fn remove_dependency(&mut self, dependency_job_id: &str) {
        self.dependencies.retain(|dep| dep != dependency_job_id);
    }

    /// Check if this job is a dependency for another job
    pub fn is_dependency_for(&self, other_job: &Self) -> bool {
        other_job.depends_on_job(&self.id)
    }

    /// Determine job priority based on step and workflow configuration
    fn determine_priority(step: &StepDefinition, workflow: &WorkflowDefinition) -> JobPriority {
        // In the future, this could be based on:
        // - Step type (critical steps get higher priority)
        // - Workflow configuration
        // - Step tags or metadata
        JobPriority::Normal
    }

    /// Apply step configuration to job
    fn apply_step_configuration(
        job: &mut Self,
        step: &StepDefinition,
        _workflow: &WorkflowDefinition,
    ) -> Result<(), CoreError> {
        if let Some(timeout) = step.timeout {
            job.timeout_ms = Some(timeout);
        }

        if let Some(retry) = &step.retry {
            job.retry_config = RetryConfig {
                max_attempts: retry.max_attempts,
                backoff_ms: retry.backoff_ms,
                max_backoff_ms: retry.backoff_ms * 10, // Use 10x backoff as max
                jitter: true,
            };
        }

        job.add_tag("step_name".to_string(), step.name.clone());
        job.add_tag("step_action".to_string(), step.action.clone());

        Ok(())
    }

    /// Set up job dependencies based on step configuration
    fn setup_dependencies(
        job: &mut Self,
        step: &StepDefinition,
        workflow: &WorkflowDefinition,
    ) -> Result<(), CoreError> {
        for dependency_step_id in &step.depends_on {
            if !workflow.steps.iter().any(|s| s.id == *dependency_step_id) {
                return Err(CoreError::InvalidWorkflow(format!(
                    "Step '{}' depends on non-existent step '{}'",
                    step.id, dependency_step_id
                )));
            }

            let dependency_job_id = format!("{}:{}:{}", workflow.id, job.run_id, dependency_step_id);
            job.dependencies.push(dependency_job_id);
        }

        if job.dependencies.is_empty() {
            if let Some(step_index) = workflow.steps.iter().position(|s| s.id == step.id) {
                if step_index > 0 {
                    let previous_step = &workflow.steps[step_index - 1];
                    let previous_job_id = format!("{}:{}:{}", workflow.id, job.run_id, previous_step.id);
                    job.dependencies.push(previous_job_id);
                }
            }
        }

        Ok(())
    }

    /// Add workflow context to job
    fn add_workflow_context(
        job: &mut Self,
        workflow: &WorkflowDefinition,
        run: &WorkflowRun,
    ) -> Result<(), CoreError> {
        job.add_context("workflow_name".to_string(), serde_json::Value::String(workflow.name.clone()));
        if let Some(description) = &workflow.description {
            job.add_context("workflow_description".to_string(), serde_json::Value::String(description.clone()));
        }
        job.add_context("workflow_created_at".to_string(), serde_json::Value::String(workflow.created_at.to_rfc3339()));
        job.add_context("workflow_updated_at".to_string(), serde_json::Value::String(workflow.updated_at.to_rfc3339()));

        job.add_context("run_started_at".to_string(), serde_json::Value::String(run.started_at.to_rfc3339()));
        job.add_context("run_status".to_string(), serde_json::Value::String(run.status.as_str().to_string()));

        if let Some(step_index) = workflow.steps.iter().position(|s| s.id == job.step_name) {
            job.add_context("step_index".to_string(), serde_json::Value::Number(step_index.into()));
            job.add_context("total_steps".to_string(), serde_json::Value::Number(workflow.steps.len().into()));
        }

        Ok(())
    }

    /// Start the job execution
    pub fn start(&mut self) -> Result<(), CoreError> {
        if self.state != JobState::Pending && self.state != JobState::Retrying {
            return Err(CoreError::State(
                format!("Cannot start job in state: {:?}", self.state)
            ));
        }

        self.state = JobState::Running;
        self.metadata.started_at = Some(Utc::now());
        self.metadata.updated_at = Utc::now();
        self.metadata.attempt_count += 1;

        Ok(())
    }

    /// Complete the job successfully
    pub fn complete(&mut self, result: StepResult) -> Result<(), CoreError> {
        if self.state != JobState::Running {
            return Err(CoreError::State(
                format!("Cannot complete job in state: {:?}", self.state)
            ));
        }

        self.state = JobState::Completed;
        self.result = Some(result);
        self.metadata.completed_at = Some(Utc::now());
        self.metadata.updated_at = Utc::now();

        Ok(())
    }

    /// Fail the job
    pub fn fail(&mut self, error: String) -> Result<(), CoreError> {
        if self.state != JobState::Running {
            return Err(CoreError::State(
                format!("Cannot fail job in state: {:?}", self.state)
            ));
        }

        self.state = JobState::Failed;
        self.metadata.last_error = Some(error);
        self.metadata.completed_at = Some(Utc::now());
        self.metadata.updated_at = Utc::now();

        Ok(())
    }

    /// Retry the job
    pub fn retry(&mut self) -> Result<(), CoreError> {
        if self.state != JobState::Failed {
            return Err(CoreError::State(
                format!("Cannot retry job in state: {:?}", self.state)
            ));
        }

        if self.metadata.attempt_count >= self.retry_config.max_attempts {
            return Err(CoreError::Configuration(
                "Maximum retry attempts exceeded".to_string()
            ));
        }

        self.state = JobState::Retrying;
        self.metadata.updated_at = Utc::now();

        Ok(())
    }

    /// Cancel the job
    pub fn cancel(&mut self) -> Result<(), CoreError> {
        if self.state == JobState::Completed || self.state == JobState::Failed {
            return Err(CoreError::State(
                format!("Cannot cancel job in state: {:?}", self.state)
            ));
        }

        self.state = JobState::Cancelled;
        self.metadata.completed_at = Some(Utc::now());
        self.metadata.updated_at = Utc::now();

        Ok(())
    }

    /// Check if job is ready to execute (dependencies satisfied)
    pub fn is_ready(&self, completed_jobs: &[String]) -> bool {
        if self.state != JobState::Pending && self.state != JobState::Retrying {
            return false;
        }

        self.dependencies.iter().all(|dep_id| completed_jobs.contains(dep_id))
    }

    /// Check if job has timed out
    pub fn is_timed_out(&self) -> bool {
        if let Some(timeout_ms) = self.timeout_ms {
            if let Some(started_at) = self.metadata.started_at {
                let elapsed = Utc::now().signed_duration_since(started_at);
                return elapsed.num_milliseconds() as u64 > timeout_ms;
            }
        }
        false
    }

    /// Check if job can be retried
    pub fn can_retry(&self) -> bool {
        self.state == JobState::Failed 
            && self.metadata.attempt_count < self.retry_config.max_attempts
    }

    /// Calculate next retry delay with exponential backoff
    pub fn next_retry_delay(&self) -> u64 {
        let base_delay = self.retry_config.backoff_ms;
        let attempt = self.metadata.attempt_count.saturating_sub(1);
        let delay = base_delay * 2_u64.pow(attempt);
        
        // Cap at max backoff
        delay.min(self.retry_config.max_backoff_ms)
    }

    /// Validate job configuration
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.id.is_empty() {
            return Err(CoreError::Configuration("Job ID cannot be empty".to_string()));
        }

        if self.workflow_id.is_empty() {
            return Err(CoreError::Configuration("Workflow ID cannot be empty".to_string()));
        }

        if self.run_id.is_empty() {
            return Err(CoreError::Configuration("Run ID cannot be empty".to_string()));
        }

        if self.step_name.is_empty() {
            return Err(CoreError::Configuration("Step name cannot be empty".to_string()));
        }

        if self.retry_config.max_attempts == 0 {
            return Err(CoreError::Configuration("Max attempts must be greater than 0".to_string()));
        }

        Ok(())
    }

    /// Add a tag to the job
    pub fn add_tag(&mut self, key: String, value: String) {
        self.metadata.tags.insert(key, value);
        self.metadata.updated_at = Utc::now();
    }

    /// Get a tag value
    pub fn get_tag(&self, key: &str) -> Option<&String> {
        self.metadata.tags.get(key)
    }

    /// Add context data
    pub fn add_context(&mut self, key: String, value: serde_json::Value) {
        self.context.insert(key, value);
        self.metadata.updated_at = Utc::now();
    }

    /// Get context data
    pub fn get_context(&self, key: &str) -> Option<&serde_json::Value> {
        self.context.get(key)
    }
}

/// Job queue for managing job execution order
#[derive(Debug, Clone)]
pub struct JobQueue {
    pub jobs: Vec<Job>,
}

impl JobQueue {
    /// Create a new job queue
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
        }
    }

    /// Add a job to the queue
    pub fn enqueue(&mut self, job: Job) -> Result<(), CoreError> {
        job.validate()?;
        self.jobs.push(job);
        Ok(())
    }

    /// Get the next job to execute (highest priority, oldest first)
    pub fn dequeue(&mut self, completed_jobs: &[String]) -> Option<Job> {
        let ready_jobs: Vec<_> = self.jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| job.is_ready(completed_jobs))
            .collect();

        if ready_jobs.is_empty() {
            return None;
        }

        // Sort by priority (highest first), then by creation time (oldest first)
        let next_job_index = ready_jobs
            .iter()
            .max_by(|(_, a), (_, b)| {
                a.priority.cmp(&b.priority)
                    .then(a.metadata.created_at.cmp(&b.metadata.created_at))
            })
            .map(|(index, _)| *index)?;

        Some(self.jobs.remove(next_job_index))
    }

    /// Get all jobs in the queue
    pub fn get_jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// Get job by ID
    pub fn get_job(&self, job_id: &str) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == job_id)
    }

    /// Get job by ID (mutable)
    pub fn get_job_mut(&mut self, job_id: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|job| job.id == job_id)
    }

    /// Remove job by ID
    pub fn remove_job(&mut self, job_id: &str) -> Option<Job> {
        let index = self.jobs.iter().position(|job| job.id == job_id)?;
        Some(self.jobs.remove(index))
    }

    /// Get queue statistics
    pub fn stats(&self) -> JobQueueStats {
        let mut stats = JobQueueStats::default();
        
        for job in &self.jobs {
            match job.state {
                JobState::Pending => stats.pending += 1,
                JobState::Running => stats.running += 1,
                JobState::Completed => stats.completed += 1,
                JobState::Failed => stats.failed += 1,
                JobState::Cancelled => stats.cancelled += 1,
                JobState::Retrying => stats.retrying += 1,
            }
        }

        stats
    }

    /// Clear completed and failed jobs
    pub fn cleanup(&mut self) {
        self.jobs.retain(|job| {
            job.state != JobState::Completed && job.state != JobState::Failed
        });
    }
}

/// Statistics for job queue
#[derive(Debug, Clone, Default)]
pub struct JobQueueStats {
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub retrying: usize,
}

impl JobQueueStats {
    /// Get total jobs in queue
    pub fn total(&self) -> usize {
        self.pending + self.running + self.completed + self.failed + self.cancelled + self.retrying
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{WorkflowDefinition, StepDefinition, TriggerDefinition, RunStatus, StepStatus, RetryConfig as ModelsRetryConfig};
    use chrono::Utc;
    use uuid::Uuid;

    fn create_test_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            id: "test-workflow".to_string(),
            name: "Test Workflow".to_string(),
            description: Some("A test workflow".to_string()),
            steps: vec![
                StepDefinition {
                    id: "step-1".to_string(),
                    name: "Step 1".to_string(),
                    action: "test_action_1".to_string(),
                    timeout: Some(5000),
                    retry: Some(ModelsRetryConfig {
                        max_attempts: 3,
                        backoff_ms: 1000,
                    }),
                    depends_on: vec![],
                },
                StepDefinition {
                    id: "step-2".to_string(),
                    name: "Step 2".to_string(),
                    action: "test_action_2".to_string(),
                    timeout: Some(10000),
                    retry: None,
                    depends_on: vec!["step-1".to_string()],
                },
                StepDefinition {
                    id: "step-3".to_string(),
                    name: "Step 3".to_string(),
                    action: "test_action_3".to_string(),
                    timeout: None,
                    retry: Some(ModelsRetryConfig {
                        max_attempts: 2,
                        backoff_ms: 2000,
                    }),
                    depends_on: vec!["step-1".to_string(), "step-2".to_string()],
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
            status: RunStatus::Running,
            payload: serde_json::json!({"test": "data"}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    #[test]
    fn test_job_creation() {
        let job = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        assert_eq!(job.workflow_id, "workflow-1");
        assert_eq!(job.run_id, "run-1");
        assert_eq!(job.step_name, "step-1");
        assert_eq!(job.state, JobState::Pending);
        assert_eq!(job.priority, JobPriority::Normal);
    }

    #[test]
    fn test_job_lifecycle() {
        let mut job = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        // Start job
        assert!(job.start().is_ok());
        assert_eq!(job.state, JobState::Running);
        assert!(job.metadata.started_at.is_some());

        // Complete job
        let result = StepResult {
            step_id: "step-1".to_string(),
            status: StepStatus::Completed,
            output: Some(serde_json::json!({"result": "success"})),
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(100),
        };

        assert!(job.complete(result).is_ok());
        assert_eq!(job.state, JobState::Completed);
        assert!(job.metadata.completed_at.is_some());
    }

    #[test]
    fn test_job_validation() {
        let mut job = Job::new(
            "".to_string(), // Invalid: empty workflow ID
            "run-1".to_string(),
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        assert!(job.validate().is_err());

        let mut job = Job::new(
            "workflow-1".to_string(),
            "".to_string(), // Invalid: empty run ID
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        assert!(job.validate().is_err());
    }

    #[test]
    fn test_job_queue() {
        let mut queue = JobQueue::new();
        let job = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        assert!(queue.enqueue(job).is_ok());
        assert_eq!(queue.get_jobs().len(), 1);

        let dequeued = queue.dequeue(&[]);
        assert!(dequeued.is_some());
        assert_eq!(queue.get_jobs().len(), 0);
    }

    #[test]
    fn test_job_retry() {
        let mut job = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        job.retry_config.max_attempts = 3;
        job.retry_config.backoff_ms = 1000;

        // Start and fail job
        job.start().unwrap();
        job.fail("Test error".to_string()).unwrap();

        // Retry job
        assert!(job.retry().is_ok());
        assert_eq!(job.state, JobState::Retrying);
        assert_eq!(job.metadata.attempt_count, 1); // First retry attempt

        // Start the retrying job
        job.start().unwrap();
        assert_eq!(job.state, JobState::Running);

        job.fail("Test error".to_string()).unwrap();
        assert!(job.retry().is_ok()); // Second retry
        job.start().unwrap();
        job.fail("Test error".to_string()).unwrap();
        assert!(job.retry().is_ok()); // Third retry
        job.start().unwrap();
        job.fail("Test error".to_string()).unwrap();

        // Should not be able to retry anymore (max attempts reached)
        assert!(job.retry().is_err());
    }

    #[test]
    fn test_from_workflow_step() {
        let workflow = create_test_workflow();
        let run = create_test_run();
        let payload = serde_json::json!({"test": "data"});

        let job = Job::from_workflow_step(&workflow, &run, "step-1", payload.clone()).unwrap();

        assert_eq!(job.workflow_id, workflow.id);
        assert_eq!(job.run_id, run.id.to_string());
        assert_eq!(job.step_name, "step-1");
        assert_eq!(job.timeout_ms, Some(5000));
        assert_eq!(job.retry_config.max_attempts, 3);
        assert_eq!(job.retry_config.backoff_ms, 1000);
        assert_eq!(job.dependencies.len(), 0); // No explicit dependencies
    }

    #[test]
    fn test_from_workflow_step_with_dependencies() {
        let workflow = create_test_workflow();
        let run = create_test_run();
        let payload = serde_json::json!({"test": "data"});

        let job = Job::from_workflow_step(&workflow, &run, "step-2", payload.clone()).unwrap();

        assert_eq!(job.step_name, "step-2");
        assert_eq!(job.dependencies.len(), 1);
        assert!(job.dependencies.contains(&format!("{}:{}:step-1", workflow.id, run.id)));
    }

    #[test]
    fn test_create_workflow_jobs() {
        let workflow = create_test_workflow();
        let run = create_test_run();
        let payload = serde_json::json!({"test": "data"});

        let jobs = Job::create_workflow_jobs(&workflow, &run, payload).unwrap();

        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].step_name, "step-1");
        assert_eq!(jobs[1].step_name, "step-2");
        assert_eq!(jobs[2].step_name, "step-3");

        assert_eq!(jobs[0].dependencies.len(), 0); // First step has no dependencies
        assert_eq!(jobs[1].dependencies.len(), 1); // Depends on step-1
        assert_eq!(jobs[2].dependencies.len(), 2); // Depends on step-1 and step-2
    }

    #[test]
    fn test_create_step_jobs() {
        let workflow = create_test_workflow();
        let run = create_test_run();
        let payload = serde_json::json!({"test": "data"});

        let step_ids = vec!["step-1".to_string(), "step-3".to_string()];
        let jobs = Job::create_step_jobs(&workflow, &run, &step_ids, payload).unwrap();

        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].step_name, "step-1");
        assert_eq!(jobs[1].step_name, "step-3");
    }

    #[test]
    fn test_job_id_utilities() {
        let job_id = Job::get_job_id("workflow-1", "run-1", "step-1");
        assert_eq!(job_id, "workflow-1:run-1:step-1");

        let (workflow_id, run_id, step_id) = Job::parse_job_id(&job_id).unwrap();
        assert_eq!(workflow_id, "workflow-1");
        assert_eq!(run_id, "run-1");
        assert_eq!(step_id, "step-1");

        assert!(Job::parse_job_id("invalid").is_err());
    }

    #[test]
    fn test_job_dependencies() {
        let mut job1 = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-1".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        let mut job2 = Job::new(
            "workflow-1".to_string(),
            "run-1".to_string(),
            "step-2".to_string(),
            serde_json::json!({"test": "data"}),
            JobPriority::Normal,
        );

        job2.add_dependency(job1.id.clone());
        assert!(job2.depends_on_job(&job1.id));
        assert!(job1.is_dependency_for(&job2));

        // Remove dependency
        job2.remove_dependency(&job1.id);
        assert!(!job2.depends_on_job(&job1.id));
        assert!(!job1.is_dependency_for(&job2));
    }

    #[test]
    fn test_workflow_context() {
        let workflow = create_test_workflow();
        let run = create_test_run();
        let payload = serde_json::json!({"test": "data"});

        let job = Job::from_workflow_step(&workflow, &run, "step-1", payload).unwrap();

        assert_eq!(job.get_context("workflow_name").unwrap(), &serde_json::Value::String(workflow.name));
        assert_eq!(job.get_context("step_index").unwrap(), &serde_json::Value::Number(0.into()));
        assert_eq!(job.get_context("total_steps").unwrap(), &serde_json::Value::Number(3.into()));

        assert_eq!(job.get_tag("step_name").unwrap(), "Step 1");
        assert_eq!(job.get_tag("step_action").unwrap(), "test_action_1");
    }
} 