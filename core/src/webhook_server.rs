//! Webhook HTTP server for the Node-Cronflow Core Engine
//! 
//! This module provides an HTTP server that can receive webhook requests
//! and trigger workflows based on the incoming requests.

use actix_web::{web, App, HttpServer, HttpRequest, HttpResponse, Responder, middleware};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use log;

use crate::error::{CoreError, CoreResult};
use crate::triggers::{TriggerManager, WebhookRequest, WebhookResponse};
use crate::state::StateManager;

/// Webhook server configuration
#[derive(Debug, Clone)]
pub struct WebhookServerConfig {
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
}

impl Default for WebhookServerConfig {
    fn default() -> Self {
        // Use centralized configuration
        let core_config = crate::config::CoreConfig::default();
        Self {
            host: core_config.webhook.host,
            port: core_config.webhook.port,
            max_connections: core_config.webhook.max_connections,
        }
    }
}

/// Webhook server instance
pub struct WebhookServer {
    config: WebhookServerConfig,
    trigger_manager: Arc<Mutex<TriggerManager>>,
    state_manager: Arc<Mutex<StateManager>>,
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
        }
    }

    /// Start the webhook server
    pub async fn start(&self) -> CoreResult<()> {
        log::info!("Starting webhook server on {}:{}", self.config.host, self.config.port);
        
        let trigger_manager = self.trigger_manager.clone();
        let state_manager = self.state_manager.clone();
        
        HttpServer::new(move || {
            App::new()
                .wrap(middleware::Logger::default())
                .app_data(web::Data::new(trigger_manager.clone()))
                .app_data(web::Data::new(state_manager.clone()))
                .route("/webhook/{path:.*}", web::post().to(webhook_handler))
                .route("/health", web::get().to(health_check))
        })
        .bind(format!("{}:{}", self.config.host, self.config.port))
        .map_err(|e| CoreError::Configuration(format!("Failed to bind webhook server: {}", e)))?
        .workers(4)
        .run()
        .await
        .map_err(|e| CoreError::Internal(format!("Webhook server error: {}", e)))?;
        
        Ok(())
    }

    /// Get server configuration
    pub fn get_config(&self) -> &WebhookServerConfig {
        &self.config
    }
}

/// Health check endpoint
async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "node-cronflow-webhook-server",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

/// Main webhook handler
async fn webhook_handler(
    req: HttpRequest,
    payload: web::Json<Value>,
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
    
    let webhook_request = WebhookRequest::new(method.clone(), path.clone())
        .with_headers(headers)
        .with_body(payload.to_string())
        .with_query_params(query_params);
    
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
    }

    #[test]
    fn test_webhook_server_builder() {
        let server = WebhookServerBuilder::new()
            .host("0.0.0.0".to_string())
            .port(8080)
            .max_connections(2000)
            .build(
                Arc::new(Mutex::new(TriggerManager::new())),
                Arc::new(Mutex::new(StateManager::new("test.db").unwrap())),
            );
        
        let config = server.get_config();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.max_connections, 2000);
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
} 