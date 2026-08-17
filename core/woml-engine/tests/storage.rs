use std::{
  io::{Read, Write},
  net::{SocketAddr, TcpListener, TcpStream},
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
  },
  thread,
  time::Duration,
};

use serde_json::{json, Map};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, RunEventPayload, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const HASH: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new(name: &str) -> Self {
    let path = std::env::temp_dir().join(format!(
      "woml-sc9-integration-{name}-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }

  fn state(&self) -> PathBuf {
    self.0.join("state.sqlite")
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

struct LargeHttpServer {
  address: SocketAddr,
  stop: Arc<AtomicBool>,
  thread: Option<thread::JoinHandle<()>>,
}

impl LargeHttpServer {
  fn start() -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
      while !thread_stop.load(Ordering::Relaxed) {
        match listener.accept() {
          Ok((stream, _)) => {
            thread::spawn(move || send_large_response(stream));
          }
          Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            thread::sleep(Duration::from_millis(1));
            continue;
          }
          Err(_) => break,
        };
      }
    });
    Self {
      address,
      stop,
      thread: Some(handle),
    }
  }

  fn url(&self) -> String {
    format!("http://{}/large", self.address)
  }
}

impl Drop for LargeHttpServer {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(self.address);
    if let Some(handle) = self.thread.take() {
      let _ = handle.join();
    }
  }
}

fn send_large_response(mut stream: TcpStream) {
  let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
  let mut request = [0_u8; 8_192];
  let _ = stream.read(&mut request);
  const SIZE: usize = 5 * 1_024 * 1_024;
  let header = format!(
    "HTTP/1.1 200 OK\r\nContent-Type: application/x-woml-sc9\r\nContent-Length: {SIZE}\r\nConnection: close\r\n\r\n"
  );
  if stream.write_all(header.as_bytes()).is_err() {
    return;
  }
  let chunk = vec![b'z'; 64 * 1_024];
  for _ in 0..(SIZE / chunk.len()) {
    if stream.write_all(&chunk).is_err() {
      return;
    }
  }
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
  workflow.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  let woml_engine::model::ValueExpression::Object { fields } = &mut workflow.graph.nodes[0].inputs
  else {
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

#[tokio::test]
async fn storage_v1_runs_through_rust_and_persists_only_explicit_results() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("operations");
  let source = r#"
    const stored = await services.storage.put({
      key: "reports/today.json",
      value: { message: "sensitive-sc9-body", total: 42 }
    });
    const loaded = await services.storage.get({
      key: stored.key, responseType: "json", ifVersion: stored.version
    });
    if (loaded.data.message !== "sensitive-sc9-body") throw new Error("round trip failed");
    const head = await services.storage.head({ key: stored.key });
    const listed = await services.storage.list({ prefix: "reports/" });
    const removed = await services.storage.delete({
      key: stored.key, ifVersion: stored.version
    });
    return {
      key: stored.key,
      version: head.version,
      size: head.size,
      listed: listed.objects.length,
      deleted: removed.deleted
    };
  "#;
  let result = execute_workflow_durable(
    workflow_with_source(source.to_string()),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    directory.state(),
  )
  .await
  .unwrap();

  assert_eq!(result.result["key"], "reports/today.json");
  assert_eq!(result.result["listed"], 1);
  assert_eq!(result.result["deleted"], true);
  assert!(result.result["version"]
    .as_str()
    .unwrap()
    .starts_with("v1:"));
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
      .count(),
    5
  );
  let history = serde_json::to_string(&result.events).unwrap();
  assert!(!history.contains("sensitive-sc9-body"));
  assert!(!history.contains("objects-v1"));
  assert!(history.contains("reports/today.json"));
}

#[tokio::test]
async fn managed_http_streams_a_body_larger_than_the_capability_limit_directly_to_storage() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new("http-storage");
  let server = LargeHttpServer::start();
  let source = format!(
    r#"
      const response = await services.http.request({{
        url: {},
        responseType: "storage",
        storage: {{ key: "imports/large.bin" }}
      }});
      const head = await services.storage.head({{ key: "imports/large.bin" }});
      return {{ status: response.status, object: response.data, head }};
    "#,
    serde_json::to_string(&server.url()).unwrap()
  );
  let result = execute_workflow_durable(
    workflow_with_source(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 15_000),
    directory.state(),
  )
  .await
  .unwrap();

  assert_eq!(result.result["status"], 200);
  assert_eq!(result.result["object"]["key"], "imports/large.bin");
  assert_eq!(result.result["object"]["size"], 5 * 1_024 * 1_024);
  assert_eq!(
    result.result["object"]["contentType"],
    "application/x-woml-sc9"
  );
  assert_eq!(result.result["head"], result.result["object"]);
  let history = serde_json::to_string(&result.events).unwrap();
  assert!(history.len() < 100_000);
  assert!(!history.contains(&"z".repeat(1_024)));
  assert!(directory.0.join("objects-v1").is_dir());
}
