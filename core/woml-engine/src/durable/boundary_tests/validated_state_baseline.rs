//! Isolated VSR0 baseline. Production cache/validation paths stay untouched.
use super::TemporaryDatabase;
use crate::durable::{load_events, write_run_summary, DurableEventStore};
use crate::engine::validate_event_history_against_definition;
use crate::projection::ProjectionFoldState;
use crate::{CompiledWorkflowDefinition, RunEvent, RunEventPayload};
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::time::Instant;

const RUN: &str = "run_vsr_baseline";

fn fixture(length: usize) -> (CompiledWorkflowDefinition, String, Vec<RunEvent>) {
  assert!(matches!(length, 128 | 512 | 2048));
  let mut model: Value = serde_json::from_str(super::MODEL).unwrap();
  model["workflowId"] = json!("vsr-baseline");
  let mut template = model["graph"]["nodes"][0].clone();
  template["inputs"] = json!({"kind":"object", "fields":{"source":{"kind":"literal", "value":"return { value: 1 };"}}});
  let nodes: Vec<_> = (0..length / 2)
    .map(|index| {
      let mut node = template.clone();
      node["id"] = json!(format!("node{index}"));
      node
    })
    .collect();
  let edges: Vec<_> = (1..nodes.len()).map(|index| json!({"id":format!("edge{index}"),"from":format!("node{}", index-1),"to":format!("node{index}"),"condition":{"kind":"always"}})).collect();
  model["graph"]["entryNodeIds"] = json!(["node0"]);
  model["graph"]["nodes"] = json!(nodes);
  model["graph"]["edges"] = json!(edges);
  let model: CompiledWorkflowDefinition = serde_json::from_value(model).unwrap();
  model.validate_structure().unwrap();
  let hash = format!(
    "sha256:{:x}",
    Sha256::digest(serde_json_canonicalizer::to_vec(&model).unwrap())
  );
  let mut events = Vec::with_capacity(length);
  for sequence in 1..=length {
    let payload = if sequence == 1 {
      json!({"workflowId":model.workflow_id,"definitionHash":hash,"trigger":{}})
    } else {
      let index = (sequence - 2) / 2;
      if sequence % 2 == 0 {
        json!({"nodeId":format!("node{index}"),"attempt":1,"invocationId":format!("inv_vsr_{index}"),"handler":"runtime.script"})
      } else {
        json!({"nodeId":format!("node{index}"),"attempt":1,"invocationId":format!("inv_vsr_{index}"),"output":{"value":1}})
      }
    };
    events.push(serde_json::from_value(json!({"eventSchemaVersion":1,"eventId":format!("evt_vsr_{sequence}"),"runId":RUN,"sequence":sequence,"occurredAt":"2026-11-03T00:00:00Z","type":if sequence==1 {"run_started"} else if sequence%2==0 {"step_attempt_started"} else {"step_attempt_succeeded"},"data":payload})).unwrap());
  }
  validate_event_history_against_definition(&model, &hash, &events).unwrap();
  (model, hash, events)
}

fn seed(
  database: &TemporaryDatabase,
  model: &CompiledWorkflowDefinition,
  hash: &str,
  events: &[RunEvent],
) {
  let mut store = DurableEventStore::open(&database.0).unwrap();
  store.register_definition(model, hash).unwrap();
  let RunEventPayload::RunStarted(start) = &events[0].payload else {
    panic!("start")
  };
  store
    .start_run(
      &events[0].event_id,
      RUN,
      events[0].occurred_at,
      &start.workflow_id,
      hash,
      start.trigger.clone(),
    )
    .unwrap();
  // Test fixture setup only: avoid O(N) production append validations for every
  // synthetic row. Validate complete inputs above, then derive the real summary
  // and check decoded indexed rows below. No such path ships in production.
  let tx = store.connection.transaction().unwrap();
  for event in &events[1..] {
    tx.execute("INSERT INTO woml_run_events(run_id,sequence,event_id,event_schema_version,event_json) VALUES (?1,?2,?3,?4,?5)", params![RUN,event.sequence,event.event_id,event.event_schema_version,serde_json::to_string(event).unwrap()]).unwrap();
  }
  write_run_summary(&tx, events).unwrap();
  tx.commit().unwrap();
  assert_eq!(load_events(&store.connection, RUN).unwrap(), events);
}

// Thread CPU, not host-tree CPU. These FFI calls have no workflow effect and
// are compiled only into the unit-test binary. Unsupported platforms report
// null instead of substituting wall time for CPU.
#[cfg(unix)]
fn thread_cpu_ms() -> Option<f64> {
  let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
  // SAFETY: clock_gettime writes a timespec to a valid aligned output pointer;
  // inspect the initialized value only on success.
  if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, value.as_mut_ptr()) } != 0 {
    return None;
  }
  let value = unsafe { value.assume_init() };
  Some(value.tv_sec as f64 * 1000.0 + value.tv_nsec as f64 / 1_000_000.0)
}

#[cfg(not(unix))]
fn thread_cpu_ms() -> Option<f64> {
  None
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Value) {
  let cpu = thread_cpu_ms();
  let started = Instant::now();
  let result = operation();
  let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
  let cpu_ms = cpu
    .zip(thread_cpu_ms())
    .map(|(before, after)| after - before);
  (result, json!({"wallMs":wall_ms,"threadCpuMs":cpu_ms}))
}

#[test]
fn generated_histories_are_exact_valid_active_snapshots_and_cache_normally() {
  // The ignored optimized capture checks every long fixture too. Keep routine
  // unit runs small: full graph checking at 2,048 events is expensive in debug.
  for length in [128] {
    let (model, hash, events) = fixture(length);
    assert_eq!(events.len(), length);
    let expected = ProjectionFoldState::from_events(&events).unwrap();
    let database = TemporaryDatabase::new();
    seed(&database, &model, &hash, &events);
    let store = DurableEventStore::open_ready(&database.0).unwrap();
    assert!(!store.projection_cache.borrow().contains_key(RUN));
    assert_eq!(&store.projection(RUN).unwrap(), expected.projection());
    assert!(store.projection_cache.borrow().contains_key(RUN));
    assert_eq!(
      store.projection(RUN).unwrap(),
      store.projection(RUN).unwrap()
    );
  }
}

#[test]
#[ignore = "explicit VSR capture only; build separately and run an exact test binary"]
fn capture() {
  let output = std::env::var_os("WOML_VSR_CAPTURE").expect("WOML_VSR_CAPTURE output file required");
  let mut rows = Vec::new();
  for length in [128, 512, 2048] {
    let (model, hash, events) = fixture(length);
    let fixture_hash = format!(
      "{:x}",
      Sha256::digest(serde_json::to_vec(&(&model, &hash, &events)).unwrap())
    );
    let database = TemporaryDatabase::new();
    seed(&database, &model, &hash, &events);
    let expected = crate::fold_events(&events).unwrap();
    // Two independent connection warmups, then five samples per process.
    // Every measured miss is the FIRST projection on a newly opened connection;
    // no global cache disabling/eviction switch or terminal-history trick.
    for sample in 0..7 {
      let store = DurableEventStore::open_ready(&database.0).unwrap();
      assert!(!store.projection_cache.borrow().contains_key(RUN));
      let (projection, miss) = measure(|| store.projection(RUN).unwrap());
      assert_eq!(projection, expected);
      assert!(store.projection_cache.borrow().contains_key(RUN));
      let mut hits = Vec::new();
      for _ in 0..5 {
        let (projection, timing) = measure(|| store.projection(RUN).unwrap());
        assert_eq!(projection, expected);
        hits.push(timing);
      }
      // Attribution only: one separately measured pure fold, not subtraction
      // of overlapping spans or a replacement for the real cache-miss sample.
      let (_, fold) = measure(|| black_box(ProjectionFoldState::from_events(&events).unwrap()));
      rows.push(json!({"events":length,"fixtureSha256":fixture_hash,"warmup":sample<2,"sample":sample,"miss":miss,"hits":hits,"structuralFold":fold}));
    }
  }
  let report = json!({"profile":"woml.validated-state-baseline/v1","rows":rows,"allocationMeasurement":null,"peakLiveBytes":null,"limitations":["Allocation counts/peak live memory require a separate allocator capture; null is not zero.","Fixtures are synthetic valid Event v1 active snapshots, not an end-to-end workflow or hydration benchmark.","Thread CPU excludes script hosts and other threads; tracing capture is attribution only."]});
  std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
