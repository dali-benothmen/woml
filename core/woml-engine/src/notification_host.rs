use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::notification_protocol::{
  NotificationCompletedMessage, NotificationInteractionMessage, NotificationReadyMessage,
  NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
};

const HEADER_PREFIX: &str = "Content-Length: ";
const MAX_HEADER_BYTES: usize = 128;

#[derive(Debug, Clone)]
pub struct NotificationHostProcessOptions {
  pub bun_executable: PathBuf,
  pub host_script_path: PathBuf,
  pub startup_timeout: Duration,
  pub shutdown_timeout: Duration,
  pub max_frame_bytes: usize,
}

impl NotificationHostProcessOptions {
  pub fn new(bun_executable: impl Into<PathBuf>, host_script_path: impl Into<PathBuf>) -> Self {
    Self {
      bun_executable: bun_executable.into(),
      host_script_path: host_script_path.into(),
      startup_timeout: Duration::from_secs(5),
      shutdown_timeout: Duration::from_secs(2),
      max_frame_bytes: NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotificationHostClientError {
  #[error("failed to start the Bun notification provider host: {0}")]
  Startup(String),
  #[error("notification-provider protocol violation: {0}")]
  Protocol(String),
  #[error("the Bun notification provider host crashed: {0}")]
  HostCrashed(String),
  #[error("timed out waiting for a provider interaction")]
  InteractionTimedOut,
}

type PendingResult = Result<NotificationCompletedMessage, NotificationHostClientError>;

#[derive(Debug, Default)]
struct SharedState {
  pending: HashMap<String, oneshot::Sender<PendingResult>>,
  terminal_error: Option<NotificationHostClientError>,
}

#[derive(Debug)]
pub struct NotificationHostClient {
  child: Child,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  shared: Arc<Mutex<SharedState>>,
  interactions: Mutex<mpsc::UnboundedReceiver<NotificationInteractionMessage>>,
  reader_task: JoinHandle<()>,
  shutdown_timeout: Duration,
  pub host_instance_id: String,
}

impl NotificationHostClient {
  pub async fn spawn(
    options: NotificationHostProcessOptions,
  ) -> Result<Self, NotificationHostClientError> {
    let mut child = Command::new(&options.bun_executable)
      .arg(&options.host_script_path)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .kill_on_drop(true)
      .spawn()
      .map_err(|error| NotificationHostClientError::Startup(error.to_string()))?;
    let stdin = child.stdin.take().ok_or_else(|| {
      NotificationHostClientError::Startup("child stdin was not available".to_string())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
      NotificationHostClientError::Startup("child stdout was not available".to_string())
    })?;
    let mut reader = BufReader::new(stdout);
    let ready = match timeout(
      options.startup_timeout,
      read_json_frame::<_, NotificationReadyMessage>(&mut reader, options.max_frame_bytes),
    )
    .await
    {
      Err(_) => {
        let _ = child.kill().await;
        return Err(NotificationHostClientError::Startup(
          "the host did not become ready before its deadline".to_string(),
        ));
      }
      Ok(Err(error)) => {
        let _ = child.kill().await;
        return Err(error);
      }
      Ok(Ok(None)) => {
        let _ = child.kill().await;
        return Err(NotificationHostClientError::Startup(
          "the host exited before sending ready".to_string(),
        ));
      }
      Ok(Ok(Some(ready))) => ready,
    };
    ready
      .validate()
      .map_err(NotificationHostClientError::Protocol)?;

    let shared = Arc::new(Mutex::new(SharedState::default()));
    let reader_shared = Arc::clone(&shared);
    let (interaction_sender, interaction_receiver) = mpsc::unbounded_channel();
    let max_frame_bytes = options.max_frame_bytes;
    let reader_task = tokio::spawn(async move {
      outbound_reader(reader, reader_shared, interaction_sender, max_frame_bytes).await;
    });

    Ok(Self {
      child,
      stdin: Arc::new(Mutex::new(Some(stdin))),
      shared,
      interactions: Mutex::new(interaction_receiver),
      reader_task,
      shutdown_timeout: options.shutdown_timeout,
      host_instance_id: ready.host_instance_id,
    })
  }

  pub async fn invoke<T: Serialize>(
    &self,
    invocation_id: &str,
    message: &T,
  ) -> Result<NotificationCompletedMessage, NotificationHostClientError> {
    let (sender, receiver) = oneshot::channel();
    {
      let mut shared = self.shared.lock().await;
      if let Some(error) = &shared.terminal_error {
        return Err(error.clone());
      }
      if shared.pending.contains_key(invocation_id) {
        return Err(NotificationHostClientError::Protocol(format!(
          "invocation ID {invocation_id:?} is already active"
        )));
      }
      shared.pending.insert(invocation_id.to_string(), sender);
    }
    if let Err(error) = self.write_message(message).await {
      fail_all(&self.shared, error.clone()).await;
      return Err(error);
    }
    receiver.await.unwrap_or_else(|_| {
      Err(NotificationHostClientError::HostCrashed(
        "the completion channel closed without a response".to_string(),
      ))
    })
  }

  pub async fn next_interaction(
    &self,
    wait: Duration,
  ) -> Result<NotificationInteractionMessage, NotificationHostClientError> {
    match timeout(wait, self.interactions.lock().await.recv()).await {
      Err(_) => Err(NotificationHostClientError::InteractionTimedOut),
      Ok(Some(message)) => Ok(message),
      Ok(None) => Err(NotificationHostClientError::HostCrashed(
        "the interaction stream closed".to_string(),
      )),
    }
  }

  async fn write_message<T: Serialize>(
    &self,
    message: &T,
  ) -> Result<(), NotificationHostClientError> {
    let body = serde_json::to_vec(message)
      .map_err(|error| NotificationHostClientError::Protocol(error.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut stdin = self.stdin.lock().await;
    let writer = stdin.as_mut().ok_or_else(|| {
      NotificationHostClientError::HostCrashed("the host input stream is closed".to_string())
    })?;
    writer
      .write_all(header.as_bytes())
      .await
      .map_err(|error| NotificationHostClientError::HostCrashed(error.to_string()))?;
    writer
      .write_all(&body)
      .await
      .map_err(|error| NotificationHostClientError::HostCrashed(error.to_string()))?;
    writer
      .flush()
      .await
      .map_err(|error| NotificationHostClientError::HostCrashed(error.to_string()))
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NotificationOutboundMessage {
  Completed(NotificationCompletedMessage),
  Interaction(NotificationInteractionMessage),
}

async fn outbound_reader<R: AsyncRead + Unpin>(
  mut reader: BufReader<R>,
  shared: Arc<Mutex<SharedState>>,
  interactions: mpsc::UnboundedSender<NotificationInteractionMessage>,
  max_frame_bytes: usize,
) {
  loop {
    let message =
      match read_json_frame::<_, NotificationOutboundMessage>(&mut reader, max_frame_bytes).await {
        Ok(Some(message)) => message,
        Ok(None) => {
          fail_all(
            &shared,
            NotificationHostClientError::HostCrashed(
              "unexpected EOF on protocol stdout".to_string(),
            ),
          )
          .await;
          return;
        }
        Err(error) => {
          fail_all(&shared, error).await;
          return;
        }
      };
    match message {
      NotificationOutboundMessage::Completed(message) => {
        if let Err(error) = message.validate() {
          fail_all(&shared, NotificationHostClientError::Protocol(error)).await;
          return;
        }
        let sender = shared.lock().await.pending.remove(&message.invocation_id);
        let Some(sender) = sender else {
          fail_all(
            &shared,
            NotificationHostClientError::Protocol(format!(
              "received completion for unknown invocation {:?}",
              message.invocation_id
            )),
          )
          .await;
          return;
        };
        let _ = sender.send(Ok(message));
      }
      NotificationOutboundMessage::Interaction(message) => {
        if let Err(error) = message.validate() {
          fail_all(&shared, NotificationHostClientError::Protocol(error)).await;
          return;
        }
        if interactions.send(message).is_err() {
          return;
        }
      }
    }
  }
}

async fn fail_all(shared: &Arc<Mutex<SharedState>>, error: NotificationHostClientError) {
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
  max_frame_bytes: usize,
) -> Result<Option<T>, NotificationHostClientError>
where
  R: AsyncRead + Unpin,
  T: DeserializeOwned,
{
  let mut header = Vec::new();
  let read = reader
    .read_until(b'\n', &mut header)
    .await
    .map_err(|error| NotificationHostClientError::HostCrashed(error.to_string()))?;
  if read == 0 {
    return Ok(None);
  }
  if header.len() > MAX_HEADER_BYTES || !header.ends_with(b"\r\n") {
    return Err(NotificationHostClientError::Protocol(
      "invalid Content-Length header".to_string(),
    ));
  }
  let header_text = std::str::from_utf8(&header[..header.len() - 2])
    .map_err(|_| NotificationHostClientError::Protocol("frame header is not ASCII".to_string()))?;
  let length = header_text.strip_prefix(HEADER_PREFIX).ok_or_else(|| {
    NotificationHostClientError::Protocol("invalid Content-Length header".to_string())
  })?;
  if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
    return Err(NotificationHostClientError::Protocol(
      "invalid Content-Length value".to_string(),
    ));
  }
  let content_length = length.parse::<usize>().map_err(|_| {
    NotificationHostClientError::Protocol("Content-Length does not fit this platform".to_string())
  })?;
  if content_length > max_frame_bytes {
    return Err(NotificationHostClientError::Protocol(format!(
      "frame declares {content_length} bytes and exceeds the configured limit"
    )));
  }
  let mut separator = [0_u8; 2];
  reader
    .read_exact(&mut separator)
    .await
    .map_err(|error| NotificationHostClientError::HostCrashed(error.to_string()))?;
  if separator != *b"\r\n" {
    return Err(NotificationHostClientError::Protocol(
      "frame header is not followed by CRLF".to_string(),
    ));
  }
  let mut body = vec![0_u8; content_length];
  reader
    .read_exact(&mut body)
    .await
    .map_err(|error| NotificationHostClientError::HostCrashed(error.to_string()))?;
  serde_json::from_slice(&body)
    .map(Some)
    .map_err(|error| NotificationHostClientError::Protocol(error.to_string()))
}
