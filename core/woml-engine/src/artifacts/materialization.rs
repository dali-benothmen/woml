//! Authorized invocation-private copies, cancellation/partial cleanup, exact length/digest checks, and verified publication.

use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::artifact_too_large;
use crate::artifacts::backend_unavailable;
use crate::artifacts::directory_bytes;
use crate::artifacts::ensure_private_directory;
use crate::artifacts::is_windows_reparse;
use crate::artifacts::materialization_unavailable;
use crate::artifacts::sync_directory;
use crate::artifacts::ArtifactReferenceRequestV1;
use crate::artifacts::ArtifactStage;
use crate::artifacts::ArtifactStore;
use crate::artifacts::ARTIFACTS_CONTRACT;
use crate::artifacts::ARTIFACTS_CONTRACT_VERSION;
use crate::artifacts::STREAM_BUFFER_BYTES;
use crate::CapabilityCallRequest;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use serde_json::json;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub(in crate::artifacts) fn verify_materialized_file(
  path: &Path,
  expected_digest: &str,
  expected_size: u64,
) -> Result<(), CapabilityFailure> {
  let metadata = fs::symlink_metadata(path).map_err(|_| materialization_unavailable())?;
  if metadata.file_type().is_symlink()
    || !metadata.is_file()
    || is_windows_reparse(&metadata)
    || metadata.len() != expected_size
  {
    return Err(artifact_corrupt());
  }
  let mut source = File::open(path).map_err(|_| materialization_unavailable())?;
  let mut digest = Sha256::new();
  let mut size = 0_u64;
  let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
  loop {
    let read = source.read(&mut buffer).map_err(|_| artifact_corrupt())?;
    if read == 0 {
      break;
    }
    size = size.saturating_add(read as u64);
    digest.update(&buffer[..read]);
  }
  if size != expected_size || hex::encode(digest.finalize()) != expected_digest {
    return Err(artifact_corrupt());
  }
  Ok(())
}

pub(in crate::artifacts) fn stream_materialization(
  source_path: &Path,
  final_path: &Path,
  expected_digest: &str,
  expected_size: u64,
  cancellation: &CapabilityCancellationToken,
) -> Result<(), CapabilityFailure> {
  let stage_path = final_path.with_extension(format!("{}.part", uuid::Uuid::new_v4().simple()));
  let mut options = OpenOptions::new();
  options.create_new(true).write(true).read(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  let destination = options
    .open(&stage_path)
    .map_err(|_| materialization_unavailable())?;
  let mut stage = ArtifactStage {
    path: stage_path,
    file: Some(destination),
  };
  let mut source = File::open(source_path).map_err(|_| backend_unavailable())?;
  let mut digest = Sha256::new();
  let mut size = 0_u64;
  let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
  loop {
    if cancellation.is_cancelled() {
      return Err(artifact_cancelled());
    }
    let read = source.read(&mut buffer).map_err(|_| artifact_corrupt())?;
    if read == 0 {
      break;
    }
    size = size.saturating_add(read as u64);
    if size > expected_size {
      return Err(artifact_corrupt());
    }
    digest.update(&buffer[..read]);
    stage
      .file
      .as_mut()
      .ok_or_else(materialization_unavailable)?
      .write_all(&buffer[..read])
      .map_err(|_| materialization_unavailable())?;
  }
  if size != expected_size || hex::encode(digest.finalize()) != expected_digest {
    return Err(artifact_corrupt());
  }
  let file = stage.file.take().ok_or_else(materialization_unavailable)?;
  file.sync_all().map_err(|_| materialization_unavailable())?;
  drop(file);
  match fs::hard_link(&stage.path, final_path) {
    Ok(()) => {}
    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
      verify_materialized_file(final_path, expected_digest, expected_size)?;
    }
    Err(_) => return Err(materialization_unavailable()),
  }
  fs::remove_file(&stage.path).map_err(|_| materialization_unavailable())?;
  sync_directory(
    final_path
      .parent()
      .ok_or_else(materialization_unavailable)?,
  )?;
  verify_materialized_file(final_path, expected_digest, expected_size)
}

impl ArtifactStore {
  pub(in crate::artifacts) fn invocation_materialization_directory(
    &self,
    invocation_id: &str,
  ) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(b"woml.artifact-materialization\0v1\0");
    digest.update(invocation_id.as_bytes());
    self.materializations.join(hex::encode(digest.finalize()))
  }

  pub(in crate::artifacts) fn cleanup_materialization(
    &self,
    invocation_id: &str,
  ) -> Result<(), CapabilityFailure> {
    let directory = self.invocation_materialization_directory(invocation_id);
    match fs::symlink_metadata(&directory) {
      Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
        fs::remove_dir_all(directory).map_err(|_| materialization_unavailable())
      }
      Ok(_) => Err(materialization_unavailable()),
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
      Err(_) => Err(materialization_unavailable()),
    }
  }

  pub(in crate::artifacts) fn materialize(
    &self,
    request: &CapabilityCallRequest,
    parsed: &ArtifactReferenceRequestV1,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<Value, CapabilityFailure> {
    let artifact = self.authorize_reference(&request.run_id, &parsed.input.artifact)?;
    if artifact.reference.size > self.policy.max_materialization_bytes_per_attempt {
      return Err(artifact_too_large());
    }
    let source = self
      .backend
      .verify(&artifact.reference.sha256, artifact.reference.size)?;
    let directory = self.invocation_materialization_directory(&request.invocation_id);
    ensure_private_directory(&directory).map_err(|_| materialization_unavailable())?;
    let existing_bytes = directory_bytes(&directory)?;
    let final_path = directory.join(&artifact.reference.id);
    if final_path.exists() {
      verify_materialized_file(
        &final_path,
        &artifact.reference.sha256,
        artifact.reference.size,
      )?;
    } else {
      if existing_bytes.saturating_add(artifact.reference.size)
        > self.policy.max_materialization_bytes_per_attempt
      {
        return Err(artifact_too_large());
      }
      stream_materialization(
        &source.path,
        &final_path,
        &artifact.reference.sha256,
        artifact.reference.size,
        cancellation,
      )?;
    }
    let path = final_path
      .to_str()
      .filter(|path| path.len() <= 4_096)
      .ok_or_else(materialization_unavailable)?;
    Ok(json!({
      "contract": ARTIFACTS_CONTRACT,
      "contractVersion": ARTIFACTS_CONTRACT_VERSION,
      "kind": "result",
      "operation": "materialize",
      "data": {
        "artifact": artifact.reference,
        "path": path
      }
    }))
  }
}
