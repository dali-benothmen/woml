//! Source containment/identity, link and reparse rejection, bounded source scanning, private directories, permissions, and synchronization.

use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_path_invalid;
use crate::artifacts::artifact_quarantined;
use crate::artifacts::artifact_too_large;
use crate::artifacts::backend_unavailable;
use crate::artifacts::contains_bytes;
use crate::artifacts::materialization_unavailable;
use crate::artifacts::source_changed;
use crate::artifacts::source_denied;
use crate::artifacts::source_type_unsupported;
use crate::artifacts::ArtifactPolicyError;
use crate::artifacts::ArtifactStore;
use crate::artifacts::KnownSecretAction;
use crate::artifacts::PreparedArtifactPolicy;
use crate::artifacts::STREAM_BUFFER_BYTES;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::artifacts) struct SourceIdentity {
  pub(in crate::artifacts) length: u64,
  pub(in crate::artifacts) modified: Option<std::time::SystemTime>,
  #[cfg(unix)]
  pub(in crate::artifacts) device: u64,
  #[cfg(unix)]
  pub(in crate::artifacts) inode: u64,
}

fn authorize_source(
  policy: &PreparedArtifactPolicy,
  authored: &str,
) -> Result<PathBuf, CapabilityFailure> {
  validate_authored_path(authored)?;
  let lexical = policy.project_root.join(authored);
  reject_link_components(&policy.project_root, &lexical)?;
  let canonical = fs::canonicalize(&lexical).map_err(|_| source_denied())?;
  if !policy
    .read_roots
    .iter()
    .any(|root| canonical.starts_with(root))
  {
    return Err(source_denied());
  }
  let metadata = fs::symlink_metadata(&canonical).map_err(|_| source_denied())?;
  if metadata.file_type().is_symlink() || !metadata.is_file() || is_windows_reparse(&metadata) {
    return Err(source_type_unsupported());
  }
  Ok(canonical)
}

pub(in crate::artifacts) fn validate_authored_path(value: &str) -> Result<(), CapabilityFailure> {
  if value.is_empty()
    || value.len() > 4096
    || value.contains('\\')
    || value.chars().any(|c| c.is_control())
    || Path::new(value).is_absolute()
    || value
      .split('/')
      .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    || Path::new(value)
      .components()
      .any(|component| !matches!(component, Component::Normal(_)))
  {
    return Err(artifact_path_invalid());
  }
  Ok(())
}

pub(in crate::artifacts) fn source_identity(
  file: &File,
) -> Result<SourceIdentity, CapabilityFailure> {
  let metadata = file.metadata().map_err(|_| source_changed())?;
  if !metadata.is_file() {
    return Err(source_type_unsupported());
  }
  #[cfg(unix)]
  use std::os::unix::fs::MetadataExt;
  Ok(SourceIdentity {
    length: metadata.len(),
    modified: metadata.modified().ok(),
    #[cfg(unix)]
    device: metadata.dev(),
    #[cfg(unix)]
    inode: metadata.ino(),
  })
}

fn reject_link_components(root: &Path, target: &Path) -> Result<(), CapabilityFailure> {
  let relative = target.strip_prefix(root).map_err(|_| source_denied())?;
  let mut current = root.to_path_buf();
  for component in relative.components() {
    let Component::Normal(component) = component else {
      return Err(artifact_path_invalid());
    };
    current.push(component);
    let metadata = fs::symlink_metadata(&current).map_err(|_| source_denied())?;
    if metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
      return Err(source_denied());
    }
  }
  Ok(())
}

pub(in crate::artifacts) fn cleanup_directory_contents(
  directory: &Path,
) -> Result<(), CapabilityFailure> {
  for entry in fs::read_dir(directory).map_err(|_| materialization_unavailable())? {
    let entry = entry.map_err(|_| materialization_unavailable())?;
    let path = entry.path();
    let metadata = fs::symlink_metadata(&path).map_err(|_| materialization_unavailable())?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
      fs::remove_file(path).map_err(|_| materialization_unavailable())?;
    } else if metadata.is_dir() && !is_windows_reparse(&metadata) {
      fs::remove_dir_all(path).map_err(|_| materialization_unavailable())?;
    } else {
      return Err(materialization_unavailable());
    }
  }
  sync_directory(directory)
}

pub(in crate::artifacts) fn directory_bytes(directory: &Path) -> Result<u64, CapabilityFailure> {
  let mut total = 0_u64;
  for entry in fs::read_dir(directory).map_err(|_| materialization_unavailable())? {
    let entry = entry.map_err(|_| materialization_unavailable())?;
    let metadata = fs::symlink_metadata(entry.path()).map_err(|_| materialization_unavailable())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || is_windows_reparse(&metadata) {
      return Err(materialization_unavailable());
    }
    total = total.saturating_add(metadata.len());
  }
  Ok(total)
}

#[cfg(windows)]
pub(in crate::artifacts) fn is_windows_reparse(metadata: &fs::Metadata) -> bool {
  use std::os::windows::fs::MetadataExt;
  metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
pub(in crate::artifacts) fn is_windows_reparse(_metadata: &fs::Metadata) -> bool {
  false
}

pub(in crate::artifacts) fn canonical_directory(
  path: &Path,
  label: &str,
) -> Result<PathBuf, ArtifactPolicyError> {
  let canonical = fs::canonicalize(path)
    .map_err(|_| ArtifactPolicyError::Invalid(format!("the {label} is missing")))?;
  let metadata = fs::symlink_metadata(&canonical)
    .map_err(|_| ArtifactPolicyError::Invalid(format!("the {label} is unavailable")))?;
  if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
    return Err(ArtifactPolicyError::Invalid(format!(
      "the {label} is not a safe directory"
    )));
  }
  Ok(canonical)
}

pub(in crate::artifacts) fn ensure_private_directory(path: &Path) -> Result<(), CapabilityFailure> {
  if let Ok(metadata) = fs::symlink_metadata(path) {
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
      return Err(backend_unavailable());
    }
  } else {
    fs::create_dir_all(path).map_err(|_| backend_unavailable())?;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
      .map_err(|_| backend_unavailable())?;
  }
  Ok(())
}

#[cfg(unix)]
pub(in crate::artifacts) fn harden_file_permissions(path: &Path) -> Result<(), CapabilityFailure> {
  use std::os::unix::fs::PermissionsExt;
  fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    .map_err(|_| backend_unavailable())?;
  Ok(())
}

#[cfg(not(unix))]
pub(in crate::artifacts) fn harden_file_permissions(_path: &Path) -> Result<(), CapabilityFailure> {
  Ok(())
}

#[cfg(unix)]
pub(in crate::artifacts) fn sync_directory(path: &Path) -> Result<(), CapabilityFailure> {
  File::open(path)
    .and_then(|file| file.sync_all())
    .map_err(|_| backend_unavailable())
}

#[cfg(not(unix))]
pub(in crate::artifacts) fn sync_directory(_path: &Path) -> Result<(), CapabilityFailure> {
  // Windows does not support opening a directory as a regular File for
  // sync_all. File contents are still flushed before atomic publication.
  Ok(())
}

impl ArtifactStore {
  pub(in crate::artifacts) fn read_source(
    &self,
    authored: &str,
    cancellation: &CapabilityCancellationToken,
    known_secrets: &[Vec<u8>],
    mut destination: Option<&mut File>,
  ) -> Result<(SourceIdentity, String, u64), CapabilityFailure> {
    let path = authorize_source(&self.policy, authored)?;
    let mut source = File::open(&path).map_err(|_| source_denied())?;
    let before = source_identity(&source)?;
    if before.length > self.policy.limits.max_artifact_bytes {
      return Err(artifact_too_large());
    }
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
    let overlap_size = known_secrets
      .iter()
      .map(Vec::len)
      .max()
      .unwrap_or(1)
      .saturating_sub(1);
    let mut overlap = Vec::new();
    loop {
      if cancellation.is_cancelled() {
        return Err(artifact_cancelled());
      }
      let read = source.read(&mut buffer).map_err(|_| source_changed())?;
      if read == 0 {
        break;
      }
      size = size.saturating_add(read as u64);
      if size > self.policy.limits.max_artifact_bytes {
        return Err(artifact_too_large());
      }
      let chunk = &buffer[..read];
      if self.policy.secret_action != KnownSecretAction::Allow && !known_secrets.is_empty() {
        let mut scan = overlap.clone();
        scan.extend_from_slice(chunk);
        if known_secrets
          .iter()
          .any(|secret| contains_bytes(&scan, secret))
        {
          return Err(artifact_quarantined());
        }
        let keep = scan.len().min(overlap_size);
        overlap = scan[scan.len() - keep..].to_vec();
      }
      digest.update(chunk);
      if let Some(file) = destination.as_deref_mut() {
        file.write_all(chunk).map_err(|_| backend_unavailable())?;
      }
    }
    let after = source_identity(&source)?;
    if before != after || size != before.length {
      return Err(source_changed());
    }
    reject_link_components(&self.policy.project_root, &path)?;
    if fs::canonicalize(&path).map_err(|_| source_changed())? != path {
      return Err(source_changed());
    }
    Ok((before, hex::encode(digest.finalize()), size))
  }
}
