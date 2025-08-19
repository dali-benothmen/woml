//! Webhook HTTP server for the Node-Cronflow Core Engine
//! 
//! This module provides an HTTP server that can receive webhook requests
//! and trigger workflows based on the incoming requests.

use actix_web::{web, App, HttpServer, HttpRequest, HttpResponse, Responder, middleware};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use tokio::signal;
use log;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sha1::Sha1;
use hex;

use crate::error::{CoreError, CoreResult};
use crate::triggers::{TriggerManager, WebhookRequest, WebhookResponse};
use crate::state::StateManager;

/// Webhook server configuration
#[derive(Debug, Clone)]
pub struct WebhookServerConfig {
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
    pub graceful_shutdown_timeout: Duration,
}

impl Default for WebhookServerConfig {
    fn default() -> Self {
        // Use centralized configuration
        let core_config = crate::config::CoreConfig::default();
        Self {
            host: core_config.webhook.host,
            port: core_config.webhook.port,
            max_connections: core_config.webhook.max_connections,
            graceful_shutdown_timeout: Duration::from_secs(30),
        }
    }
}

/// Webhook server instance with graceful shutdown support
pub struct WebhookServer {
    config: WebhookServerConfig,
    trigger_manager: Arc<Mutex<TriggerManager>>,
    state_manager: Arc<Mutex<StateManager>>,
    shutdown_flag: Arc<AtomicBool>,
    server_handle: Option<tokio::task::JoinHandle<Result<(), std::io::Error>>>,
}

impl WebhookServer {
    /// Create a new webhook server
    pub fn new(
        config: WebhookServerConfig,
        trigger_manager: Arc<Mutex<TriggerManager>>,
        state_manager: Arc<Mutex<StateManager>>,
    ) -> Self {
        Self {
            config,
            trigger_manager,
            state_manager,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            server_handle: None,
        }
    }

    /// Start the webhook server with graceful shutdown support
    pub async fn start(&mut self) -> CoreResult<()> {
        log::info!("Starting webhook server on {}:{}", self.config.host, self.config.port);
        
        let trigger_manager = self.trigger_manager.clone();
        let state_manager = self.state_manager.clone();
        let shutdown_flag = self.shutdown_flag.clone();
        let graceful_timeout = self.config.graceful_shutdown_timeout;
        
        let server = HttpServer::new(move || {
            App::new()
                .wrap(middleware::Logger::default())
                .app_data(web::Data::new(trigger_manager.clone()))
                .app_data(web::Data::new(state_manager.clone()))
                .route("/webhook/{path:.*}", web::post().to(webhook_handler))
                .route("/health", web::get().to(health_check))
                .route("/shutdown", web::post().to(shutdown_handler))
        })
        .bind(format!("{}:{}", self.config.host, self.config.port))
        .map_err(|e| CoreError::Configuration(format!("Failed to bind webhook server: {}", e)))?
        .workers(4)
        .shutdown_timeout(graceful_timeout.as_secs())
        .run();
        
        // Handle graceful shutdown
        let server_handle = tokio::spawn(async move {
            tokio::select! {
                result = server => {
                    log::info!("Webhook server stopped");
                    result
                }
                _ = wait_for_shutdown_signal(shutdown_flag) => {
                    log::info!("Shutdown signal received, stopping webhook server gracefully");
                    Ok(())
                }
            }
        });
        
        self.server_handle = Some(server_handle);
        log::info!("Webhook server started successfully with graceful shutdown support");
        Ok(())
    }

    /// Stop the webhook server gracefully
    pub async fn stop(&mut self) -> CoreResult<()> {
        log::info!("Initiating graceful shutdown of webhook server");
        
        // Set shutdown flag
        self.shutdown_flag.store(true, Ordering::SeqCst);
        
        // Wait for server to shutdown
        if let Some(handle) = self.server_handle.take() {
            match tokio::time::timeout(self.config.graceful_shutdown_timeout, handle).await {
                Ok(result) => {
                    match result {
                        Ok(_) => log::info!("Webhook server stopped gracefully"),
                        Err(e) => log::error!("Error during server shutdown: {}", e),
                    }
                }
                Err(_) => {
                    log::warn!("Webhook server shutdown timed out after {:?}", self.config.graceful_shutdown_timeout);
                }
            }
        }
        
        log::info!("Webhook server shutdown complete");
        Ok(())
    }

    /// Check if the server is running
    pub fn is_running(&self) -> bool {
        self.server_handle.as_ref().map_or(false, |handle| !handle.is_finished())
    }

    /// Get server configuration
    pub fn get_config(&self) -> &WebhookServerConfig {
        &self.config
    }
}

/// Wait for shutdown signal (SIGINT or SIGTERM)
async fn wait_for_shutdown_signal(shutdown_flag: Arc<AtomicBool>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let shutdown_check = async {
        loop {
            if shutdown_flag.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = shutdown_check => {},
    }
}

/// Health check endpoint
async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "node-cronflow-webhook-server",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "version": crate::VERSION,
    }))
}

/// Graceful shutdown endpoint
async fn shutdown_handler() -> impl Responder {
    log::info!("Shutdown endpoint called");
    HttpResponse::Ok().json(serde_json::json!({
        "status": "shutdown_initiated",
        "message": "Server shutdown initiated",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

/// Main webhook handler with signature validation
async fn webhook_handler(
    req: HttpRequest,
    body: web::Bytes,
    trigger_manager: web::Data<Arc<Mutex<TriggerManager>>>,
    state_manager: web::Data<Arc<Mutex<StateManager>>>,
) -> impl Responder {
    let path = req.path().to_string();
    let method = req.method().as_str().to_string();
    
    log::info!("Received webhook request: {} {}", method, path);
    
    // Extract headers
    let mut headers = HashMap::new();
    for (key, value) in req.headers() {
        if let Ok(value_str) = value.to_str() {
            headers.insert(key.as_str().to_string(), value_str.to_string());
        }
    }
    
    // Extract query parameters
    let mut query_params = HashMap::new();
    if let Some(query) = req.uri().query() {
        for param in query.split('&') {
            if let Some((k, v)) = param.split_once('=') {
                query_params.insert(k.to_string(), v.to_string());
            }
        }
    }
    
    // Convert body to string
    let body_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Invalid UTF-8 in request body: {}", e);
            return HttpResponse::BadRequest().json(serde_json::json!({
                "status": "error",
                "message": "Invalid request body encoding",
                "workflow_triggered": false,
            }));
        }
    };
    
    let webhook_request = WebhookRequest::new(method.clone(), path.clone())
        .with_headers(headers.clone())
        .with_body(body_str.clone())
        .with_query_params(query_params);
    
    // Validate signature if configured
    if let Err(signature_error) = validate_webhook_signature(&webhook_request, &body.to_vec(), &trigger_manager).await {
        log::error!("Webhook signature validation failed: {} {} - {}", method, path, signature_error);
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "status": "error",
            "message": format!("Signature validation failed: {}", signature_error),
            "workflow_triggered": false,
        }));
    }
    
    // Handle the webhook request
    match handle_webhook_request(webhook_request, trigger_manager, state_manager).await {
        Ok(_response) => {
            log::info!("Webhook request processed successfully: {} {}", method, path);
            HttpResponse::Ok().json(serde_json::json!({
                "status": "success",
                "message": "Webhook processed successfully",
                "workflow_triggered": true,
            }))
        }
        Err(e) => {
            log::error!("Webhook request failed: {} {} - {}", method, path, e);
            HttpResponse::BadRequest().json(serde_json::json!({
                "status": "error",
                "message": e.to_string(),
                "workflow_triggered": false,
            }))
        }
    }
}

/// Handle webhook request and trigger workflow
async fn handle_webhook_request(
    request: WebhookRequest,
    trigger_manager: web::Data<Arc<Mutex<TriggerManager>>>,
    state_manager: web::Data<Arc<Mutex<StateManager>>>,
) -> CoreResult<WebhookResponse> {
    let trigger_manager_guard = trigger_manager.lock()
        .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
    
    // Handle the webhook request
    let (workflow_id, payload) = trigger_manager_guard.handle_webhook_request(request)?;
    
    let mut state_manager_guard = state_manager.lock()
        .map_err(|e| CoreError::Internal(format!("Failed to acquire state manager lock: {}", e)))?;
    
    let run_id = state_manager_guard.create_run(&workflow_id, payload)?;
    
    log::info!("Created workflow run {} for webhook-triggered workflow {}", run_id, workflow_id);
    
    Ok(WebhookResponse::success())
}

/// Validate webhook signature using HMAC
async fn validate_webhook_signature(
    request: &WebhookRequest,
    body: &[u8],
    trigger_manager: &web::Data<Arc<Mutex<TriggerManager>>>,
) -> CoreResult<()> {
    let trigger_manager_guard = trigger_manager.lock()
        .map_err(|e| CoreError::Internal(format!("Failed to acquire trigger manager lock: {}", e)))?;
    
    // Get the webhook trigger configuration
    let (trigger, _workflow_id) = trigger_manager_guard.get_webhook_trigger(&request.path)
        .ok_or_else(|| CoreError::TriggerNotFound(format!("No webhook trigger found for path: {}", request.path)))?;
    
    // If no validation is configured, skip signature validation
    let validation = match &trigger.validation {
        Some(v) => v,
        None => return Ok(()),
    };
    
    // If no secret is configured, skip signature validation
    let secret = match &validation.secret {
        Some(s) => s,
        None => return Ok(()),
    };
    
    // Get signature header (default to X-Hub-Signature-256 for GitHub compatibility)
    let signature_header = validation.signature_header.as_deref().unwrap_or("x-hub-signature-256");
    let signature_algorithm = validation.signature_algorithm.as_deref().unwrap_or("sha256");
    
    // Get the signature from headers
    let received_signature = request.headers.get(signature_header)
        .ok_or_else(|| CoreError::InvalidTrigger(format!("Missing signature header: {}", signature_header)))?;
    
    // Validate signature based on algorithm
    match signature_algorithm.to_lowercase().as_str() {
        "sha256" => validate_hmac_sha256(secret, body, received_signature),
        "sha1" => validate_hmac_sha1(secret, body, received_signature),
        _ => Err(CoreError::InvalidTrigger(format!("Unsupported signature algorithm: {}", signature_algorithm))),
    }
}

/// Validate HMAC SHA-256 signature
pub fn validate_hmac_sha256(secret: &str, body: &[u8], received_signature: &str) -> CoreResult<()> {
    type HmacSha256 = Hmac<Sha256>;
    
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| CoreError::Internal(format!("Invalid HMAC key: {}", e)))?;
    
    mac.update(body);
    let expected_signature = mac.finalize().into_bytes();
    let expected_hex = hex::encode(expected_signature);
    
    // Support both GitHub format (sha256=...) and raw hex
    let received_hex = if received_signature.starts_with("sha256=") {
        &received_signature[7..]
    } else {
        received_signature
    };
    
    if expected_hex.eq_ignore_ascii_case(received_hex) {
        Ok(())
    } else {
        Err(CoreError::InvalidTrigger("HMAC signature mismatch".to_string()))
    }
}

/// Validate HMAC SHA-1 signature
pub fn validate_hmac_sha1(secret: &str, body: &[u8], received_signature: &str) -> CoreResult<()> {
    type HmacSha1 = Hmac<Sha1>;
    
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
        .map_err(|e| CoreError::Internal(format!("Invalid HMAC key: {}", e)))?;
    
    mac.update(body);
    let expected_signature = mac.finalize().into_bytes();
    let expected_hex = hex::encode(expected_signature);
    
    // Support both GitHub format (sha1=...) and raw hex
    let received_hex = if received_signature.starts_with("sha1=") {
        &received_signature[6..]
    } else {
        received_signature
    };
    
    if expected_hex.eq_ignore_ascii_case(received_hex) {
        Ok(())
    } else {
        Err(CoreError::InvalidTrigger("HMAC signature mismatch".to_string()))
    }
}

/// Webhook server builder for easy configuration
pub struct WebhookServerBuilder {
    config: WebhookServerConfig,
}

impl WebhookServerBuilder {
    /// Create a new webhook server builder
    pub fn new() -> Self {
        Self {
            config: WebhookServerConfig::default(),
        }
    }
    
    /// Set the host address
    pub fn host(mut self, host: String) -> Self {
        self.config.host = host;
        self
    }
    
    /// Set the port number
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }
    
    /// Set the maximum number of connections
    pub fn max_connections(mut self, max_connections: usize) -> Self {
        self.config.max_connections = max_connections;
        self
    }
    
    /// Set the graceful shutdown timeout
    pub fn graceful_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.config.graceful_shutdown_timeout = timeout;
        self
    }
    
    /// Build the webhook server
    pub fn build(
        self,
        trigger_manager: Arc<Mutex<TriggerManager>>,
        state_manager: Arc<Mutex<StateManager>>,
    ) -> WebhookServer {
        WebhookServer::new(self.config, trigger_manager, state_manager)
    }
}

impl Default for WebhookServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{WebhookTrigger, TriggerManager};
    use crate::state::StateManager;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tempfile::NamedTempFile;

    #[test]
    fn test_webhook_server_config() {
        let config = WebhookServerConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
        assert_eq!(config.max_connections, 1000);
        assert_eq!(config.graceful_shutdown_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_webhook_server_builder() {
        let server = WebhookServerBuilder::new()
            .host("0.0.0.0".to_string())
            .port(8080)
            .max_connections(2000)
            .graceful_shutdown_timeout(Duration::from_secs(60))
            .build(
                Arc::new(Mutex::new(TriggerManager::new())),
                Arc::new(Mutex::new(StateManager::new("test.db").unwrap())),
            );
        
        let config = server.get_config();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.max_connections, 2000);
        assert_eq!(config.graceful_shutdown_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_webhook_trigger_registration() {
        let mut trigger_manager = TriggerManager::new();
        let trigger = WebhookTrigger::new("/webhook/test".to_string(), "POST".to_string());
        
        let result = trigger_manager.register_webhook_trigger("test-workflow", trigger);
        assert!(result.is_ok(), "Webhook trigger registration should succeed");
        
        assert!(trigger_manager.has_webhook_trigger("/webhook/test"));
        assert_eq!(
            trigger_manager.get_workflow_id_for_webhook("/webhook/test"),
            Some(&"test-workflow".to_string())
        );
    }

    #[test]
    fn test_hmac_sha256_validation() {
        let secret = "my-secret-key";
        let body = b"test payload";
        
        // Generate expected signature
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let expected_signature = hex::encode(mac.finalize().into_bytes());
        
        // Test with GitHub format
        let github_format = format!("sha256={}", expected_signature);
        assert!(validate_hmac_sha256(secret, body, &github_format).is_ok());
        
        // Test with raw hex format
        assert!(validate_hmac_sha256(secret, body, &expected_signature).is_ok());
        
        // Test with invalid signature
        assert!(validate_hmac_sha256(secret, body, "invalid-signature").is_err());
    }

    #[test]
    fn test_hmac_sha1_validation() {
        let secret = "my-secret-key";
        let body = b"test payload";
        
        // Generate expected signature
        type HmacSha1 = Hmac<Sha1>;
        let mut mac = HmacSha1::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let expected_signature = hex::encode(mac.finalize().into_bytes());
        
        // Test with GitHub format
        let github_format = format!("sha1={}", expected_signature);
        assert!(validate_hmac_sha1(secret, body, &github_format).is_ok());
        
        // Test with raw hex format
        assert!(validate_hmac_sha1(secret, body, &expected_signature).is_ok());
        
        // Test with invalid signature
        assert!(validate_hmac_sha1(secret, body, "invalid-signature").is_err());
    }

    #[test]
    fn test_webhook_signature_validation_with_trigger() {
        use crate::triggers::{WebhookValidation};
        
        let mut trigger_manager = TriggerManager::new();
        
        // Create a webhook trigger with signature validation
        let validation = WebhookValidation::new()
            .with_secret("test-secret".to_string())
            .with_signature_header("x-hub-signature-256".to_string(), "sha256".to_string());
        
        let trigger = WebhookTrigger::new("/webhook/secure".to_string(), "POST".to_string())
            .with_validation(validation);
        
        trigger_manager.register_webhook_trigger("secure-workflow", trigger).unwrap();
        
        // Test that we can retrieve the trigger configuration
        let (retrieved_trigger, workflow_id) = trigger_manager.get_webhook_trigger("/webhook/secure").unwrap();
        assert_eq!(workflow_id, "secure-workflow");
        assert!(retrieved_trigger.validation.is_some());
        
        let validation = retrieved_trigger.validation.as_ref().unwrap();
        assert_eq!(validation.secret.as_ref().unwrap(), "test-secret");
        assert_eq!(validation.signature_header.as_ref().unwrap(), "x-hub-signature-256");
        assert_eq!(validation.signature_algorithm.as_ref().unwrap(), "sha256");
    }
} 