use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::protocol::{CancelMessage, CompletedMessage, ExecuteMessage, ReadyMessage};

const HEADER_PREFIX: &str = "Content-Length: ";
const MAX_HEADER_BYTES: usize = 128;

#[derive(Debug, Clone)]
pub struct ScriptHostProcessOptions {
  pub bun_executable: PathBuf,
  pub host_script_path: PathBuf,
  pub startup_timeout: Duration,
  pub shutdown_timeout: Duration,
  pub max_frame_bytes: Option<usize>,
}

impl ScriptHostProcessOptions {
  pub fn new(bun_executable: impl Into<PathBuf>, host_script_path: impl Into<PathBuf>) -> Self {
    Self {
      bun_executable: bun_executable.into(),
      host_script_path: host_script_path.into(),
      startup_timeout: Duration::from_secs(5),
      shutdown_timeout: Duration::from_secs(2),
      max_frame_bytes: None,
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

#[derive(Debug, Default)]
struct SharedState {
  pending: HashMap<String, oneshot::Sender<PendingResult>>,
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
    let mut child = Command::new(&options.bun_executable)
      .arg(&options.host_script_path)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .kill_on_drop(true)
      .spawn()
      .map_err(|error| ScriptHostClientError::Startup(error.to_string()))?;

    let stdin = child
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

    let shared = Arc::new(Mutex::new(SharedState::default()));
    let reader_shared = Arc::clone(&shared);
    let max_frame_bytes = options.max_frame_bytes;
    let reader_task = tokio::spawn(async move {
      completion_reader(reader, reader_shared, max_frame_bytes).await;
    });

    Ok(Self {
      child,
      stdin: Arc::new(Mutex::new(Some(stdin))),
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
      shared.pending.insert(invocation_id.clone(), sender);
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

async fn completion_reader<R: AsyncRead + Unpin>(
  mut reader: BufReader<R>,
  shared: Arc<Mutex<SharedState>>,
  max_frame_bytes: Option<usize>,
) {
  loop {
    let message = match read_json_frame::<_, CompletedMessage>(&mut reader, max_frame_bytes).await {
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
    if let Err(message_error) = message.validate() {
      fail_all(&shared, ScriptHostClientError::Protocol(message_error)).await;
      return;
    }

    let sender = {
      let mut state = shared.lock().await;
      state.pending.remove(&message.invocation_id)
    };
    let Some(sender) = sender else {
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
    let _ = sender.send(Ok(message));
  }
}

async fn fail_all(shared: &Arc<Mutex<SharedState>>, error: ScriptHostClientError) {
  let pending = {
    let mut state = shared.lock().await;
    if state.terminal_error.is_none() {
      state.terminal_error = Some(error.clone());
    }
    std::mem::take(&mut state.pending)
  };
  for (_, sender) in pending {
    let _ = sender.send(Err(error.clone()));
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
