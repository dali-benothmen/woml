//! Pinned executable identity and replacement checks. Resolution never degrades into a later PATH search.

use crate::process::platform_executable;
use crate::process::rejected;
#[cfg(windows)]
use crate::process::windows_file_identity;
use crate::process::ApprovedExecutable;
use crate::process::ProcessPolicyError;
use crate::CapabilityFailure;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::process) struct ExecutableIdentity {
  pub(in crate::process) length: u64,
  pub(in crate::process) modified: Option<std::time::SystemTime>,
  #[cfg(unix)]
  pub(in crate::process) device: u64,
  #[cfg(unix)]
  pub(in crate::process) inode: u64,
  #[cfg(windows)]
  pub(in crate::process) volume_serial_number: Option<u32>,
  #[cfg(windows)]
  pub(in crate::process) file_index: Option<u64>,
}

pub(in crate::process) fn verify_executable_policy(
  path: &Path,
  alias: &str,
) -> Result<ExecutableIdentity, ProcessPolicyError> {
  let metadata = std::fs::metadata(path).map_err(|_| {
    ProcessPolicyError::Invalid(format!("executable alias '{alias}' is unavailable"))
  })?;
  if !metadata.is_file() || !platform_executable(&metadata) {
    return Err(ProcessPolicyError::Invalid(format!(
      "executable alias '{alias}' does not point to an executable file"
    )));
  }
  executable_identity(path, &metadata).map_err(|_| {
    ProcessPolicyError::Invalid(format!(
      "executable alias '{alias}' identity could not be inspected"
    ))
  })
}

pub(in crate::process) fn verify_executable(
  executable: &ApprovedExecutable,
) -> Result<(), CapabilityFailure> {
  let canonical = std::fs::canonicalize(&executable.path).map_err(|_| {
    rejected(
      "WOML_PROCESS_EXECUTABLE_NOT_FOUND",
      "The approved executable is no longer available.",
    )
  })?;
  let metadata = std::fs::metadata(&canonical).map_err(|_| {
    rejected(
      "WOML_PROCESS_EXECUTABLE_NOT_FOUND",
      "The approved executable is no longer available.",
    )
  })?;
  let identity = executable_identity(&canonical, &metadata).map_err(|_| {
    rejected(
      "WOML_PROCESS_EXECUTABLE_CHANGED",
      "The approved executable identity can no longer be verified.",
    )
  })?;
  if canonical != executable.path
    || !metadata.is_file()
    || !platform_executable(&metadata)
    || identity != executable.identity
  {
    return Err(rejected(
      "WOML_PROCESS_EXECUTABLE_CHANGED",
      "The approved executable changed after runtime preflight.",
    ));
  }
  Ok(())
}

fn executable_identity(
  path: &Path,
  metadata: &std::fs::Metadata,
) -> io::Result<ExecutableIdentity> {
  #[cfg(unix)]
  use std::os::unix::fs::MetadataExt;
  #[cfg(unix)]
  let _ = path;
  #[cfg(windows)]
  let (volume_serial_number, file_index) = windows_file_identity(path)?;
  Ok(ExecutableIdentity {
    length: metadata.len(),
    modified: metadata.modified().ok(),
    #[cfg(unix)]
    device: metadata.dev(),
    #[cfg(unix)]
    inode: metadata.ino(),
    #[cfg(windows)]
    volume_serial_number: Some(volume_serial_number),
    #[cfg(windows)]
    file_index: Some(file_index),
  })
}
