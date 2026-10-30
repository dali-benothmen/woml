//! Private local content-addressed publication, deduplication, verification, inventory, and store binding.

use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::backend_unavailable;
use crate::artifacts::ensure_private_directory;
use crate::artifacts::is_windows_reparse;
use crate::artifacts::sync_directory;
use crate::artifacts::ArtifactBackend;
use crate::artifacts::ArtifactStage;
use crate::artifacts::VerifiedArtifactBlob;
use crate::artifacts::STREAM_BUFFER_BYTES;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use rusqlite::Connection;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug)]
pub(in crate::artifacts) struct LocalArtifactBackend {
  pub(in crate::artifacts) root: PathBuf,
  pub(in crate::artifacts) staging: PathBuf,
  pub(in crate::artifacts) blobs: PathBuf,
}

impl LocalArtifactBackend {
  pub(in crate::artifacts) fn open(path: &Path) -> Result<Self, CapabilityFailure> {
    ensure_private_directory(path)?;
    let root = fs::canonicalize(path).map_err(|_| backend_unavailable())?;
    let staging = root.join("staging");
    let blobs = root.join("blobs").join("sha256");
    ensure_private_directory(&staging)?;
    ensure_private_directory(&root.join("blobs"))?;
    ensure_private_directory(&blobs)?;
    Ok(Self {
      root,
      staging,
      blobs,
    })
  }

  pub(in crate::artifacts) fn blob_path(&self, digest: &str) -> PathBuf {
    self.blobs.join(&digest[..2]).join(&digest[2..])
  }

  pub(in crate::artifacts) fn verify_blob(
    &self,
    path: &Path,
    expected_digest: &str,
    expected_size: u64,
  ) -> Result<(), CapabilityFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| backend_unavailable())?;
    if metadata.file_type().is_symlink()
      || !metadata.is_file()
      || is_windows_reparse(&metadata)
      || metadata.len() != expected_size
    {
      return Err(artifact_corrupt());
    }
    let mut file = File::open(path).map_err(|_| backend_unavailable())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
    let mut size = 0_u64;
    loop {
      let read = file.read(&mut buffer).map_err(|_| artifact_corrupt())?;
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

  pub(in crate::artifacts) fn bind_store(
    &self,
    connection: &Connection,
    store_id: &str,
  ) -> Result<(), CapabilityFailure> {
    let marker = self.root.join("store-id");
    match fs::read_to_string(&marker) {
      Ok(value) => {
        if value != format!("woml.artifact-store/v1\n{store_id}\n") {
          return Err(backend_unavailable());
        }
        return Ok(());
      }
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
      Err(_) => return Err(backend_unavailable()),
    }
    // A pre-marker DA3 store may be adopted only when every published blob is
    // already reachable from this SQLite authority. This prevents a newly
    // selected state database from claiming or collecting another store.
    for prefix in fs::read_dir(&self.blobs).map_err(|_| backend_unavailable())? {
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
        let digest = format!(
          "{}{}",
          prefix.file_name().to_string_lossy(),
          entry.file_name().to_string_lossy()
        );
        let reachable: bool = connection
          .query_row(
            "SELECT EXISTS(SELECT 1 FROM woml_artifact_blobs WHERE sha256 = ?1)",
            [&digest],
            |row| row.get(0),
          )
          .map_err(|_| backend_unavailable())?;
        if !reachable {
          return Err(backend_unavailable());
        }
      }
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      options.mode(0o600);
    }
    let mut file = options.open(&marker).map_err(|_| backend_unavailable())?;
    file
      .write_all(format!("woml.artifact-store/v1\n{store_id}\n").as_bytes())
      .map_err(|_| backend_unavailable())?;
    file.sync_all().map_err(|_| backend_unavailable())?;
    sync_directory(&self.root)
  }
}

impl ArtifactBackend for LocalArtifactBackend {
  fn identity(&self) -> &str {
    self.root.to_str().unwrap_or("local")
  }

  fn bind_store(&self, connection: &Connection, store_id: &str) -> Result<(), CapabilityFailure> {
    LocalArtifactBackend::bind_store(self, connection, store_id)
  }

  fn begin_stage(&self) -> Result<ArtifactStage, CapabilityFailure> {
    let path = self
      .staging
      .join(format!("{}.part", uuid::Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true).read(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      options.mode(0o600);
    }
    let file = options.open(&path).map_err(|_| backend_unavailable())?;
    Ok(ArtifactStage {
      path,
      file: Some(file),
    })
  }

  fn publish(
    &self,
    mut stage: ArtifactStage,
    sha256: &str,
    size: u64,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<(), CapabilityFailure> {
    if cancellation.is_cancelled() {
      return Err(artifact_cancelled());
    }
    let file = stage.file.take().ok_or_else(backend_unavailable)?;
    file.sync_all().map_err(|_| backend_unavailable())?;
    drop(file);
    let directory = self.blobs.join(&sha256[..2]);
    ensure_private_directory(&directory)?;
    let target = self.blob_path(sha256);
    if target.exists() {
      self.verify_blob(&target, sha256, size)?;
      fs::remove_file(&stage.path).map_err(|_| backend_unavailable())?;
      sync_directory(&directory)?;
      return Ok(());
    }
    match fs::hard_link(&stage.path, &target) {
      Ok(()) => {}
      Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
      Err(_) => return Err(backend_unavailable()),
    }
    self.verify_blob(&target, sha256, size)?;
    fs::remove_file(&stage.path).map_err(|_| backend_unavailable())?;
    sync_directory(&directory)?;
    sync_directory(&self.staging)?;
    Ok(())
  }

  fn cleanup_startup(&self, connection: &Connection) -> Result<(), CapabilityFailure> {
    for entry in fs::read_dir(&self.staging).map_err(|_| backend_unavailable())? {
      let path = entry.map_err(|_| backend_unavailable())?.path();
      let metadata = fs::symlink_metadata(&path).map_err(|_| backend_unavailable())?;
      if metadata.is_file() && !metadata.file_type().is_symlink() {
        fs::remove_file(path).map_err(|_| backend_unavailable())?;
      }
    }
    // Published-but-uncommitted blobs are unreachable and safe to collect.
    for prefix in fs::read_dir(&self.blobs).map_err(|_| backend_unavailable())? {
      let prefix = prefix.map_err(|_| backend_unavailable())?;
      if !prefix
        .file_type()
        .map_err(|_| backend_unavailable())?
        .is_dir()
      {
        continue;
      }
      for entry in fs::read_dir(prefix.path()).map_err(|_| backend_unavailable())? {
        let entry = entry.map_err(|_| backend_unavailable())?;
        let digest = format!(
          "{}{}",
          prefix.file_name().to_string_lossy(),
          entry.file_name().to_string_lossy()
        );
        let reachable: bool = connection
          .query_row(
            "SELECT EXISTS(SELECT 1 FROM woml_artifact_blobs WHERE sha256 = ?1)",
            [&digest],
            |row| row.get(0),
          )
          .map_err(|_| backend_unavailable())?;
        if !reachable {
          let metadata = entry.metadata().map_err(|_| backend_unavailable())?;
          if metadata.is_file() {
            fs::remove_file(entry.path()).map_err(|_| backend_unavailable())?;
          }
        }
      }
    }
    sync_directory(&self.root)
  }

  fn verify(&self, sha256: &str, size: u64) -> Result<VerifiedArtifactBlob, CapabilityFailure> {
    let path = self.blob_path(sha256);
    self.verify_blob(&path, sha256, size)?;
    Ok(VerifiedArtifactBlob {
      path,
      temporary: false,
    })
  }

  fn delete(&self, sha256: &str) -> Result<(), CapabilityFailure> {
    let path = self.blob_path(sha256);
    match fs::remove_file(&path) {
      Ok(()) => {
        if let Some(parent) = path.parent() {
          sync_directory(parent)?;
          if fs::read_dir(parent)
            .map_err(|_| backend_unavailable())?
            .next()
            .is_none()
          {
            let _ = fs::remove_dir(parent);
          }
        }
        Ok(())
      }
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
      Err(_) => Err(backend_unavailable()),
    }
  }

  fn inventory(&self, limit: usize) -> Result<Vec<String>, CapabilityFailure> {
    let mut digests = Vec::new();
    for prefix in fs::read_dir(&self.blobs).map_err(|_| backend_unavailable())? {
      let prefix = prefix.map_err(|_| backend_unavailable())?;
      if !prefix
        .file_type()
        .map_err(|_| backend_unavailable())?
        .is_dir()
      {
        continue;
      }
      let prefix_name = prefix.file_name().to_string_lossy().into_owned();
      for entry in fs::read_dir(prefix.path()).map_err(|_| backend_unavailable())? {
        let entry = entry.map_err(|_| backend_unavailable())?;
        if entry
          .file_type()
          .map_err(|_| backend_unavailable())?
          .is_file()
        {
          let digest = format!("{}{}", prefix_name, entry.file_name().to_string_lossy());
          if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            digests.push(digest);
            if digests.len() >= limit {
              digests.sort();
              return Ok(digests);
            }
          }
        }
      }
    }
    digests.sort();
    Ok(digests)
  }
}
