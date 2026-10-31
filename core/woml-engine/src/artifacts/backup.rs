//! Metadata-derived verified blob inventories and supported artifact export/restore/verification. Metadata alone cannot recreate bytes.

use crate::artifacts::artifact_corrupt;
use crate::artifacts::artifact_failure;
use crate::artifacts::backend_unavailable;
use crate::artifacts::ensure_private_directory;
use crate::artifacts::is_windows_reparse;
use crate::artifacts::stream_materialization;
use crate::artifacts::sync_directory;
use crate::artifacts::validate_schema;
use crate::artifacts::verify_materialized_file;
use crate::artifacts::ArtifactBackupInventoryV1;
use crate::artifacts::ArtifactPolicyV1;
use crate::artifacts::ArtifactRuntimeMode;
use crate::artifacts::ArtifactStore;
use crate::artifacts::PreparedArtifactPolicy;
use crate::artifacts::STREAM_BUFFER_BYTES;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::DurableEventStore;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

pub fn export_artifact_backup(
  state_path: impl AsRef<Path>,
  destination: impl AsRef<Path>,
  policy: ArtifactPolicyV1,
  mode: ArtifactRuntimeMode,
  resolved_secrets: &BTreeMap<String, String>,
) -> Result<ArtifactBackupInventoryV1, CapabilityFailure> {
  let policy = Arc::new(
    PreparedArtifactPolicy::prepare(policy, mode, resolved_secrets).map_err(|error| {
      artifact_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_ARTIFACT_POLICY_INVALID",
        &error.to_string(),
        false,
        false,
      )
    })?,
  );
  let store = ArtifactStore::open_with_startup_cleanup(state_path.as_ref(), policy, false)?;
  let destination = destination.as_ref();
  ensure_private_directory(destination)?;
  if fs::read_dir(destination)
    .map_err(|_| backend_unavailable())?
    .next()
    .is_some()
  {
    return Err(backend_unavailable());
  }
  let connection = store.connection()?;
  let inventory = artifact_blob_inventory(&connection)?;
  let cancellation = CapabilityCancellationToken::default();
  for (digest, size) in &inventory {
    let source = store.backend.verify(digest, *size)?;
    let target = destination
      .join("blobs/sha256")
      .join(&digest[..2])
      .join(&digest[2..]);
    ensure_private_directory(target.parent().ok_or_else(backend_unavailable)?)?;
    stream_materialization(&source.path, &target, digest, *size, &cancellation)?;
  }
  sync_directory(destination)?;
  Ok(artifact_backup_inventory(&inventory))
}

pub fn restore_artifact_backup(
  state_path: impl AsRef<Path>,
  source: impl AsRef<Path>,
  policy: ArtifactPolicyV1,
  mode: ArtifactRuntimeMode,
  resolved_secrets: &BTreeMap<String, String>,
) -> Result<ArtifactBackupInventoryV1, CapabilityFailure> {
  let policy = Arc::new(
    PreparedArtifactPolicy::prepare(policy, mode, resolved_secrets).map_err(|error| {
      artifact_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_ARTIFACT_POLICY_INVALID",
        &error.to_string(),
        false,
        false,
      )
    })?,
  );
  let store = ArtifactStore::open_with_startup_cleanup(state_path.as_ref(), policy, false)?;
  let connection = store.connection()?;
  let inventory = artifact_blob_inventory(&connection)?;
  let source = fs::canonicalize(source.as_ref()).map_err(|_| backend_unavailable())?;
  let metadata = fs::symlink_metadata(&source).map_err(|_| backend_unavailable())?;
  if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
    return Err(backend_unavailable());
  }
  let cancellation = CapabilityCancellationToken::default();
  for (digest, size) in &inventory {
    let backup_path = source
      .join("blobs/sha256")
      .join(&digest[..2])
      .join(&digest[2..]);
    verify_materialized_file(&backup_path, digest, *size)?;
    let mut stage = store.backend.begin_stage()?;
    let target = stage.file.as_mut().ok_or_else(backend_unavailable)?;
    let mut input = File::open(&backup_path).map_err(|_| backend_unavailable())?;
    let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
    loop {
      let read = input.read(&mut buffer).map_err(|_| backend_unavailable())?;
      if read == 0 {
        break;
      }
      target
        .write_all(&buffer[..read])
        .map_err(|_| backend_unavailable())?;
    }
    store.backend.publish(stage, digest, *size, &cancellation)?;
  }
  for (digest, size) in &inventory {
    drop(store.backend.verify(digest, *size)?);
  }
  Ok(artifact_backup_inventory(&inventory))
}

pub fn verify_artifact_backup(
  state_path: impl AsRef<Path>,
  source: impl AsRef<Path>,
) -> Result<ArtifactBackupInventoryV1, CapabilityFailure> {
  DurableEventStore::open(state_path.as_ref()).map_err(|_| backend_unavailable())?;
  let connection = Connection::open_with_flags(
    state_path.as_ref(),
    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
  )
  .map_err(|_| backend_unavailable())?;
  let inventory = artifact_blob_inventory(&connection)?;
  let source = fs::canonicalize(source.as_ref()).map_err(|_| backend_unavailable())?;
  let metadata = fs::symlink_metadata(&source).map_err(|_| backend_unavailable())?;
  if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
    return Err(backend_unavailable());
  }
  for (digest, size) in &inventory {
    verify_materialized_file(
      &source
        .join("blobs/sha256")
        .join(&digest[..2])
        .join(&digest[2..]),
      digest,
      *size,
    )?;
  }
  let expected = inventory
    .iter()
    .map(|(digest, _)| digest.clone())
    .collect::<std::collections::BTreeSet<_>>();
  let mut actual = std::collections::BTreeSet::new();
  let blobs = source.join("blobs/sha256");
  if blobs.exists() {
    for prefix in fs::read_dir(&blobs).map_err(|_| backend_unavailable())? {
      let prefix = prefix.map_err(|_| backend_unavailable())?;
      if !prefix
        .file_type()
        .map_err(|_| backend_unavailable())?
        .is_dir()
      {
        return Err(backend_unavailable());
      }
      for entry in fs::read_dir(prefix.path()).map_err(|_| backend_unavailable())? {
        let entry = entry.map_err(|_| backend_unavailable())?;
        if !entry
          .file_type()
          .map_err(|_| backend_unavailable())?
          .is_file()
        {
          return Err(backend_unavailable());
        }
        actual.insert(format!(
          "{}{}",
          prefix.file_name().to_string_lossy(),
          entry.file_name().to_string_lossy()
        ));
      }
    }
  }
  if actual != expected {
    return Err(artifact_corrupt());
  }
  Ok(artifact_backup_inventory(&inventory))
}

fn artifact_blob_inventory(
  connection: &Connection,
) -> Result<Vec<(String, u64)>, CapabilityFailure> {
  validate_schema(connection)?;
  let mut statement = connection
    .prepare("SELECT sha256, size FROM woml_artifact_blobs ORDER BY sha256")
    .map_err(|_| backend_unavailable())?;
  let inventory = statement
    .query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })
    .map_err(|_| backend_unavailable())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| backend_unavailable())?;
  Ok(inventory)
}

fn artifact_backup_inventory(inventory: &[(String, u64)]) -> ArtifactBackupInventoryV1 {
  let mut digest = Sha256::new();
  digest.update(b"woml.artifact-backup-inventory\0v1\0");
  for (sha256, size) in inventory {
    digest.update((sha256.len() as u64).to_be_bytes());
    digest.update(sha256.as_bytes());
    digest.update(size.to_be_bytes());
  }
  ArtifactBackupInventoryV1 {
    profile: "woml.artifact-backup-inventory/v1".to_string(),
    directory: "artifacts".to_string(),
    blob_count: inventory.len() as u64,
    total_bytes: inventory.iter().map(|(_, size)| *size).sum(),
    inventory_digest: format!("sha256:{}", hex::encode(digest.finalize())),
    verified: true,
  }
}
