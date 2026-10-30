//! Shared deterministic fixtures for artifact characterization and backend conformance.

mod backends;
mod backup;
mod inspection;
mod lifecycle;
mod output;
mod publication;
mod references;

use super::*;
use crate::capability::CapabilityIdentityMode;
use crate::CapabilityCallRequest;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailureKind;
use crate::CapabilityHandler;
use crate::DurableEventStore;
use crate::{derive_operation_key, CapabilityCallIdentity, CapabilityCallLimits};
use chrono::Utc;
use reqwest::Url;
use rusqlite::params;
use rusqlite::Connection;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::{
  net::{TcpListener, TcpStream},
  sync::atomic::{AtomicBool, AtomicUsize, Ordering},
  thread::JoinHandle,
};

struct TempDir(PathBuf);

impl TempDir {
  fn new() -> std::io::Result<Self> {
    let path = std::env::temp_dir().join(format!(
      "woml-durable-artifacts-{}",
      uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&path)?;
    Ok(Self(path))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

#[derive(Debug, Clone)]
struct MockS3Request {
  method: String,
  target: String,
  headers: BTreeMap<String, String>,
}

struct MockS3Server {
  endpoint: String,
  requests: Arc<Mutex<Vec<MockS3Request>>>,
  stop: Arc<AtomicBool>,
  thread: Option<JoinHandle<()>>,
}

impl MockS3Server {
  fn start() -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let objects = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let failed_blob_puts = Arc::new(AtomicUsize::new(1));
    let invisible_blob_reads = Arc::new(AtomicUsize::new(1));
    let failed_store_id_reads = Arc::new(AtomicUsize::new(1));
    let failed_deletes = Arc::new(AtomicUsize::new(1));
    let failed_inventory_reads = Arc::new(AtomicUsize::new(1));
    let thread_objects = Arc::clone(&objects);
    let thread_requests = Arc::clone(&requests);
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
      while !thread_stop.load(Ordering::SeqCst) {
        match listener.accept() {
          Ok((stream, _)) => handle_mock_s3_connection(
            stream,
            &thread_objects,
            &thread_requests,
            &failed_blob_puts,
            &invisible_blob_reads,
            &failed_store_id_reads,
            &failed_deletes,
            &failed_inventory_reads,
          ),
          Err(_) => {
            // A nonblocking Windows listener can surface an aborted or
            // reset connection from a client pool through accept(). Those
            // errors affect only that connection and must not terminate the
            // mock S3 service before a rotated client reconnects. The stop
            // flag, not a connection-scoped accept error, owns shutdown.
            std::thread::sleep(Duration::from_millis(2));
          }
        }
      }
    });
    Self {
      endpoint: format!("http://{address}"),
      requests,
      stop,
      thread: Some(thread),
    }
  }
}

impl Drop for MockS3Server {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(self.endpoint.trim_start_matches("http://"));
    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}

fn handle_mock_s3_connection(
  mut stream: TcpStream,
  objects: &Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
  requests: &Arc<Mutex<Vec<MockS3Request>>>,
  failed_blob_puts: &AtomicUsize,
  invisible_blob_reads: &AtomicUsize,
  failed_store_id_reads: &AtomicUsize,
  failed_deletes: &AtomicUsize,
  failed_inventory_reads: &AtomicUsize,
) {
  let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
  let mut received = Vec::new();
  let mut buffer = [0_u8; 8192];
  let (header_end, content_length) = loop {
    let Ok(read) = stream.read(&mut buffer) else {
      return;
    };
    if read == 0 {
      return;
    }
    received.extend_from_slice(&buffer[..read]);
    if received.len() > 2 * 1024 * 1024 {
      return;
    }
    if let Some(position) = received.windows(4).position(|value| value == b"\r\n\r\n") {
      let header_end = position + 4;
      let headers = String::from_utf8_lossy(&received[..position]);
      let content_length = headers
        .lines()
        .find_map(|line| {
          let (name, value) = line.split_once(':')?;
          name
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
        })
        .unwrap_or(0);
      break (header_end, content_length);
    }
  };
  while received.len() < header_end + content_length {
    let Ok(read) = stream.read(&mut buffer) else {
      return;
    };
    if read == 0 {
      return;
    }
    received.extend_from_slice(&buffer[..read]);
  }
  let head = String::from_utf8_lossy(&received[..header_end - 4]);
  let mut lines = head.lines();
  let Some(request_line) = lines.next() else {
    return;
  };
  let mut request_parts = request_line.split_whitespace();
  let method = request_parts.next().unwrap_or_default().to_string();
  let target = request_parts.next().unwrap_or_default().to_string();
  let headers = lines
    .filter_map(|line| {
      let (name, value) = line.split_once(':')?;
      Some((name.to_ascii_lowercase(), value.trim().to_string()))
    })
    .collect::<BTreeMap<_, _>>();
  requests.lock().unwrap().push(MockS3Request {
    method: method.clone(),
    target: target.clone(),
    headers: headers.clone(),
  });
  let body = received[header_end..header_end + content_length].to_vec();
  let path = target.split('?').next().unwrap_or_default();
  let key = path.strip_prefix("/woml-test/").unwrap_or_default();
  let is_blob = key.contains("blobs/sha256/");

  if method == "GET" && key.ends_with("store-id") && decrement_if_positive(failed_store_id_reads) {
    write_mock_response(&mut stream, 503, "", &[]);
    return;
  }
  if method == "DELETE" && decrement_if_positive(failed_deletes) {
    write_mock_response(&mut stream, 503, "", &[]);
    return;
  }
  if method == "GET"
    && target.contains("list-type=2")
    && decrement_if_positive(failed_inventory_reads)
  {
    write_mock_response(&mut stream, 503, "", &[]);
    return;
  }

  if method == "GET" && target.contains("list-type=2") {
    let prefix = target
      .split('?')
      .nth(1)
      .unwrap_or_default()
      .split('&')
      .find_map(|part| part.strip_prefix("prefix="))
      .map(percent_decode_test)
      .unwrap_or_default();
    let contents = objects
      .lock()
      .unwrap()
      .keys()
      .filter(|key| key.starts_with(&prefix))
      .map(|key| format!("<Contents><Key>{key}</Key></Contents>"))
      .collect::<String>();
    write_mock_response(
      &mut stream,
      200,
      &format!("<ListBucketResult>{contents}</ListBucketResult>"),
      &[],
    );
    return;
  }
  if method == "PUT" && is_blob && decrement_if_positive(failed_blob_puts) {
    write_mock_response(&mut stream, 503, "", &[]);
    return;
  }
  match method.as_str() {
    "PUT" => {
      let mut objects = objects.lock().unwrap();
      if headers.get("if-none-match").map(String::as_str) == Some("*") && objects.contains_key(key)
      {
        write_mock_response(&mut stream, 412, "", &[]);
      } else {
        objects.insert(key.to_string(), body);
        write_mock_response(&mut stream, 200, "", &[("ETag", "\"mock\"")]);
      }
    }
    "GET" => {
      if is_blob && decrement_if_positive(invisible_blob_reads) {
        write_mock_response(&mut stream, 404, "", &[]);
        return;
      }
      match objects.lock().unwrap().get(key).cloned() {
        Some(value) => write_mock_bytes(&mut stream, 200, &value, &[]),
        None => write_mock_response(&mut stream, 404, "", &[]),
      }
    }
    "DELETE" => {
      objects.lock().unwrap().remove(key);
      write_mock_response(&mut stream, 204, "", &[]);
    }
    _ => write_mock_response(&mut stream, 400, "", &[]),
  }
}

fn decrement_if_positive(value: &AtomicUsize) -> bool {
  let mut current = value.load(Ordering::SeqCst);
  while current > 0 {
    match value.compare_exchange_weak(current, current - 1, Ordering::SeqCst, Ordering::SeqCst) {
      Ok(_) => return true,
      Err(observed) => current = observed,
    }
  }
  false
}

fn write_mock_response(stream: &mut TcpStream, status: u16, body: &str, headers: &[(&str, &str)]) {
  write_mock_bytes(stream, status, body.as_bytes(), headers);
}

fn write_mock_bytes(stream: &mut TcpStream, status: u16, body: &[u8], headers: &[(&str, &str)]) {
  let reason = match status {
    200 => "OK",
    204 => "No Content",
    400 => "Bad Request",
    404 => "Not Found",
    412 => "Precondition Failed",
    503 => "Service Unavailable",
    _ => "Unknown",
  };
  let extra = headers
    .iter()
    .map(|(name, value)| format!("{name}: {value}\r\n"))
    .collect::<String>();
  let response = format!(
    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
    body.len()
  );
  let _ = stream.write_all(response.as_bytes());
  let _ = stream.write_all(body);
  let _ = stream.flush();
}

fn percent_decode_test(value: &str) -> String {
  let bytes = value.as_bytes();
  let mut decoded = Vec::with_capacity(bytes.len());
  let mut index = 0;
  while index < bytes.len() {
    if bytes[index] == b'%' && index + 2 < bytes.len() {
      let encoded = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default();
      if let Ok(byte) = u8::from_str_radix(encoded, 16) {
        decoded.push(byte);
        index += 3;
        continue;
      }
    }
    decoded.push(bytes[index]);
    index += 1;
  }
  String::from_utf8(decoded).unwrap()
}

fn policy(root: &Path, backend: &Path) -> ArtifactPolicyV1 {
  ArtifactPolicyV1 {
    contract: ARTIFACT_POLICY_CONTRACT.to_string(),
    contract_version: 1,
    enabled: true,
    modes: ArtifactPolicyModes {
      foreground: true,
      background: false,
    },
    project_root: root.to_string_lossy().into_owned(),
    read_roots: vec![".".to_string()],
    materialization: ArtifactMaterializationPolicy {
      directory: root.join("materialized").to_string_lossy().into_owned(),
      cleanup_on_start: true,
      max_bytes_per_attempt: 1_048_576,
    },
    limits: ArtifactLimits {
      max_artifact_bytes: 1_048_576,
      max_run_bytes: 2_097_152,
      max_store_bytes: 4_194_304,
      max_concurrent_transfers: 2,
      transfer_timeout_ms: 30_000,
    },
    retention: ArtifactRetentionPolicy {
      default_ms: 86_400_000,
      minimum_ms: 3_600_000,
      maximum_ms: 31_536_000_000,
    },
    secret_scanning: ArtifactSecretScanningPolicy {
      known_secret_action: KnownSecretAction::Reject,
      minimum_secret_bytes: 8,
    },
    operational_output: None,
    backend: ArtifactBackendPolicy::Local {
      contract: "woml.artifact-backend".to_string(),
      contract_version: 1,
      directory: backend.to_string_lossy().into_owned(),
    },
  }
}

fn call(path: &str, name: &str) -> CapabilityCallRequest {
  let step_key = "step-key".to_string();
  CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: "invocation-artifact".to_string(),
    call_id: "call-artifact".to_string(),
    run_id: "run-artifact".to_string(),
    node_id: "create".to_string(),
    attempt_number: 1,
    capability: "artifacts".to_string(),
    operation: "put".to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: CapabilityIdentityMode::Named,
      step_idempotency_key: step_key.clone(),
      operation_name: name.to_string(),
      operation_key: derive_operation_key(&step_key, name),
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits {
      input_bytes: 4096,
      result_bytes: 16384,
      timeout_ms: 30_000,
    },
    input: json!({
      "contract": "woml.artifacts",
      "contractVersion": 1,
      "kind": "request",
      "operation": "put",
      "input": {
        "sourcePath": path,
        "name": name,
        "contentType": "text/plain",
        "retentionMs": 86400000
      }
    }),
  }
}

fn reference_call(
  operation: &str,
  artifact: &ArtifactReferenceV1,
  run_id: &str,
  invocation_id: &str,
) -> CapabilityCallRequest {
  let step_key = format!("step-key-{run_id}");
  let operation_name = format!("artifacts.{operation}#1");
  CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: invocation_id.to_string(),
    call_id: format!("call-{operation}"),
    run_id: run_id.to_string(),
    node_id: "consume".to_string(),
    attempt_number: 1,
    capability: "artifacts".to_string(),
    operation: operation.to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: CapabilityIdentityMode::Automatic,
      step_idempotency_key: step_key.clone(),
      operation_name: operation_name.clone(),
      operation_key: derive_operation_key(&step_key, &operation_name),
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits {
      input_bytes: 16_384,
      result_bytes: 16_384,
      timeout_ms: 30_000,
    },
    input: json!({
      "contract": "woml.artifacts",
      "contractVersion": 1,
      "kind": "request",
      "operation": operation,
      "input": { "artifact": artifact }
    }),
  }
}
