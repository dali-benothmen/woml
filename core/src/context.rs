use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::models::{WorkflowRun, StepResult};
use crate::error::CoreError;

/// Context object passed to Bun.js for job execution
/// Contains all necessary information for step execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    /// Unique identifier for this workflow run
    pub run_id: String,
    /// Unique identifier for the workflow
    pub workflow_id: String,
    /// Current step being executed
    pub step_name: String,
    /// Input payload for the workflow
    pub payload: serde_json::Value,
    /// Results from completed steps
    pub steps: HashMap<String, StepResult>,
    /// Current workflow run state
    pub run: WorkflowRun,
    /// Metadata about the execution
    pub metadata: ContextMetadata,
    /// Serialization metadata for performance tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serialization_info: Option<SerializationInfo>,
}

/// Metadata about the context execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetadata {
    /// Timestamp when context was created
    pub created_at: String,
    /// Current step index
    pub step_index: usize,
    /// Total number of steps
    pub total_steps: usize,
    /// Execution timeout in seconds
    pub timeout: Option<u64>,
    /// Retry count for current step
    pub retry_count: u32,
    /// Maximum retries allowed
    pub max_retries: u32,
    /// Context version for compatibility
    pub version: String,
    /// Checksum for data integrity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// Information about context serialization for performance tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializationInfo {
    /// Timestamp when context was serialized
    pub serialized_at: String,
    /// Size of serialized context in bytes
    pub size_bytes: usize,
    /// Whether context was compressed
    pub compressed: bool,
    /// Compression ratio if compressed
    pub compression_ratio: Option<f64>,
    /// Serialization duration in microseconds
    pub serialization_duration_us: u64,
    /// Context complexity score (1-10)
    pub complexity_score: u8,
}

impl Context {
    /// Create a new context from job data
    pub fn new(
        run_id: String,
        workflow_id: String,
        step_name: String,
        payload: serde_json::Value,
        run: WorkflowRun,
        completed_steps: Vec<StepResult>,
    ) -> Result<Self, CoreError> {
        // Validate required fields
        if run_id.is_empty() {
            return Err(CoreError::Validation("run_id cannot be empty".to_string()));
        }
        if workflow_id.is_empty() {
            return Err(CoreError::Validation("workflow_id cannot be empty".to_string()));
        }
        if step_name.is_empty() {
            return Err(CoreError::Validation("step_name cannot be empty".to_string()));
        }

        let mut steps = HashMap::new();
        for step_result in completed_steps {
            steps.insert(step_result.step_id.clone(), step_result);
        }

        let metadata = ContextMetadata {
            created_at: chrono::Utc::now().to_rfc3339(),
            step_index: 0, // Will be updated by caller
            total_steps: 0, // Will be updated by caller
            timeout: None,
            retry_count: 0,
            max_retries: crate::config::CoreConfig::default().execution.max_retries,
            version: "1.0.0".to_string(),
            checksum: None,
        };

        Ok(Context {
            run_id,
            workflow_id,
            step_name,
            payload,
            steps,
            run,
            metadata,
            serialization_info: None,
        })
    }

    /// Get a completed step result
    pub fn get_step_result(&self, step_name: &str) -> Option<&StepResult> {
        self.steps.get(step_name)
    }

    /// Get all completed step results
    pub fn get_completed_steps(&self) -> Vec<&StepResult> {
        self.steps.values().collect()
    }

    /// Update step metadata
    pub fn update_step_metadata(&mut self, step_index: usize, total_steps: usize) {
        self.metadata.step_index = step_index;
        self.metadata.total_steps = total_steps;
    }

    /// Set execution timeout
    pub fn set_timeout(&mut self, timeout_seconds: u64) {
        self.metadata.timeout = Some(timeout_seconds);
    }

    /// Increment retry count and check if retry is allowed
    pub fn increment_retry(&mut self) -> bool {
        self.metadata.retry_count += 1;
        self.metadata.retry_count <= self.metadata.max_retries
    }

    /// Reset retry count
    pub fn reset_retry_count(&mut self) {
        self.metadata.retry_count = 0;
    }

    /// Validate context data
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.run_id.is_empty() {
            return Err(CoreError::Validation("run_id cannot be empty".to_string()));
        }
        if self.workflow_id.is_empty() {
            return Err(CoreError::Validation("workflow_id cannot be empty".to_string()));
        }
        if self.step_name.is_empty() {
            return Err(CoreError::Validation("step_name cannot be empty".to_string()));
        }
        
        // Validate payload size (prevent oversized contexts)
        let payload_size = serde_json::to_string(&self.payload)
            .map_err(|e| CoreError::Serialization(e))?
            .len();
        let config = crate::config::CoreConfig::default();
        if payload_size > config.payload.max_size_bytes {
            return Err(CoreError::Validation(format!(
                "Payload too large: {} bytes (max: {})", 
                payload_size, config.payload.max_size_bytes
            )));
        }

        Ok(())
    }

    /// Calculate context complexity score (1-10)
    pub fn calculate_complexity_score(&self) -> u8 {
        let mut score = 1u8;
        
        // Add points for payload complexity
        let config = crate::config::CoreConfig::default();
        if let Some(payload_size) = serde_json::to_string(&self.payload).ok().map(|s| s.len()) {
            if payload_size > config.payload.large_payload_threshold { score += 2; } // Large payload
            else if payload_size > config.payload.medium_payload_threshold { score += 1; } // Medium payload
        }
        
        // Add points for number of steps
        if self.steps.len() > config.payload.max_step_count_large { score += 2; }
        else if self.steps.len() > config.payload.max_step_count_medium { score += 1; }
        
        // Add points for nested payload structure
        if self.has_deep_nesting(&self.payload, 0) { score += 1; }
        
        score.min(10)
    }

    /// Check if JSON value has deep nesting
    fn has_deep_nesting(&self, value: &serde_json::Value, depth: u8) -> bool {
        if depth > 5 { return true; }
        
        match value {
            serde_json::Value::Object(map) => {
                map.values().any(|v| self.has_deep_nesting(v, depth + 1))
            }
            serde_json::Value::Array(arr) => {
                arr.iter().any(|v| self.has_deep_nesting(v, depth + 1))
            }
            _ => false,
        }
    }

    /// Generate a checksum for the context
    pub fn generate_checksum(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        self.run_id.hash(&mut hasher);
        self.workflow_id.hash(&mut hasher);
        self.step_name.hash(&mut hasher);
        
        // Hash the payload as a string
        if let Ok(payload_str) = serde_json::to_string(&self.payload) {
            payload_str.hash(&mut hasher);
        }
        
        format!("{:x}", hasher.finish())
    }

    /// Convert context to JSON string
    pub fn to_json(&self) -> Result<String, CoreError> {
        // Generate checksum before serialization
        let checksum = self.generate_checksum();
        
        // Create a copy with checksum for serialization
        let mut context_for_serialization = self.clone();
        context_for_serialization.metadata.checksum = Some(checksum);
        
        // Calculate complexity score
        let complexity_score = self.calculate_complexity_score();
        
        // Create serialization info
        let start_time = std::time::Instant::now();
        let json_result = serde_json::to_string(&context_for_serialization);
        let serialization_duration = start_time.elapsed();
        
        let json_string = json_result.map_err(|e| CoreError::Serialization(e))?;
        let size_bytes = json_string.len();
        
        // Update serialization info
        context_for_serialization.serialization_info = Some(SerializationInfo {
            serialized_at: chrono::Utc::now().to_rfc3339(),
            size_bytes,
            compressed: false,
            compression_ratio: None,
            serialization_duration_us: serialization_duration.as_micros() as u64,
            complexity_score,
        });
        
        // Serialize again with the updated info
        serde_json::to_string(&context_for_serialization)
            .map_err(|e| CoreError::Serialization(e))
    }

    /// Convert context to compressed JSON string
    pub fn to_json_compressed(&self) -> Result<String, CoreError> {
        // Currently returns uncompressed JSON
        // Enhancement: Add gzip/zstd compression for large payloads when needed
        self.to_json()
    }

    /// Create context from JSON string
    pub fn from_json(json: &str) -> Result<Self, CoreError> {
        let context: Context = serde_json::from_str(json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Validate the context
        context.validate()?;
        
        Ok(context)
    }

    /// Convert context to serde_json::Value
    pub fn to_json_value(&self) -> Result<serde_json::Value, CoreError> {
        serde_json::to_value(self)
            .map_err(|e| CoreError::Serialization(e))
    }

    /// Get the size of the context in bytes
    pub fn size_bytes(&self) -> Result<usize, CoreError> {
        let json_string = serde_json::to_string(self)
            .map_err(|e| CoreError::Serialization(e))?;
        Ok(json_string.len())
    }

    /// Check if context is oversized
    pub fn is_oversized(&self, max_size_bytes: usize) -> Result<bool, CoreError> {
        Ok(self.size_bytes()? > max_size_bytes)
    }

    /// Get context statistics
    pub fn get_statistics(&self) -> ContextStatistics {
        ContextStatistics {
            total_steps: self.steps.len(),
            payload_size_bytes: serde_json::to_string(&self.payload)
                .map(|s| s.len())
                .unwrap_or(0),
            complexity_score: self.calculate_complexity_score(),
            has_checksum: self.metadata.checksum.is_some(),
        }
    }
}

/// Statistics about context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStatistics {
    pub total_steps: usize,
    pub payload_size_bytes: usize,
    pub complexity_score: u8,
    pub has_checksum: bool,
}

impl Default for ContextMetadata {
    fn default() -> Self {
        Self {
            created_at: chrono::Utc::now().to_rfc3339(),
            step_index: 0,
            total_steps: 0,
            timeout: None,
            retry_count: 0,
            max_retries: crate::config::CoreConfig::default().execution.max_retries,
            version: "1.0.0".to_string(),
            checksum: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{WorkflowRun, RunStatus, StepResult, StepStatus};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_context_creation() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({"test": "data"}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        let context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({"input": "value"}),
            run,
            vec![],
        ).unwrap();

        assert_eq!(context.run_id, "run-123");
        assert_eq!(context.workflow_id, "workflow-123");
        assert_eq!(context.step_name, "test-step");
        assert!(context.validate().is_ok());
    }

    #[test]
    fn test_context_with_completed_steps() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        let completed_step = StepResult {
            step_id: "previous-step".to_string(),
            status: StepStatus::Completed,
            output: Some(serde_json::json!({"output": "success"})),
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(1000),
        };

        let context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({}),
            run,
            vec![completed_step],
        ).unwrap();

        assert_eq!(context.steps.len(), 1);
        assert!(context.get_step_result("previous-step").is_some());
        assert!(context.get_step_result("non-existent").is_none());
    }

    #[test]
    fn test_context_serialization() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        let context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({"test": "data"}),
            run,
            vec![],
        ).unwrap();

        // Test JSON serialization
        let json_string = context.to_json().unwrap();
        let deserialized_context = Context::from_json(&json_string).unwrap();
        
        assert_eq!(context.run_id, deserialized_context.run_id);
        assert_eq!(context.workflow_id, deserialized_context.workflow_id);
        assert_eq!(context.step_name, deserialized_context.step_name);
        
        // Test compressed serialization
        let compressed_json = context.to_json_compressed().unwrap();
        assert!(!compressed_json.is_empty());
    }

    #[test]
    fn test_context_complexity_scoring() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        let context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({"simple": "data"}),
            run,
            vec![],
        ).unwrap();

        let score = context.calculate_complexity_score();
        assert!(score >= 1 && score <= 10);
    }

    #[test]
    fn test_context_validation() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        // Test valid context
        let valid_context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({}),
            run.clone(),
            vec![],
        ).unwrap();
        assert!(valid_context.validate().is_ok());

        // Test invalid context with empty run_id
        let invalid_context = Context::new(
            "".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({}),
            run.clone(),
            vec![],
        ).unwrap();
        assert!(invalid_context.validate().is_err());
    }

    #[test]
    fn test_context_checksum() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        let context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({"test": "data"}),
            run,
            vec![],
        ).unwrap();

        let checksum1 = context.generate_checksum();
        let checksum2 = context.generate_checksum();
        
        // Checksums should be consistent
        assert_eq!(checksum1, checksum2);
        
        // Checksums should be different for different contexts
        let different_context = Context::new(
            "run-456".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({"test": "data"}),
            run,
            vec![],
        ).unwrap();
        
        let checksum3 = different_context.generate_checksum();
        assert_ne!(checksum1, checksum3);
    }

    #[test]
    fn test_context_metadata() {
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            workflow_id: "workflow-123".to_string(),
            status: RunStatus::Running,
            payload: serde_json::json!({}),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        };

        let mut context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({}),
            run,
            vec![],
        ).unwrap();

        // Test metadata updates
        context.update_step_metadata(2, 5);
        assert_eq!(context.metadata.step_index, 2);
        assert_eq!(context.metadata.total_steps, 5);

        // Test timeout
        context.set_timeout(30);
        assert_eq!(context.metadata.timeout, Some(30));

        // Test retry logic
        assert_eq!(context.metadata.retry_count, 0);
        assert!(context.increment_retry());
        assert_eq!(context.metadata.retry_count, 1);
        assert!(context.increment_retry());
        assert!(context.increment_retry());
        assert!(!context.increment_retry()); // Should fail on 4th retry

        context.reset_retry_count();
        assert_eq!(context.metadata.retry_count, 0);
    }
} 