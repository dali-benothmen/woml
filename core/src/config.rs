//! Core configuration management for Node-Cronflow
//! 
//! This module provides centralized configuration for all core components,
//! supporting both default values and environment variable overrides.

use std::env;

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub worker_pool: WorkerPoolConfig,
    pub execution: ExecutionConfig,
    pub webhook: WebhookConfig,
    pub database: DatabaseConfig,
    pub payload: PayloadConfig,
}

#[derive(Debug, Clone)]
pub struct WorkerPoolConfig {
    pub min_workers: usize,
    pub max_workers: usize,
    pub worker_timeout_ms: u64,
    pub queue_size: usize,
}

#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    pub max_concurrent_steps: usize,
    pub default_timeout_ms: Option<u64>,
    pub fail_fast: bool,
    pub retry_attempts: u32,
    pub retry_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub retry_jitter: bool,
    pub max_retries: u32,
}

/// Webhook server configuration
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub default_path: String,
    pub connection_timeout_ms: u64,
    pub max_connections: usize,
}

#[derive(Debug, Clone)]
pub struct PayloadConfig {
    pub max_size_bytes: usize,
    pub large_payload_threshold: usize,
    pub medium_payload_threshold: usize,
    pub max_step_count_large: usize,
    pub max_step_count_medium: usize,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            worker_pool: WorkerPoolConfig::default(),
            execution: ExecutionConfig::default(),
            webhook: WebhookConfig::default(),
            database: DatabaseConfig::default(),
            payload: PayloadConfig::default(),
        }
    }
}

impl Default for WorkerPoolConfig {
    fn default() -> Self {
        Self {
            min_workers: env::var("CRONFLOW_MIN_WORKERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            max_workers: env::var("CRONFLOW_MAX_WORKERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            worker_timeout_ms: env::var("CRONFLOW_WORKER_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30000), // 30 seconds
            queue_size: env::var("CRONFLOW_QUEUE_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_concurrent_steps: env::var("CRONFLOW_MAX_CONCURRENT_STEPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            default_timeout_ms: env::var("CRONFLOW_DEFAULT_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(Some)
                .unwrap_or(Some(30000)), // 30 seconds
            fail_fast: env::var("CRONFLOW_FAIL_FAST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(true),
            retry_attempts: env::var("CRONFLOW_RETRY_ATTEMPTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            retry_backoff_ms: env::var("CRONFLOW_RETRY_BACKOFF_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            max_backoff_ms: env::var("CRONFLOW_MAX_BACKOFF_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30000),
            retry_jitter: env::var("CRONFLOW_RETRY_JITTER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(true),
            max_retries: env::var("CRONFLOW_MAX_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
        }
    }
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            host: env::var("CRONFLOW_WEBHOOK_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("CRONFLOW_WEBHOOK_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            max_connections: env::var("CRONFLOW_WEBHOOK_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            default_path: env::var("CRONFLOW_DB_PATH")
                .unwrap_or_else(|_| "cronflow.db".to_string()),
            connection_timeout_ms: env::var("CRONFLOW_DB_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            max_connections: env::var("CRONFLOW_DB_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        }
    }
}

impl Default for PayloadConfig {
    fn default() -> Self {
        Self {
            max_size_bytes: env::var("CRONFLOW_MAX_PAYLOAD_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000_000), // 10MB
            large_payload_threshold: env::var("CRONFLOW_LARGE_PAYLOAD_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100_000), // 100KB
            medium_payload_threshold: env::var("CRONFLOW_MEDIUM_PAYLOAD_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000), // 10KB
            max_step_count_large: env::var("CRONFLOW_MAX_STEPS_LARGE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            max_step_count_medium: env::var("CRONFLOW_MAX_STEPS_MEDIUM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        }
    }
}

impl CoreConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_env() -> Self {
        Self::default() // Already loads from env in Default impl
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.worker_pool.min_workers == 0 {
            return Err("Minimum workers must be greater than 0".to_string());
        }
        
        if self.worker_pool.max_workers < self.worker_pool.min_workers {
            return Err("Maximum workers must be >= minimum workers".to_string());
        }
        
        if self.worker_pool.queue_size == 0 {
            return Err("Queue size must be greater than 0".to_string());
        }

        if self.execution.max_concurrent_steps == 0 {
            return Err("Max concurrent steps must be greater than 0".to_string());
        }
        
        if self.execution.retry_attempts == 0 {
            return Err("Retry attempts must be greater than 0".to_string());
        }

        if self.payload.max_size_bytes == 0 {
            return Err("Max payload size must be greater than 0".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_creation() {
        let config = CoreConfig::new();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_default_values() {
        let config = CoreConfig::default();
        
        assert_eq!(config.worker_pool.min_workers, 2);
        assert_eq!(config.worker_pool.max_workers, 10);
        assert_eq!(config.worker_pool.worker_timeout_ms, 30000);
        assert_eq!(config.worker_pool.queue_size, 1000);
        
        assert_eq!(config.execution.max_concurrent_steps, 10);
        assert_eq!(config.execution.default_timeout_ms, Some(30000));
        assert_eq!(config.execution.fail_fast, true);
        assert_eq!(config.execution.retry_attempts, 3);
        
        assert_eq!(config.payload.max_size_bytes, 10_000_000);
        assert_eq!(config.payload.large_payload_threshold, 100_000);
        assert_eq!(config.payload.medium_payload_threshold, 10_000);
    }

    #[test]
    fn test_config_validation() {
        let mut config = CoreConfig::default();
        
        config.worker_pool.min_workers = 0;
        assert!(config.validate().is_err());
        
        config.worker_pool.min_workers = 5;
        config.worker_pool.max_workers = 3;
        assert!(config.validate().is_err());
        
        config.worker_pool.min_workers = 2;
        config.worker_pool.max_workers = 10;
        assert!(config.validate().is_ok());
    }
}
