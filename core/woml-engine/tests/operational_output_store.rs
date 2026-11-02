use std::{
  fs,
  path::{Path, PathBuf},
  sync::Arc,
  thread,
  time::Instant,
};

use chrono::{Duration, Utc};
use rusqlite::Connection;
use uuid::Uuid;
use woml_engine::{
  LazyOperationalOutputStore, OperationalAppendOutcomeV1, OperationalBodyV1,
  OperationalCloseStatusV1, OperationalExecutionPhaseV1, OperationalIdentityV1,
  OperationalOutputError, OperationalOutputLimitsV1, OperationalOutputStore,
  OperationalProgressStatusV1, OperationalRecordDraftV1, OperationalSourceV1,
  MAX_OPERATIONAL_PAGE_BYTES, MAX_OPERATIONAL_PAGE_RECORDS,
  OPERATIONAL_OUTPUT_STORE_SCHEMA_VERSION,
};

const CONTRACTS: &str =
  include_str!("../../../woml/tests/fixtures/streaming-output/contracts.v1.json");
const PERFORMANCE_BUDGETS: &str =
  include_str!("../../../examples/streamingOutput/performance-budgets.v1.json");

struct TemporaryStore {
  directory: PathBuf,
  path: PathBuf,
}

impl TemporaryStore {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!(
      "woml-operational-output-{}",
      Uuid::new_v4().simple()
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("output.sqlite");
    Self { directory, path }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryStore {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.directory);
  }
}

fn identity(run_id: &str) -> OperationalIdentityV1 {
  OperationalIdentityV1 {
    workflow_id: "deployment-health".to_string(),
    run_id: run_id.to_string(),
    node_id: Some("inspect".to_string()),
    attempt: Some(1),
    invocation_id: Some(format!("inv-{run_id}")),
    phase: OperationalExecutionPhaseV1::Forward,
  }
}

fn close_run(run_id: &str) -> OperationalRecordDraftV1 {
  OperationalRecordDraftV1 {
    identity: OperationalIdentityV1 {
      workflow_id: "deployment-health".to_string(),
      run_id: run_id.to_string(),
      node_id: None,
      attempt: None,
      invocation_id: None,
      phase: OperationalExecutionPhaseV1::Forward,
    },
    body: OperationalBodyV1::Closed {
      status: OperationalCloseStatusV1::Succeeded,
      reason_code: None,
    },
  }
}

fn output(run_id: &str, text: impl Into<String>) -> OperationalRecordDraftV1 {
  let text = text.into();
  OperationalRecordDraftV1 {
    identity: identity(run_id),
    body: OperationalBodyV1::Output {
      source: OperationalSourceV1::Stdout,
      level: None,
      byte_count: text.len() as u64,
      text,
    },
  }
}

fn close(run_id: &str) -> OperationalRecordDraftV1 {
  close_run(run_id)
}

fn progress(run_id: &str, current: f64) -> OperationalRecordDraftV1 {
  OperationalRecordDraftV1 {
    identity: identity(run_id),
    body: OperationalBodyV1::Progress {
      progress_id: "items".to_string(),
      current: Some(current),
      total: Some(2_000.0),
      unit: Some("items".to_string()),
      message: Some(format!("processed {current}")),
      status: OperationalProgressStatusV1::Active,
    },
  }
}

#[test]
fn lazy_store_has_no_filesystem_cost_until_first_output() {
  let temporary = TemporaryStore::new();
  let lazy = LazyOperationalOutputStore::new(temporary.path(), "deployment-a");
  assert!(!lazy.is_active());
  assert!(!temporary.path().exists());

  let first = lazy.activate().unwrap();
  let second = lazy.activate().unwrap();
  assert!(lazy.is_active());
  assert!(temporary.path().exists());
  assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn state_output_identity_is_stable_before_and_after_first_state_creation() {
  let temporary = TemporaryStore::new();
  // The spelling changes on canonicalization even on Linux, reproducing
  // /var -> /private/var on macOS and verbatim path prefixes on Windows.
  let state = temporary.directory.join(".").join("state.sqlite");
  let pending = woml_engine::operational_output_store_for_state(&state).unwrap();
  assert!(!state.exists());
  assert!(!pending.path().exists());
  fs::write(&state, b"state identity fixture").unwrap();
  let writer = pending.activate().unwrap();
  let record = writer.append(output("run-first", "first output")).unwrap();
  let canonical_state = state.canonicalize().unwrap();
  let reader = woml_engine::operational_output_store_for_state(&canonical_state)
    .unwrap()
    .activate()
    .expect("first-run output must be readable through the canonical state path");
  assert_eq!(writer.deployment_id(), reader.deployment_id());
  assert_eq!(
    reader.read_run_page("run-first", 0, 100).unwrap().records,
    vec![record]
  );
}

#[cfg(unix)]
#[test]
fn first_output_through_an_ancestor_alias_reopens_through_the_canonical_state() {
  use std::os::unix::fs::symlink;
  let temporary = TemporaryStore::new();
  let real = temporary.directory.join("real");
  fs::create_dir_all(real.join("child")).unwrap();
  let alias = temporary.directory.join("alias");
  symlink(&real, &alias).unwrap();
  // Match /var/folders/... versus /private/var/folders/... on macOS: the
  // immediate store parent is a real directory, but an ancestor is aliased.
  let state = alias.join("child").join("custom.sqlite");
  let pending = woml_engine::operational_output_store_for_state(&state).unwrap();
  assert!(!state.exists());
  assert!(!pending.path().exists());
  fs::write(&state, b"state identity fixture").unwrap();
  let writer = pending.activate().unwrap();
  writer.append(output("run-alias", "alias output")).unwrap();
  let reader = woml_engine::operational_output_store_for_state(&state.canonicalize().unwrap())
    .unwrap()
    .activate()
    .unwrap();
  assert_eq!(writer.deployment_id(), reader.deployment_id());
  assert_eq!(
    reader
      .read_run_page("run-alias", 0, 100)
      .unwrap()
      .records
      .len(),
    1
  );
}

#[test]
fn concurrent_appenders_receive_one_strictly_increasing_sequence() {
  let temporary = TemporaryStore::new();
  let store = Arc::new(OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap());
  let mut workers = Vec::new();
  for worker in 0..8 {
    let store = Arc::clone(&store);
    workers.push(thread::spawn(move || {
      let mut sequences = Vec::new();
      for record in 0..25 {
        let appended = store
          .append(output(
            &format!("run-{worker}"),
            format!("worker {worker} record {record}"),
          ))
          .unwrap();
        sequences.push(appended.sequence);
      }
      sequences
    }));
  }
  let mut sequences = workers
    .into_iter()
    .flat_map(|worker| worker.join().unwrap())
    .collect::<Vec<_>>();
  sequences.sort_unstable();
  assert_eq!(sequences, (1..=200).collect::<Vec<_>>());
  let page = store.read_page(0, MAX_OPERATIONAL_PAGE_RECORDS).unwrap();
  assert_eq!(page.records.len(), 200);
  assert_eq!(page.next_after, 200);
}

#[test]
fn restart_preserves_cursor_order_progress_and_deployment_binding() {
  let temporary = TemporaryStore::new();
  {
    let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
    store.append(output("run-a", "starting")).unwrap();
    store
      .append(OperationalRecordDraftV1 {
        identity: identity("run-a"),
        body: OperationalBodyV1::Progress {
          progress_id: "packages".to_string(),
          current: Some(3.0),
          total: Some(10.0),
          unit: Some("packages".to_string()),
          message: None,
          status: OperationalProgressStatusV1::Active,
        },
      })
      .unwrap();
  }

  let reopened = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  assert_eq!(
    reopened
      .append(output("run-a", "resumed"))
      .unwrap()
      .sequence,
    3
  );
  let page = reopened.read_run_page("run-a", 0, 10).unwrap();
  assert_eq!(
    page
      .records
      .iter()
      .map(|record| record.sequence)
      .collect::<Vec<_>>(),
    vec![1, 2, 3]
  );
  let progress = reopened.latest_progress("run-a").unwrap();
  assert_eq!(progress.len(), 1);
  assert_eq!(progress[0].current, Some(3.0));
  assert_eq!(progress[0].status, OperationalProgressStatusV1::Active);
  assert_eq!(reopened.append(close("run-a")).unwrap().sequence, 4);
  assert_eq!(
    reopened.latest_progress("run-a").unwrap()[0].status,
    OperationalProgressStatusV1::Closed
  );
  drop(reopened);

  assert!(matches!(
    OperationalOutputStore::open(temporary.path(), "deployment-b"),
    Err(OperationalOutputError::DeploymentMismatch)
  ));
}

#[test]
fn progress_floods_coalesce_durably_and_the_final_snapshot_is_replayable() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  let first = progress("run-progress", 0.0);
  assert!(!store.coalesce_progress_snapshot(&first).unwrap());
  store.append(first).unwrap();

  for current in 1..1_000 {
    let draft = progress("run-progress", current as f64);
    if !store.coalesce_progress_snapshot(&draft).unwrap() {
      store.append(draft).unwrap();
    }
  }
  let latest = store.latest_progress("run-progress").unwrap();
  assert_eq!(latest.len(), 1);
  assert_eq!(latest[0].current, Some(999.0));

  // The Script Host force-appends its newest accepted lane before closure so
  // cursor replay contains the final value even when intermediate snapshots
  // were projection-only updates.
  store.append(progress("run-progress", 999.0)).unwrap();
  let replay = store.read_run_page("run-progress", 0, 1_000).unwrap();
  assert!(replay.records.len() < 1_000);
  assert!(matches!(
    replay.records.last().map(|record| &record.body),
    Some(OperationalBodyV1::Progress {
      current: Some(999.0),
      ..
    })
  ));
}

#[test]
fn cursor_pages_are_record_and_byte_bounded() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  for index in 0..80 {
    store
      .append(output("run-a", format!("{index}:{}", "x".repeat(15_000))))
      .unwrap();
  }
  let page = store.read_page(0, MAX_OPERATIONAL_PAGE_RECORDS).unwrap();
  assert!(page.records.len() < 80);
  assert!(serde_json::to_vec(&page.records).unwrap().len() <= MAX_OPERATIONAL_PAGE_BYTES + 32_768);
  assert!(page.has_more);
  assert!(matches!(
    store.read_page(0, MAX_OPERATIONAL_PAGE_RECORDS + 1),
    Err(OperationalOutputError::InvalidCursor)
  ));
}

#[test]
fn exact_budgets_emit_one_marker_then_keep_accepting_drained_bytes() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open_with_limits(
    temporary.path(),
    "deployment-a",
    OperationalOutputLimitsV1 {
      max_attempt_bytes: 10,
      max_run_bytes: 15,
      max_store_bytes: 20,
      retention_ms: 60_000,
    },
  )
  .unwrap();
  assert!(matches!(
    store.append_bounded(output("run-a", "12345678")).unwrap(),
    OperationalAppendOutcomeV1::Appended(_)
  ));
  let marker = store.append_bounded(output("run-a", "abcd")).unwrap();
  assert!(matches!(marker, OperationalAppendOutcomeV1::Truncated(_)));
  assert!(matches!(
    store
      .append_bounded(output("run-a", "more dropped bytes"))
      .unwrap(),
    OperationalAppendOutcomeV1::Dropped { .. }
  ));
  let page = store.read_run_page("run-a", 0, 100).unwrap();
  assert_eq!(page.records.len(), 2);
  assert!(matches!(
    &page.records[1].body,
    OperationalBodyV1::Truncated {
      reason,
      dropped_bytes: 4,
      ..
    } if reason == "inline_attempt_budget"
  ));
  assert_eq!(store.inspect().unwrap().retained_output_bytes, 8);
  assert_eq!(store.inspect().unwrap().truncated_streams, 1);
}

#[test]
fn recovery_marks_only_open_attempt_lanes_and_is_idempotent() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  store.append(output("run-open", "before crash")).unwrap();
  store
    .append(output("run-other", "still executing"))
    .unwrap();
  store.append(output("run-closed", "complete")).unwrap();
  store.append(close("run-closed")).unwrap();

  let recovered = store.recover_interrupted_run("run-open").unwrap();
  assert_eq!(recovered.len(), 1);
  assert_eq!(recovered[0].identity.run_id, "run-open");
  assert!(matches!(
    recovered[0].body,
    OperationalBodyV1::Closed {
      status: OperationalCloseStatusV1::Interrupted,
      ..
    }
  ));
  assert!(store
    .recover_interrupted_run("run-open")
    .unwrap()
    .is_empty());
  let remaining = store.recover_interrupted_streams().unwrap();
  assert_eq!(remaining.len(), 1);
  assert_eq!(remaining[0].identity.run_id, "run-other");
}

#[test]
fn retention_plan_matches_prune_and_records_operator_health() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  store.append(output("run-old", "retained")).unwrap();
  store.append(close_run("run-old")).unwrap();
  store.append(output("run-active", "protected")).unwrap();
  let cutoff = Utc::now() + Duration::seconds(1);
  let plan = store.plan_prune_terminal_before(cutoff, 100).unwrap();
  assert_eq!(plan.eligible_runs, 1);
  assert_eq!(plan.eligible_records, 2);
  let result = store.prune_terminal_before(cutoff, 100).unwrap();
  assert_eq!(result.pruned_runs, plan.eligible_runs);
  assert_eq!(result.pruned_records, plan.eligible_records);
  let inspection = store.inspect().unwrap();
  assert_eq!(inspection.active_runs, 1);
  assert!(inspection.last_pruned_at.is_some());
}

#[test]
fn corrective_records_remain_labelled_and_run_isolated() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  let mut corrective = output("run-child", "rollback complete");
  corrective.identity.phase = OperationalExecutionPhaseV1::Corrective;
  store.append(corrective).unwrap();
  store.append(output("run-parent", "parent output")).unwrap();

  let child = store.read_run_page("run-child", 0, 100).unwrap();
  assert_eq!(child.records.len(), 1);
  assert_eq!(
    child.records[0].identity.phase,
    OperationalExecutionPhaseV1::Corrective
  );
  assert!(child
    .records
    .iter()
    .all(|record| record.identity.run_id == "run-child"));
}

#[test]
fn retries_and_control_flow_lanes_keep_exact_attempt_identity() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  for (node, attempt, invocation, text) in [
    ("apply", 1, "inv-apply-1", "attempt one failed"),
    ("apply", 2, "inv-apply-2", "attempt two running"),
    ("items-iteration-3", 1, "inv-item-3", "iteration output"),
    ("fork-production", 1, "inv-fork-production", "branch output"),
    (
      "approval-resume",
      1,
      "inv-approval-resume",
      "resumed output",
    ),
    ("locked-apply", 1, "inv-locked-apply", "lease-owned output"),
  ] {
    let mut draft = output("run-composed", text);
    draft.identity.node_id = Some(node.to_string());
    draft.identity.attempt = Some(attempt);
    draft.identity.invocation_id = Some(invocation.to_string());
    store.append(draft).unwrap();
  }

  let page = store.read_run_page("run-composed", 0, 100).unwrap();
  let identities = page
    .records
    .iter()
    .map(|record| {
      (
        record.identity.node_id.as_deref().unwrap(),
        record.identity.attempt.unwrap(),
        record.identity.invocation_id.as_deref().unwrap(),
      )
    })
    .collect::<Vec<_>>();
  assert_eq!(identities[0], ("apply", 1, "inv-apply-1"));
  assert_eq!(identities[1], ("apply", 2, "inv-apply-2"));
  assert_ne!(identities[0], identities[1]);
  assert_eq!(identities.len(), 6);
}

#[test]
fn retention_protects_active_runs_and_reports_a_stale_cursor_gap() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  store.append(output("active-run", "still running")).unwrap();
  store
    .append(output("terminal-run", "finished output"))
    .unwrap();
  store.append(close("terminal-run")).unwrap();
  assert!(matches!(
    store.append(output("terminal-run", "too late")),
    Err(OperationalOutputError::InvalidRecord(_))
  ));

  let result = store
    .prune_terminal_before(Utc::now() + Duration::seconds(1), 10)
    .unwrap();
  assert_eq!(result.pruned_runs, 1);
  assert_eq!(
    store
      .read_run_page("active-run", 0, 10)
      .unwrap()
      .records
      .len(),
    1
  );
  let pruned = store.read_run_page("terminal-run", 0, 10).unwrap();
  assert!(pruned.records.is_empty());
  assert_eq!(pruned.gap.unwrap().pruned_through, 3);
  assert_eq!(
    store
      .append(output("active-run", "continued"))
      .unwrap()
      .sequence,
    4
  );
}

#[test]
fn future_malformed_corrupt_and_unsafe_stores_fail_closed() {
  let future = TemporaryStore::new();
  {
    let store = OperationalOutputStore::open(future.path(), "deployment-a").unwrap();
    assert_eq!(
      store.inspect().unwrap().schema_version,
      OPERATIONAL_OUTPUT_STORE_SCHEMA_VERSION
    );
  }
  Connection::open(future.path())
    .unwrap()
    .execute(
      "UPDATE woml_operational_metadata SET value = '2' WHERE key = 'schema_version'",
      [],
    )
    .unwrap();
  assert!(matches!(
    OperationalOutputStore::open(future.path(), "deployment-a"),
    Err(OperationalOutputError::UnsupportedVersion(version)) if version == "2"
  ));

  let corrupt = TemporaryStore::new();
  fs::write(corrupt.path(), b"not a sqlite database").unwrap();
  assert!(matches!(
    OperationalOutputStore::open(corrupt.path(), "deployment-a"),
    Err(OperationalOutputError::Corrupt | OperationalOutputError::Unavailable)
  ));

  #[cfg(unix)]
  {
    use std::os::unix::fs::symlink;
    let unsafe_store = TemporaryStore::new();
    let target = unsafe_store.directory.join("target.sqlite");
    fs::write(&target, []).unwrap();
    symlink(&target, unsafe_store.path()).unwrap();
    assert!(matches!(
      OperationalOutputStore::open(unsafe_store.path(), "deployment-a"),
      Err(OperationalOutputError::UnsafePath)
    ));
  }
}

#[test]
fn output_store_contains_no_authoritative_workflow_event_tables() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  store.append(output("run-a", "diagnostic only")).unwrap();
  drop(store);
  let connection = Connection::open(temporary.path()).unwrap();
  let authoritative_table: bool = connection
    .query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'woml_run_events')",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert!(!authoritative_table);
}

#[test]
fn records_reject_hostile_sizes_non_finite_progress_and_forged_gaps() {
  let temporary = TemporaryStore::new();
  let store = OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap();
  assert!(matches!(
    store.append(output("run-a", "x".repeat(16 * 1024 + 1))),
    Err(OperationalOutputError::InvalidRecord(_))
  ));
  assert!(matches!(
    store.append(OperationalRecordDraftV1 {
      identity: identity("run-a"),
      body: OperationalBodyV1::Progress {
        progress_id: "bad".to_string(),
        current: Some(f64::NAN),
        total: None,
        unit: None,
        message: None,
        status: OperationalProgressStatusV1::Active,
      },
    }),
    Err(OperationalOutputError::InvalidRecord(_))
  ));
  assert!(matches!(
    store.append(OperationalRecordDraftV1 {
      identity: identity("run-a"),
      body: OperationalBodyV1::Gap {
        requested_after: 0,
        pruned_through: 3,
        earliest_available: Some(4),
      },
    }),
    Err(OperationalOutputError::InvalidRecord(_))
  ));
}

#[test]
fn reviewed_operational_records_round_trip_in_rust_without_shape_drift() {
  let contracts: serde_json::Value = serde_json::from_str(CONTRACTS).unwrap();
  for name in ["record", "progressRecord"] {
    let record: woml_engine::OperationalRecordV1 =
      serde_json::from_value(contracts[name].clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), contracts[name]);
  }
  let mut unknown = contracts["record"].clone();
  unknown["authorSuppliedIdentity"] = serde_json::json!("forbidden");
  assert!(serde_json::from_value::<woml_engine::OperationalRecordV1>(unknown).is_err());
}

#[test]
fn generated_records_cursors_utf8_and_retention_races_preserve_invariants() {
  let temporary = TemporaryStore::new();
  let store = Arc::new(OperationalOutputStore::open(temporary.path(), "deployment-a").unwrap());
  let mut state = 0x4d59_5df4_d0f3_3173u64;
  let mut expected = Vec::new();
  for index in 0..257u64 {
    state = state
      .wrapping_mul(6_364_136_223_846_793_005)
      .wrapping_add(1_442_695_040_888_963_407);
    let repeat = (state as usize % 127) + 1;
    let text = format!("{index}:{}:{}", "🚀".repeat(repeat % 7), "x".repeat(repeat));
    let record = store.append(output("run-generated", text.clone())).unwrap();
    assert_eq!(record.sequence, index + 1);
    let encoded = serde_json::to_vec(&record).unwrap();
    let decoded: woml_engine::OperationalRecordV1 = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, record);
    expected.push((record.sequence, text));
  }

  let mut after = 0;
  let mut replayed = Vec::new();
  while replayed.len() < expected.len() {
    state = state
      .wrapping_mul(6_364_136_223_846_793_005)
      .wrapping_add(1_442_695_040_888_963_407);
    let limit = (state as usize % 31) + 1;
    let page = store.read_run_page("run-generated", after, limit).unwrap();
    assert!(page.records.len() <= limit);
    assert!(page
      .records
      .windows(2)
      .all(|pair| pair[0].sequence < pair[1].sequence));
    for record in page.records {
      let OperationalBodyV1::Output { text, .. } = record.body else {
        panic!("generated record changed kind");
      };
      replayed.push((record.sequence, text));
      after = record.sequence;
    }
  }
  assert_eq!(replayed, expected);

  for index in 0..32 {
    let run_id = format!("run-terminal-{index}");
    store.append(output(&run_id, "finished")).unwrap();
    store.append(close(&run_id)).unwrap();
  }
  let appender = {
    let store = Arc::clone(&store);
    thread::spawn(move || {
      for index in 0..64 {
        store
          .append(output("run-active-race", format!("active-{index}")))
          .unwrap();
      }
    })
  };
  let pruner = {
    let store = Arc::clone(&store);
    thread::spawn(move || {
      store
        .prune_terminal_before(Utc::now() + Duration::seconds(1), 1_000)
        .unwrap()
    })
  };
  appender.join().unwrap();
  assert_eq!(pruner.join().unwrap().pruned_runs, 32);
  let active = store
    .read_run_page("run-active-race", 0, MAX_OPERATIONAL_PAGE_RECORDS)
    .unwrap();
  assert_eq!(active.records.len(), 64);
  assert!(active.gap.is_none());
}

#[test]
fn publication_budgets_cover_activation_append_replay_progress_concurrency_and_prune() {
  let budgets: serde_json::Value = serde_json::from_str(PERFORMANCE_BUDGETS).unwrap();
  assert_eq!(
    budgets["profile"],
    "woml.streaming-output-performance-budgets/v1"
  );
  let maximum = |name: &str| budgets[name].as_f64().expect("numeric performance budget");
  let temporary = TemporaryStore::new();
  let activation_started = Instant::now();
  let store =
    Arc::new(OperationalOutputStore::open(temporary.path(), "deployment-benchmark").unwrap());
  let activation_ms = activation_started.elapsed().as_secs_f64() * 1_000.0;

  let append_started = Instant::now();
  for index in 0..500 {
    store
      .append(output(
        "run-benchmark",
        format!("{index}:{}", "x".repeat(512)),
      ))
      .unwrap();
  }
  let append_average_micros = append_started.elapsed().as_secs_f64() * 1_000_000.0 / 500.0;

  let replay_started = Instant::now();
  let replay = store
    .read_run_page("run-benchmark", 0, MAX_OPERATIONAL_PAGE_RECORDS)
    .unwrap();
  let replay_page_ms = replay_started.elapsed().as_secs_f64() * 1_000.0;
  assert_eq!(replay.records.len(), 500);

  let progress_started = Instant::now();
  let first = progress("run-progress-benchmark", 0.0);
  store.append(first).unwrap();
  for current in 1..2_000 {
    let draft = progress("run-progress-benchmark", current as f64);
    if !store.coalesce_progress_snapshot(&draft).unwrap() {
      store.append(draft).unwrap();
    }
  }
  let progress_flood_ms = progress_started.elapsed().as_secs_f64() * 1_000.0;
  assert_eq!(
    store.latest_progress("run-progress-benchmark").unwrap()[0].current,
    Some(1_999.0)
  );

  let concurrent_started = Instant::now();
  let mut appenders = Vec::new();
  for worker in 0..4 {
    let store = Arc::clone(&store);
    appenders.push(thread::spawn(move || {
      for index in 0..100 {
        store
          .append(output(
            &format!("run-concurrent-{worker}"),
            format!("record-{index}"),
          ))
          .unwrap();
      }
    }));
  }
  for appender in appenders {
    appender.join().unwrap();
  }
  let concurrent_append_ms = concurrent_started.elapsed().as_secs_f64() * 1_000.0;

  for index in 0..100 {
    let run_id = format!("run-prune-{index}");
    store.append(output(&run_id, "old")).unwrap();
    store.append(close(&run_id)).unwrap();
  }
  let prune_started = Instant::now();
  let pruned = store
    .prune_terminal_before(Utc::now() + Duration::seconds(1), 1_000)
    .unwrap();
  let prune_hundred_runs_ms = prune_started.elapsed().as_secs_f64() * 1_000.0;
  assert_eq!(pruned.pruned_runs, 100);
  let store_bytes = fs::metadata(temporary.path()).unwrap().len() as f64;

  let measurements = [
    ("storeActivationMsMax", activation_ms),
    ("appendAverageMicrosMax", append_average_micros),
    ("replayPageMsMax", replay_page_ms),
    ("progressFloodMsMax", progress_flood_ms),
    ("concurrentAppendMsMax", concurrent_append_ms),
    ("pruneHundredRunsMsMax", prune_hundred_runs_ms),
    ("benchmarkStoreBytesMax", store_bytes),
  ];
  for (name, observed) in measurements {
    assert!(
      observed <= maximum(name),
      "{name} exceeded: {observed:.2} > {:.2}",
      maximum(name)
    );
  }
  println!(
    "WOML_STREAMING_BENCHMARK {}",
    serde_json::json!({
      "profile": "woml.streaming-output-benchmark/v1",
      "storeActivationMs": activation_ms,
      "appendAverageMicros": append_average_micros,
      "replayPageMs": replay_page_ms,
      "progressFloodMs": progress_flood_ms,
      "concurrentAppendMs": concurrent_append_ms,
      "pruneHundredRunsMs": prune_hundred_runs_ms,
      "storeBytes": store_bytes,
    })
  );
}
