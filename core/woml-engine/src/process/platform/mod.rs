//! Private cfg-gated platform operations and existing fallback behavior. Unix and Windows implementations retain their original contracts.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(in crate::process) use unix::exit_signal;
#[cfg(unix)]
pub(in crate::process) use unix::platform_executable;
#[cfg(unix)]
pub(in crate::process) use unix::prepare_process_containment;
#[cfg(unix)]
pub(in crate::process) use unix::process_group_exists;
#[cfg(unix)]
pub(in crate::process) use unix::signal_process_group;
#[cfg(windows)]
pub(in crate::process) use windows::windows_file_identity;
#[cfg(windows)]
pub(in crate::process) use windows::windows_system_root;
#[cfg(windows)]
pub(in crate::process) use windows::WindowsJob;

#[cfg(not(unix))]
use tokio::process::Command;

#[cfg(windows)]
pub(in crate::process) fn prepare_process_containment(_command: &mut Command) {}

#[cfg(not(any(unix, windows)))]
pub(in crate::process) fn prepare_process_containment(_command: &mut Command) {}

#[cfg(not(unix))]
pub(in crate::process) fn platform_executable(_metadata: &std::fs::Metadata) -> bool {
  true
}

#[cfg(not(unix))]
pub(in crate::process) fn exit_signal(_status: &std::process::ExitStatus) -> Option<String> {
  None
}
