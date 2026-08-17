use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use uuid::Uuid;

use crate::custom_notification_provider_protocol::{
  CustomProviderCancelMessage, CustomProviderCompletedMessage, CustomProviderExecuteMessage,
  CustomProviderReadyMessage, CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL, CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
};

const HEADER_PREFIX: &str = "Content-Length: ";
const MAX_HEADER_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProviderScriptArtifact {
  pub script_artifact_id: String,
  pub definition_digest: String,
  pub source: String,
}

#[derive(Debug, Serialize)]
struct ArtifactManifest<'a> {
  artifacts: &'a [CustomProviderScriptArtifact],
}

#[derive(Debug, Clone)]
pub struct CustomNotificationHostProcessOptions {
  pub bun_executable: PathBuf,
  pub host_script_path: PathBuf,
  pub startup_timeout: Duration,
  pub shutdown_timeout: Duration,
  pub max_frame_bytes: usize,
  pub artifacts: Vec<CustomProviderScriptArtifact>,
}

impl CustomNotificationHostProcessOptions {
  pub fn new(bun_executable: impl Into<PathBuf>, host_script_path: impl Into<PathBuf>) -> Self {
    Self {
      bun_executable: bun_executable.into(),
      host_script_path: host_script_path.into(),
      startup_timeout: Duration::from_secs(5),
      shutdown_timeout: Duration::from_secs(2),
      max_frame_bytes: CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
      artifacts: Vec::new(),
    }
  }

  pub fn with_artifacts(mut self, artifacts: Vec<CustomProviderScriptArtifact>) -> Self {
    self.artifacts = artifacts;
    self
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CustomNotificationHostClientError {
  #[error("failed to start the Bun custom-provider host: {0}")]
  Startup(String),
  #[error("custom-provider protocol violation: {0}")]
  Protocol(String),
  #[error("the Bun custom-provider host crashed: {0}")]
  HostCrashed(String),
}

type PendingResult = Result<CustomProviderCompletedMessage, CustomNotificationHostClientError>;

#[derive(Debug, Default)]
struct SharedState {
  pending: HashMap<String, oneshot::Sender<PendingResult>>,
  terminal_error: Option<CustomNotificationHostClientError>,
}

#[derive(Debug)]
pub struct CustomNotificationHostClient {
  child: Child,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  shared: Arc<Mutex<SharedState>>,
  reader_task: JoinHandle<()>,
  shutdown_timeout: Duration,
  manifest_path: PathBuf,
  pub host_instance_id: String,
}

impl CustomNotificationHostClient {
  pub async fn spawn(
    options: CustomNotificationHostProcessOptions,
  ) -> Result<Self, CustomNotificationHostClientError> {
    validate_artifacts(&options.artifacts)?;
    let manifest_path = std::env::temp_dir().join(format!(
      "woml-custom-provider-artifacts-{}.json",
      Uuid::new_v4().simple()
    ));
    let manifest = serde_json::to_vec(&ArtifactManifest {
      artifacts: &options.artifacts,
    })
    .map_err(|error| CustomNotificationHostClientError::Startup(error.to_string()))?;
    std::fs::write(&manifest_path, manifest)
      .map_err(|error| CustomNotificationHostClientError::Startup(error.to_string()))?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o600));
    }

    let mut child = match Command::new(&options.bun_executable)
      .arg(&options.host_script_path)
      .arg(&manifest_path)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .kill_on_drop(true)
      .spawn()
    {
      Ok(child) => child,
      Err(error) => {
        let _ = std::fs::remove_file(&manifest_path);
        return Err(CustomNotificationHostClientError::Startup(
          error.to_string(),
        ));
      }
    };
    let stdin = child.stdin.take().ok_or_else(|| {
      CustomNotificationHostClientError::Startup("child stdin was not available".to_string())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
      CustomNotificationHostClientError::Startup("child stdout was not available".to_string())
    })?;
    let mut reader = BufReader::new(stdout);
    let ready = match timeout(
      options.startup_timeout,
      read_json_frame::<_, CustomProviderReadyMessage>(&mut reader, options.max_frame_bytes),
    )
    .await
    {
      Err(_) => {
        let _ = child.kill().await;
        let _ = std::fs::remove_file(&manifest_path);
        return Err(CustomNotificationHostClientError::Startup(
          "the host did not become ready before its deadline".to_string(),
        ));
      }
      Ok(Err(error)) => {
        let _ = child.kill().await;
        let _ = std::fs::remove_file(&manifest_path);
        return Err(error);
      }
      Ok(Ok(None)) => {
        let _ = child.kill().await;
        let _ = std::fs::remove_file(&manifest_path);
        return Err(CustomNotificationHostClientError::Startup(
          "the host exited before sending ready".to_string(),
        ));
      }
      Ok(Ok(Some(ready))) => ready,
    };
    ready
      .validate()
      .map_err(CustomNotificationHostClientError::Protocol)?;
    // The host has loaded the immutable manifest before announcing readiness.
    let _ = std::fs::remove_file(&manifest_path);

    let shared = Arc::new(Mutex::new(SharedState::default()));
    let reader_shared = Arc::clone(&shared);
    let max_frame_bytes = options.max_frame_bytes;
    let reader_task = tokio::spawn(async move {
      outbound_reader(reader, reader_shared, max_frame_bytes).await;
    });
    Ok(Self {
      child,
      stdin: Arc::new(Mutex::new(Some(stdin))),
      shared,
      reader_task,
      shutdown_timeout: options.shutdown_timeout,
      manifest_path,
      host_instance_id: ready.host_instance_id,
    })
  }

  pub async fn invoke(
    &self,
    message: &CustomProviderExecuteMessage,
  ) -> Result<CustomProviderCompletedMessage, CustomNotificationHostClientError> {
    message
      .validate()
      .map_err(CustomNotificationHostClientError::Protocol)?;
    let invocation_id = &message.invocation_id;
    let (sender, receiver) = oneshot::channel();
    {
      let mut shared = self.shared.lock().await;
      if let Some(error) = &shared.terminal_error {
        return Err(error.clone());
      }
      if shared.pending.contains_key(invocation_id) {
        return Err(CustomNotificationHostClientError::Protocol(format!(
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
      Err(CustomNotificationHostClientError::HostCrashed(
        "the completion channel closed without a response".to_string(),
      ))
    })
  }

  pub async fn cancel(&self, invocation_id: &str) -> Result<(), CustomNotificationHostClientError> {
    self
      .write_message(&CustomProviderCancelMessage {
        protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL.to_string(),
        protocol_version: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
        message_type: "cancel".to_string(),
        invocation_id: invocation_id.to_string(),
      })
      .await
  }

  async fn write_message<T: Serialize>(
    &self,
    message: &T,
  ) -> Result<(), CustomNotificationHostClientError> {
    let body = serde_json::to_vec(message)
      .map_err(|error| CustomNotificationHostClientError::Protocol(error.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut stdin = self.stdin.lock().await;
    let writer = stdin.as_mut().ok_or_else(|| {
      CustomNotificationHostClientError::HostCrashed("the host input is closed".to_string())
    })?;
    writer
      .write_all(header.as_bytes())
      .await
      .map_err(|error| CustomNotificationHostClientError::HostCrashed(error.to_string()))?;
    writer
      .write_all(&body)
      .await
      .map_err(|error| CustomNotificationHostClientError::HostCrashed(error.to_string()))?;
    writer
      .flush()
      .await
      .map_err(|error| CustomNotificationHostClientError::HostCrashed(error.to_string()))
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
    let _ = std::fs::remove_file(&self.manifest_path);
  }
}

fn validate_artifacts(
  artifacts: &[CustomProviderScriptArtifact],
) -> Result<(), CustomNotificationHostClientError> {
  let mut ids = HashSet::new();
  if artifacts.is_empty()
    || artifacts.iter().any(|artifact| {
      artifact.script_artifact_id.is_empty()
        || !artifact.definition_digest.starts_with("sha256:")
        || artifact.source.len() > 1_048_576
        || !ids.insert(artifact.script_artifact_id.as_str())
    })
  {
    return Err(CustomNotificationHostClientError::Startup(
      "custom-provider artifacts do not match their immutable contract".to_string(),
    ));
  }
  Ok(())
}

async fn outbound_reader<R: AsyncRead + Unpin>(
  mut reader: BufReader<R>,
  shared: Arc<Mutex<SharedState>>,
  max_frame_bytes: usize,
) {
  loop {
    let message = match read_json_frame::<_, CustomProviderCompletedMessage>(
      &mut reader,
      max_frame_bytes,
    )
    .await
    {
      Ok(Some(message)) => message,
      Ok(None) => {
        fail_all(
          &shared,
          CustomNotificationHostClientError::HostCrashed(
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
    if let Err(error) = message.validate() {
      fail_all(&shared, CustomNotificationHostClientError::Protocol(error)).await;
      return;
    }
    let sender = shared.lock().await.pending.remove(&message.invocation_id);
    let Some(sender) = sender else {
      fail_all(
        &shared,
        CustomNotificationHostClientError::Protocol(format!(
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

async fn fail_all(shared: &Arc<Mutex<SharedState>>, error: CustomNotificationHostClientError) {
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
) -> Result<Option<T>, CustomNotificationHostClientError>
where
  R: AsyncRead + Unpin,
  T: DeserializeOwned,
{
  let mut header = Vec::new();
  let read = reader
    .read_until(b'\n', &mut header)
    .await
    .map_err(|error| CustomNotificationHostClientError::HostCrashed(error.to_string()))?;
  if read == 0 {
    return Ok(None);
  }
  if header.len() > MAX_HEADER_BYTES || !header.ends_with(b"\r\n") {
    return Err(CustomNotificationHostClientError::Protocol(
      "invalid Content-Length header".to_string(),
    ));
  }
  let text = std::str::from_utf8(&header[..header.len() - 2])
    .map_err(|_| CustomNotificationHostClientError::Protocol("header is not ASCII".to_string()))?;
  let content_length = text
    .strip_prefix(HEADER_PREFIX)
    .and_then(|value| value.parse::<usize>().ok())
    .ok_or_else(|| {
      CustomNotificationHostClientError::Protocol("invalid Content-Length header".to_string())
    })?;
  if content_length > max_frame_bytes {
    return Err(CustomNotificationHostClientError::Protocol(
      "frame exceeds the configured size limit".to_string(),
    ));
  }
  let mut separator = [0_u8; 2];
  reader
    .read_exact(&mut separator)
    .await
    .map_err(|error| CustomNotificationHostClientError::HostCrashed(error.to_string()))?;
  if separator != *b"\r\n" {
    return Err(CustomNotificationHostClientError::Protocol(
      "frame header is not followed by CRLF".to_string(),
    ));
  }
  let mut body = vec![0; content_length];
  reader
    .read_exact(&mut body)
    .await
    .map_err(|error| CustomNotificationHostClientError::HostCrashed(error.to_string()))?;
  serde_json::from_slice(&body)
    .map(Some)
    .map_err(|error| CustomNotificationHostClientError::Protocol(error.to_string()))
}
