//! Standalone test for webhook server functionality
//! This test verifies the enhanced webhook server implementation works correctly

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::webhook_server::{WebhookServer, WebhookServerConfig, WebhookServerBuilder};
use crate::triggers::{TriggerManager, WebhookTrigger, WebhookValidation};
use crate::state::StateManager;

#[test]
fn test_webhook_server_creation() {
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
            Arc::new(Mutex::new(StateManager::new(":memory:").unwrap())),
        );
    
    let config = server.get_config();
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.port, 8080);
    assert_eq!(config.max_connections, 2000);
    assert_eq!(config.graceful_shutdown_timeout, Duration::from_secs(60));
}

#[test]
fn test_webhook_trigger_with_validation() {
    let mut trigger_manager = TriggerManager::new();
    
    // Create a webhook trigger with signature validation
    let validation = WebhookValidation::new()
        .with_secret("test-secret".to_string())
        .with_signature_header("x-hub-signature-256".to_string(), "sha256".to_string());
    
    let trigger = WebhookTrigger::new("/webhook/secure".to_string(), "POST".to_string())
        .with_validation(validation);
    
    let result = trigger_manager.register_webhook_trigger("secure-workflow", trigger);
    assert!(result.is_ok(), "Webhook trigger registration should succeed");
    
    // Test that we can retrieve the trigger configuration
    let (retrieved_trigger, workflow_id) = trigger_manager.get_webhook_trigger("/webhook/secure").unwrap();
    assert_eq!(workflow_id, "secure-workflow");
    assert!(retrieved_trigger.validation.is_some());
    
    let validation = retrieved_trigger.validation.as_ref().unwrap();
    assert_eq!(validation.secret.as_ref().unwrap(), "test-secret");
    assert_eq!(validation.signature_header.as_ref().unwrap(), "x-hub-signature-256");
    assert_eq!(validation.signature_algorithm.as_ref().unwrap(), "sha256");
}

#[test]
fn test_hmac_sha256_validation() {
    use crate::webhook_server::validate_hmac_sha256;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use hex;
    
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
    use crate::webhook_server::validate_hmac_sha1;
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    use hex;
    
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
fn test_webhook_server_lifecycle() {
    let mut server = WebhookServerBuilder::new()
        .host("127.0.0.1".to_string())
        .port(0) // Use port 0 for automatic port assignment in tests
        .graceful_shutdown_timeout(Duration::from_millis(100))
        .build(
            Arc::new(Mutex::new(TriggerManager::new())),
            Arc::new(Mutex::new(StateManager::new(":memory:").unwrap())),
        );
    
    // Test initial state
    assert!(!server.is_running());
    
    // Note: We don't test actual server start/stop in unit tests as it requires 
    // async runtime and port binding, but we verify the configuration is correct
    let config = server.get_config();
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 0);
    assert_eq!(config.graceful_shutdown_timeout, Duration::from_millis(100));
}
