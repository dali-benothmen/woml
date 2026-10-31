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
  ModuleRegisteredMessage, ProgressAckMessage, ProgressReportMessage, ReadyMessage,
  RegisterModuleMessage, ScriptLogMessage,
};
use crate::{
  capability_transport_failure, CapabilityCancellationToken, CapabilityFailure,
  CapabilityFailureKind, DurableCapabilityAuthority, NativeFetchInvocationIdentity,
};

const HEADER_PREFIX: &str = "Content-Length: ";
const MAX_HEADER_BYTES: usize = 128;

// Covers the registry's bounded cancellation cleanup and durable settlement.
// Expiry fails the host closed; it is never proof that an effect did not occur.
const CAPABILITY_SETTLEMENT_GRACE: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
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
  pub source_map_digest: String,
  pub source_map: String,
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

struct PendingInvocation {
  sender: oneshot::Sender<PendingResult>,
  identity: NativeFetchInvocationIdentity,
  authority: Option<Arc<DurableCapabilityAuthority>>,
  last_operational_frame_sequence: u64,
  operational_emitted: bool,
  cancellation_requested: bool,
  latest_progress: HashMap<String, crate::OperationalBodyV1>,
}

impl std::fmt::Debug for PendingInvocation {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("PendingInvocation")
      .field("identity", &self.identity)
      .field("authority", &self.authority.as_ref().map(|_| "configured"))
      .finish_non_exhaustive()
  }
}

#[derive(Debug, Default)]
struct SharedState {
  pending: HashMap<String, PendingInvocation>,
  active_calls: HashMap<(String, String), ActiveCapabilityCall>,
  terminal_error: Option<ScriptHostClientError>,
}

#[derive(Debug)]
struct ActiveCapabilityCall {
  cancellation: CapabilityCancellationToken,
  settled: oneshot::Receiver<()>,
}

pub struct ScriptHostClient {
  child: Child,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  shared: Arc<Mutex<SharedState>>,
  reader_task: JoinHandle<()>,
  shutdown_timeout: Duration,
  default_authority: Option<Arc<DurableCapabilityAuthority>>,
  pub host_instance_id: String,
}

impl std::fmt::Debug for ScriptHostClient {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ScriptHostClient")
      .field("host_instance_id", &self.host_instance_id)
      .field(
        "default_authority",
        &self.default_authority.as_ref().map(|_| "configured"),
      )
      .finish_non_exhaustive()
  }
}

impl ScriptHostClient {
  pub async fn spawn(options: ScriptHostProcessOptions) -> Result<Self, ScriptHostClientError> {
    Self::spawn_with_authority(options, None).await
  }

  pub async fn spawn_with_authority(
    options: ScriptHostProcessOptions,
    authority: Option<Arc<DurableCapabilityAuthority>>,
  ) -> Result<Self, ScriptHostClientError> {
    let mut spawn_span = crate::performance::PerformanceSpan::new("host", "host.spawn_process");
    spawn_span.count("modules", options.module_artifacts.len());
    let mut command = Command::new(&options.bun_executable);
    command.arg(&options.host_script_path).env(
      "WOML_SCRIPT_HOST_PROTOCOL_VERSION",
      crate::protocol::SCRIPT_HOST_PROTOCOL_VERSION.to_string(),
    );
    if let Some(limit) = options.max_frame_bytes {
      command.env("WOML_SCRIPT_HOST_MAX_FRAME_BYTES", limit.to_string());
    }
    if crate::performance::enabled() {
      command.env("WOML_PROFILE_PROCESS", "host");
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

    let mut module_span = crate::performance::PerformanceSpan::new("host", "host.register_modules");
    module_span.count("modules", options.module_artifacts.len());
    module_span.bytes(
      "modules",
      options
        .module_artifacts
        .iter()
        .map(|artifact| {
          artifact
            .bundle
            .len()
            .saturating_add(artifact.source_map.len())
        })
        .sum(),
    );
    for artifact in &options.module_artifacts {
      write_json_frame(
        &mut stdin,
        &RegisterModuleMessage::new(
          &artifact.bundle_digest,
          &artifact.bundle,
          &artifact.source_map_digest,
          &artifact.source_map,
        ),
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
        .validate(&artifact.bundle_digest, &artifact.source_map_digest)
        .map_err(ScriptHostClientError::Protocol)?;
      if !registered.accepted {
        let message = registered
          .message
          .unwrap_or_else(|| "The module bundle was rejected.".to_string());
        return Err(ScriptHostClientError::Startup(message));
      }
    }
    module_span.succeed();

    let stdin = Arc::new(Mutex::new(Some(stdin)));
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let reader_shared = Arc::clone(&shared);
    let reader_stdin = Arc::clone(&stdin);
    let max_frame_bytes = options.max_frame_bytes;
    let reader_task = tokio::spawn(async move {
      host_message_reader(reader, reader_stdin, reader_shared, max_frame_bytes).await;
    });

    let client = Self {
      child,
      stdin,
      shared,
      reader_task,
      shutdown_timeout: options.shutdown_timeout,
      default_authority: authority,
      host_instance_id: ready.host_instance_id,
    };
    spawn_span.succeed();
    Ok(client)
  }

  pub async fn execute(
    &self,
    message: &ExecuteMessage<'_>,
  ) -> Result<CompletedMessage, ScriptHostClientError> {
    self
      .execute_with_authority(message, self.default_authority.clone())
      .await
  }

  pub async fn execute_with_authority(
    &self,
    message: &ExecuteMessage<'_>,
    authority: Option<Arc<DurableCapabilityAuthority>>,
  ) -> Result<CompletedMessage, ScriptHostClientError> {
    let mut span = crate::performance::PerformanceSpan::new("host", "host.execute_invocation");
    span.run_id(message.run_id.to_string());
    span.invocation_id(message.invocation_id.to_string());
    if crate::performance::enabled() {
      if let Ok(encoded) = serde_json::to_vec(message) {
        span.bytes("request", encoded.len());
      }
      if let Ok(context) = serde_json::to_vec(&message.context) {
        span.bytes("context", context.len());
      }
      span.bytes("source", message.source.len());
    }
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
          authority,
          last_operational_frame_sequence: 0,
          operational_emitted: false,
          cancellation_requested: false,
          latest_progress: HashMap::new(),
        },
      );
    }

    if let Err(error) = self.write_message(message).await {
      fail_all(&self.shared, error.clone()).await;
      return Err(error);
    }

    let result = receiver.await.unwrap_or_else(|_| {
      Err(ScriptHostClientError::HostCrashed(
        "the completion channel closed without a response".to_string(),
      ))
    });
    if result.is_ok() {
      span.succeed();
    }
    result
  }

  pub async fn is_healthy(&self) -> bool {
    self.shared.lock().await.terminal_error.is_none()
  }

  pub fn set_default_authority(&mut self, authority: Option<Arc<DurableCapabilityAuthority>>) {
    self.default_authority = authority;
  }

  pub async fn cancel(&self, invocation_id: &str) -> Result<(), ScriptHostClientError> {
    self
      .cancel_with_message(CancelMessage::parallel_fail_fast(invocation_id))
      .await
  }

  pub async fn cancel_run(&self, invocation_id: &str) -> Result<(), ScriptHostClientError> {
    self
      .cancel_with_message(CancelMessage::run_cancelled(invocation_id))
      .await
  }

  async fn cancel_with_message(
    &self,
    message: CancelMessage<'_>,
  ) -> Result<(), ScriptHostClientError> {
    let invocation_id = message.invocation_id;
    {
      let mut shared = self.shared.lock().await;
      if let Some(pending) = shared.pending.get_mut(invocation_id) {
        pending.cancellation_requested = true;
      }
      for ((active_invocation, _), call) in &shared.active_calls {
        if active_invocation == invocation_id {
          call.cancellation.cancel();
        }
      }
    }
    if let Err(error) = self.write_message(&message).await {
      fail_all(&self.shared, error.clone()).await;
      return Err(error);
    }
    Ok(())
  }

  async fn write_message<T: Serialize>(&self, message: &T) -> Result<(), ScriptHostClientError> {
    let mut span = crate::performance::PerformanceSpan::new("host", "host.write_frame");
    let body = serde_json::to_vec(message)
      .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
    span.bytes("body", body.len());
    span.count("frames", 1);
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
      .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
    span.succeed();
    Ok(())
  }

  pub async fn shutdown(mut self) {
    let mut span = crate::performance::PerformanceSpan::new("host", "host.shutdown_process");
    self.stdin.lock().await.take();
    if timeout(self.shutdown_timeout, self.child.wait())
      .await
      .is_err()
    {
      let _ = self.child.kill().await;
      let _ = self.child.wait().await;
    }
    let _ = self.reader_task.await;
    span.succeed();
  }
}

async fn write_json_frame<W: AsyncWrite + Unpin, T: Serialize>(
  writer: &mut W,
  message: &T,
) -> Result<(), ScriptHostClientError> {
  let mut span = crate::performance::PerformanceSpan::new("host", "host.write_frame");
  let body = serde_json::to_vec(message)
    .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
  span.bytes("body", body.len());
  span.count("frames", 1);
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
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  span.succeed();
  Ok(())
}

async fn host_message_reader<R: AsyncRead + Unpin + Send + 'static>(
  mut reader: BufReader<R>,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  shared: Arc<Mutex<SharedState>>,
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
        let pending = {
          let state = shared.lock().await;
          state.pending.get(&message.invocation_id).map(|pending| {
            (
              pending.identity.clone(),
              pending.authority.clone(),
              pending
                .latest_progress
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            )
          })
        };
        let Some((identity, authority, latest_progress)) = pending else {
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
          for progress in latest_progress {
            if authority
              .force_append_script_operational_record(&identity, progress)
              .await
              .is_err()
            {
              fail_all(
                &shared,
                ScriptHostClientError::Protocol(
                  "the final structured-progress snapshot could not be persisted".to_string(),
                ),
              )
              .await;
              return;
            }
          }
          let operational_emitted = shared
            .lock()
            .await
            .pending
            .get(&message.invocation_id)
            .is_some_and(|pending| pending.operational_emitted);
          if operational_emitted {
            let (status, reason_code) = operational_close(&message.outcome);
            if authority
              .append_script_operational_record(
                &identity,
                crate::OperationalBodyV1::Closed {
                  status,
                  reason_code,
                },
              )
              .await
              .is_err()
            {
              fail_all(
                &shared,
                ScriptHostClientError::Protocol(
                  "script operational-output closure could not be persisted".to_string(),
                ),
              )
              .await;
              return;
            }
          }
          authority.close_programmatic_lock_sessions(&identity.invocation_id);
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
        if let Err(error) = cancel_invocation_calls(&shared, &message.invocation_id).await {
          fail_all(&shared, error).await;
          return;
        }
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
      Some("script_log") => {
        let message: ScriptLogMessage = match serde_json::from_value(message) {
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
        let pending = {
          let state = shared.lock().await;
          state.pending.get(&message.request_id).map(|pending| {
            (
              pending.identity.clone(),
              pending.authority.clone(),
              pending.last_operational_frame_sequence,
              pending.cancellation_requested,
            )
          })
        };
        let Some((identity, Some(authority), previous, cancellation_requested)) = pending else {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "received a script-log frame for an unknown invocation".to_string(),
            ),
          )
          .await;
          return;
        };
        if cancellation_requested {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "received a script-log frame after invocation cancellation".to_string(),
            ),
          )
          .await;
          return;
        }
        if message.frame_sequence != previous.saturating_add(1) {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "received an out-of-order Script Host v12 operational frame".to_string(),
            ),
          )
          .await;
          return;
        }
        let byte_count = message.text.len() as u64;
        if authority
          .append_script_operational_record(
            &identity,
            crate::OperationalBodyV1::Output {
              source: crate::OperationalSourceV1::Script,
              level: Some(message.level.into()),
              text: message.text,
              byte_count,
            },
          )
          .await
          .is_err()
        {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "script operational output could not be persisted".to_string(),
            ),
          )
          .await;
          return;
        }
        let mut state = shared.lock().await;
        if let Some(pending) = state.pending.get_mut(&message.request_id) {
          pending.last_operational_frame_sequence = message.frame_sequence;
          pending.operational_emitted = true;
        }
      }
      Some("progress_report") => {
        let message: ProgressReportMessage = match serde_json::from_value(message) {
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
        let pending = {
          let state = shared.lock().await;
          state.pending.get(&message.request_id).map(|pending| {
            (
              pending.identity.clone(),
              pending.authority.clone(),
              pending.last_operational_frame_sequence,
              pending.cancellation_requested,
            )
          })
        };
        let Some((identity, Some(authority), previous, cancellation_requested)) = pending else {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "received a progress frame for an unknown invocation".to_string(),
            ),
          )
          .await;
          return;
        };
        if cancellation_requested {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "received a progress frame after invocation cancellation".to_string(),
            ),
          )
          .await;
          return;
        }
        if message.frame_sequence != previous.saturating_add(1) {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "received an out-of-order Script Host v12 operational frame".to_string(),
            ),
          )
          .await;
          return;
        }
        let progress_id = message.progress_id.clone();
        let progress = crate::OperationalBodyV1::Progress {
          progress_id: message.progress_id,
          current: message.current,
          total: message.total,
          unit: message.unit,
          message: message.message,
          status: crate::OperationalProgressStatusV1::Active,
        };
        if authority
          .append_script_operational_record(&identity, progress.clone())
          .await
          .is_err()
        {
          fail_all(
            &shared,
            ScriptHostClientError::Protocol(
              "structured progress could not be persisted".to_string(),
            ),
          )
          .await;
          return;
        }
        {
          let mut state = shared.lock().await;
          if let Some(pending) = state.pending.get_mut(&message.request_id) {
            pending.last_operational_frame_sequence = message.frame_sequence;
            pending.operational_emitted = true;
            pending.latest_progress.insert(progress_id, progress);
          }
        }
        if let Err(error) = write_serialized_message(
          &stdin,
          &ProgressAckMessage::accepted(&message.request_id, message.frame_sequence),
        )
        .await
        {
          fail_all(&shared, error).await;
          return;
        }
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
        let (settled, settlement) = oneshot::channel();
        let call_authority = {
          let mut state = shared.lock().await;
          let Some(authority) = state
            .pending
            .get(&message.invocation_id)
            .map(|pending| pending.authority.clone())
          else {
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
          };
          if state
            .active_calls
            .insert(
              key.clone(),
              ActiveCapabilityCall {
                cancellation: cancellation.clone(),
                settled: settlement,
              },
            )
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
          authority
        };
        let call = message.call;
        let call_shared = Arc::clone(&shared);
        let call_stdin = Arc::clone(&stdin);
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
          let should_send = {
            let mut shared = call_shared.lock().await;
            let active = shared.active_calls.remove(&key).is_some();
            active
              && shared
                .pending
                .get(&call.invocation_id)
                .is_some_and(|pending| !pending.cancellation_requested)
          };
          // Authority execution returns only after its durable terminal commit.
          // A cancelled Worker must not settle its attempt ahead of that commit.
          let _ = settled.send(());
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
        let pending = {
          let state = shared.lock().await;
          state
            .pending
            .get(&message.invocation_id)
            .map(|pending| (pending.identity.clone(), pending.authority.clone()))
        };
        let Some((identity, observation_authority)) = pending else {
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
            "received an unsupported Bun-to-Rust Script Host v12 message".to_string(),
          ),
        )
        .await;
        return;
      }
    }
  }
}

async fn cancel_invocation_calls(
  shared: &Arc<Mutex<SharedState>>,
  invocation_id: &str,
) -> Result<(), ScriptHostClientError> {
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
  for call in &calls {
    call.cancellation.cancel();
  }
  // Never hold SharedState while handlers cancel and commit. Remove transport
  // ownership first so their late responses cannot target the completed Worker.
  timeout(CAPABILITY_SETTLEMENT_GRACE, async {
    for call in calls {
      call.settled.await.map_err(|_| {
        ScriptHostClientError::HostCrashed(
          "a managed capability ended without acknowledging settlement".to_string(),
        )
      })?;
    }
    Ok(())
  })
  .await
  .map_err(|_| {
    ScriptHostClientError::HostCrashed(
      "managed capability settlement exceeded its completion grace".to_string(),
    )
  })?
}

async fn write_serialized_message<T: Serialize>(
  stdin: &Arc<Mutex<Option<ChildStdin>>>,
  message: &T,
) -> Result<(), ScriptHostClientError> {
  let mut span = crate::performance::PerformanceSpan::new("host", "host.write_frame");
  let body = serde_json::to_vec(message)
    .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
  span.bytes("body", body.len());
  span.count("frames", 1);
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
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  span.succeed();
  Ok(())
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
    call.cancellation.cancel();
  }
  for (_, pending) in pending {
    if let Some(authority) = &pending.authority {
      for progress in pending.latest_progress.values().cloned() {
        let _ = authority
          .force_append_script_operational_record(&pending.identity, progress)
          .await;
      }
      if pending.operational_emitted {
        let _ = authority
          .append_script_operational_record(
            &pending.identity,
            crate::OperationalBodyV1::Closed {
              status: crate::OperationalCloseStatusV1::Interrupted,
              reason_code: Some("WOML_SCRIPT_HOST_INTERRUPTED".to_string()),
            },
          )
          .await;
      }
      authority.close_programmatic_lock_sessions(&pending.identity.invocation_id);
    }
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

fn operational_close(outcome: &HostOutcome) -> (crate::OperationalCloseStatusV1, Option<String>) {
  match outcome {
    HostOutcome::Success { .. } => (crate::OperationalCloseStatusV1::Succeeded, None),
    HostOutcome::Failure { error }
      if error.kind == HostReportedFailureKind::InvocationCancelled =>
    {
      (
        crate::OperationalCloseStatusV1::Cancelled,
        Some(error.code.clone()),
      )
    }
    HostOutcome::Failure { error } if error.kind == HostReportedFailureKind::ScriptTimedOut => (
      crate::OperationalCloseStatusV1::TimedOut,
      Some(error.code.clone()),
    ),
    HostOutcome::Failure { error } if error.kind == HostReportedFailureKind::WorkerCrashed => (
      crate::OperationalCloseStatusV1::Interrupted,
      Some(error.code.clone()),
    ),
    HostOutcome::Failure { error } => (
      crate::OperationalCloseStatusV1::Failed,
      Some(error.code.clone()),
    ),
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
  let mut span = crate::performance::PerformanceSpan::new("host", "host.read_frame");
  let mut header = Vec::new();
  let read = reader
    .read_until(b'\n', &mut header)
    .await
    .map_err(|error| ScriptHostClientError::HostCrashed(error.to_string()))?;
  if read == 0 {
    span.succeed();
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
  span.bytes("body", content_length);
  span.count("frames", 1);
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
  let decoded = serde_json::from_slice(&body)
    .map(Some)
    .map_err(|error| ScriptHostClientError::Protocol(error.to_string()))?;
  span.succeed();
  Ok(decoded)
}
