use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::model::{BackoffPolicy, RetryPolicy, ValueExpression};
use woml_engine::{
  execute_workflow_durable, AttemptFailureKind, CompiledWorkflowDefinition, DurableEventStore,
  RunEventPayload, RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const RETRY_MODEL: &str = include_str!("../../../woml/tests/fixtures/retry.compiled.v6.json");
const PARALLEL_MODEL: &str = include_str!("../../../woml/tests/fixtures/parallel.compiled.v3.json");
const HASH: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn model(source: &str) -> CompiledWorkflowDefinition {
  serde_json::from_str(source).unwrap()
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(bun, host))
}

fn options(host: ScriptHostProcessOptions) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, 5_000)
}

fn node_mut<'a>(
  workflow: &'a mut CompiledWorkflowDefinition,
  node_id: &str,
) -> &'a mut woml_engine::model::CompiledWorkflowNode {
  workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap()
}

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: &str) {
  let ValueExpression::Object { fields } = &mut node_mut(workflow, node_id).inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source.to_string()),
    },
  );
}

fn set_retry(
  workflow: &mut CompiledWorkflowDefinition,
  node_id: &str,
  max_attempts: u32,
  delay_ms: u64,
) {
  node_mut(workflow, node_id).retry_policy = Some(RetryPolicy {
    max_attempts,
    backoff: BackoffPolicy::Fixed { delay_ms },
  });
}

fn single_retry_node(source: &str) -> CompiledWorkflowDefinition {
  let mut workflow = model(RETRY_MODEL);
  workflow.graph.nodes.retain(|node| node.id == "greet");
  workflow.graph.entry_node_ids = vec!["greet".to_string()];
  workflow.graph.edges.clear();
  set_script(&mut workflow, "greet", source);
  set_retry(&mut workflow, "greet", 3, 1);
  workflow.validate_for_durable_execution().unwrap();
  workflow
}

fn attempts(events: &[woml_engine::RunEvent], node_id: &str) -> Vec<u32> {
  events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptStarted(data) if data.node_id == node_id => Some(data.attempt),
      _ => None,
    })
    .collect()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-ri7-{label}-{}.sqlite",
        Uuid::new_v4().simple()
      )),
    }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

#[test]
fn fixed_and_exponential_backoff_cover_every_boundary_and_cap() {
  let fixed = RetryPolicy {
    max_attempts: 10,
    backoff: BackoffPolicy::Fixed {
      delay_ms: 86_400_000,
    },
  };
  assert_eq!(fixed.delay_before_attempt(1), None);
  for attempt in 2..=10 {
    assert_eq!(fixed.delay_before_attempt(attempt), Some(86_400_000));
  }
  assert_eq!(fixed.delay_before_attempt(11), None);

  let exponential = RetryPolicy {
    max_attempts: 10,
    backoff: BackoffPolicy::Exponential {
      initial_delay_ms: 1_000,
      multiplier: 2.0,
      maximum_delay_ms: Some(5_000),
    },
  };
  assert_eq!(
    (1..=11)
      .map(|attempt| exponential.delay_before_attempt(attempt))
      .collect::<Vec<_>>(),
    [
      None,
      Some(1_000),
      Some(2_000),
      Some(4_000),
      Some(5_000),
      Some(5_000),
      Some(5_000),
      Some(5_000),
      Some(5_000),
      Some(5_000),
      None,
    ]
  );
}

#[derive(Debug, Default)]
struct FakeEffectState {
  requests: Vec<String>,
  applied_keys: HashSet<String>,
}

fn start_fake_idempotent_service() -> (String, Arc<Mutex<FakeEffectState>>, thread::JoinHandle<()>)
{
  let listener = TcpListener::bind("127.0.0.1:0").unwrap();
  listener.set_nonblocking(true).unwrap();
  let address = listener.local_addr().unwrap();
  let state = Arc::new(Mutex::new(FakeEffectState::default()));
  let shared = Arc::clone(&state);
  let server = thread::spawn(move || {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
      let (mut stream, _) = match listener.accept() {
        Ok(connection) => connection,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
          thread::sleep(Duration::from_millis(2));
          continue;
        }
        Err(error) => panic!("fake service accept failed: {error}"),
      };
      stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
      let mut bytes = vec![0_u8; 16_384];
      let count = stream.read(&mut bytes).unwrap();
      let request = String::from_utf8_lossy(&bytes[..count]);
      let key = request
        .lines()
        .find_map(|line| {
          let (name, value) = line.split_once(':')?;
          name
            .eq_ignore_ascii_case("idempotency-key")
            .then(|| value.trim().to_string())
        })
        .expect("request must carry Idempotency-Key");
      let (duplicate, effects, request_count) = {
        let mut state = shared.lock().unwrap();
        state.requests.push(key.clone());
        let duplicate = !state.applied_keys.insert(key);
        (duplicate, state.applied_keys.len(), state.requests.len())
      };
      let body = serde_json::to_string(&json!({
        "duplicate": duplicate,
        "effects": effects,
      }))
      .unwrap();
      write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
      )
      .unwrap();
      stream.flush().unwrap();
      if request_count == 2 {
        return;
      }
    }
    panic!("fake service did not receive both retry attempts");
  });
  (format!("http://{address}/effect"), state, server)
}

#[tokio::test]
async fn stable_key_allows_an_external_service_to_deduplicate_a_retried_effect() {
  let Some(host) = host_options() else {
    return;
  };
  let (url, state, server) = start_fake_idempotent_service();
  let database = TemporaryDatabase::new("idempotent-service");
  let source = format!(
    r#"
      const response = await fetch({url:?}, {{
        method: "POST",
        headers: {{ "Idempotency-Key": attempt.idempotencyKey }}
      }});
      const result = await response.json();
      if (attempt.number === 1) throw new Error("lost response after effect");
      return result;
    "#
  );
  let workflow = single_retry_node(&source);
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  server.join().unwrap();

  assert_eq!(result.result, json!({ "duplicate": true, "effects": 1 }));
  let state = state.lock().unwrap();
  assert_eq!(state.requests.len(), 2);
  assert_eq!(state.applied_keys.len(), 1);
  assert_eq!(state.requests[0], state.requests[1]);
}

#[tokio::test]
async fn retry_carries_large_context_and_publishes_only_the_large_success_result() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("large-values");
  let mut workflow = model(RETRY_MODEL);
  set_retry(&mut workflow, "greet", 2, 1);
  set_script(
    &mut workflow,
    "prepare",
    "return { payload: 'x'.repeat(131072) };",
  );
  set_script(
    &mut workflow,
    "greet",
    "if (attempt.number === 1) throw new Error('temporary'); return { inputLength: context.steps.prepare.payload.length, payload: 'y'.repeat(131072) };",
  );
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result["inputLength"], 131_072);
  assert_eq!(result.result["payload"].as_str().unwrap().len(), 131_072);
  assert_eq!(attempts(&result.events, "greet"), [1, 2]);
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "greet"
      ))
      .count(),
    1
  );
}

async fn assert_crash_fails_without_retry(
  workflow: CompiledWorkflowDefinition,
  host: ScriptHostProcessOptions,
  expected: AttemptFailureKind,
  label: &str,
) {
  let database = TemporaryDatabase::new(label);
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("crash must fail the run");
  };
  assert_eq!(details.failure.kind, expected);
  assert_eq!(attempts(&details.events, "greet"), [1]);
  assert!(!details
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::StepRetryScheduled(_))));
}

#[tokio::test]
async fn host_and_worker_crashes_remain_distinct_and_neither_is_retried() {
  let Some(host) = host_options() else {
    return;
  };
  let crashing_host = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../woml-cli/tests/fixtures/crashing-script-host.ts");
  assert_crash_fails_without_retry(
    single_retry_node("return { unreachable: true };"),
    ScriptHostProcessOptions::new("bun", crashing_host),
    AttemptFailureKind::HostCrashed,
    "host-crash",
  )
  .await;
  assert_crash_fails_without_retry(
    single_retry_node("return { unreachable: true };"),
    ScriptHostProcessOptions::new(
      host.bun_executable,
      PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../woml-cli/tests/fixtures/worker-crashing-script-host.ts"),
    ),
    AttemptFailureKind::WorkerCrashed,
    "worker-crash",
  )
  .await;
}

#[tokio::test]
async fn concurrent_parallel_failures_create_independent_durable_retry_schedules() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("concurrent-schedules");
  let mut workflow = model(PARALLEL_MODEL);
  workflow.schema_version = 6;
  for (node_id, output) in [
    ("loadWeather", "return { temperature: 22 };"),
    ("loadSoil", "return { moisture: 41 };"),
  ] {
    set_retry(&mut workflow, node_id, 2, 10);
    set_script(
      &mut workflow,
      node_id,
      &format!("if (attempt.number === 1) throw new Error('temporary'); {output}"),
    );
  }
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options(host),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  let schedules = result
    .events
    .iter()
    .enumerate()
    .filter_map(|(index, event)| match &event.payload {
      RunEventPayload::StepRetryScheduled(schedule) => Some((index, schedule)),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(schedules.len(), 2);
  assert_eq!(
    schedules
      .iter()
      .map(|(_, schedule)| schedule.node_id.as_str())
      .collect::<HashSet<_>>(),
    HashSet::from(["loadWeather", "loadSoil"])
  );
  for (index, schedule) in schedules {
    assert!(matches!(
      &result.events[index - 1].payload,
      RunEventPayload::StepAttemptFailed(failed)
        if failed.node_id == schedule.node_id && failed.attempt == schedule.failed_attempt
    ));
  }
  assert_eq!(attempts(&result.events, "loadWeather"), [1, 2]);
  assert_eq!(attempts(&result.events, "loadSoil"), [1, 2]);
  let store = DurableEventStore::open(database.path()).unwrap();
  assert!(store
    .projection(&result.run_id)
    .unwrap()
    .pending_retries
    .is_empty());
}
