use std::{collections::BTreeMap, path::PathBuf};

use serde_json::{json, Map};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::TcpListener,
  sync::oneshot,
};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, OperationExecutionMode, RunEventPayload,
  RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn host_options() -> Option<ScriptHostProcessOptions> {
  std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then(|| {
      ScriptHostProcessOptions::new(
        PathBuf::from("bun"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts"),
      )
    })
}

fn fetch_workflow(source: String) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  let node = workflow.graph.nodes.first_mut().unwrap();
  let woml_engine::model::ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(source),
    },
  );
  workflow
}

fn remove_state(path: &std::path::Path) {
  let _ = std::fs::remove_file(path);
  let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
  let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
}

#[test]
fn frozen_native_fetch_observations_validate_in_rust() {
  for fixture in [
    include_str!("../../../woml/tests/fixtures/services-contracts/native-fetch-started.v1.json"),
    include_str!("../../../woml/tests/fixtures/services-contracts/native-fetch-completed.v1.json"),
    include_str!("../../../woml/tests/fixtures/services-contracts/native-fetch-failed.v1.json"),
  ] {
    let observation: woml_engine::NativeFetchObservation = serde_json::from_str(fixture).unwrap();
    observation.validate().unwrap();
  }
  let missing_null_body = json!({
    "contract": "woml.native-fetch-observation",
    "contractVersion": 1,
    "observationType": "completed",
    "invocationId": "invocation-a",
    "requestId": "fetch-a",
    "status": 200,
    "durationMs": 1,
    "completedAt": "2026-08-09T08:00:00.000Z"
  });
  assert!(
    serde_json::from_value::<woml_engine::NativeFetchObservation>(missing_null_body).is_err()
  );
}

#[tokio::test]
async fn native_fetch_is_executed_by_bun_and_observed_durably_without_secrets() {
  let Some(host) = host_options() else {
    return;
  };
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let (request_sender, request_receiver) = oneshot::channel();
  let server = tokio::spawn(async move {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut bytes = vec![0; 8_192];
    let read = socket.read(&mut bytes).await.unwrap();
    let request = String::from_utf8_lossy(&bytes[..read]).to_string();
    let _ = request_sender.send(request);
    socket
      .write_all(
        b"HTTP/1.1 201 Created\r\ncontent-type: text/plain\r\ncontent-length: 11\r\nconnection: close\r\n\r\nhello fetch",
      )
      .await
      .unwrap();
  });

  let source = format!(
    r#"
      const response = await fetch(new Request(
        "http://{address}/customers/%E2%9C%93?token=" + secrets.CUSTOMER_API_TOKEN,
        {{ headers: {{ authorization: "Bearer " + secrets.CUSTOMER_API_TOKEN }} }}
      ));
      return {{
        nativeResponse: response instanceof Response,
        status: response.status,
        body: await response.text()
      }};
    "#
  );
  let database_path = std::env::temp_dir().join(format!(
    "woml-sc4-fetch-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let result = execute_workflow_durable(
    fetch_workflow(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000).with_resolved_secrets(BTreeMap::from([(
      "CUSTOMER_API_TOKEN".to_string(),
      "sc4-secret-value".to_string(),
    )])),
    database_path.clone(),
  )
  .await
  .unwrap();
  server.await.unwrap();
  let request = request_receiver.await.unwrap();

  assert!(request.contains("GET /customers/%E2%9C%93?token=sc4-secret-value HTTP/1.1"));
  assert!(request
    .to_ascii_lowercase()
    .contains("authorization: bearer sc4-secret-value"));
  assert_eq!(
    result.result,
    json!({ "nativeResponse": true, "status": 201, "body": "hello fetch" })
  );
  let started = result.events.iter().find_map(|event| match &event.payload {
    RunEventPayload::OperationStarted(data) => Some(data),
    _ => None,
  });
  let started = started.expect("native Fetch must have a durable start");
  assert_eq!(started.execution_mode, OperationExecutionMode::Observed);
  assert_eq!(started.capability, "http");
  assert_eq!(started.operation, "fetch");
  assert_eq!(
    started.metadata.get("path"),
    Some(&json!("/customers/%E2%9C%93"))
  );
  assert_eq!(
    started.metadata.get("origin"),
    Some(&json!(format!("http://{address}")))
  );
  let completed = result.events.iter().find_map(|event| match &event.payload {
    RunEventPayload::OperationSucceeded(data) => Some(data),
    _ => None,
  });
  assert_eq!(completed.unwrap().metadata.get("status"), Some(&json!(201)));
  let history = serde_json::to_string(&result.events).unwrap();
  assert!(!history.contains("sc4-secret-value"));
  assert!(!history.contains("token="));
  assert!(!history.to_ascii_lowercase().contains("authorization"));

  remove_state(&database_path);
}

#[tokio::test]
async fn rejected_native_fetch_keeps_buns_error_and_records_a_safe_failure() {
  let Some(host) = host_options() else {
    return;
  };
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  drop(listener);
  let source = format!(
    r#"
      try {{
        await fetch("http://{address}/unavailable?secret=must-not-persist");
        return {{ rejected: false }};
      }} catch (error) {{
        return {{ rejected: true, nativeName: error.name }};
      }}
    "#
  );
  let mut workflow = fetch_workflow(source);
  workflow
    .graph
    .nodes
    .first_mut()
    .unwrap()
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  let database_path = std::env::temp_dir().join(format!(
    "woml-sc4-fetch-failure-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000),
    database_path.clone(),
  )
  .await
  .unwrap();

  assert_eq!(result.result.get("rejected"), Some(&json!(true)));
  assert_eq!(result.result.get("nativeName"), Some(&json!("Error")));
  assert!(result
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::OperationFailed(_))));
  let history = serde_json::to_string(&result.events).unwrap();
  assert!(!history.contains("must-not-persist"));
  assert!(!history.contains("secret="));

  remove_state(&database_path);
}
