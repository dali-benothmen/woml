//! SRS1 executable design only. No production caller can construct this
//! prototype or use its connection opening. SRS2 must integrate lifecycle and
//! transactional operation guards before enabling any optimized request path.

use super::{DurableEventStore, STORE_SCHEMA_VERSION_V20};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OpenFlags};
use std::{
  fs::{self, File, OpenOptions},
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
  },
};

const ACTIVE: u8 = 0;
const DRAINING: u8 = 1;
const REVOKED: u8 = 2;

#[derive(Debug, PartialEq, Eq)]
enum Rejection {
  Unavailable,
  Identity,
  UnsupportedVfs,
  Incompatible,
  Owner,
  Expired,
  Draining,
  Revoked,
}
type Result<T> = std::result::Result<T, Rejection>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
  deployment: String,
  activation: String,
  runtime: String,
}
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

struct Inner {
  canonical: PathBuf,
  pin: File,
  identity: FileIdentity,
  owner: Owner,
  schema_cookie: i64,
  state: AtomicU8,
}
#[derive(Clone)]
struct ReadinessPrototype(Arc<Inner>);
// This only models lifetime eligibility, NOT permission to perform an effect.
// SRS2 must bind real continuations to their durable run/attempt/claim too.
struct ExistingWorkPermit(Arc<Inner>);

impl ReadinessPrototype {
  fn after_startup(
    path: &Path,
    startup: &DurableEventStore,
    owner: Owner,
    now: DateTime<Utc>,
  ) -> Result<Self> {
    let canonical = path.canonicalize().map_err(|_| Rejection::Unavailable)?;
    let pin = pin_file(&canonical)?;
    let identity = file_identity(&pin)?;
    let schema_cookie = startup
      .connection
      .query_row("PRAGMA schema_version", [], |row| row.get(0))
      .map_err(|_| Rejection::Incompatible)?;
    let prototype = Self(Arc::new(Inner {
      canonical,
      pin,
      identity,
      owner,
      schema_cookie,
      state: AtomicU8::new(ACTIVE),
    }));
    prototype.check(&startup.connection, None, false, now)?;
    Ok(prototype)
  }

  fn check(
    &self,
    connection: &Connection,
    permit: Option<&ExistingWorkPermit>,
    renewal: bool,
    now: DateTime<Utc>,
  ) -> Result<()> {
    match self.0.state.load(Ordering::Acquire) {
      REVOKED => return Err(Rejection::Revoked),
      DRAINING if !renewal && !permit.is_some_and(|permit| Arc::ptr_eq(&permit.0, &self.0)) => {
        return Err(Rejection::Draining)
      }
      _ => {}
    }
    let result = (|| {
      let check_path = || {
        if file_identity(&self.0.pin)? != self.0.identity
          || file_identity(&pin_file(&self.0.canonical)?)? != self.0.identity
        {
          return Err(Rejection::Identity);
        }
        Ok(())
      };
      check_path()?;
      let opened = connection.path().ok_or(Rejection::UnsupportedVfs)?;
      if Path::new(opened)
        .canonicalize()
        .map_err(|_| Rejection::Identity)?
        != self.0.canonical
      {
        return Err(Rejection::Identity);
      }
      verify_native_file(connection, &self.0.identity)?;
      check_path()?;
      let version: String = connection
        .query_row(
          "SELECT value FROM woml_store_metadata WHERE key='schema_version'",
          [],
          |row| row.get(0),
        )
        .map_err(|_| Rejection::Incompatible)?;
      let cookie: i64 = connection
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .map_err(|_| Rejection::Incompatible)?;
      if version != STORE_SCHEMA_VERSION_V20 || cookie != self.0.schema_cookie {
        return Err(Rejection::Incompatible);
      }
      let owner: Option<(String, String, String, String)> = {
        use rusqlite::OptionalExtension;
        connection.query_row("SELECT deployment_id, activation_id, runtime_instance_id, lease_expires_at FROM woml_runtime_owner WHERE singleton=1", [],
          |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
          .optional().map_err(|_| Rejection::Owner)?
      };
      let Some((deployment, activation, runtime, expires)) = owner else {
        return Err(Rejection::Owner);
      };
      if (deployment, activation, runtime)
        != (
          self.0.owner.deployment.clone(),
          self.0.owner.activation.clone(),
          self.0.owner.runtime.clone(),
        )
      {
        return Err(Rejection::Owner);
      }
      let expires = DateTime::parse_from_rfc3339(&expires)
        .map_err(|_| Rejection::Owner)?
        .with_timezone(&Utc);
      // Existing renew_runtime_owner can renew its exact row after expiry.
      // Do not change that contract by rejecting heartbeat eligibility here.
      if !renewal && expires <= now {
        return Err(Rejection::Expired);
      }
      if self.0.state.load(Ordering::Acquire) == REVOKED {
        return Err(Rejection::Revoked);
      }
      Ok(())
    })();
    if result
      .as_ref()
      .is_err_and(|error| !matches!(error, Rejection::Expired))
    {
      self.revoke();
    }
    result
  }

  fn open(
    &self,
    path: &Path,
    permit: Option<&ExistingWorkPermit>,
    renewal: bool,
    now: DateTime<Utc>,
  ) -> Result<Connection> {
    if path.canonicalize().map_err(|_| {
      self.revoke();
      Rejection::Unavailable
    })?
      != self.0.canonical
    {
      return Err(Rejection::Identity);
    }
    let connection = Connection::open_with_flags(
      &self.0.canonical,
      OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| Rejection::Unavailable)?;
    connection
      .busy_timeout(std::time::Duration::from_secs(5))
      .map_err(|_| Rejection::Unavailable)?;
    connection
      .execute_batch("PRAGMA foreign_keys=ON;")
      .map_err(|_| Rejection::Unavailable)?;
    self.check(&connection, permit, renewal, now)?;
    Ok(connection)
  }
  fn admit(&self) -> Result<ExistingWorkPermit> {
    if self.0.state.load(Ordering::Acquire) != ACTIVE {
      return Err(Rejection::Draining);
    }
    Ok(ExistingWorkPermit(Arc::clone(&self.0)))
  }
  fn drain(&self) {
    let _ = self
      .0
      .state
      .compare_exchange(ACTIVE, DRAINING, Ordering::AcqRel, Ordering::Acquire);
  }
  fn revoke(&self) {
    self.0.state.store(REVOKED, Ordering::Release);
  }
}

struct TemporaryDirectory(PathBuf);
impl std::ops::Deref for TemporaryDirectory {
  type Target = Path;
  fn deref(&self) -> &Path {
    &self.0
  }
}
impl Drop for TemporaryDirectory {
  fn drop(&mut self) {
    fs::remove_dir_all(&self.0)
      .expect("all prototype connections and pins must close before cleanup");
  }
}
// Fields drop in declaration order: release SQLite before the directory owner.
struct Fixture {
  path: PathBuf,
  store: DurableEventStore,
  now: DateTime<Utc>,
  owner: Owner,
  directory: TemporaryDirectory,
}
impl Fixture {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!(
      "woml-prepared-contract-{}",
      uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("state.sqlite");
    let mut store = DurableEventStore::open(&path).unwrap();
    store.audit_integrity().unwrap();
    let now = Utc::now();
    let owner = Owner {
      deployment: "contract".into(),
      activation: format!("sha256:{}", "a".repeat(64)),
      runtime: "runtime-a".into(),
    };
    store
      .acquire_runtime_owner(
        &owner.deployment,
        &owner.activation,
        &owner.runtime,
        now,
        now + Duration::seconds(10),
      )
      .unwrap();
    Self {
      directory: TemporaryDirectory(directory),
      path,
      store,
      now,
      owner,
    }
  }
  fn prepare(&self) -> ReadinessPrototype {
    ReadinessPrototype::after_startup(&self.path, &self.store, self.owner.clone(), self.now)
      .unwrap()
  }
}

#[test]
fn construction_requires_exact_owner_and_an_existing_prepared_store() {
  let fixture = Fixture::new();
  let mut wrong = fixture.owner.clone();
  wrong.runtime = "foreign".into();
  assert!(matches!(
    ReadinessPrototype::after_startup(&fixture.path, &fixture.store, wrong, fixture.now),
    Err(Rejection::Owner)
  ));
  let missing = fixture.directory.join("missing.sqlite");
  assert!(matches!(
    ReadinessPrototype::after_startup(&missing, &fixture.store, fixture.owner.clone(), fixture.now),
    Err(Rejection::Unavailable)
  ));
  assert!(!missing.exists());
  let prepared = fixture.prepare();
  assert!(prepared
    .open(
      &fixture.directory.join(".").join("state.sqlite"),
      None,
      false,
      fixture.now
    )
    .is_ok());
  let other = Fixture::new();
  assert!(matches!(
    ReadinessPrototype::after_startup(&other.path, &fixture.store, other.owner.clone(), other.now),
    Err(Rejection::Identity)
  ));
}
#[test]
fn revocation_is_shared_and_drain_distinguishes_already_admitted_work() {
  let fixture = Fixture::new();
  let prepared = fixture.prepare();
  let clone = prepared.clone();
  let permit = prepared.admit().unwrap();
  prepared.drain();
  assert!(matches!(clone.admit(), Err(Rejection::Draining)));
  assert!(matches!(
    clone.open(&fixture.path, None, false, fixture.now),
    Err(Rejection::Draining)
  ));
  assert!(clone
    .open(&fixture.path, Some(&permit), false, fixture.now)
    .is_ok());
  prepared.revoke();
  assert!(matches!(
    clone.open(&fixture.path, Some(&permit), false, fixture.now),
    Err(Rejection::Revoked)
  ));
}
#[test]
fn a_foreign_lifetime_cannot_supply_a_drain_permit_or_store() {
  let first = Fixture::new();
  let second = Fixture::new();
  let prepared = first.prepare();
  let permit = second.prepare().admit().unwrap();
  prepared.drain();
  assert!(matches!(
    prepared.open(&first.path, Some(&permit), false, first.now),
    Err(Rejection::Draining)
  ));
  assert!(matches!(
    prepared.open(&second.path, None, true, first.now),
    Err(Rejection::Identity)
  ));
}
#[test]
fn owner_transfer_after_open_is_detected_on_the_actual_connection() {
  let mut fixture = Fixture::new();
  let prepared = fixture.prepare();
  let connection = prepared
    .open(&fixture.path, None, false, fixture.now)
    .unwrap();
  fixture
    .store
    .release_runtime_owner(&fixture.owner.runtime)
    .unwrap();
  fixture
    .store
    .acquire_runtime_owner(
      "other",
      &fixture.owner.activation,
      "runtime-b",
      fixture.now,
      fixture.now + Duration::seconds(10),
    )
    .unwrap();
  assert_eq!(
    prepared.check(&connection, None, false, fixture.now),
    Err(Rejection::Owner)
  );
  assert!(matches!(
    prepared.open(&fixture.path, None, true, fixture.now),
    Err(Rejection::Revoked)
  ));
}
#[test]
fn expiry_stops_requests_but_preserves_existing_exact_owner_renewal_semantics() {
  let mut fixture = Fixture::new();
  let prepared = fixture.prepare();
  let later = fixture.now + Duration::seconds(11);
  assert!(matches!(
    prepared.open(&fixture.path, None, false, later),
    Err(Rejection::Expired)
  ));
  assert!(prepared.open(&fixture.path, None, true, later).is_ok());
  assert!(fixture
    .store
    .renew_runtime_owner(&fixture.owner.runtime, later, later + Duration::seconds(10))
    .unwrap());
  assert!(prepared.open(&fixture.path, None, false, later).is_ok());
}
#[test]
fn normal_cross_connection_commits_remain_visible_without_schema_invalidation() {
  let fixture = Fixture::new();
  let prepared = fixture.prepare();
  fixture
    .store
    .connection
    .execute(
      "INSERT INTO woml_store_metadata(key,value) VALUES('contract_counter','1')",
      [],
    )
    .unwrap();
  let connection = prepared
    .open(&fixture.path, None, false, fixture.now)
    .unwrap();
  let value: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key='contract_counter'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(value, "1");
}
#[test]
fn prepared_open_does_not_require_startup_writer_work() {
  let fixture = Fixture::new();
  let prepared = fixture.prepare();
  let mut writer = Connection::open(&fixture.path).unwrap();
  let transaction = writer
    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
    .unwrap();
  assert!(prepared
    .open(&fixture.path, None, false, fixture.now)
    .is_ok());
  transaction.rollback().unwrap();
}
#[test]
fn incompatible_changes_are_rejected_without_repair_or_migration() {
  for change in [
    "DROP INDEX woml_run_summaries_updated",
    "UPDATE woml_store_metadata SET value='21' WHERE key='schema_version'",
  ] {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.store.connection.execute_batch(change).unwrap();
    assert!(matches!(
      prepared.open(&fixture.path, None, false, fixture.now),
      Err(Rejection::Incompatible)
    ));
    assert!(matches!(
      prepared.open(&fixture.path, None, true, fixture.now),
      Err(Rejection::Revoked)
    ));
  }
}
#[cfg(unix)]
#[test]
fn identical_owner_and_schema_in_a_replacement_do_not_prove_identity() {
  let fixture = Fixture::new();
  let prepared = fixture.prepare();
  let old = fixture.directory.join("old.sqlite");
  fs::rename(&fixture.path, &old).unwrap();
  fs::copy(&old, &fixture.path).unwrap();
  assert!(matches!(
    prepared.open(&fixture.path, None, false, fixture.now),
    Err(Rejection::Identity)
  ));
}
#[cfg(unix)]
#[test]
fn an_aba_replacement_during_open_is_detected_by_the_sqlite_vfs() {
  let fixture = Fixture::new();
  let prepared = fixture.prepare();
  let old = fixture.directory.join("old.sqlite");
  fs::rename(&fixture.path, &old).unwrap();
  fs::copy(&old, &fixture.path).unwrap();
  let interloper = Connection::open(&fixture.path).unwrap();
  fs::remove_file(&fixture.path).unwrap();
  fs::rename(&old, &fixture.path).unwrap();
  assert_eq!(
    prepared.check(&interloper, None, false, fixture.now),
    Err(Rejection::Identity)
  );
}
#[test]
fn unsupported_vfs_identity_is_never_treated_as_success() {
  let connection = Connection::open_in_memory().unwrap();
  assert_eq!(
    verify_native_file(&connection, &FileIdentity(0, 0)),
    Err(Rejection::UnsupportedVfs)
  );
}
