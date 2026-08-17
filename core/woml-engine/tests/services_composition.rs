use std::{
  collections::BTreeMap,
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
const HASH: &str = "sha256:1414141414141414141414141414141414141414141414141414141414141414";
const SECRET: &str = "sc14-secret-must-not-persist";

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new() -> Self {
    let path = std::env::temp_dir().join(format!(
      "woml-sc14-services-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }

  fn state(&self) -> PathBuf {
    self.0.join("state.sqlite")
  }

  fn user_database(&self) -> PathBuf {
    self.0.join("application.sqlite")
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

struct LocalJsonServer {
  address: SocketAddr,
  stop: Arc<AtomicBool>,
  thread: Option<thread::JoinHandle<()>>,
}

impl LocalJsonServer {
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
            thread::spawn(move || respond(stream));
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
      stop,
      thread: Some(handle),
    }
  }

  fn url(&self, path: &str) -> String {
    format!("http://{}{}", self.address, path)
  }
}

impl Drop for LocalJsonServer {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(self.address);
    if let Some(handle) = self.thread.take() {
      let _ = handle.join();
    }
  }
}

fn respond(mut stream: TcpStream) {
  let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
  let mut request = [0_u8; 16_384];
  let _ = stream.read(&mut request);
  let body = br#"{"customer":"Alex","source":"sc14-local"}"#;
  let response = format!(
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  );
  if stream.write_all(response.as_bytes()).is_ok() {
    let _ = stream.write_all(body);
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

fn workflow(source: String) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  workflow.workflow_id = "sc14-services-composition".to_string();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  let node = workflow.graph.nodes.first_mut().unwrap();
  node.script_runtime.as_mut().unwrap().required_secrets = vec!["SC14_TOKEN".to_string()];
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

#[tokio::test]
async fn native_fetch_and_all_non_event_services_compose_in_one_script_attempt() {
  let Some(host) = host_options() else { return };
  let directory = TestDirectory::new();
  let server = LocalJsonServer::start();
  let database = serde_json::to_string(&directory.user_database()).unwrap();
  let native_url = serde_json::to_string(&server.url("/native")).unwrap();
  let managed_url = serde_json::to_string(&server.url("/managed")).unwrap();
  let source = format!(
    r#"
      const db = services.db({{ driver: "sqlite", connection: {database} }});
      await db.execute({{
        text: "CREATE TABLE records (id TEXT PRIMARY KEY, value TEXT NOT NULL)"
      }}, {{ name: "create-records" }});

      const [nativeResponse, managedResponse, databaseWrite, object, cached] =
        await Promise.all([
          fetch({native_url}, {{
            headers: {{ authorization: `Bearer ${{secrets.SC14_TOKEN}}` }}
          }}),
          services.http.request({{
            url: {managed_url},
            headers: {{ authorization: `Bearer ${{secrets.SC14_TOKEN}}` }}
          }}),
          db.execute({{
            text: "INSERT INTO records (id, value) VALUES (?, ?)",
            values: ["record-42", "private-db-value"]
          }}, {{ name: "insert-record" }}),
          services.storage.put({{
            key: "sc14/record.json",
            value: {{ id: "record-42", value: "private-storage-value" }}
          }}, {{ name: "store-record" }}),
          services.cache.set(
            "private-cache-key",
            {{ value: "private-cache-value" }},
            {{ ttl: "5m", name: "cache-record" }}
          )
        ]);

      const nativeData = await nativeResponse.json();
      const [databaseRead, stored, cacheRead] = await Promise.all([
        db.query({{
          text: "SELECT id, value FROM records WHERE id = ?",
          values: ["record-42"]
        }}),
        services.storage.get({{
          key: object.key,
          responseType: "json",
          ifVersion: object.version
        }}),
        services.cache.get("private-cache-key")
      ]);

      return {{
        nativeHttp: nativeResponse.status === 200 && nativeData.customer === "Alex",
        managedHttp: managedResponse.status === 200 && managedResponse.data.customer === "Alex",
        database: databaseWrite.rowsAffected === 1 && databaseRead.rows[0].id === "record-42",
        storage: stored.data.id === "record-42",
        cache: cached.stored && cacheRead.hit
      }};
    "#
  );
  let result = execute_workflow_durable(
    workflow(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000).with_resolved_secrets(BTreeMap::from([(
      "SC14_TOKEN".to_string(),
      SECRET.to_string(),
    )])),
    directory.state(),
  )
  .await
  .unwrap();

  assert_eq!(
    result.result,
    json!({
      "nativeHttp": true,
      "managedHttp": true,
      "database": true,
      "storage": true,
      "cache": true,
    })
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
      .count(),
    9
  );
  let history = serde_json::to_string(&result.events).unwrap();
  for private in [
    SECRET,
    "private-db-value",
    "private-storage-value",
    "private-cache-key",
    "private-cache-value",
  ] {
    assert!(!history.contains(private), "history leaked {private}");
  }
}
