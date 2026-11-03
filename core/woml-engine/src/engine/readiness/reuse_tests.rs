//! ISQ2/3 isolated probe. Not an installed-runtime performance certificate.
mod allocations;

#[test]
fn observer_accounts_requested_bytes_and_resets_after_scope() {
  let (bytes, retained) = allocations::measure(|| vec![7u8; 1024]);
  assert_eq!(retained.live, 1024);
  assert_eq!(retained.peak, 1024);
  let (_, released) = allocations::measure(|| drop(bytes));
  assert_eq!(released.live, -1024);
  assert_eq!(allocations::measure(|| ()).1.live, 0);
}

#[test]
fn retained_topology_is_not_cloned_with_definition_and_drops_with_owner() {
  let (model, hash, events) = super::investigation::chain(16);
  let projection = crate::fold_events(&events[..1]).unwrap();
  let (owner, retained) = allocations::measure(|| {
    let owner = super::ReadinessDefinition::new(model.clone());
    drop(
      owner
        .ready_at(&hash, &projection, events[0].occurred_at)
        .unwrap(),
    );
    owner
  });
  assert!(retained.live > 0);
  let (_, warmed) = allocations::measure(|| {
    drop(
      owner
        .ready_at(&hash, &projection, events[0].occurred_at)
        .unwrap(),
    )
  });
  assert_eq!(warmed.live, 0);
  let copy = (*owner).clone();
  assert_eq!(
    serde_json::to_value(copy).unwrap(),
    serde_json::to_value(&model).unwrap()
  );
  let (_, released) = allocations::measure(|| drop(owner));
  assert_eq!(released.live, -retained.live);
}

#[test]
fn durable_engine_refreshes_projection_and_resume_rebuilds_only_topology() {
  use super::incremental::counted;
  use crate::{DurableDagEngine, DurableEventStore};
  let (model, hash, events) = super::investigation::chain(4);
  let mut engine = DurableDagEngine::new_for_event_history(
    model,
    &hash,
    DurableEventStore::open_in_memory().unwrap(),
  )
  .unwrap();
  let run = &events[0].run_id;
  engine
    .start_run(
      "evt_reuse_start",
      run,
      events[0].occurred_at,
      serde_json::Map::new(),
    )
    .unwrap();
  let (_, work) = counted(|| {
    for _ in 0..3 {
      assert_eq!(engine.ready_node_ids(run).unwrap(), vec!["node0"]);
    }
  });
  assert_eq!(work.graph_preparations, 1);
  engine
    .append_payload(
      "evt_reuse_attempt",
      run,
      events[1].occurred_at,
      events[1].payload.clone(),
    )
    .unwrap();
  assert!(engine.ready_node_ids(run).unwrap().is_empty());
  engine
    .append_payload(
      "evt_reuse_done",
      run,
      events[2].occurred_at,
      events[2].payload.clone(),
    )
    .unwrap();
  let (_, work) = counted(|| assert_eq!(engine.ready_node_ids(run).unwrap(), vec!["node1"]));
  assert_eq!(work.graph_preparations, 0);
  let store = engine.into_store();
  let resumed = DurableDagEngine::resume(store, run).unwrap();
  let (_, work) = counted(|| {
    for _ in 0..2 {
      assert_eq!(resumed.ready_node_ids(run).unwrap(), vec!["node1"]);
    }
  });
  assert_eq!(work.graph_preparations, 1);
}

#[test]
fn prepared_topology_observes_external_commits_and_reopen() {
  use crate::{DurableDagEngine, DurableEventStore};
  let directory = std::env::temp_dir().join(format!("woml-isq-topology-{}", uuid::Uuid::new_v4()));
  std::fs::create_dir(&directory).unwrap();
  let database = directory.join("state.sqlite");
  let (model, hash, events) = super::investigation::chain(4);
  let run = &events[0].run_id;
  let mut reader = DurableDagEngine::new_for_event_history(
    model,
    &hash,
    DurableEventStore::open(&database).unwrap(),
  )
  .unwrap();
  reader
    .start_run(
      "evt_isq_disk_start",
      run,
      events[0].occurred_at,
      serde_json::Map::new(),
    )
    .unwrap();
  assert_eq!(reader.ready_node_ids(run).unwrap(), vec!["node0"]);
  let mut writer =
    DurableDagEngine::resume(DurableEventStore::open_ready(&database).unwrap(), run).unwrap();
  writer
    .append_payload(
      "evt_isq_disk_attempt",
      run,
      events[1].occurred_at,
      events[1].payload.clone(),
    )
    .unwrap();
  assert!(reader.ready_node_ids(run).unwrap().is_empty());
  writer
    .append_payload(
      "evt_isq_disk_done",
      run,
      events[2].occurred_at,
      events[2].payload.clone(),
    )
    .unwrap();
  let (_, work) =
    super::incremental::counted(|| assert_eq!(reader.ready_node_ids(run).unwrap(), vec!["node1"]));
  // Reloading an externally committed tail validates its history with one
  // borrowed graph. The live owner's topology is not rebuilt as well.
  assert_eq!(work.graph_preparations, 1);
  let (_, work) =
    super::incremental::counted(|| assert_eq!(reader.ready_node_ids(run).unwrap(), vec!["node1"]));
  assert_eq!(work.graph_preparations, 0);
  drop(writer);
  drop(reader);
  let reopened =
    DurableDagEngine::resume(DurableEventStore::open_ready(&database).unwrap(), run).unwrap();
  let (_, work) = super::incremental::counted(|| {
    assert_eq!(reopened.ready_node_ids(run).unwrap(), vec!["node1"])
  });
  // A new connection first validates history, then prepares its new owner.
  assert_eq!(work.graph_preparations, 2);
  let (_, work) = super::incremental::counted(|| {
    assert_eq!(reopened.ready_node_ids(run).unwrap(), vec!["node1"])
  });
  assert_eq!(work.graph_preparations, 0);
  drop(reopened);
  std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "explicit optimized ISQ reuse capture; build separately"]
fn capture_reuse() {
  use super::investigation::{chain, timed};
  use crate::InMemoryDagEngine;
  use serde_json::json;
  use sha2::{Digest, Sha256};
  assert!(!cfg!(debug_assertions));
  assert!(std::env::var_os("WOML_PROFILE").is_none());
  let output = std::env::var_os("WOML_ISQ_REUSE_CAPTURE").expect("explicit output path");
  let resources = std::env::var_os("WOML_ISQ_REUSE_RESOURCES").is_some();
  let mut rows = Vec::new();
  for nodes in [1, 4, 16, 64, 256, 1024] {
    let (model, hash, events) = chain(nodes);
    crate::engine::validate_event_history_against_definition(&model, &hash, &events).unwrap();
    let fixture = format!(
      "{:x}",
      Sha256::digest(serde_json::to_vec(&(&model, &hash, &events)).unwrap())
    );
    for sample in 0..4 {
      let copy = model.clone();
      let (engine, startup) =
        timed(|| InMemoryDagEngine::new_for_event_history(copy, hash.clone()).unwrap());
      let mut engine = engine;
      // Setup only: use the real store's structural append after independently
      // validating the exact model-bound history. No end-to-end execution claim.
      for event in events.iter().take(1 + 2 * (nodes / 2)) {
        engine.store.append(event.clone()).unwrap();
      }
      let run = &events[0].run_id;
      let expected = vec![format!("node{}", nodes / 2)];
      let (first, first_query, retained) = if resources {
        let (_, allocation) = allocations::measure(|| {
          let result = engine.ready_node_ids(run).unwrap();
          assert_eq!(result, expected);
          drop(result);
        });
        (expected.clone(), serde_json::Value::Null, json!(allocation))
      } else {
        let (result, time) = timed(|| engine.ready_node_ids(run).unwrap());
        (result, time, serde_json::Value::Null)
      };
      assert_eq!(first, expected);
      let (_, warm) = timed(|| {
        for _ in 0..20 {
          std::hint::black_box(engine.ready_node_ids(run).unwrap());
        }
      });
      let (_, cloning) = timed(|| {
        for _ in 0..20 {
          std::hint::black_box(engine.workflow().clone());
        }
      });
      let history = if resources {
        let (_, allocation) = allocations::measure(|| {
          crate::engine::validate_event_history_against_definition(&model, &hash, &events).unwrap()
        });
        json!({"allocation":allocation})
      } else {
        let (result, time) = timed(|| {
          crate::engine::validate_event_history_against_definition(&model, &hash, &events)
        });
        result.unwrap();
        time
      };
      rows.push(json!({"nodes":nodes,"events":events.len(),"sample":sample,"warmup":sample==0,"fixtureSha256":fixture,
        "startup":if resources {serde_json::Value::Null} else {startup},"firstQuery":first_query,
        "warmQueryBatch20":if resources {serde_json::Value::Null} else {warm},"definitionCloneBatch20":if resources {serde_json::Value::Null} else {cloning},
        "history":history,"firstQueryAllocation":retained,"engineStackBytes":std::mem::size_of::<InMemoryDagEngine>()}));
    }
  }
  std::fs::write(output, serde_json::to_vec_pretty(&json!({"profile":"woml.isq-reuse-probe/v1","resources":resources,"rows":rows,"releaseAccepted":false})).unwrap()).unwrap();
}
