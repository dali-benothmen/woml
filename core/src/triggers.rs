//! Trigger system for the Node-Cronflow Core Engine
//!
//! This module provides trigger functionality for:
//! - Webhook triggers (HTTP-based)
//! - Manual triggers (programmatic)

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use crate::error::{CoreError, CoreResult};
use log;
use std::str::FromStr;

/// Webhook trigger configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTrigger {
    pub path: String,
    pub method: String,
    pub headers: Option<HashMap<String, String>>,
    pub validation: Option<WebhookValidation>,
}

impl WebhookTrigger {
    /// Create a new webhook trigger
    pub fn new(path: String, method: String) -> Self {
        Self {
            path,
            method: method.to_uppercase(),
            headers: None,
            validation: None,
        }
    }

    /// Add headers to the webhook trigger
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Add validation to the webhook trigger
    pub fn with_validation(mut self, validation: WebhookValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Validate the webhook trigger configuration
    pub fn validate(&self) -> CoreResult<()> {
        if self.path.is_empty() {
            return Err(CoreError::InvalidTrigger("Webhook path cannot be empty".to_string()));
        }

        if !self.path.starts_with('/') {
            return Err(CoreError::InvalidTrigger("Webhook path must start with /".to_string()));
        }

        let valid_methods = ["GET", "POST", "PUT", "DELETE", "PATCH"];
        if !valid_methods.contains(&self.method.as_str()) {
            return Err(CoreError::InvalidTrigger(format!("Invalid HTTP method: {}", self.method)));
        }

        Ok(())
    }
}

/// Webhook validation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookValidation {
    pub secret: Option<String>,
    pub signature_header: Option<String>,
    pub signature_algorithm: Option<String>,
    pub required_fields: Option<Vec<String>>,
}

impl WebhookValidation {
    /// Create a new webhook validation configuration
    pub fn new() -> Self {
        Self {
            secret: None,
            signature_header: None,
            signature_algorithm: None,
            required_fields: None,
        }
    }

    /// Add secret for signature validation
    pub fn with_secret(mut self, secret: String) -> Self {
        self.secret = Some(secret);
        self
    }

    /// Add signature header configuration
    pub fn with_signature_header(mut self, header: String, algorithm: String) -> Self {
        self.signature_header = Some(header);
        self.signature_algorithm = Some(algorithm);
        self
    }

    /// Add required fields validation
    pub fn with_required_fields(mut self, fields: Vec<String>) -> Self {
        self.required_fields = Some(fields);
        self
    }
}

/// Webhook request payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub query_params: HashMap<String, String>,
}

impl WebhookRequest {
    /// Create a new webhook request
    pub fn new(method: String, path: String) -> Self {
        Self {
            method: method.to_uppercase(),
            path,
            headers: HashMap::new(),
            body: None,
            query_params: HashMap::new(),
        }
    }

    /// Add headers to the request
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Add body to the request
    pub fn with_body(mut self, body: String) -> Self {
        self.body = Some(body);
        self
    }

    /// Add query parameters to the request
    pub fn with_query_params(mut self, params: HashMap<String, String>) -> Self {
        self.query_params = params;
        self
    }

    /// Validate the webhook request
    pub fn validate(&self) -> CoreResult<()> {
        if self.path.is_empty() {
            return Err(CoreError::InvalidTrigger("Request path cannot be empty".to_string()));
        }

        let valid_methods = ["GET", "POST", "PUT", "DELETE", "PATCH"];
        if !valid_methods.contains(&self.method.as_str()) {
            return Err(CoreError::InvalidTrigger(format!("Invalid HTTP method: {}", self.method)));
        }

        Ok(())
    }
}

/// Webhook response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

impl WebhookResponse {
    /// Create a new webhook response
    pub fn new(status_code: u16) -> Self {
        Self {
            status_code,
            headers: HashMap::new(),
            body: None,
        }
    }

    /// Add headers to the response
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Add body to the response
    pub fn with_body(mut self, body: String) -> Self {
        self.body = Some(body);
        self
    }

    /// Create a success response
    pub fn success() -> Self {
        Self::new(200).with_body("{\"status\":\"success\"}".to_string())
    }

    /// Create an error response
    pub fn error(status_code: u16, message: String) -> Self {
        Self::new(status_code).with_body(format!("{{\"error\":\"{}\"}}", message))
    }
}

/// Trigger manager for handling different types of triggers
#[derive(Debug)]
pub struct TriggerManager {
    pub webhook_triggers: HashMap<String, (WebhookTrigger, String)>, // path -> (trigger, workflow_id)
}

impl TriggerManager {
    /// Create a new trigger manager
    pub fn new() -> Self {
        Self {
            webhook_triggers: HashMap::new(),
        }
    }

    /// Register a webhook trigger for a workflow
    pub fn register_webhook_trigger(&mut self, workflow_id: &str, trigger: WebhookTrigger) -> CoreResult<()> {
        log::info!("Registering webhook trigger for workflow: {} at path: {}", workflow_id, trigger.path);
        
        // Validate the trigger
        trigger.validate()?;
        
        // Check if path is already registered
        if self.webhook_triggers.contains_key(&trigger.path) {
            return Err(CoreError::InvalidTrigger(format!("Webhook path {} is already registered", trigger.path)));
        }
        
        // Register the trigger
        let path = trigger.path.clone();
        self.webhook_triggers.insert(path.clone(), (trigger, workflow_id.to_string()));
        
        log::info!("Successfully registered webhook trigger for workflow: {} at path: {}", workflow_id, path);
        Ok(())
    }

    /// Handle a webhook request
    pub fn handle_webhook_request(&self, request: WebhookRequest) -> CoreResult<(String, serde_json::Value)> {
        log::info!("Handling webhook request: {} {}", request.method, request.path);
        
        // Validate the request
        request.validate()?;
        
        // Find the trigger for this path
        let (trigger, workflow_id) = self.webhook_triggers.get(&request.path)
            .ok_or_else(|| CoreError::TriggerNotFound(format!("No webhook trigger found for path: {}", request.path)))?;
        
        // Validate method matches
        if trigger.method != request.method {
            return Err(CoreError::InvalidTrigger(format!(
                "Method mismatch: expected {}, got {}", trigger.method, request.method
            )));
        }
        
        // Validate webhook if configured
        if let Some(validation) = &trigger.validation {
            self.validate_webhook(&request, validation)?;
        }
        
        // Prepare payload for workflow
        let payload = self.prepare_workflow_payload(&request)?;
        
        log::info!("Webhook request validated, triggering workflow: {}", workflow_id);
        Ok((workflow_id.clone(), payload))
    }

    /// Validate webhook request based on validation rules
    fn validate_webhook(&self, request: &WebhookRequest, validation: &WebhookValidation) -> CoreResult<()> {
        // Check required fields if specified
        if let Some(required_fields) = &validation.required_fields {
            // Parse body as JSON to check for required fields
            if let Some(body) = &request.body {
                let body_json: serde_json::Value = serde_json::from_str(body)
                    .map_err(|e| CoreError::InvalidTrigger(format!("Invalid JSON body: {}", e)))?;
                
                for field in required_fields {
                    if !body_json.get(field).is_some() {
                        return Err(CoreError::InvalidTrigger(format!("Missing required field: {}", field)));
                    }
                }
            } else {
                return Err(CoreError::InvalidTrigger("Request body is required for field validation".to_string()));
            }
        }
        
        // TODO: Implement signature validation when needed
        // For now, we'll skip signature validation as it requires crypto libraries
        
        Ok(())
    }

    /// Prepare payload for workflow execution
    fn prepare_workflow_payload(&self, request: &WebhookRequest) -> CoreResult<serde_json::Value> {
        let mut payload = serde_json::json!({
            "method": request.method,
            "path": request.path,
            "headers": request.headers,
            "query_params": request.query_params,
        });
        
        // Add body if present
        if let Some(body) = &request.body {
            // Try to parse as JSON, fallback to string
            if let Ok(body_json) = serde_json::from_str::<serde_json::Value>(body) {
                payload["body"] = body_json;
            } else {
                payload["body"] = serde_json::Value::String(body.clone());
            }
        }
        
        Ok(payload)
    }

    /// Get all registered webhook triggers
    pub fn get_webhook_triggers(&self) -> Vec<(String, WebhookTrigger, String)> {
        self.webhook_triggers
            .iter()
            .map(|(path, (trigger, workflow_id))| (path.clone(), trigger.clone(), workflow_id.clone()))
            .collect()
    }

    /// Check if a webhook path is registered
    pub fn has_webhook_trigger(&self, path: &str) -> bool {
        self.webhook_triggers.contains_key(path)
    }

    /// Get workflow ID for a webhook path
    pub fn get_workflow_id_for_webhook(&self, path: &str) -> Option<&String> {
        self.webhook_triggers.get(path).map(|(_, workflow_id)| workflow_id)
    }
}

impl Default for TriggerManager {
    fn default() -> Self {
        Self::new()
    }
} 