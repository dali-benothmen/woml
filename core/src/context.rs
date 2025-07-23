use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::models::{WorkflowRun, StepResult};
use crate::error::CoreError;
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Available services for this execution
    pub services: HashMap<String, serde_json::Value>,
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
            max_retries: 3,
            version: "1.0.0".to_string(),
            checksum: None,
        };

        Ok(Context {
            run_id,
            workflow_id,
            step_name,
            payload,
            steps,
            services: HashMap::new(),
            run,
            metadata,
            serialization_info: None,
        })
    }

    /// Add a service to the context
    pub fn add_service(&mut self, name: String, config: serde_json::Value) {
        self.services.insert(name, config);
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
        if payload_size > 10_000_000 { // 10MB limit
            return Err(CoreError::Validation(format!(
                "Payload too large: {} bytes (max: 10MB)", payload_size
            )));
        }

        Ok(())
    }

    /// Calculate context complexity score (1-10)
    pub fn calculate_complexity_score(&self) -> u8 {
        let mut score = 1u8;
        
        // Add points for payload complexity
        if let Some(payload_size) = serde_json::to_string(&self.payload).ok().map(|s| s.len()) {
            if payload_size > 100_000 { score += 2; } // Large payload
            else if payload_size > 10_000 { score += 1; } // Medium payload
        }
        
        // Add points for number of steps
        if self.steps.len() > 50 { score += 2; }
        else if self.steps.len() > 10 { score += 1; }
        
        // Add points for number of services
        if self.services.len() > 10 { score += 2; }
        else if self.services.len() > 5 { score += 1; }
        
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

    /// Generate checksum for data integrity
    pub fn generate_checksum(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        self.run_id.hash(&mut hasher);
        self.workflow_id.hash(&mut hasher);
        self.step_name.hash(&mut hasher);
        
        // Hash payload as string
        if let Ok(payload_str) = serde_json::to_string(&self.payload) {
            payload_str.hash(&mut hasher);
        }
        
        format!("{:x}", hasher.finish())
    }

    /// Serialize context to JSON string with performance tracking
    pub fn to_json(&self) -> Result<String, CoreError> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        
        // Validate before serialization
        self.validate()?;
        
        // Generate checksum
        let mut context = self.clone();
        context.metadata.checksum = Some(self.generate_checksum());
        
        // Serialize to JSON
        let json_string = serde_json::to_string_pretty(&context)
            .map_err(|e| CoreError::Serialization(e))?;
        
        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        
        // Create serialization info
        let serialization_info = SerializationInfo {
            serialized_at: chrono::Utc::now().to_rfc3339(),
            size_bytes: json_string.len(),
            compressed: false,
            compression_ratio: None,
            serialization_duration_us: end_time - start_time,
            complexity_score: self.calculate_complexity_score(),
        };
        
        // Log performance metrics
        log::debug!(
            "Context serialized: {} bytes, {}μs, complexity: {}",
            serialization_info.size_bytes,
            serialization_info.serialization_duration_us,
            serialization_info.complexity_score
        );
        
        Ok(json_string)
    }

    /// Serialize context to compressed JSON string
    pub fn to_json_compressed(&self) -> Result<String, CoreError> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        
        // Validate before serialization
        self.validate()?;
        
        // Generate checksum
        let mut context = self.clone();
        context.metadata.checksum = Some(self.generate_checksum());
        
        // Serialize to JSON (compact format for compression)
        let json_string = serde_json::to_string(&context)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Compress using gzip (simulated for now)
        // In a real implementation, you would use a compression library
        let compressed_size = json_string.len(); // Placeholder
        let compression_ratio = if compressed_size > 0 {
            Some(compressed_size as f64 / json_string.len() as f64)
        } else {
            None
        };
        
        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        
        // Create serialization info
        let serialization_info = SerializationInfo {
            serialized_at: chrono::Utc::now().to_rfc3339(),
            size_bytes: compressed_size,
            compressed: true,
            compression_ratio,
            serialization_duration_us: end_time - start_time,
            complexity_score: self.calculate_complexity_score(),
        };
        
        // Log performance metrics
        log::debug!(
            "Context serialized (compressed): {} bytes, {}μs, ratio: {:.2}",
            serialization_info.size_bytes,
            serialization_info.serialization_duration_us,
            compression_ratio.unwrap_or(1.0)
        );
        
        Ok(json_string) // Return uncompressed for now
    }

    /// Create context from JSON string with validation
    pub fn from_json(json: &str) -> Result<Self, CoreError> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        
        // Deserialize from JSON
        let mut context: Context = serde_json::from_str(json)
            .map_err(|e| CoreError::Serialization(e))?;
        
        // Validate checksum if present
        if let Some(expected_checksum) = &context.metadata.checksum {
            let actual_checksum = context.generate_checksum();
            if expected_checksum != &actual_checksum {
                return Err(CoreError::Validation(
                    "Context checksum validation failed".to_string()
                ));
            }
        }
        
        // Validate context
        context.validate()?;
        
        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        
        log::debug!(
            "Context deserialized: {} bytes, {}μs",
            json.len(),
            end_time - start_time
        );
        
        Ok(context)
    }

    /// Get context as a JSON value
    pub fn to_json_value(&self) -> Result<serde_json::Value, CoreError> {
        serde_json::to_value(self)
            .map_err(|e| CoreError::Serialization(e))
    }

    /// Get context size in bytes
    pub fn size_bytes(&self) -> Result<usize, CoreError> {
        Ok(serde_json::to_string(self)
            .map_err(|e| CoreError::Serialization(e))?
            .len())
    }

    /// Check if context is oversized
    pub fn is_oversized(&self, max_size_bytes: usize) -> Result<bool, CoreError> {
        Ok(self.size_bytes()? > max_size_bytes)
    }

    /// Get context statistics
    pub fn get_statistics(&self) -> ContextStatistics {
        ContextStatistics {
            total_steps: self.steps.len(),
            total_services: self.services.len(),
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
    pub total_services: usize,
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
            max_retries: 3,
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
            "current-step".to_string(),
            serde_json::json!({}),
            run,
            vec![completed_step],
        ).unwrap();

        assert_eq!(context.get_completed_steps().len(), 1);
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
            serde_json::json!({"input": "value"}),
            run,
            vec![],
        ).unwrap();

        // Test JSON serialization
        let json = context.to_json().unwrap();
        let deserialized = Context::from_json(&json).unwrap();
        
        assert_eq!(context.run_id, deserialized.run_id);
        assert_eq!(context.workflow_id, deserialized.workflow_id);
        assert_eq!(context.step_name, deserialized.step_name);
        
        // Test checksum validation
        assert!(deserialized.metadata.checksum.is_some());
        
        // Test complexity scoring
        let complexity = context.calculate_complexity_score();
        assert!(complexity >= 1 && complexity <= 10);
        
        // Test statistics
        let stats = context.get_statistics();
        assert_eq!(stats.total_steps, 0);
        assert_eq!(stats.total_services, 0);
        assert!(stats.payload_size_bytes > 0);
        assert_eq!(stats.complexity_score, complexity);
        assert!(!stats.has_checksum); // Original context doesn't have checksum
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

        // Test simple context
        let simple_context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            serde_json::json!({"simple": "data"}),
            run.clone(),
            vec![],
        ).unwrap();
        
        assert_eq!(simple_context.calculate_complexity_score(), 1);

        // Test complex context with large payload
        let large_payload = serde_json::json!({
            "data": (0..1000).map(|i| format!("item-{}", i)).collect::<Vec<_>>()
        });
        
        let complex_context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            large_payload,
            run.clone(),
            vec![],
        ).unwrap();
        
        let complexity = complex_context.calculate_complexity_score();
        assert!(complexity > 1); // Should be higher than simple context
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
            serde_json::json!({"test": "data"}),
            run.clone(),
            vec![],
        ).unwrap();
        
        assert!(valid_context.validate().is_ok());

        // Test oversized payload
        let oversized_payload = serde_json::json!({
            "data": "x".repeat(11_000_000) // 11MB, over 10MB limit
        });
        
        let oversized_context = Context::new(
            "run-123".to_string(),
            "workflow-123".to_string(),
            "test-step".to_string(),
            oversized_payload,
            run.clone(),
            vec![],
        ).unwrap();
        
        assert!(oversized_context.validate().is_err());
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

    #[test]
    fn test_context_services() {
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

        // Test adding services
        context.add_service("email".to_string(), serde_json::json!({
            "smtp": "smtp.gmail.com",
            "port": 587
        }));

        context.add_service("database".to_string(), serde_json::json!({
            "url": "postgresql://localhost:5432/mydb"
        }));

        assert_eq!(context.services.len(), 2);
        assert!(context.services.contains_key("email"));
        assert!(context.services.contains_key("database"));
    }
} 