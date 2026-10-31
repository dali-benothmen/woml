//! Owned process-tree containment and Drop cleanup across cancellation, timeout, and leader exit. Stopping a tree does not prove effects never occurred.

#[cfg(unix)]
use crate::process::process_group_exists;
#[cfg(unix)]
use crate::process::signal_process_group;
#[cfg(windows)]
use crate::process::WindowsJob;
#[cfg(unix)]
use crate::process::PROCESS_TERMINATION_GRACE;
use std::io;
use tokio::process::Child;

#[derive(Debug)]
pub(in crate::process) struct ProcessContainment {
  #[cfg(unix)]
  pub(in crate::process) process_group_id: i32,
  #[cfg(windows)]
  pub(in crate::process) job: WindowsJob,
}

impl ProcessContainment {
  pub(in crate::process) fn attach(child: &Child) -> io::Result<Self> {
    #[cfg(unix)]
    {
      let process_group_id = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| io::Error::other("spawned process has no valid process-group id"))?;
      Ok(Self { process_group_id })
    }
    #[cfg(windows)]
    {
      Ok(Self {
        job: WindowsJob::attach(child)?,
      })
    }
    #[cfg(not(any(unix, windows)))]
    {
      let _ = child;
      Ok(Self {})
    }
  }

  pub(in crate::process) fn graceful_terminate(&mut self) -> io::Result<()> {
    #[cfg(unix)]
    {
      signal_process_group(self.process_group_id, libc::SIGTERM)
    }
    #[cfg(windows)]
    {
      // Windows has no portable graceful signal for an arbitrary non-console
      // process. Job termination is therefore the reviewed forced boundary.
      self.job.terminate()
    }
    #[cfg(not(any(unix, windows)))]
    {
      Ok(())
    }
  }

  pub(in crate::process) fn force_terminate(&mut self) -> io::Result<()> {
    #[cfg(unix)]
    {
      signal_process_group(self.process_group_id, libc::SIGKILL)
    }
    #[cfg(windows)]
    {
      self.job.terminate()
    }
    #[cfg(not(any(unix, windows)))]
    {
      Ok(())
    }
  }

  pub(in crate::process) async fn close_remaining_tree(&mut self) {
    #[cfg(unix)]
    {
      if process_group_exists(self.process_group_id) {
        let _ = self.graceful_terminate();
        tokio::time::sleep(PROCESS_TERMINATION_GRACE).await;
        let _ = self.force_terminate();
      }
    }
    #[cfg(windows)]
    {
      // Closing a kill-on-close Job Object is the final descendant boundary.
      let _ = self.job.terminate();
    }
  }
}

impl Drop for ProcessContainment {
  fn drop(&mut self) {
    #[cfg(unix)]
    {
      // A dropped handler future (for example, host shutdown) must not bypass
      // containment cleanup. SIGKILL is the only synchronous fail-closed
      // action available during Drop.
      let _ = signal_process_group(self.process_group_id, libc::SIGKILL);
    }
    // On Windows, dropping the WindowsJob field closes its kill-on-close job.
  }
}
