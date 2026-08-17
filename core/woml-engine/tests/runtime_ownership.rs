use std::{
  fs,
  path::PathBuf,
  sync::{Arc, Barrier},
  thread,
};

use chrono::{Duration, TimeZone, Utc};
use rusqlite::Connection;
use woml_engine::{DurableEventStore, DurableStoreError, DURABLE_STORE_SCHEMA_VERSION};

struct TestDatabase(PathBuf);

impl TestDatabase {
  fn new(label: &str) -> Self {
    let root = std::env::temp_dir().join(format!(
      "woml-pro3-{label}-{}",
      uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&root).unwrap();
    Self(root.join("state.sqlite"))
  }

  fn path(&self) -> &PathBuf {
    &self.0
  }
}

impl Drop for TestDatabase {
  fn drop(&mut self) {
    if let Some(parent) = self.0.parent() {
      let _ = fs::remove_dir_all(parent);
    }
  }
}

fn activation(seed: char) -> String {
  format!("sha256:{}", seed.to_string().repeat(64))
}

#[test]
fn store_v14_migrates_transactionally_from_v13() {
  let database = TestDatabase::new("migration");
  drop(DurableEventStore::open(database.path()).unwrap());
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute_batch(
      "DROP TABLE woml_runtime_owner;
       DROP TABLE woml_maintenance_lease;
       DROP TABLE woml_last_verified_backup;
       UPDATE woml_store_metadata SET value = '13' WHERE key = 'schema_version';",
    )
    .unwrap();
  drop(connection);

  let store = DurableEventStore::open(database.path()).unwrap();
  store.audit_integrity().unwrap();
  drop(store);
  let connection = Connection::open(database.path()).unwrap();
  let version: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, DURABLE_STORE_SCHEMA_VERSION.to_string());
  assert_eq!(DURABLE_STORE_SCHEMA_VERSION, 14);
}

#[test]
fn failed_v14_migration_leaves_v13_metadata_unchanged() {
  let database = TestDatabase::new("failed-migration");
  drop(DurableEventStore::open(database.path()).unwrap());
  let connection = Connection::open(database.path()).unwrap();
  connection
    .execute_batch(
      "DROP TABLE woml_runtime_owner;
       DROP TABLE woml_maintenance_lease;
       DROP TABLE woml_last_verified_backup;
       UPDATE woml_store_metadata SET value = '13' WHERE key = 'schema_version';
       CREATE TABLE woml_runtime_owner (wrong_column TEXT);",
    )
    .unwrap();
  drop(connection);

  assert!(DurableEventStore::open(database.path()).is_err());
  let connection = Connection::open(database.path()).unwrap();
  let version: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(version, "13");
  let maintenance_exists: bool = connection
    .query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'woml_maintenance_lease')",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert!(!maintenance_exists);
}

#[test]
fn one_live_owner_holds_the_state_boundary_and_releases_it_exactly() {
  let database = TestDatabase::new("one-owner");
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let mut first = DurableEventStore::open(database.path()).unwrap();
  first
    .acquire_runtime_owner(
      "deployment-a",
      &activation('a'),
      "runtime_first",
      now,
      now + Duration::seconds(10),
    )
    .unwrap();

  let mut second = DurableEventStore::open(database.path()).unwrap();
  let error = second
    .acquire_runtime_owner(
      "deployment-a",
      &activation('a'),
      "runtime_second",
      now + Duration::seconds(1),
      now + Duration::seconds(11),
    )
    .unwrap_err();
  assert!(matches!(
    error,
    DurableStoreError::DeploymentRuntimeOwned {
      runtime_instance_id,
      ..
    } if runtime_instance_id == "runtime_first"
  ));
  assert!(!second.release_runtime_owner("runtime_second").unwrap());
  assert!(first
    .renew_runtime_owner(
      "runtime_first",
      now + Duration::seconds(2),
      now + Duration::seconds(12),
    )
    .unwrap());
  assert!(first.release_runtime_owner("runtime_first").unwrap());
  assert!(first.runtime_owner().unwrap().is_none());
}

#[test]
fn expired_owner_is_reclaimed_only_by_the_new_exact_runtime() {
  let database = TestDatabase::new("takeover");
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .acquire_runtime_owner(
      "deployment-a",
      &activation('a'),
      "runtime_crashed",
      now,
      now + Duration::seconds(10),
    )
    .unwrap();
  store.audit_integrity().unwrap();
  store
    .acquire_runtime_owner(
      "deployment-a",
      &activation('b'),
      "runtime_replacement",
      now + Duration::seconds(11),
      now + Duration::seconds(21),
    )
    .unwrap();
  let owner = store.runtime_owner().unwrap().unwrap();
  assert_eq!(owner.runtime_instance_id, "runtime_replacement");
  assert_eq!(owner.activation_id, activation('b'));
  assert!(!store.release_runtime_owner("runtime_crashed").unwrap());
  assert!(store.release_runtime_owner("runtime_replacement").unwrap());
}

#[test]
fn concurrent_acquisition_elects_exactly_one_local_owner() {
  let database = TestDatabase::new("concurrent");
  drop(DurableEventStore::open(database.path()).unwrap());
  let barrier = Arc::new(Barrier::new(2));
  let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
  let handles = ["runtime_a", "runtime_b"].map(|runtime_id| {
    let path = database.path().clone();
    let barrier = Arc::clone(&barrier);
    thread::spawn(move || {
      let mut store = DurableEventStore::open(path).unwrap();
      barrier.wait();
      store.acquire_runtime_owner(
        "deployment-a",
        &activation('c'),
        runtime_id,
        now,
        now + Duration::seconds(10),
      )
    })
  });
  let outcomes = handles.map(|handle| handle.join().unwrap());
  assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
  assert_eq!(
    outcomes
      .iter()
      .filter(|outcome| matches!(
        outcome,
        Err(DurableStoreError::DeploymentRuntimeOwned { .. })
      ))
      .count(),
    1
  );
}
