use std::{
  collections::BTreeMap,
  io::{Read, Write},
  net::{SocketAddr, TcpListener, TcpStream},
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
  },
  thread,
  time::Duration,
};

use serde_json::{json, Map};
use woml_engine::{
  execute_workflow_durable, AttemptFailureKind, CompiledWorkflowDefinition, RunEventPayload,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const HASH: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[derive(Debug, Clone)]
struct CapturedRequest {
  method: String,
  target: String,
  authorization: Option<String>,
  idempotency_key: Option<String>,
  body: Vec<u8>,
}

struct LocalHttpServer {
  address: SocketAddr,
  requests: Arc<Mutex<Vec<CapturedRequest>>>,
  stop: Arc<AtomicBool>,
  thread: Option<thread::JoinHandle<()>>,
}

impl LocalHttpServer {
  fn start() -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_requests = requests.clone();
    let thread_stop = stop.clone();
    let handle = thread::spawn(move || {
      while !thread_stop.load(Ordering::Relaxed) {
        match listener.accept() {
          Ok((stream, _)) => {
            let requests = thread_requests.clone();
            thread::spawn(move || handle_connection(stream, address, requests));
          }
          Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            thread::sleep(Duration::from_millis(1));
          }
          Err(_) => break,
        }
      }
    });
    Self {
      address,
      requests,
      stop,
      thread: Some(handle),
    }
  }

  fn url(&self, path: &str) -> String {
    format!("http://{}{}", self.address, path)
  }
}

impl Drop for LocalHttpServer {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(self.address);
    if let Some(handle) = self.thread.take() {
      let _ = handle.join();
    }
  }
}

fn handle_connection(
  mut stream: TcpStream,
  address: SocketAddr,
  requests: Arc<Mutex<Vec<CapturedRequest>>>,
) {
  let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
  let mut bytes = Vec::new();
  let mut buffer = [0_u8; 8_192];
  let header_end = loop {
    let Ok(read) = stream.read(&mut buffer) else {
      return;
    };
    if read == 0 {
      return;
    }
    bytes.extend_from_slice(&buffer[..read]);
    if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
      break position + 4;
    }
    if bytes.len() > 65_536 {
      return;
    }
  };
  let headers = String::from_utf8_lossy(&bytes[..header_end]);
  let mut lines = headers.split("\r\n");
  let mut request_line = lines.next().unwrap_or_default().split_whitespace();
  let method = request_line.next().unwrap_or_default().to_string();
  let target = request_line.next().unwrap_or_default().to_string();
  let mut content_length = 0_usize;
  let mut authorization = None;
  let mut idempotency_key = None;
  for line in lines {
    let Some((name, value)) = line.split_once(':') else {
      continue;
    };
    let value = value.trim();
    if name.eq_ignore_ascii_case("content-length") {
      content_length = value.parse().unwrap_or_default();
    } else if name.eq_ignore_ascii_case("authorization") {
      authorization = Some(value.to_string());
    } else if name.eq_ignore_ascii_case("idempotency-key") {
      idempotency_key = Some(value.to_string());
    }
  }
  while bytes.len() < header_end + content_length {
    let Ok(read) = stream.read(&mut buffer) else {
      return;
    };
    if read == 0 {
      break;
    }
    bytes.extend_from_slice(&buffer[..read]);
  }
  let body = bytes[header_end..bytes.len().min(header_end + content_length)].to_vec();
  requests.lock().unwrap().push(CapturedRequest {
    method,
    target: target.clone(),
    authorization,
    idempotency_key,
    body,
  });

  let path = target.split('?').next().unwrap_or_default();
  let response = match path {
    "/redirect" => format!(
      "HTTP/1.1 302 Found\r\nLocation: http://{address}/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes(),
    "/json" => response(200, "application/json", br#"{"source":"redirect"}"#),
    "/text" => response(200, "text/plain; charset=utf-8", "Héllo WOML".as_bytes()),
    "/bytes" => response(200, "application/octet-stream", &[0, 1, 2, 255]),
    "/slow" => {
      thread::sleep(Duration::from_millis(150));
      response(200, "application/json", br#"{"late":true}"#)
    }
    "/invalid-json" => response(200, "application/json", b"not-json"),
    "/rejected" => response(418, "application/json", br#"{"error":"teapot"}"#),
    _ => response(201, "application/json", br#"{"created":true}"#),
  };
  let _ = stream.write_all(&response);
}

fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
  let reason = match status {
    200 => "OK",
    201 => "Created",
    418 => "I'm a teapot",
    _ => "Response",
  };
  let mut response = format!(
    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  )
  .into_bytes();
  response.extend_from_slice(body);
  response
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(())?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(PathBuf::from("bun"), host))
}

fn workflow_with_source(source: String) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  let node = workflow.graph.nodes.first_mut().unwrap();
  node
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
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

fn database_path(name: &str) -> PathBuf {
  std::env::temp_dir().join(format!(
    "woml-sc5-{name}-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ))
}

fn remove_database(path: &std::path::Path) {
  let _ = std::fs::remove_file(path);
  let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
  let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
}

#[tokio::test]
async fn managed_http_runs_through_rust_with_public_results_and_redacted_events() {
  let Some(host) = host_options() else { return };
  let server = LocalHttpServer::start();
  let base = format!("http://{}", server.address);
  let source = format!(
    r#"
      const [created, redirected, text, bytes] = await Promise.all([
        services.http.request({{
          url: "{base}/orders",
          method: "post",
          query: {{ expand: "customer", tag: ["one", "two"] }},
          headers: {{ authorization: `Bearer ${{secrets.API_TOKEN}}` }},
          json: {{ amount: 42 }},
          timeout: "2s",
          idempotency: {{ header: "Idempotency-Key", value: "external-order-42" }}
        }}, {{ name: "create-order" }}),
        services.http.request({{ url: "{base}/redirect" }}),
        services.http.request({{ url: "{base}/text", responseType: "text" }}),
        services.http.request({{ url: "{base}/bytes", responseType: "bytes" }})
      ]);
      return {{ created, redirected, text, bytes }};
    "#
  );
  let database = database_path("success");
  let mut workflow = workflow_with_source(source);
  workflow.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .push("API_TOKEN".to_string());
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 5_000).with_resolved_secrets(BTreeMap::from([(
      "API_TOKEN".to_string(),
      "sc5-secret-token".to_string(),
    )])),
    database.clone(),
  )
  .await
  .unwrap();

  assert_eq!(result.result["created"]["status"], 201);
  assert_eq!(result.result["created"]["data"], json!({ "created": true }));
  assert!(result.result["created"].get("contract").is_none());
  assert_eq!(result.result["redirected"]["redirected"], true);
  assert_eq!(
    result.result["redirected"]["data"],
    json!({ "source": "redirect" })
  );
  assert_eq!(result.result["text"]["data"], "Héllo WOML");
  assert_eq!(result.result["bytes"]["data"], "AAEC/w==");

  let requests = server.requests.lock().unwrap().clone();
  let post = requests
    .iter()
    .find(|request| request.method == "POST")
    .unwrap();
  assert!(post.target.contains("expand=customer"));
  assert!(post.target.contains("tag=one"));
  assert!(post.target.contains("tag=two"));
  assert_eq!(
    post.authorization.as_deref(),
    Some("Bearer sc5-secret-token")
  );
  assert_eq!(post.idempotency_key.as_deref(), Some("external-order-42"));
  assert_eq!(
    serde_json::from_slice::<serde_json::Value>(&post.body).unwrap(),
    json!({ "amount": 42 })
  );

  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
      .count(),
    4
  );
  let history = serde_json::to_string(&result.events).unwrap();
  assert!(!history.contains("sc5-secret-token"));
  assert!(!history.contains("external-order-42"));
  assert!(!history.contains("authorization"));
  assert!(!history.contains("amount"));
  assert!(history.contains("\"method\":\"POST\""));
  assert!(history.contains("\"status\":201"));
  remove_database(&database);
}

#[tokio::test]
async fn managed_http_classifies_timeout_status_and_invalid_json_without_leaking_bodies() {
  let Some(host) = host_options() else { return };
  let server = LocalHttpServer::start();
  for (name, path, options, expected_code, expected_kind) in [
    (
      "timeout",
      "/slow",
      ", timeoutMs: 20",
      "WOML_HTTP_TIMED_OUT",
      AttemptFailureKind::ServiceFailed,
    ),
    (
      "status",
      "/rejected",
      "",
      "WOML_HTTP_STATUS_REJECTED",
      AttemptFailureKind::ServiceFailed,
    ),
    (
      "json",
      "/invalid-json",
      "",
      "WOML_HTTP_RESPONSE_JSON_INVALID",
      AttemptFailureKind::ServiceFailed,
    ),
  ] {
    let source = format!(
      "return await services.http.request({{ url: {:?}{} }});",
      server.url(path),
      options
    );
    let database = database_path(name);
    let error = execute_workflow_durable(
      workflow_with_source(source),
      HASH.to_string(),
      Map::new(),
      RuntimeExecutionOptions::new(host.clone(), 3_000),
      database.clone(),
    )
    .await
    .unwrap_err();
    let RuntimeExecutionError::RunFailed(details) = error else {
      panic!("expected failed run");
    };
    assert_eq!(details.failure.kind, expected_kind);
    assert_eq!(details.failure.code, expected_code);
    let history = serde_json::to_string(&details.events).unwrap();
    assert!(!history.contains("teapot"));
    assert!(!history.contains("not-json"));
    remove_database(&database);
  }
}
