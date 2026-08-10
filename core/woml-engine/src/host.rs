use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::protocol::{
  CancelMessage, CapabilityCallMessage, CapabilityResultMessage, CompletedMessage, ExecuteMessage,
  FetchObservationAckMessage, FetchObservationMessage, HostOutcome, HostReportedFailureKind,
  ModuleRegisteredMessage, ReadyMessage, RegisterModuleMessage,
};
use crate::{
  capability_transport_failure, CapabilityCancellationToken, CapabilityFailure,
  CapabilityFailureKind, DurableCapabilityAuthority, NativeFetchInvocationIdentity,
};

const HEADER_PREFIX: &str = "Content-Length: ";
const MAX_HEADER_BYTES: usize = 128;

#[derive(Debug, Clone)]
pub struct ScriptHostProcessOptions {
  pub bun_executable: PathBuf,
  pub host_script_path: PathBuf,
  pub startup_timeout: Duration,
  pub shutdown_timeout: Duration,
  pub max_frame_bytes: Option<usize>,
  pub module_artifacts: Vec<ScriptHostModuleArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptHostModuleArtifact {
  pub bundle_digest: String,
  pub bundle: String,
}

impl ScriptHostProcessOptions {
  pub fn new(bun_executable: impl Into<PathBuf>, host_script_path: impl Into<PathBuf>) -> Self {
    Self {
      bun_executable: bun_executable.into(),
      host_script_path: host_script_path.into(),
      startup_timeout: Duration::from_secs(5),
      shutdown_timeout: Duration::from_secs(2),
      max_frame_bytes: Some(crate::DEFAULT_CAPABILITY_FRAME_BYTES as usize),
      module_artifacts: Vec::new(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScriptHostClientError {
  #[error("failed to start the Bun script host: {0}")]
  Startup(String),
  #[error("script-host protocol violation: {0}")]
  Protocol(String),
  #[error("the Bun script host crashed: {0}")]
  HostCrashed(String),
}

type PendingResult = Result<CompletedMessage, ScriptHostClientError>;

#[derive(Debug)]
struct PendingInvocation {
  sender: oneshot::Sender<PendingResult>,
  identity: NativeFetchInvocationIdentity,
}

#[derive(Debug, Default)]
struct SharedState {
  pending: HashMap<String, PendingInvocation>,
  active_calls: HashMap<(String, String), CapabilityCancellationToken>,
  terminal_error: Option<ScriptHostClientError>,
}

#[derive(Debug)]
pub struct ScriptHostClient {
  child: Child,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  shared: Arc<Mutex<SharedState>>,
  reader_task: JoinHandle<()>,
  shutdown_timeout: Duration,
  pub host_instance_id: String,
}

impl ScriptHostClient {
  pub async fn spawn(options: ScriptHostProcessOptions) -> Result<Self, ScriptHostClientError> {
    Self::spawn_with_authority(options, None).await
  }

  pub async fn spawn_with_authority(
    options: ScriptHostProcessOptions,
    authority: Option<Arc<DurableCapabilityAuthority>>,
  ) -> Result<Self, ScriptHostClientError> {
    let mut command = Command::new(&options.bun_executable);
    command.arg(&options.host_script_path).env(
      "WOML_SCRIPT_HOST_PROTOCOL_VERSION",
      crate::protocol::SCRIPT_HOST_PROTOCOL_VERSION.to_string(),
    );
    if let Some(limit) = options.max_frame_bytes {
      command.env("WOML_SCRIPT_HOST_MAX_FRAME_BYTES", limit.to_string());
    }
    let mut child = command
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .kill_on_drop(true)
      .spawn()
      .map_err(|error| ScriptHostClientError::Startup(error.to_string()))?;

    let mut stdin = child
      .stdin
      .take()
      .ok_or_else(|| ScriptHostClientError::Startup("child stdin was not available".to_string()))?;
    let stdout = child.stdout.take().ok_or_else(|| {
      ScriptHostClientError::Startup("child stdout was not available".to_string())
    })?;
    let mut reader = BufReader::new(stdout);
    let ready_result = timeout(
      options.startup_timeout,
      read_json_frame::<_, ReadyMessage>(&mut reader, options.max_frame_bytes),
    )
    .await;
    let ready = match ready_result {
      Err(_) => {
        let _ = child.kill().await;
        return Err(ScriptHostClientError::Startup(
          "the host did not become ready before its startup deadline".to_string(),
        ));
      }
      Ok(Err(error)) => {
        let _ = child.kill().await;
        return Err(error);
      }
      Ok(Ok(None)) => {
        let _ = child.kill().await;
        return Err(ScriptHostClientError::Startup(
          "the host exited before sending ready".to_string(),
        ));
      }
      Ok(Ok(Some(ready))) => ready,
    };
    ready.validate().map_err(ScriptHostClientError::Protocol)?;

    for artifact in &options.module_artifacts {
      write_json_frame(
        &mut stdin,
        &RegisterModuleMessage::new(&artifact.bundle_digest, &artifact.bundle),
      )
      .await?;
      let registered = timeout(
        options.startup_timeout,
        read_json_frame::<_, ModuleRegisteredMessage>(&mut reader, options.max_frame_bytes),
      )
      .await
      .map_err(|_| {
        ScriptHostClientError::Startup(
          "the host did not register a module before its startup deadline".to_string(),
        )
      })??
      .ok_or_else(|| {
        ScriptHostClientError::Startup(
          "the host exited while registering an immutable module".to_string(),
        )
      })?;
      registered
        .validate(&artifact.bundle_digest)
        .map_err(ScriptHostClientError::Protocol)?;
      if !registered.accepted {
        let message = registered
          .message
          .unwrap_or_else(|| "The module bundle was rejected.".to_string());
        return Err(ScriptHostClientError::Startup(message));
      }
    }

    let stdin = Arc::new(Mutex::new(Some(stdin)));
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let reader_shared = Arc::clone(&shared);
    let reader_stdin = Arc::clone(&stdin);
    let max_frame_bytes = options.max_frame_bytes;
    let reader_task = tokio::spawn(async move {
      host_message_reader(
        reader,
        reader_stdin,
        reader_shared,
        authority,
        max_frame_bytes,
      )
      .await;
    });

    Ok(Self {
      child,
      stdin,
      shared,
      reader_task,
      shutdown_timeout: options.shutdown_timeout,
      host_instance_id: ready.host_instance_id,
    })
  }

  pub async fn execute(
    &self,
    message: &ExecuteMessage<'_>,
  ) -> Result<CompletedMessage, ScriptHostClientError> {
    let invocation_id = message.invocation_id.to_string();
    let (sender, receiver) = oneshot::channel();
    {
      let mut shared = self.shared.lock().await;
      if let Some(error) = &shared.terminal_error {
        return Err(error.clone());
      }
      if shared.pending.contains_key(&invocation_id) {
        return Err(ScriptHostClientError::Protocol(format!(
          "invocation ID {invocation_id:?} is already active"
        )));
      }
      shared.pending.insert(
        invocation_id.clone(),
        PendingInvocation {
          sender,
          identity: NativeFetchInvocationIdentity {
            run_id: message.run_id.to_string(),
            node_id: message.node_id.to_string(),
            attempt_number: message.attempt.number,
            invocation_id: invocation_id.clone(),
            step_idempotency_key: message.attempt.idempotency_key.to_string(),
          },
        },
      );
    }

    if let Err(error) = self.write_message(message).await {
      fail_all(&self.shared, error.clone()).await;
      return Err(error);
    }

    receiver.await.unwrap_or_else(|_| {
      Err(ScriptHostClientError::HostCrashed(
        "the completion channel closed without a response".to_string(),
      ))
    })
  }

  pub async fn cancel(&self, invocation_id: &str) -> Result<(), ScriptHostClientError> {
    cancel_invocation_calls(&self.shared, invocation_id).await;
    if let Err(error) = self
      .write_message(&CancelMessage::parallel_fail_fast(invocation_id))
      .await
    {
      fail_all(&self.shared, error.clone()).await;
      return Err(error);
    }
    Ok(())
  }

  async fn write_message<T: Serialize>(&self, message: &T) -> Result<(), ScriptHostClientError> {
    let body = serde_json::to_vec(message)
      .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut stdin = self.stdin.lock().await;
    let writer = stdin.as_mut().ok_or_else(|| {
      ScriptHostClientError::HostCrashed("the host input stream is closed".to_string())
    })?;
    writer
      .write_all(header.as_bytes())
      .await
      .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
    writer
      .write_all(&body)
      .await
      .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
    writer
      .flush()
      .await
      .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))
  }

  pub async fn shutdown(mut self) {
    self.stdin.lock().await.take();
    if timeout(self.shutdown_timeout, self.child.wait())
      .await
      .is_err()
    {
      let _ = self.child.kill().await;
      let _ = self.child.wait().await;
    }
    let _ = self.reader_task.await;
  }
}

async fn write_json_frame<W: AsyncWrite + Unpin, T: Serialize>(
  writer: &mut W,
  message: &T,
) -> Result<(), ScriptHostClientError> {
  let body = serde_json::to_vec(message)
    .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
  let header = format!("Content-Length: {}\r\n\r\n", body.len());
  writer
    .write_all(header.as_bytes())
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  writer
    .write_all(&body)
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  writer
    .flush()
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))
}

async fn host_message_reader<R: AsyncRead + Unpin + Send + 'static>(
  mut reader: BufReader<R>,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  shared: Arc<Mutex<SharedState>>,
  authority: Option<Arc<DurableCapabilityAuthority>>,
  max_frame_bytes: Option<usize>,
) {
  loop {
    let message = match read_json_frame::<_, serde_json::Value>(&mut reader, max_frame_bytes).await
    {
      Ok(Some(message)) => message,
      Ok(None) => {
        fail_all(
          &shared,
          ScriptHostClientError::HostCrashed("unexpected EOF on protocol stdout".to_string()),
        )
        .await;
        return;
      }
      Err(error) => {
        fail_all(&shared, error).await;
        return;
      }
    };
    match message
      .get("messageType")
      .and_then(serde_json::Value::as_str)
    {
      Some("completed") => {
        let message: CompletedMessage = match serde_json::from_value(message) {
          Ok(message) => message,
          Err(error) => {
            fail_all(&shared, ScriptHostClientError::Protocol(error.to_string())).await;
            return;
          }
        };
        if let Err(message_error) = message.validate() {
          fail_all(&shared, ScriptHostClientError::Protocol(message_error)).await;
          return;
        }
        let identity = {
          let state = shared.lock().await;
          state
            .pending
            .get(&message.invocation_id)
            .map(|pending| pending.identity.clone())
        };
        let Some(identity) = identity else {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(format!(
              "received completion for unknown invocation {:?}",
              message.invocation_id
            )),
          )
          .await;
          return;
        };
        if let Some(authority) = &authority {
          if authority
            .close_active_native_fetches(&identity, completion_fetch_failure(&message.outcome))
            .await
            .is_err()
          {
            fail_all(
              &shared,
              ScriptHostClientError::Protocol(
                "native Fetch terminal recovery could not be persisted".to_string(),
              ),
            )
            .await;
            return;
          }
        }
        cancel_invocation_calls(&shared, &message.invocation_id).await;
        let sender = {
          let mut state = shared.lock().await;
          state.pending.remove(&message.invocation_id)
        };
        let Some(pending) = sender else {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(format!(
              "received completion for unknown invocation {:?}",
              message.invocation_id
            )),
          )
          .await;
          return;
        };
        let _ = pending.sender.send(Ok(message));
      }
      Some("capability_call") => {
        let message: CapabilityCallMessage = match serde_json::from_value(message) {
          Ok(message) => message,
          Err(error) => {
            fail_all(&shared, ScriptHostClientError::Protocol(error.to_string())).await;
            return;
          }
        };
        if let Err(error) = message.validate() {
          fail_all(&shared, ScriptHostClientError::Protocol(error)).await;
          return;
        }
        let key = (message.invocation_id.clone(), message.call_id.clone());
        let cancellation = CapabilityCancellationToken::default();
        {
          let mut state = shared.lock().await;
          if !state.pending.contains_key(&message.invocation_id) {
            drop(state);
            fail_all(
              &shared,
              ScriptHostClientError::Protocol(format!(
                "received capability call for unknown invocation {:?}",
                message.invocation_id
              )),
            )
            .await;
            return;
          }
          if state
            .active_calls
            .insert(key.clone(), cancellation.clone())
            .is_some()
          {
            drop(state);
            fail_all(
              &shared,
              ScriptHostClientError::Protocol(format!(
                "received duplicate capability call ID {:?}",
                message.call_id
              )),
            )
            .await;
            return;
          }
        }
        let call = message.call;
        let call_shared = Arc::clone(&shared);
        let call_stdin = Arc::clone(&stdin);
        let call_authority = authority.clone();
        tokio::spawn(async move {
          let result = match call_authority {
            Some(authority) => authority
              .execute(call.clone(), cancellation)
              .await
              .unwrap_or_else(|error| {
                capability_transport_failure(
                  &call,
                  "WOML_CAPABILITY_AUTHORITY_FAILED",
                  &error.to_string(),
                  false,
                  true,
                )
              }),
            None => capability_transport_failure(
              &call,
              "WOML_CAPABILITY_AUTHORITY_UNAVAILABLE",
              "The durable capability authority is unavailable for this invocation.",
              true,
              false,
            ),
          };
          let should_send = call_shared.lock().await.active_calls.remove(&key).is_some();
          if should_send {
            if let Err(error) =
              write_serialized_message(&call_stdin, &CapabilityResultMessage::new(&result)).await
            {
              fail_all(&call_shared, error).await;
            }
          }
        });
      }
      Some("fetch_observation") => {
        let message: FetchObservationMessage = match serde_json::from_value(message) {
          Ok(message) => message,
          Err(error) => {
            fail_all(&shared, ScriptHostClientError::Protocol(error.to_string())).await;
            return;
          }
        };
        if let Err(error) = message.validate() {
          fail_all(&shared, ScriptHostClientError::Protocol(error)).await;
          return;
        }
        let identity = {
          let state = shared.lock().await;
          state
            .pending
            .get(&message.invocation_id)
            .map(|pending| pending.identity.clone())
        };
        let Some(identity) = identity else {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(format!(
              "received native-Fetch observation for unknown invocation {:?}",
              message.invocation_id
            )),
          )
          .await;
          return;
        };
        let observation_shared = Arc::clone(&shared);
        let observation_stdin = Arc::clone(&stdin);
        let observation_authority = authority.clone();
        tokio::spawn(async move {
          let error = match observation_authority {
            Some(authority) => authority
              .observe_native_fetch(&identity, message.observation)
              .await
              .err()
              .map(|_| fetch_tracking_failure(true)),
            None => Some(fetch_tracking_failure(false)),
          };
          let ack = match &error {
            Some(error) => FetchObservationAckMessage::rejected(
              &message.invocation_id,
              &message.request_id,
              error,
            ),
            None => {
              FetchObservationAckMessage::accepted(&message.invocation_id, &message.request_id)
            }
          };
          if let Err(error) = write_serialized_message(&observation_stdin, &ack).await {
            fail_all(&observation_shared, error).await;
          }
        });
      }
      _ => {
        fail_all(
          &shared,
          ScriptHostClientError::Protocol(
            "received an unsupported Bun-to-Rust script-host v5 message".to_string(),
          ),
        )
        .await;
        return;
      }
    }
  }
}

async fn cancel_invocation_calls(shared: &Arc<Mutex<SharedState>>, invocation_id: &str) {
  let calls = {
    let mut state = shared.lock().await;
    let keys = state
      .active_calls
      .keys()
      .filter(|(active_invocation, _)| active_invocation == invocation_id)
      .cloned()
      .collect::<Vec<_>>();
    keys
      .into_iter()
      .filter_map(|key| state.active_calls.remove(&key))
      .collect::<Vec<_>>()
  };
  for call in calls {
    call.cancel();
  }
}

async fn write_serialized_message<T: Serialize>(
  stdin: &Arc<Mutex<Option<ChildStdin>>>,
  message: &T,
) -> Result<(), ScriptHostClientError> {
  let body = serde_json::to_vec(message)
    .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
  let header = format!("Content-Length: {}\r\n\r\n", body.len());
  let mut stdin = stdin.lock().await;
  let writer = stdin.as_mut().ok_or_else(|| {
    ScriptHostClientError::HostCrashed("the host input stream is closed".to_string())
  })?;
  writer
    .write_all(header.as_bytes())
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  writer
    .write_all(&body)
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  writer
    .flush()
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))
}

async fn fail_all(shared: &Arc<Mutex<SharedState>>, error: ScriptHostClientError) {
  let (pending, active_calls) = {
    let mut state = shared.lock().await;
    if state.terminal_error.is_none() {
      state.terminal_error = Some(error.clone());
    }
    (
      std::mem::take(&mut state.pending),
      std::mem::take(&mut state.active_calls),
    )
  };
  for (_, call) in active_calls {
    call.cancel();
  }
  for (_, pending) in pending {
    let _ = pending.sender.send(Err(error.clone()));
  }
}

fn fetch_tracking_failure(ambiguous: bool) -> CapabilityFailure {
  CapabilityFailure {
    kind: CapabilityFailureKind::TransportFailed,
    code: "WOML_NATIVE_FETCH_TRACKING_FAILED".to_string(),
    message: if ambiguous {
      "WOML could not durably record the native Fetch observation.".to_string()
    } else {
      "Durable native Fetch tracking is unavailable for this invocation.".to_string()
    },
    retryable: false,
    ambiguous,
    details: None,
  }
}

fn completion_fetch_failure(outcome: &HostOutcome) -> CapabilityFailure {
  let (kind, code, message) = match outcome {
    HostOutcome::Failure { error }
      if error.kind == HostReportedFailureKind::InvocationCancelled =>
    {
      (
        CapabilityFailureKind::Cancelled,
        "WOML_NATIVE_FETCH_CANCELLED",
        "The script invocation ended while native Fetch was active.",
      )
    }
    HostOutcome::Failure { error } if error.kind == HostReportedFailureKind::ScriptTimedOut => (
      CapabilityFailureKind::TimedOut,
      "WOML_NATIVE_FETCH_TIMED_OUT",
      "The script deadline expired while native Fetch was active.",
    ),
    HostOutcome::Failure { error } if error.kind == HostReportedFailureKind::WorkerCrashed => (
      CapabilityFailureKind::WorkerCrashed,
      "WOML_NATIVE_FETCH_WORKER_CRASHED",
      "The script Worker exited while native Fetch was active.",
    ),
    _ => (
      CapabilityFailureKind::Interrupted,
      "WOML_NATIVE_FETCH_INTERRUPTED",
      "The script ended without a terminal native Fetch observation.",
    ),
  };
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable: false,
    ambiguous: true,
    details: None,
  }
}

async fn read_json_frame<R, T>(
  reader: &mut BufReader<R>,
  max_frame_bytes: Option<usize>,
) -> Result<Option<T>, ScriptHostClientError>
where
  R: AsyncRead + Unpin,
  T: DeserializeOwned,
{
  let mut header = Vec::new();
  let read = reader
    .read_until(b'\n', &mut header)
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  if read == 0 {
    return Ok(None);
  }
  if header.len() > MAX_HEADER_BYTES || !header.ends_with(b"\r\n") {
    return Err(ScriptHostClientError::Protocol(
      "invalid Content-Length header".to_string(),
    ));
  }
  let header_text = std::str::from_utf8(&header[..header.len() - 2])
    .map_err(|_| ScriptHostClientError::Protocol("frame header is not ASCII".to_string()))?;
  let length_text = header_text
    .strip_prefix(HEADER_PREFIX)
    .ok_or_else(|| ScriptHostClientError::Protocol("invalid Content-Length header".to_string()))?;
  if length_text.is_empty() || !length_text.bytes().all(|byte| byte.is_ascii_digit()) {
    return Err(ScriptHostClientError::Protocol(
      "invalid Content-Length value".to_string(),
    ));
  }
  let content_length = length_text.parse::<usize>().map_err(|_| {
    ScriptHostClientError::Protocol("Content-Length does not fit this platform".to_string())
  })?;
  if max_frame_bytes.is_some_and(|limit| content_length > limit) {
    return Err(ScriptHostClientError::Protocol(format!(
      "frame declares {content_length} bytes and exceeds the configured limit"
    )));
  }

  let mut separator = [0_u8; 2];
  reader
    .read_exact(&mut separator)
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  if separator != *b"\r\n" {
    return Err(ScriptHostClientError::Protocol(
      "frame header is not followed by CRLF".to_string(),
    ));
  }

  let mut body = vec![0_u8; content_length];
  reader
    .read_exact(&mut body)
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  serde_json::from_slice(&body)
    .map(Some)
    .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))
}
