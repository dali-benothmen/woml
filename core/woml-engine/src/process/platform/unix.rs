//! Unix process-group creation, signaling, liveness, executable-mode checks, and exit-signal decoding.

use std::io;
use tokio::process::Command;

#[cfg(unix)]
pub(in crate::process) fn prepare_process_containment(command: &mut Command) {
  use std::os::unix::process::CommandExt;
  command.as_std_mut().process_group(0);
}

#[cfg(unix)]
pub(in crate::process) fn signal_process_group(
  process_group_id: i32,
  signal: i32,
) -> io::Result<()> {
  // SAFETY: kill is called with a positive, engine-owned process-group id and
  // a platform signal constant. A negative pid targets only that group.
  let result = unsafe { libc::kill(-process_group_id, signal) };
  if result == 0 {
    return Ok(());
  }
  let error = io::Error::last_os_error();
  if error.raw_os_error() == Some(libc::ESRCH) {
    Ok(())
  } else {
    Err(error)
  }
}

#[cfg(unix)]
pub(in crate::process) fn process_group_exists(process_group_id: i32) -> bool {
  // SAFETY: signal 0 performs existence/permission checking without sending a
  // signal. The id was captured from the child created by this handler.
  let result = unsafe { libc::kill(-process_group_id, 0) };
  result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
pub(in crate::process) fn platform_executable(metadata: &std::fs::Metadata) -> bool {
  use std::os::unix::fs::PermissionsExt;
  metadata.permissions().mode() & 0o111 != 0
}

#[cfg(unix)]
pub(in crate::process) fn exit_signal(status: &std::process::ExitStatus) -> Option<String> {
  use std::os::unix::process::ExitStatusExt;
  status.signal().map(|signal| signal.to_string())
}
