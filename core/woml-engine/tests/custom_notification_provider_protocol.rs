use std::collections::HashSet;

use serde_json::{json, Value};
use woml_engine::{
  decode_custom_provider_frame, encode_custom_provider_frame, CustomProviderCancelMessage,
  CustomProviderCompletedMessage, CustomProviderExecuteMessage, CustomProviderOutcome,
  CustomProviderReadyMessage, CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
};

fn fixture() -> Value {
  serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/reusable-definitions/provider-protocol.v1.json"
  ))
  .expect("provider protocol fixture")
}

#[test]
fn validates_frozen_execute_cancel_ready_and_completed_messages() {
  let fixture = fixture();
  let execute: CustomProviderExecuteMessage =
    serde_json::from_value(fixture["execute"].clone()).expect("execute");
  execute.validate().expect("valid execute");
  execute
    .validate_with_artifacts(&HashSet::from([execute.script_artifact_id.clone()]))
    .expect("registered artifact");
  assert!(execute
    .validate_with_artifacts(&HashSet::from(["another-artifact".to_string()]))
    .unwrap_err()
    .contains("not registered"));

  let cancel: CustomProviderCancelMessage =
    serde_json::from_value(fixture["cancel"].clone()).expect("cancel");
  cancel.validate().expect("valid cancel");
  let ready: CustomProviderReadyMessage =
    serde_json::from_value(fixture["ready"].clone()).expect("ready");
  ready.validate().expect("valid ready");
  let completed: CustomProviderCompletedMessage =
    serde_json::from_value(fixture["completed"].clone()).expect("completed");
  completed.validate().expect("valid completed");
}

#[test]
fn frames_multibyte_and_literal_crlf_by_utf8_bytes() {
  let execute: CustomProviderExecuteMessage =
    serde_json::from_value(fixture()["execute"].clone()).expect("execute");
  let frame = encode_custom_provider_frame(&execute).expect("frame");
  let header_end = frame
    .windows(4)
    .position(|window| window == b"\r\n\r\n")
    .expect("header terminator");
  let header = std::str::from_utf8(&frame[..header_end]).expect("header");
  let declared = header
    .strip_prefix("Content-Length: ")
    .expect("length header")
    .parse::<usize>()
    .expect("byte count");
  assert_eq!(declared, frame.len() - header_end - 4);
  assert!(declared > serde_json::to_string(&execute).unwrap().chars().count());

  let decoded: CustomProviderExecuteMessage =
    decode_custom_provider_frame(&frame, CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES)
      .expect("decode");
  assert_eq!(decoded, execute);
  assert!(
    decode_custom_provider_frame::<CustomProviderExecuteMessage>(&frame, 16)
      .unwrap_err()
      .contains("size limit")
  );
}

#[test]
fn accepts_out_of_order_completions_and_every_failure_kind() {
  let fixture = fixture();
  let mut second = fixture["completed"].clone();
  second["invocationId"] = json!("provider_invocation_2");
  let mut first = fixture["completed"].clone();
  first["invocationId"] = json!("provider_invocation_1");
  let out_of_order = [second, first]
    .into_iter()
    .map(|value| {
      let message: CustomProviderCompletedMessage =
        serde_json::from_value(value).expect("completion");
      message.validate().expect("valid completion");
      message.invocation_id
    })
    .collect::<Vec<_>>();
  assert_eq!(
    out_of_order,
    ["provider_invocation_2", "provider_invocation_1"]
  );

  for kind in [
    "script_threw",
    "timed_out",
    "cancelled",
    "non_json",
    "worker_crashed",
    "host_crashed",
    "context_too_large",
    "result_too_large",
    "delivery_ambiguous",
    "service_failed",
    "request_invalid",
  ] {
    let value = json!({
      "protocol": "woml.custom-notification-provider",
      "protocolVersion": 1,
      "messageType": "completed",
      "invocationId": format!("failure_{kind}"),
      "durationMs": 1.0,
      "outcome": {
        "kind": "failed",
        "error": {
          "kind": kind,
          "code": format!("WOML_PROVIDER_{}", kind.to_uppercase()),
          "message": format!("Safe {kind} failure."),
          "retryable": kind == "service_failed"
        }
      }
    });
    let message: CustomProviderCompletedMessage =
      serde_json::from_value(value).expect("failure kind");
    message.validate().expect("valid failure");
  }
}

#[test]
fn rejects_arbitrary_receipt_persistence_and_invalid_attempts() {
  let mut response_bearing = fixture()["completed"].clone();
  response_bearing["outcome"]["receipt"]["responseBody"] = json!("secret response");
  assert!(serde_json::from_value::<CustomProviderCompletedMessage>(response_bearing).is_err());

  let mut execute: CustomProviderExecuteMessage =
    serde_json::from_value(fixture()["execute"].clone()).expect("execute");
  execute.attempt.number = execute.attempt.max + 1;
  assert!(execute.validate().is_err());

  let completed: CustomProviderCompletedMessage =
    serde_json::from_value(fixture()["completed"].clone()).expect("completed");
  assert!(matches!(
    completed.outcome,
    CustomProviderOutcome::Succeeded { .. }
  ));
}
