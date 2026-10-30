//! Original ingress authentication regression cases; test paths remain stable.

use super::authentication::bearer_token_matches;
use super::whatsapp::{
  bounded_whatsapp_message_id, whatsapp_decision_payload, whatsapp_signature_valid,
};
use sha2::Digest;
use sha2::Sha256;

#[test]
fn bearer_comparison_hashes_every_candidate_to_the_same_fixed_width() {
  let expected: [u8; 32] = Sha256::digest(b"correct-token").into();
  assert!(bearer_token_matches(&expected, "correct-token"));
  assert!(!bearer_token_matches(&expected, "x"));
  assert!(!bearer_token_matches(&expected, &"x".repeat(8_192)));
}

#[test]
fn whatsapp_signature_uses_the_exact_raw_body_and_decision_payload_is_bounded() {
  let request = actix_web::test::TestRequest::default()
    .insert_header((
      "X-Hub-Signature-256",
      "sha256=a1b988f9e9b280c23f5805491795698bc4ea24900417d0ef08726447ced6ebe7",
    ))
    .to_http_request();
  assert!(whatsapp_signature_valid(
    &request,
    br#"{"text":"hello"}"#,
    "test-secret"
  ));
  assert!(!whatsapp_signature_valid(
    &request,
    br#"{"text":"hello!"}"#,
    "test-secret"
  ));
  assert!(matches!(
    whatsapp_decision_payload(
      "woml_approve:ncap_1234567890abcdef.1234567890abcdef1234567890abcdef"
    ),
    Some((
      crate::ApprovalDecision::Approved,
      "ncap_1234567890abcdef.1234567890abcdef1234567890abcdef"
    ))
  ));
  assert!(whatsapp_decision_payload("woml_approve:ncap_1234.secret").is_none());
  assert!(whatsapp_decision_payload(&format!(
    "woml_reject:ncap_{}.{}x",
    "a".repeat(16),
    "b".repeat(32)
  ))
  .is_none());
  assert!(bounded_whatsapp_message_id("wamid.message-1"));
  assert!(!bounded_whatsapp_message_id(&"x".repeat(513)));
}
