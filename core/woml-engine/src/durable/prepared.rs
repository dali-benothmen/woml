//! Owned, non-creating request connections. Readiness is not effect authority.
//! The writer lock covers owner validation and SQL through the pre-commit check.

use super::{DurableEventStore, DurableStoreError, STORE_SCHEMA_VERSION_V20};
use chrono::{DateTime, Utc};
use rusqlite::{
  Connection, OpenFlags, OptionalExtension, Params, Transaction, TransactionBehavior,
};
use std::{
  cell::RefCell,
  collections::{HashMap, HashSet},
  fs::{File, OpenOptions},
  ops::{Deref, DerefMut},
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicU8, AtomicUsize, Ordering},
    Arc, Mutex,
  },
};

const ACTIVE: u8 = 0;
const DRAINING: u8 = 1;
const REVOKED: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedOwner {
  pub(crate) deployment: String,
  pub(crate) activation: String,
  pub(crate) runtime: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Rejection {
  Unavailable,
  Identity,
  UnsupportedVfs,
}
type Result<T> = std::result::Result<T, Rejection>;

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity(u64, u64);

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity> {
  use std::os::unix::fs::MetadataExt;
  let metadata = file.metadata().map_err(|_| Rejection::Unavailable)?;
  Ok(FileIdentity(metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn handle_identity(handle: windows_sys::Win32::Foundation::HANDLE) -> Result<FileIdentity> {
  use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
  };
  // SAFETY: handle is borrowed from a live File or SQLite connection. Neither
  // this function nor its caller transfers or closes that owner's handle.
  let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
  if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
    return Err(Rejection::Unavailable);
  }
  Ok(FileIdentity(
    u64::from(info.dwVolumeSerialNumber),
    (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
  ))
}
#[cfg(windows)]
fn file_identity(file: &File) -> Result<FileIdentity> {
  use std::os::windows::io::AsRawHandle;
  handle_identity(file.as_raw_handle())
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> Result<FileIdentity> {
  Err(Rejection::UnsupportedVfs)
}

fn pin_file(path: &Path) -> Result<File> {
  let mut options = OpenOptions::new();
  options.read(true);
  #[cfg(windows)]
  {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    // Permit ordinary SQLite traffic but not deletion/replacement of the pin.
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
  }
  options.open(path).map_err(|_| Rejection::Unavailable)
}

fn verify_native_file(connection: &Connection, expected: &FileIdentity) -> Result<()> {
  #[cfg(unix)]
  {
    let _ = expected; // compared with the pinned pathname before and after this call
    let mut moved: std::ffi::c_int = 0;
    // SAFETY: SQLite owns a live connection; the opcode expects an int pointer
    // and does not retain it. Inspect the actual VFS handle, not db_filename.
    let code = unsafe {
      rusqlite::ffi::sqlite3_file_control(
        connection.handle(),
        c"main".as_ptr(),
        rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
        (&mut moved as *mut std::ffi::c_int).cast(),
      )
    };
    if code != rusqlite::ffi::SQLITE_OK {
      return Err(Rejection::UnsupportedVfs);
    }
    if moved != 0 {
      return Err(Rejection::Identity);
    }
    Ok(())
  }
  #[cfg(windows)]
  {
    let mut handle: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();
    // SAFETY: this documented opcode writes a borrowed native handle into the
    // supplied pointer; it remains owned by the live SQLite connection.
    let code = unsafe {
      rusqlite::ffi::sqlite3_file_control(
        connection.handle(),
        c"main".as_ptr(),
        rusqlite::ffi::SQLITE_FCNTL_WIN32_GET_HANDLE,
        (&mut handle as *mut windows_sys::Win32::Foundation::HANDLE).cast(),
      )
    };
    if code != rusqlite::ffi::SQLITE_OK || handle.is_null() {
      return Err(Rejection::UnsupportedVfs);
    }
    if handle_identity(handle)? != *expected {
      return Err(Rejection::Identity);
    }
    Ok(())
  }
  #[cfg(not(any(unix, windows)))]
  {
    let _ = (connection, expected);
    Err(Rejection::UnsupportedVfs)
  }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PreparedScope {
  Dispatch,
  Authority,
  Continuation,
  Heartbeat,
  Maintenance,
}

/// Lifetime-owned structural evidence, never process-global or thread-local.
#[derive(Debug, Default)]
pub(super) struct StartupWork {
  pub(super) full_attempts: AtomicUsize,
  pub(super) full_successes: AtomicUsize,
  pub(super) correction_rebuilds: AtomicUsize,
  pub(super) general_recovery: AtomicUsize,
  prepared_attempts: AtomicUsize,
}

struct Inner {
  canonical: PathBuf,
  pin: Mutex<Option<File>>,
  identity: FileIdentity,
  owner: PreparedOwner,
  schema_cookie: i64,
  state: AtomicU8,
  startup: Arc<StartupWork>,
  work: [Arc<StartupWork>; 5],
  pending_dispatch: Mutex<HashSet<String>>,
  #[cfg(test)]
  before_commit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[derive(Clone)]
pub(crate) struct PreparedRuntime(Arc<Inner>);

impl std::fmt::Debug for PreparedRuntime {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("PreparedRuntime")
      .field("state", &self.0.state.load(Ordering::Acquire))
      .field(
        "startup_complete",
        &(self.0.startup.full_successes.load(Ordering::Relaxed) > 0),
      )
      .finish_non_exhaustive()
  }
}

fn rejected(reason: &str) -> rusqlite::Error {
  rusqlite::Error::InvalidParameterName(format!("WOML_RUNTIME_PREPARATION_INVALID: {reason}"))
}

impl PreparedRuntime {
  fn check_transaction(
    &self,
    connection: &Connection,
    maintenance: bool,
    run: Option<&str>,
  ) -> rusqlite::Result<()> {
    if self.0.state.load(Ordering::Acquire) == DRAINING && !maintenance && run.is_none() {
      return Err(rejected("admission closed"));
    }
    self.check(connection, maintenance)?;
    if let Some(run_id) = run {
      super::ensure_run_exists(connection, run_id).map_err(|_| rejected("admitted run missing"))?;
    }
    Ok(())
  }
  /// Only the trigger preparation boundary calls this after full open, recovery,
  /// definition validation and module registration. Never inferred from a path.
  pub(crate) fn publish(
    path: &Path,
    store: &DurableEventStore,
    owner: PreparedOwner,
    acquired_store: &DurableEventStore,
  ) -> std::result::Result<Self, DurableStoreError> {
    if store.connection.work.full_successes.load(Ordering::Relaxed) == 0 {
      return Err(rejected("full startup receipt missing").into());
    }
    let canonical = path
      .canonicalize()
      .map_err(|_| rejected("state unavailable"))?;
    let pin = pin_file(&canonical).map_err(|_| rejected("state unavailable"))?;
    let identity = file_identity(&pin).map_err(|_| rejected("unsupported file identity"))?;
    let schema_cookie = store
      .connection
      .query_row("PRAGMA schema_version", [], |r| r.get(0))?;
    let prepared = Self(Arc::new(Inner {
      canonical,
      pin: Mutex::new(Some(pin)),
      identity,
      owner,
      schema_cookie,
      state: AtomicU8::new(ACTIVE),
      startup: Arc::clone(&store.connection.work),
      work: std::array::from_fn(|_| Arc::new(StartupWork::default())),
      pending_dispatch: Mutex::new(HashSet::new()),
      #[cfg(test)]
      before_commit: Mutex::new(None),
    }));
    prepared.check(&store.connection, false)?;
    prepared.check(&acquired_store.connection, false)?;
    Ok(prepared)
  }

  fn identity(&self, connection: &Connection) -> rusqlite::Result<()> {
    let path_identity = || -> rusqlite::Result<()> {
      let pin = self.0.pin.lock().map_err(|_| rejected("pin unavailable"))?;
      if pin
        .as_ref()
        .and_then(|file| file_identity(file).ok())
        .as_ref()
        != Some(&self.0.identity)
        || pin_file(&self.0.canonical)
          .and_then(|file| file_identity(&file))
          .ok()
          .as_ref()
          != Some(&self.0.identity)
      {
        return Err(rejected("database replaced or removed"));
      }
      Ok(())
    };
    path_identity()?;
    if connection
      .path()
      .and_then(|p| Path::new(p).canonicalize().ok())
      .as_ref()
      != Some(&self.0.canonical)
    {
      return Err(rejected("foreign database"));
    }
    verify_native_file(connection, &self.0.identity)
      .map_err(|_| rejected("VFS identity unavailable or moved"))?;
    path_identity()?;
    let version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key='schema_version'",
      [],
      |r| r.get(0),
    )?;
    let cookie: i64 = connection.query_row("PRAGMA schema_version", [], |r| r.get(0))?;
    if version != STORE_SCHEMA_VERSION_V20 || cookie != self.0.schema_cookie {
      return Err(rejected("schema changed"));
    }
    Ok(())
  }

  fn check(&self, connection: &Connection, maintenance: bool) -> rusqlite::Result<()> {
    if self.0.state.load(Ordering::Acquire) == REVOKED {
      return Err(rejected("lifetime revoked"));
    }
    let result = (|| {
      self.identity(connection)?;
      let owner: Option<(String, String, String, String)> = connection.query_row(
        "SELECT deployment_id, activation_id, runtime_instance_id, lease_expires_at FROM woml_runtime_owner WHERE singleton=1", [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
      let Some((deployment, activation, runtime, expires)) = owner else {
        return Err(rejected("owner missing"));
      };
      if deployment != self.0.owner.deployment
        || activation != self.0.owner.activation
        || runtime != self.0.owner.runtime
      {
        return Err(rejected("owner transferred"));
      }
      let expires =
        DateTime::parse_from_rfc3339(&expires).map_err(|_| rejected("invalid lease"))?;
      Ok(expires > Utc::now())
    })();
    match result {
      Err(error) => {
        self.revoke();
        Err(error)
      }
      Ok(false) if !maintenance => Err(rejected("owner lease expired")),
      Ok(_) if self.0.state.load(Ordering::Acquire) == REVOKED => Err(rejected("lifetime revoked")),
      Ok(_) => Ok(()),
    }
  }

  pub(crate) fn bind_run(
    &self,
    path: &Path,
    run_id: &str,
  ) -> std::result::Result<(), DurableStoreError> {
    let committed_here = self
      .0
      .pending_dispatch
      .lock()
      .map_err(|_| rejected("admission registry unavailable"))?
      .contains(run_id);
    if self.0.state.load(Ordering::Acquire) != ACTIVE && !committed_here {
      return Err(rejected("admission closed").into());
    }
    let store = self.open(
      path,
      PreparedScope::Dispatch,
      committed_here.then_some(run_id),
    )?;
    store.run_binding(run_id)?;
    self
      .0
      .pending_dispatch
      .lock()
      .map_err(|_| rejected("admission registry unavailable"))?
      .remove(run_id);
    Ok(())
  }

  pub(crate) fn open(
    &self,
    path: &Path,
    scope: PreparedScope,
    run: Option<&str>,
  ) -> std::result::Result<DurableEventStore, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.open_prepared");
    self.0.work[scope as usize]
      .prepared_attempts
      .fetch_add(1, Ordering::Relaxed);
    let maintenance = matches!(scope, PreparedScope::Heartbeat | PreparedScope::Maintenance);
    if self.0.state.load(Ordering::Acquire) == DRAINING && !maintenance && run.is_none() {
      return Err(rejected("admission closed").into());
    }
    if path.canonicalize().ok().as_ref() != Some(&self.0.canonical) {
      self.revoke();
      return Err(rejected("foreign or missing database").into());
    }
    let connection = Connection::open_with_flags(
      &self.0.canonical,
      OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys=ON;")?;
    self.check(&connection, maintenance)?;
    if let Some(run_id) = run {
      super::ensure_run_exists(&connection, run_id)?;
    }
    span.succeed();
    Ok(DurableEventStore {
      connection: StoreConnection {
        connection,
        preparation: Some(self.clone()),
        maintenance,
        bound_run: run.map(str::to_owned),
        work: Arc::clone(&self.0.work[scope as usize]),
      },
      projection_cache: RefCell::new(HashMap::new()),
    })
  }

  pub(crate) fn drain(&self) {
    let _ = self
      .0
      .state
      .compare_exchange(ACTIVE, DRAINING, Ordering::AcqRel, Ordering::Acquire);
  }
  pub(crate) fn revoke(&self) {
    self.0.state.store(REVOKED, Ordering::Release);
    if let Ok(mut pin) = self.0.pin.lock() {
      pin.take();
    }
  }
  pub(crate) fn is_revoked(&self) -> bool {
    self.0.state.load(Ordering::Acquire) == REVOKED
  }
  pub(crate) fn renew(
    &self,
    path: &Path,
    now: DateTime<Utc>,
    expires: DateTime<Utc>,
  ) -> std::result::Result<bool, DurableStoreError> {
    self
      .open(path, PreparedScope::Heartbeat, None)?
      .renew_runtime_owner(&self.0.owner.runtime, now, expires)
  }
  pub(crate) fn release(&self, path: &Path) -> std::result::Result<(), DurableStoreError> {
    let result = (|| {
      let mut store = self.open(path, PreparedScope::Maintenance, None)?;
      // The exact-owner check and deletion share the writer lock. No automatic
      // post-delete owner check: this transaction intentionally removes that row.
      let tx = store
        .connection
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
      self.check(&tx, true)?;
      tx.execute("DELETE FROM woml_runtime_owner WHERE singleton=1 AND deployment_id=?1 AND activation_id=?2 AND runtime_instance_id=?3", rusqlite::params![self.0.owner.deployment, self.0.owner.activation, self.0.owner.runtime])?;
      self.identity(&tx)?;
      tx.commit()?;
      Ok(())
    })();
    self.revoke();
    result
  }
}

/// Private adapter: existing transactions retain their scope/order and only
/// prepared connections gain owner/identity guards. No shared mutable cache.
#[derive(Debug)]
pub(crate) struct StoreConnection {
  connection: Connection,
  preparation: Option<PreparedRuntime>,
  maintenance: bool,
  bound_run: Option<String>,
  pub(super) work: Arc<StartupWork>,
}
impl From<Connection> for StoreConnection {
  fn from(connection: Connection) -> Self {
    Self {
      connection,
      preparation: None,
      maintenance: false,
      bound_run: None,
      work: Arc::new(StartupWork::default()),
    }
  }
}
impl Deref for StoreConnection {
  type Target = Connection;
  fn deref(&self) -> &Connection {
    &self.connection
  }
}
impl DerefMut for StoreConnection {
  fn deref_mut(&mut self) -> &mut Connection {
    &mut self.connection
  }
}

impl StoreConnection {
  pub(super) fn initialized(connection: Connection, work: Arc<StartupWork>) -> Self {
    Self {
      work,
      ..Self::from(connection)
    }
  }
  pub(super) fn general_recovery(&self) -> rusqlite::Result<()> {
    self.work.general_recovery.fetch_add(1, Ordering::Relaxed);
    if self.preparation.is_some() {
      return Err(rejected("general recovery is startup-only"));
    }
    Ok(())
  }
  pub(crate) fn transaction_with_behavior(
    &mut self,
    behavior: TransactionBehavior,
  ) -> rusqlite::Result<StoreTransaction<'_>> {
    let tx = self.connection.transaction_with_behavior(behavior)?;
    if let Some(context) = &self.preparation {
      context.check_transaction(&tx, self.maintenance, self.bound_run.as_deref())?;
    }
    Ok(StoreTransaction {
      tx,
      preparation: self.preparation.clone(),
      maintenance: self.maintenance,
      bound_run: self.bound_run.clone(),
      admissions: RefCell::new(Vec::new()),
    })
  }
  pub(crate) fn execute<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
    if self.preparation.is_none() {
      return self.connection.execute(sql, params);
    }
    let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
    let context = self.preparation.as_ref().expect("checked preparation");
    context.check_transaction(&tx, self.maintenance, self.bound_run.as_deref())?;
    let changed = tx.execute(sql, params)?;
    context.check_transaction(&tx, self.maintenance, self.bound_run.as_deref())?;
    tx.commit()?;
    Ok(changed)
  }
}

pub(crate) struct StoreTransaction<'a> {
  tx: Transaction<'a>,
  preparation: Option<PreparedRuntime>,
  maintenance: bool,
  bound_run: Option<String>,
  admissions: RefCell<Vec<String>>,
}
impl<'a> Deref for StoreTransaction<'a> {
  type Target = Transaction<'a>;
  fn deref(&self) -> &Self::Target {
    &self.tx
  }
}
impl StoreTransaction<'_> {
  pub(in crate::durable) fn admitted(&self, run_id: &str, duplicate: bool) {
    if self.preparation.is_some() && !duplicate {
      self.admissions.borrow_mut().push(run_id.to_owned());
    }
  }
  pub(crate) fn commit(self) -> rusqlite::Result<()> {
    #[cfg(test)]
    if let Some(context) = &self.preparation {
      let hook = context
        .0
        .before_commit
        .lock()
        .expect("test hook lock")
        .take();
      if let Some(hook) = hook {
        hook();
      }
    }
    if let Some(context) = &self.preparation {
      context.check_transaction(&self.tx, self.maintenance, self.bound_run.as_deref())?;
    }
    self.tx.commit()?;
    if let Some(context) = &self.preparation {
      // Publication follows SQL commit. A blocked HTTP admission may return
      // after drain began; it still owns this exact committed dispatch identity.
      context
        .0
        .pending_dispatch
        .lock()
        .map_err(|_| rejected("admission registry unavailable"))?
        .extend(self.admissions.into_inner());
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests;
