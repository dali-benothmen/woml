//! Cleared child environment with only the existing Windows kernel-derived SystemRoot bootstrap exception; application environment remains policy-controlled.

#[cfg(windows)]
use crate::process::windows_system_root;
use std::io;
use tokio::process::Command;

pub(in crate::process) fn prepare_process_environment(command: &mut Command) -> io::Result<()> {
  command.env_clear();
  #[cfg(windows)]
  {
    // Windows runtime initialization (including Node's crypto and Winsock)
    // requires SystemRoot. Derive this OS bootstrap value from the kernel,
    // not the ambient environment; application variables remain policy-only.
    command.env("SystemRoot", windows_system_root()?);
  }
  Ok(())
}
