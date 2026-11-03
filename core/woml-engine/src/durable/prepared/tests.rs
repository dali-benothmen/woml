//! Tests the production factory/transactions, not the SRS1 model.
use super::*;
use crate::event::StepAttemptStartedData;
use crate::{CompiledWorkflowDefinition, RunEventPayload};
use chrono::Duration;
use serde_json::Map;

const MODEL: &str = include_str!("../../../../../woml/tests/fixtures/hello.compiled.v1.json");
const HASH: &str = "sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a";
const RUN: &str = "run_prepared_contract";

struct Directory(PathBuf);
impl Drop for Directory {
  fn drop(&mut self) {
    std::fs::remove_dir_all(&self.0)
      .expect("SQLite connections and file pins must close before cleanup");
  }
}
struct Fixture {
  path: PathBuf,
  store: DurableEventStore,
  owner: PreparedOwner,
  directory: Directory,
}
impl Fixture {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!(
      "woml-prepared-production-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("state.sqlite");
    let mut store = DurableEventStore::open(&path).unwrap();
    store.audit_integrity().unwrap();
    store.recover_interrupted_runs().unwrap();
    let owner = PreparedOwner {
      deployment: "production-test".into(),
      activation: format!("sha256:{}", "a".repeat(64)),
      runtime: "owner-a".into(),
    };
    let now = Utc::now();
    store
      .acquire_runtime_owner(
        &owner.deployment,
        &owner.activation,
        &owner.runtime,
        now,
        now + Duration::minutes(5),
      )
      .unwrap();
    store
      .register_definition(
        &serde_json::from_str::<CompiledWorkflowDefinition>(MODEL).unwrap(),
        HASH,
      )
      .unwrap();
    store
      .start_run("evt_prepared_start", RUN, now, "hello", HASH, Map::new())
      .unwrap();
    Self {
      path,
      store,
      owner,
      directory: Directory(directory),
    }
  }
  fn prepare(&self) -> PreparedRuntime {
    PreparedRuntime::publish(&self.path, &self.store, self.owner.clone(), &self.store).unwrap()
  }
}
fn attempt() -> RunEventPayload {
  RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
    node_id: "a".into(),
    attempt: 1,
    invocation_id: "inv_prepared_a".into(),
    handler: "runtime.script".into(),
    idempotency_key: None,
  })
}
fn open(context: &PreparedRuntime, f: &Fixture) -> DurableEventStore {
  context
    .open(&f.path, PreparedScope::Dispatch, Some(RUN))
    .unwrap()
}
fn counter(value: &AtomicUsize) -> usize {
  value.load(Ordering::Relaxed)
}

#[test]
fn publication_requires_full_startup_and_the_actual_acquisition_connection() {
  let f = Fixture::new();
  let ready = DurableEventStore::open_ready(&f.path).unwrap();
  assert!(PreparedRuntime::publish(&f.path, &ready, f.owner.clone(), &f.store).is_err());
  let other = Fixture::new();
  assert!(PreparedRuntime::publish(&f.path, &f.store, f.owner.clone(), &other.store).is_err());
  let mut wrong = f.owner.clone();
  wrong.activation = format!("sha256:{}", "b".repeat(64));
  assert!(PreparedRuntime::publish(&f.path, &f.store, wrong, &f.store).is_err());
}

#[test]
fn operator_metadata_is_prepared_before_cookie_publication_without_a_backend() {
  let mut f = Fixture::new();
  crate::artifacts::prepare_inspection_schema(&mut f.store.connection).unwrap();
  let context = f.prepare();
  let cookie = context.0.schema_cookie;
  assert!(crate::inspect_run_artifacts(&f.path, RUN)
    .unwrap()
    .artifacts
    .is_empty());
  let store = open(&context, &f);
  assert_eq!(
    store
      .connection
      .query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
      .unwrap(),
    cookie
  );
  assert_eq!(
    store
      .connection
      .query_row(
        "SELECT COUNT(*) FROM woml_artifact_operator_audit",
        [],
        |r| r.get::<_, u64>(0)
      )
      .unwrap(),
    1
  );
  assert_eq!(
    store
      .connection
      .query_row(
        "SELECT COUNT(*) FROM woml_artifact_store_metadata WHERE key='store_id'",
        [],
        |r| r.get::<_, u64>(0)
      )
      .unwrap(),
    0
  );
  assert!(!f.directory.0.join("objects-v1").exists());
  context
    .renew(&f.path, Utc::now(), Utc::now() + Duration::minutes(1))
    .unwrap();
  store
    .connection
    .execute(
      "UPDATE woml_artifact_store_metadata SET value='999' WHERE key='schema_version'",
      [],
    )
    .unwrap();
  assert!(crate::artifacts::prepare_inspection_schema(&mut f.store.connection).is_err());
}

#[test]
fn transaction_checks_reject_owner_transfer_on_an_already_open_connection() {
  let f = Fixture::new();
  let context = f.prepare();
  let mut store = open(&context, &f);
  f.store
    .connection
    .execute(
      "UPDATE woml_runtime_owner SET runtime_instance_id='owner-b'",
      [],
    )
    .unwrap();
  assert!(store
    .append_payload(RUN, "evt_rejected", Utc::now(), attempt())
    .is_err());
  assert_eq!(f.store.events(RUN).unwrap().len(), 1);
  assert!(context.is_revoked());
  assert!(context
    .renew(&f.path, Utc::now(), Utc::now() + Duration::minutes(1))
    .is_err());
  assert!(context.release(&f.path).is_err());
  assert_eq!(
    f.store
      .runtime_owner()
      .unwrap()
      .unwrap()
      .runtime_instance_id,
    "owner-b"
  );
}

#[test]
fn precommit_revocation_rolls_back_events_indexes_and_projection_cache() {
  let f = Fixture::new();
  let context = f.prepare();
  let mut store = open(&context, &f);
  let before = store.projection(RUN).unwrap();
  let revoke = context.clone();
  *context.0.before_commit.lock().unwrap() = Some(Box::new(move || revoke.revoke()));
  assert!(store
    .append_payload(RUN, "evt_rolled_back", Utc::now(), attempt())
    .is_err());
  assert_eq!(store.events(RUN).unwrap().len(), 1);
  assert_eq!(store.projection(RUN).unwrap(), before);
  assert_eq!(f.store.projection(RUN).unwrap(), before);
}

#[test]
fn autocommit_writes_also_recheck_current_ownership() {
  let f = Fixture::new();
  let context = f.prepare();
  let store = open(&context, &f);
  f.store
    .connection
    .execute("DELETE FROM woml_runtime_owner", [])
    .unwrap();
  assert!(store
    .connection
    .execute("DELETE FROM woml_runs WHERE run_id=?1", [RUN])
    .is_err());
  assert_eq!(f.store.events(RUN).unwrap().len(), 1);
}

#[test]
fn validated_policy_and_wait_results_cannot_bypass_precommit_revocation() {
  for wait in [false, true] {
    let mut f = Fixture::new();
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(if wait {
      include_str!("../../../../../woml/tests/fixtures/durable-waits/model.v17.reviewed.json")
    } else {
      include_str!(
        "../../../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
      )
    })
    .unwrap();
    let policy_hash = HASH.replace('9', "8");
    f.store
      .register_definition(&workflow, &policy_hash)
      .unwrap();
    let now = Utc::now();
    let admission = f
      .store
      .admit_trigger_occurrence(crate::TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id,
        definition_hash: policy_hash,
        trigger_id: "start".into(),
        trigger_handler: "trigger.manual".into(),
        source_identity: "vsr-precommit".into(),
        payload: Map::from_iter([("orderId".into(), serde_json::json!("order-1"))]),
        received_at: now,
      })
      .unwrap();
    let run = admission.run_id;
    if wait {
      f.store
        .try_claim_policy_run(&run, "scheduler", now, std::time::Duration::from_secs(30))
        .unwrap();
    }
    let context = f.prepare();
    let mut store = context
      .open(&f.path, PreparedScope::Dispatch, Some(&run))
      .unwrap();
    let before_events = store.events(&run).unwrap();
    let before_projection = store.projection(&run).unwrap();
    let revoke = context.clone();
    *context.0.before_commit.lock().unwrap() = Some(Box::new(move || revoke.revoke()));
    let result = if wait {
      store
        .register_durable_wait_atomically(
          &run,
          "evt_vsr_precommit",
          now,
          crate::WaitRegisteredData {
            wait_id: "pause".into(),
            node_id: "pause".into(),
            kind: crate::DurableWaitKind::Duration,
            registered_at: now,
            deadline: Some(now + Duration::seconds(1)),
            signals: None,
            correlation_digest: None,
          },
        )
        .map(|_| ())
    } else {
      store
        .try_claim_policy_run(&run, "scheduler", now, std::time::Duration::from_secs(30))
        .map(|_| ())
    };
    assert!(result.is_err());
    assert!(context.is_revoked());
    assert_eq!(store.events(&run).unwrap(), before_events);
    assert_eq!(store.projection(&run).unwrap(), before_projection);
    assert_eq!(f.store.projection(&run).unwrap(), before_projection);
    let count: i64 = f
      .store
      .connection
      .query_row(
        if wait {
          "SELECT COUNT(*) FROM woml_durable_waits"
        } else {
          "SELECT COUNT(*) FROM woml_scheduler_claims"
        },
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(count, 0);
  }
}

#[test]
fn expired_owner_refuses_requests_but_exact_renewal_can_restore_eligibility() {
  let f = Fixture::new();
  let context = f.prepare();
  let now = Utc::now();
  f.store
    .connection
    .execute(
      "UPDATE woml_runtime_owner SET lease_expires_at=?1",
      [(now - Duration::seconds(1)).to_rfc3339()],
    )
    .unwrap();
  assert!(context
    .open(&f.path, PreparedScope::Dispatch, None)
    .is_err());
  assert!(!context.is_revoked());
  assert!(context
    .renew(&f.path, now, now + Duration::minutes(1))
    .unwrap());
  assert!(context.open(&f.path, PreparedScope::Dispatch, None).is_ok());
}

#[test]
fn drain_blocks_unbound_transactions_but_preserves_durably_bound_settlement() {
  let f = Fixture::new();
  let context = f.prepare();
  context.bind_run(&f.path, RUN).unwrap();
  let unbound = context
    .open(&f.path, PreparedScope::Dispatch, None)
    .unwrap();
  let mut bound = open(&context, &f);
  context.drain();
  assert!(context.bind_run(&f.path, RUN).is_err());
  assert!(context
    .open(&f.path, PreparedScope::Dispatch, None)
    .is_err());
  assert!(unbound
    .connection
    .execute("DELETE FROM woml_runs WHERE run_id=?1", [RUN])
    .is_err());
  bound
    .append_payload(RUN, "evt_drain_settle", Utc::now(), attempt())
    .unwrap();
  assert_eq!(bound.events(RUN).unwrap().len(), 2);
  context.release(&f.path).unwrap();
  assert!(context.0.pin.lock().unwrap().is_none());
  assert!(f.store.runtime_owner().unwrap().is_none());
  assert!(bound
    .append_payload(RUN, "evt_after_stop", Utc::now(), attempt())
    .is_err());
}

#[test]
fn prepared_scopes_do_no_startup_work_and_do_not_recover_a_live_attempt() {
  let f = Fixture::new();
  let context = f.prepare();
  assert_eq!(counter(&context.0.startup.full_successes), 1);
  assert_eq!(counter(&context.0.startup.general_recovery), 1);
  for scope in [
    PreparedScope::Dispatch,
    PreparedScope::Authority,
    PreparedScope::Continuation,
  ] {
    let store = context.open(&f.path, scope, Some(RUN)).unwrap();
    assert_eq!(store.events(RUN).unwrap().len(), 1);
  }
  context
    .renew(&f.path, Utc::now(), Utc::now() + Duration::minutes(1))
    .unwrap();
  for work in &context.0.work[..4] {
    assert!(counter(&work.prepared_attempts) > 0);
    assert_eq!(counter(&work.full_attempts), 0);
    assert_eq!(counter(&work.full_successes), 0);
    assert_eq!(counter(&work.correction_rebuilds), 0);
    assert_eq!(counter(&work.general_recovery), 0);
  }
  let mut store = open(&context, &f);
  store
    .append_payload(RUN, "evt_live", Utc::now(), attempt())
    .unwrap();
  assert!(
    store.recover_interrupted_runs().is_err(),
    "prepared connection must not run global crash recovery"
  );
  assert_eq!(store.events(RUN).unwrap().len(), 2);
}

#[test]
fn prepared_reads_and_opening_do_not_need_a_startup_writer_lock() {
  let mut f = Fixture::new();
  let context = f.prepare();
  let tx = f
    .store
    .connection
    .transaction_with_behavior(TransactionBehavior::Immediate)
    .unwrap();
  let store = context
    .open(&f.path, PreparedScope::Continuation, Some(RUN))
    .unwrap();
  assert_eq!(store.events(RUN).unwrap().len(), 1);
  drop(tx);
}

#[test]
fn schema_marker_and_schema_cookie_changes_fail_without_repair() {
  for future_marker in [true, false] {
    let f = Fixture::new();
    let context = f.prepare();
    if future_marker {
      f.store
        .connection
        .execute(
          "UPDATE woml_store_metadata SET value='999' WHERE key='schema_version'",
          [],
        )
        .unwrap();
    } else {
      let name: String = f
        .store
        .connection
        .query_row(
          "SELECT name FROM sqlite_master WHERE type='index' AND sql IS NOT NULL LIMIT 1",
          [],
          |r| r.get(0),
        )
        .unwrap();
      f.store
        .connection
        .execute_batch(&format!("DROP INDEX {name}"))
        .unwrap();
    }
    assert!(context
      .open(&f.path, PreparedScope::Dispatch, None)
      .is_err());
    assert!(context.is_revoked());
  }
}

#[test]
fn normal_cross_connection_commits_remain_visible_and_aliases_preserve_identity() {
  let f = Fixture::new();
  let context = f.prepare();
  let mut first = open(&context, &f);
  let second = context
    .open(
      &f.directory.0.join(".").join("state.sqlite"),
      PreparedScope::Dispatch,
      Some(RUN),
    )
    .unwrap();
  second.projection(RUN).unwrap();
  first
    .append_payload(RUN, "evt_visible", Utc::now(), attempt())
    .unwrap();
  assert_eq!(second.events(RUN).unwrap().len(), 2);
  assert_eq!(
    second.projection(RUN).unwrap(),
    first.projection(RUN).unwrap()
  );
}

#[cfg(unix)]
#[test]
fn replacement_even_with_copied_owner_rows_revokes_all_clones_without_creation() {
  let f = Fixture::new();
  let context = f.prepare();
  let clone = context.clone();
  let moved = f.directory.0.join("original.sqlite");
  std::fs::rename(&f.path, &moved).unwrap();
  assert!(context
    .open(&f.path, PreparedScope::Dispatch, None)
    .is_err());
  assert!(!f.path.exists());
  std::fs::copy(&moved, &f.path).unwrap();
  assert!(clone.open(&f.path, PreparedScope::Dispatch, None).is_err());
  assert!(context.is_revoked());
}

#[test]
fn foreign_stores_cannot_reuse_preparation_or_create_a_missing_file() {
  let f = Fixture::new();
  let context = f.prepare();
  let other = Fixture::new();
  assert!(context
    .open(&other.path, PreparedScope::Dispatch, None)
    .is_err());
  let missing = f.directory.0.join("missing.sqlite");
  assert!(context
    .open(&missing, PreparedScope::Dispatch, None)
    .is_err());
  assert!(!missing.exists());
}

#[test]
fn a_committed_admission_can_return_after_drain_without_becoming_new_work() {
  let mut f = Fixture::new();
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/triggers-webhook.compiled.v7.json"
  ))
  .unwrap();
  f.store
    .register_definition(&workflow, &HASH.replace('9', "8"))
    .unwrap();
  let context = f.prepare();
  let mut store = context
    .open(&f.path, PreparedScope::Continuation, None)
    .unwrap();
  let trigger = workflow
    .triggers
    .iter()
    .find(|trigger| trigger.handler == "trigger.webhook")
    .unwrap();
  let request = crate::TriggerAdmissionRequest {
    workflow_id: workflow.workflow_id.clone(),
    definition_hash: HASH.replace('9', "8"),
    trigger_id: trigger.id.clone(),
    trigger_handler: trigger.handler.clone(),
    source_identity: "drain-race".into(),
    payload: Map::new(),
    received_at: Utc::now(),
  };
  let admission = store.admit_trigger_occurrence(request).unwrap();
  assert!(context
    .0
    .pending_dispatch
    .lock()
    .unwrap()
    .contains(&admission.run_id));
  context.drain();
  context.bind_run(&f.path, &admission.run_id).unwrap();
  assert!(!context
    .0
    .pending_dispatch
    .lock()
    .unwrap()
    .contains(&admission.run_id));
  assert!(
    context.bind_run(&f.path, RUN).is_err(),
    "an unrelated historical run has no drain permit"
  );
}

#[test]
fn failed_sql_index_write_does_not_publish_a_tentative_projection() {
  let f = Fixture::new();
  // Install the failure before publication; live DDL would rightly revoke the
  // schema receipt instead of testing the event transaction's SQL rollback.
  f.store.connection.execute_batch("CREATE TRIGGER prepared_fail BEFORE INSERT ON woml_run_events WHEN NEW.sequence=2 BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;").unwrap();
  let context = f.prepare();
  let mut store = open(&context, &f);
  let before = store.projection(RUN).unwrap();
  assert!(store
    .append_payload(RUN, "evt_index_failure", Utc::now(), attempt())
    .is_err());
  assert!(!context.is_revoked());
  assert_eq!(store.events(RUN).unwrap().len(), 1);
  assert_eq!(store.projection(RUN).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn production_vfs_check_detects_aba_and_a_replacement_during_startup() {
  let f = Fixture::new();
  let context = f.prepare();
  let original = f.directory.0.join("original.sqlite");
  let replacement = f.directory.0.join("replacement.sqlite");
  std::fs::rename(&f.path, &original).unwrap();
  std::fs::copy(&original, &f.path).unwrap();
  let wrong = DurableEventStore::open(&f.path).unwrap();
  std::fs::rename(&f.path, &replacement).unwrap();
  std::fs::rename(&original, &f.path).unwrap();
  assert!(context.check(&wrong.connection, false).is_err());
  assert!(PreparedRuntime::publish(&f.path, &wrong, f.owner.clone(), &f.store).is_err());
}

#[test]
fn unsupported_vfs_is_not_a_successful_identity_check() {
  let memory = Connection::open_in_memory().unwrap();
  assert!(verify_native_file(&memory, &FileIdentity(0, 0)).is_err());
}

#[tokio::test]
async fn prepared_engine_preserves_policy_locks_lifecycle_correction_and_cleanup() {
  // The pre-existing trigger-registration gate stops at Model v17. Exercise
  // modern locks/sagas at the actual engine boundary, without widening that
  // gate or pretending these are native trigger-activation coverage.
  for fail in [false, true] {
    let emitted = std::process::Command::new("bun").args(["-e", r#"
      import {compileWoml,parseWoml} from '@woml/compiler';
      import {runtimeProfileFixtures,runtimeProfilePolicy} from './scripts/runtime-profile-fixtures';
      let source=runtimeProfileFixtures().find(f=>f.id==='combined').source;
      if(process.argv.at(-1)==='fail') source=source.replace('<step id="verify"><script>', '<step id="verify"><script>throw new Error("expected correction rehearsal");');
      console.log(JSON.stringify({workflow:compileWoml(parseWoml(source)), policy:runtimeProfilePolicy}));
    "#, if fail { "fail" } else { "success" }])
      .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli"))
      .output().expect("Bun/compiler required for the prepared engine regression");
    assert!(
      emitted.status.success(),
      "{}",
      String::from_utf8_lossy(&emitted.stderr)
    );
    let fixture: serde_json::Value = serde_json::from_slice(&emitted.stdout).unwrap();
    let workflow: CompiledWorkflowDefinition =
      serde_json::from_value(fixture["workflow"].clone()).unwrap();
    assert!(workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V18);
    let policy = crate::prepare_infrastructure_policy(
      serde_json::from_value(fixture["policy"].clone()).unwrap(),
      &crate::InfrastructurePolicyTrustV1::default(),
    )
    .unwrap();
    let mut f = Fixture::new();
    let hash = format!("sha256:{}", "c".repeat(64));
    f.store.register_definition(&workflow, &hash).unwrap();
    let make_options = || {
      crate::RuntimeExecutionOptions::new(
        crate::ScriptHostProcessOptions::new(
          "bun",
          Path::new(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts"),
        ),
        30_000,
      )
      .with_shared_script_hosts()
      .with_infrastructure_policy(policy.clone())
    };
    let reference_options = make_options();
    let reference_path = f.directory.0.join("reference.sqlite");
    let reference = Box::pin(crate::execute_workflow_durable_outcome(
      workflow.clone(),
      hash.clone(),
      Map::new(),
      reference_options.clone(),
      reference_path.clone(),
    ))
    .await;
    assert_eq!(reference.is_err(), fail);
    reference_options.shutdown_shared_script_hosts().await;
    let reference_store = DurableEventStore::open_ready(&reference_path).unwrap();
    let reference_run = reference_store.run_ids().unwrap().pop().unwrap();
    let mut options = make_options();
    options.prepare_owned_state_binding(&f.path).unwrap();
    let context = f.prepare();
    let mut store = context
      .open(&f.path, PreparedScope::Dispatch, None)
      .unwrap();
    let admission = store
      .admit_trigger_occurrence(crate::TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id.clone(),
        definition_hash: hash,
        trigger_id: "start".into(),
        trigger_handler: "trigger.manual".into(),
        source_identity: "prepared-combined".into(),
        payload: Map::new(),
        received_at: Utc::now(),
      })
      .unwrap();
    drop(store);
    options.prepared_runtime = Some(context.clone());
    let outcome = crate::execute_admitted_trigger_run_durable(
      f.path.clone(),
      &admission.run_id,
      options.clone(),
    )
    .await;
    if fail {
      assert!(
        outcome.is_err(),
        "failed business work must not become success"
      );
    } else {
      let crate::WorkflowRuntimeOutcome::Succeeded { execution, .. } = outcome.unwrap() else {
        panic!("combined engine did not complete")
      };
      assert_eq!(execution.result, serde_json::json!({"value":3}));
    }
    options.shutdown_shared_script_hosts().await;
    let events = f.store.events(&admission.run_id).unwrap();
    let facts = events
      .iter()
      .map(|event| {
        serde_json::to_value(event).unwrap()["type"]
          .as_str()
          .unwrap()
          .to_owned()
      })
      .collect::<Vec<_>>();
    assert_eq!(facts.last().unwrap(), "run_finalized");
    let mut expected_facts = reference_store
      .events(&reference_run)
      .unwrap()
      .iter()
      .map(|event| {
        serde_json::to_value(event).unwrap()["type"]
          .as_str()
          .unwrap()
          .to_owned()
      })
      .collect::<Vec<_>>();
    let mut actual_facts = facts.clone();
    expected_facts.sort();
    actual_facts.sort();
    assert_eq!(
      actual_facts, expected_facts,
      "prepared mode must preserve accepted fact classes, including failure/cleanup"
    );
    assert!(facts.iter().any(|fact| fact == "lock_acquired"));
    if !fail {
      assert!(facts.iter().any(|fact| fact == "lock_released"));
    }
    assert!(facts
      .iter()
      .any(|fact| fact == "infrastructure_policy_decision_recorded"));
    if fail {
      assert!(
        facts
          .iter()
          .any(|fact| fact == "correction_attempt_succeeded"),
        "{facts:?}"
      );
    }
    for work in &context.0.work {
      assert_eq!(counter(&work.full_attempts), 0);
      assert_eq!(counter(&work.correction_rebuilds), 0);
      assert_eq!(counter(&work.general_recovery), 0);
    }
    f.store.audit_integrity().unwrap();
    context.release(&f.path).unwrap();
  }
}

#[cfg(windows)]
#[test]
fn windows_pin_prevents_active_database_replacement() {
  let f = Fixture::new();
  let context = f.prepare();
  assert!(std::fs::rename(&f.path, f.directory.0.join("moved.sqlite")).is_err());
  assert!(context.open(&f.path, PreparedScope::Dispatch, None).is_ok());
  context.release(&f.path).unwrap();
  assert!(context.0.pin.lock().unwrap().is_none());
}
