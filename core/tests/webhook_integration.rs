//! Integration tests for webhook server functionality
//! These tests verify our webhook server enhancements work correctly

use core::webhook_server::{validate_hmac_sha256, validate_hmac_sha1, WebhookServerConfig, WebhookServerBuilder};
use core::triggers::{TriggerManager, WebhookTrigger, WebhookValidation};
use core::state::StateManager;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sha1::Sha1;
use hex;

#[test]
fn test_webhook_server_configuration() {
    // Test default configuration
    let config = WebhookServerConfig::default();
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3000);
    assert_eq!(config.max_connections, 1000);
    assert_eq!(config.graceful_shutdown_timeout, Duration::from_secs(30));
    
    // Test builder pattern
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
fn test_hmac_sha256_validation_github_format() {
    let secret = "github-webhook-secret";
    let payload = b"{\"action\":\"opened\",\"pull_request\":{\"id\":1}}";
    
    // Generate GitHub-style signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload);
    let expected_signature = hex::encode(mac.finalize().into_bytes());
    let github_signature = format!("sha256={}", expected_signature);
    
    // Test validation
    assert!(validate_hmac_sha256(secret, payload, &github_signature).is_ok());
    assert!(validate_hmac_sha256(secret, payload, &expected_signature).is_ok());
    
    // Test with wrong secret
    assert!(validate_hmac_sha256("wrong-secret", payload, &github_signature).is_err());
    
    // Test with tampered payload
    let tampered_payload = b"{\"action\":\"closed\",\"pull_request\":{\"id\":1}}";
    assert!(validate_hmac_sha256(secret, tampered_payload, &github_signature).is_err());
}

#[test]
fn test_hmac_sha1_validation_github_format() {
    let secret = "github-webhook-secret-sha1";
    let payload = b"{\"zen\":\"Non-blocking is better than blocking.\"}";
    
    // Generate GitHub-style SHA1 signature
    type HmacSha1 = Hmac<Sha1>;
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload);
    let expected_signature = hex::encode(mac.finalize().into_bytes());
    let github_signature = format!("sha1={}", expected_signature);
    
    // Test validation
    assert!(validate_hmac_sha1(secret, payload, &github_signature).is_ok());
    assert!(validate_hmac_sha1(secret, payload, &expected_signature).is_ok());
    
    // Test with wrong secret
    assert!(validate_hmac_sha1("wrong-secret", payload, &github_signature).is_err());
    
    // Test with tampered payload
    let tampered_payload = b"{\"zen\":\"Blocking is better than non-blocking.\"}";
    assert!(validate_hmac_sha1(secret, tampered_payload, &github_signature).is_err());
}

#[test]
fn test_case_insensitive_signature_validation() {
    let secret = "test-case-sensitivity";
    let payload = b"test payload for case sensitivity";
    
    // Generate signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload);
    let signature = hex::encode(mac.finalize().into_bytes());
    
    // Test with different cases
    assert!(validate_hmac_sha256(secret, payload, &signature.to_lowercase()).is_ok());
    assert!(validate_hmac_sha256(secret, payload, &signature.to_uppercase()).is_ok());
    
    let github_format_lower = format!("sha256={}", signature.to_lowercase());
    let github_format_upper = format!("sha256={}", signature.to_uppercase());
    
    assert!(validate_hmac_sha256(secret, payload, &github_format_lower).is_ok());
    assert!(validate_hmac_sha256(secret, payload, &github_format_upper).is_ok());
}

#[test]
fn test_webhook_lifecycle_operations() {
    let mut trigger_manager = TriggerManager::new();
    
    // Test registering multiple webhook triggers
    let trigger1 = WebhookTrigger::new("/webhook/github".to_string(), "POST".to_string());
    let trigger2 = WebhookTrigger::new("/webhook/gitlab".to_string(), "POST".to_string());
    
    assert!(trigger_manager.register_webhook_trigger("github-workflow", trigger1).is_ok());
    assert!(trigger_manager.register_webhook_trigger("gitlab-workflow", trigger2).is_ok());
    
    // Test retrieval
    assert!(trigger_manager.has_webhook_trigger("/webhook/github"));
    assert!(trigger_manager.has_webhook_trigger("/webhook/gitlab"));
    assert!(!trigger_manager.has_webhook_trigger("/webhook/nonexistent"));
    
    // Test workflow ID retrieval
    assert_eq!(
        trigger_manager.get_workflow_id_for_webhook("/webhook/github"),
        Some(&"github-workflow".to_string())
    );
    assert_eq!(
        trigger_manager.get_workflow_id_for_webhook("/webhook/gitlab"),
        Some(&"gitlab-workflow".to_string())
    );
    
    // Test getting all triggers
    let all_triggers = trigger_manager.get_webhook_triggers();
    assert_eq!(all_triggers.len(), 2);
}

#[test]
fn test_webhook_validation_configurations() {
    // Test validation with only secret
    let validation1 = WebhookValidation::new()
        .with_secret("secret-only".to_string());
    
    assert_eq!(validation1.secret.as_ref().unwrap(), "secret-only");
    assert!(validation1.signature_header.is_none());
    assert!(validation1.signature_algorithm.is_none());
    
    // Test validation with custom header and algorithm
    let validation2 = WebhookValidation::new()
        .with_secret("custom-secret".to_string())
        .with_signature_header("X-Custom-Signature".to_string(), "sha256".to_string())
        .with_required_fields(vec!["action".to_string(), "repository".to_string()]);
    
    assert_eq!(validation2.secret.as_ref().unwrap(), "custom-secret");
    assert_eq!(validation2.signature_header.as_ref().unwrap(), "X-Custom-Signature");
    assert_eq!(validation2.signature_algorithm.as_ref().unwrap(), "sha256");
    assert_eq!(validation2.required_fields.as_ref().unwrap().len(), 2);
}

#[test]
fn test_webhook_server_is_not_running_initially() {
    let server = WebhookServerBuilder::new()
        .host("127.0.0.1".to_string())
        .port(0) // Use port 0 for automatic assignment
        .build(
            Arc::new(Mutex::new(TriggerManager::new())),
            Arc::new(Mutex::new(StateManager::new(":memory:").unwrap())),
        );
    
    // Server should not be running initially
    assert!(!server.is_running());
}
