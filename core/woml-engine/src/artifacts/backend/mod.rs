//! Existing verified-byte backend interface, staged files, and temporary verified-blob cleanup. Backends never grant run access.

mod local;
mod s3;

pub(in crate::artifacts) use local::LocalArtifactBackend;
pub(in crate::artifacts) use s3::S3ArtifactBackend;

use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use rusqlite::Connection;
use std::fs;
use std::fs::File;
use std::path::PathBuf;

pub(in crate::artifacts) const STREAM_BUFFER_BYTES: usize = 64 * 1024;

/// Rust-owned backend boundary. Logical identity and authorization never live
/// in implementations of this trait; they remain in SQLite.
pub(in crate::artifacts) trait ArtifactBackend: Send + Sync {
  fn identity(&self) -> &str;
  fn bind_store(&self, connection: &Connection, store_id: &str) -> Result<(), CapabilityFailure>;
  fn begin_stage(&self) -> Result<ArtifactStage, CapabilityFailure>;
  fn publish(
    &self,
    stage: ArtifactStage,
    sha256: &str,
    size: u64,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<(), CapabilityFailure>;
  fn cleanup_startup(&self, connection: &Connection) -> Result<(), CapabilityFailure>;
  fn verify(&self, sha256: &str, size: u64) -> Result<VerifiedArtifactBlob, CapabilityFailure>;
  fn delete(&self, sha256: &str) -> Result<(), CapabilityFailure>;
  fn inventory(&self, limit: usize) -> Result<Vec<String>, CapabilityFailure>;
}

#[derive(Debug)]
pub(in crate::artifacts) struct VerifiedArtifactBlob {
  pub(in crate::artifacts) path: PathBuf,
  pub(in crate::artifacts) temporary: bool,
}

impl Drop for VerifiedArtifactBlob {
  fn drop(&mut self) {
    if self.temporary {
      let _ = fs::remove_file(&self.path);
    }
  }
}

#[derive(Debug)]
pub(in crate::artifacts) struct ArtifactStage {
  pub(in crate::artifacts) path: PathBuf,
  pub(in crate::artifacts) file: Option<File>,
}

impl Drop for ArtifactStage {
  fn drop(&mut self) {
    self.file.take();
    let _ = fs::remove_file(&self.path);
  }
}
