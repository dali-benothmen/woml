//! Windows Job Object ownership, kill-on-close/termination, trusted system directory, and pinned file identity. The owned handle is closed once.

use std::io;
use std::path::Path;
use tokio::process::Child;

#[cfg(windows)]
pub(in crate::process) fn windows_system_root() -> io::Result<std::ffi::OsString> {
  use std::os::windows::ffi::OsStringExt;
  let mut buffer = vec![0u16; 32_768];
  // SAFETY: buffer is writable for the supplied number of UTF-16 code units.
  let length = unsafe {
    windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW(
      buffer.as_mut_ptr(),
      buffer.len() as u32,
    )
  } as usize;
  if length == 0 {
    return Err(io::Error::last_os_error());
  }
  if length >= buffer.len() {
    return Err(io::Error::new(
      io::ErrorKind::InvalidData,
      "the Windows system directory exceeds the supported path bound",
    ));
  }
  Ok(std::ffi::OsString::from_wide(&buffer[..length]))
}

#[cfg(windows)]
#[derive(Debug)]
pub(in crate::process) struct WindowsJob {
  pub(in crate::process) handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsJob {
  pub(in crate::process) fn attach(child: &Child) -> io::Result<Self> {
    use std::{mem::size_of, ptr};
    use windows_sys::Win32::{
      Foundation::CloseHandle,
      System::JobObjects::{
        AssignProcessToJobObject, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
      },
    };
    // SAFETY: every handle is checked, the info pointer and byte size match,
    // and ownership of the created job is retained by WindowsJob.
    unsafe {
      let child_handle = child.raw_handle().ok_or_else(|| {
        io::Error::new(
          io::ErrorKind::InvalidInput,
          "the child process exited before containment was attached",
        )
      })?;
      let handle =
        windows_sys::Win32::System::JobObjects::CreateJobObjectW(ptr::null(), ptr::null());
      if handle.is_null() {
        return Err(io::Error::last_os_error());
      }
      let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
      information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
      if SetInformationJobObject(
        handle,
        JobObjectExtendedLimitInformation,
        &information as *const _ as *const _,
        size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
      ) == 0
        || AssignProcessToJobObject(handle, child_handle as _) == 0
      {
        let error = io::Error::last_os_error();
        CloseHandle(handle);
        return Err(error);
      }
      Ok(Self { handle })
    }
  }

  pub(in crate::process) fn terminate(&self) -> io::Result<()> {
    // SAFETY: handle is a live Job Object owned by self.
    let result =
      unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle, 1) };
    if result == 0 {
      Err(io::Error::last_os_error())
    } else {
      Ok(())
    }
  }
}

#[cfg(windows)]
// SAFETY: WindowsJob exclusively owns a kernel handle. Moving that owned
// handle between executor threads does not change its validity or ownership.
unsafe impl Send for WindowsJob {}

#[cfg(windows)]
impl Drop for WindowsJob {
  pub(in crate::process) fn drop(&mut self) {
    // SAFETY: handle is owned by self and closed exactly once.
    unsafe {
      windows_sys::Win32::Foundation::CloseHandle(self.handle);
    }
  }
}

#[cfg(windows)]
pub(in crate::process) fn windows_file_identity(path: &Path) -> io::Result<(u32, u64)> {
  use std::os::windows::io::AsRawHandle;
  use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
  };

  let file = std::fs::File::open(path)?;
  let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
  // SAFETY: file owns a valid readable handle for the duration of the call and
  // information points to the correctly sized output structure.
  if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
    return Err(io::Error::last_os_error());
  }
  let file_index = ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64;
  Ok((information.dwVolumeSerialNumber, file_index))
}
