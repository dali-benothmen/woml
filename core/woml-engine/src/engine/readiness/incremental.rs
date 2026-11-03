//! Test-only ISQ capture/work counters. Never a scheduler or durable cache.

use super::ReadinessGraph;
use crate::{RunProjection, RunStatus};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Work {
  pub(super) prefix_attempt_visits: usize,
  pub(super) graph_preparations: usize,
  pub(super) active_node_visits: usize,
  pub(super) candidate_checks: usize,
  pub(super) attempt_visits: usize,
  pub(super) ready_vectors: usize,
}

thread_local! {
  static WORK: Cell<Option<Work>> = const { Cell::new(None) };
  static FULL_MEMBERSHIP: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn full_membership_mode() -> bool {
  FULL_MEMBERSHIP.with(Cell::get)
}

pub(super) fn note(change: impl FnOnce(&mut Work)) {
  WORK.with(|cell| {
    if let Some(mut work) = cell.get() {
      change(&mut work);
      cell.set(Some(work));
    }
  });
}

pub(super) fn counted<T>(operation: impl FnOnce() -> T) -> (T, Work) {
  struct Reset;
  impl Drop for Reset {
    fn drop(&mut self) {
      WORK.with(|cell| cell.set(None));
    }
  }
  WORK.with(|cell| assert!(cell.replace(Some(Work::default())).is_none()));
  let _reset = Reset;
  let value = operation();
  (value, WORK.with(|cell| cell.get().unwrap()))
}

fn membership(
  graph: &ReadinessGraph<'_>,
  hash: &str,
  projection: &RunProjection,
  now: DateTime<Utc>,
  node: &str,
) -> Result<bool, String> {
  graph.is_ready_at(hash, projection, now, node)
}

#[test]
fn baseline_work_counts_observe_actual_queries_without_retaining_state() {
  let (model, hash, events) = super::investigation::chain(16);
  let projection = crate::fold_events(&events[..17]).unwrap();
  let ((result, _graph), work) = counted(|| {
    let graph = ReadinessGraph::new(&model);
    (
      graph.ready_at(&hash, &projection, events[0].occurred_at),
      graph,
    )
  });
  assert_eq!(result.unwrap(), vec!["node8"]);
  assert_eq!(work.graph_preparations, 1);
  assert_eq!(work.active_node_visits, 16);
  assert_eq!(work.candidate_checks, 16);
  assert_eq!(work.attempt_visits, projection.attempts.len());
  assert_eq!(work.ready_vectors, 1);
  let mut terminal = projection.clone();
  terminal.status = RunStatus::Cancelled;
  let (_, work) = counted(|| {
    super::ready_node_ids_for_projection_at(&model, "wrong", &terminal, events[0].occurred_at)
      .unwrap()
  });
  assert_eq!(work, Work::default());
}

#[test]
fn membership_skips_unrelated_predicates_and_ready_vector_but_not_dynamic_views() {
  let (model, hash, events) = super::investigation::chain(16);
  let projection = crate::fold_events(&events[..17]).unwrap();
  let graph = ReadinessGraph::new(&model);
  for (id, expected) in [("node8", true), ("node9", false), ("missing", false)] {
    let (result, work) =
      counted(|| membership(&graph, &hash, &projection, events[0].occurred_at, id));
    assert_eq!(result, Ok(expected));
    assert_eq!(work.candidate_checks, usize::from(id != "missing"));
    assert_eq!(work.active_node_visits, 16);
    assert_eq!(work.attempt_visits, projection.attempts.len());
    assert_eq!(work.graph_preparations, 0);
    assert_eq!(work.ready_vectors, 0);
  }
  let mut other = projection.clone();
  other.workflow_id = Some("other".into());
  let (result, work) =
    counted(|| membership(&graph, &hash, &other, events[0].occurred_at, "node8"));
  assert_eq!(
    result,
    Err("The run is not bound to this compiled workflow definition.".into())
  );
  assert_eq!(work, Work::default());
  for status in [
    RunStatus::NotStarted,
    RunStatus::Queued,
    RunStatus::Waiting,
    RunStatus::Cancelling,
    RunStatus::Finalizing,
    RunStatus::Succeeded,
    RunStatus::Failed,
    RunStatus::Cancelled,
  ] {
    other.status = status;
    let (result, work) =
      counted(|| membership(&graph, "wrong", &other, events[0].occurred_at, "node8"));
    assert_eq!(result, Ok(false));
    assert_eq!(work, Work::default());
  }
}

#[test]
fn historical_step_membership_checks_one_candidate_per_start() {
  let (model, hash, events) = super::investigation::chain(16);
  let (result, work) =
    counted(|| crate::engine::validate_event_history_against_definition(&model, &hash, &events));
  assert_eq!(result, Ok(()));
  assert_eq!(work.graph_preparations, 1);
  assert_eq!(work.candidate_checks, 16); // Former full-list path checked 16 * 16.
  assert_eq!(work.ready_vectors, 0);
  assert_eq!(work.active_node_visits, 16 * 16); // Traversal is not incremental yet.
  assert_eq!(work.attempt_visits, 0);
  assert_eq!(work.prefix_attempt_visits, 15);
}

#[test]
fn duplicate_membership_is_any_source_candidate_not_first_node_lookup() {
  let (mut model, hash, events) = super::investigation::chain(1);
  let projection = crate::fold_events(&events[..1]).unwrap();
  let ordinary = model.graph.nodes[0].clone();
  model.graph.nodes[0].handler = "engine.workflow-settlement".into();
  model.graph.nodes.push(ordinary);
  let graph = ReadinessGraph::new(&model);
  assert_eq!(
    graph.node("node0").unwrap().handler,
    "engine.workflow-settlement"
  );
  let expected = super::reference::ready_node_ids_for_projection_at(
    &model,
    &hash,
    &projection,
    events[0].occurred_at,
  )
  .unwrap();
  assert_eq!(expected, vec!["node0"]);
  assert_eq!(
    membership(&graph, &hash, &projection, events[0].occurred_at, "node0"),
    Ok(true)
  );
  model.graph.nodes.reverse();
  let graph = ReadinessGraph::new(&model);
  assert_eq!(
    membership(&graph, &hash, &projection, events[0].occurred_at, "node0"),
    Ok(true)
  );
}

#[test]
fn replacement_projections_and_legacy_binding_are_not_cached() {
  let (model, hash, events) = super::investigation::chain(4);
  let first = crate::fold_events(&events[..1]).unwrap();
  let next = crate::fold_events(&events[..3]).unwrap();
  let graph = ReadinessGraph::new(&model);
  for projection in [&first, &next, &first] {
    let expected = super::reference::ready_node_ids_for_projection_at(
      &model,
      &hash,
      projection,
      events[0].occurred_at,
    )
    .unwrap();
    for node in &model.graph.nodes {
      assert_eq!(
        membership(&graph, &hash, projection, events[0].occurred_at, &node.id),
        Ok(expected.contains(&node.id))
      );
    }
  }
  let mut legacy = first.clone();
  legacy.workflow_id = None;
  legacy.event_schema_version = Some(crate::RUN_EVENT_SCHEMA_VERSION_V11);
  assert_eq!(
    membership(&graph, &hash, &legacy, events[0].occurred_at, "node0"),
    Ok(true)
  );
  legacy.event_schema_version = Some(crate::RUN_EVENT_SCHEMA_VERSION_V1);
  assert!(membership(&graph, &hash, &legacy, events[0].occurred_at, "node0").is_err());
}

#[test]
#[ignore = "explicit same-binary control diagnosis, never acceptance timing"]
fn capture_controls() {
  use super::investigation::{chain, timed};
  use serde_json::json;
  use sha2::{Digest, Sha256};
  assert!(!cfg!(debug_assertions));
  let output = std::env::var_os("WOML_ISQ_CONTROL_CAPTURE").expect("explicit diagnostic path");
  let mode = std::env::var("WOML_ISQ_CONTROL_MODE").expect("full or membership");
  assert!(mode == "full" || mode == "membership");
  struct ResetMode;
  impl Drop for ResetMode {
    fn drop(&mut self) {
      FULL_MEMBERSHIP.with(|flag| flag.set(false));
    }
  }
  let _reset = ResetMode;
  FULL_MEMBERSHIP.with(|flag| flag.set(mode == "full"));
  let (model, hash, events) = chain(1024);
  let projection = crate::fold_events(&events[..1025]).unwrap();
  let now = events[0].occurred_at;
  let fixture = format!(
    "{:x}",
    Sha256::digest(serde_json::to_vec(&(&model, &hash, &events)).unwrap())
  );
  let mut rows = Vec::new();
  // Identical functions and fixture, same optimized executable. The only
  // experimental difference is the HISTORICAL membership strategy between
  // the before/after control batches. No production flag/environment surface.
  for stage in ["before-history", "after-history"] {
    if stage == "after-history" {
      for _ in 0..4 {
        crate::engine::validate_event_history_against_definition(&model, &hash, &events).unwrap();
      }
    }
    for sample in 0..6 {
      let (_, query) = timed(|| {
        for _ in 0..100 {
          std::hint::black_box(
            super::ready_node_ids_for_projection_at(&model, &hash, &projection, now).unwrap(),
          );
        }
      });
      let (_, topology) = timed(|| {
        for _ in 0..100 {
          std::hint::black_box(ReadinessGraph::new(&model));
        }
      });
      rows.push(json!({"stage":stage,"sample":sample,"warmup":sample==0,"fullQueryBatch100":query,"topologyBatch100":topology}));
    }
  }
  std::fs::write(output, serde_json::to_vec_pretty(&json!({"profile":"woml.isq-controls-diagnostic/v1","mode":mode,"nodes":1024,"fixtureSha256":fixture,"rows":rows,"releaseAccepted":false})).unwrap()).unwrap();
}

#[test]
#[ignore = "explicit optimized ISQ capture; not a CI timing assertion"]
fn capture() {
  use super::investigation::{chain, timed};
  use serde_json::json;
  use sha2::{Digest, Sha256};
  assert!(!cfg!(debug_assertions), "Use an explicit optimized receipt");
  assert!(std::env::var_os("WOML_PROFILE").is_none());
  assert!(WORK.with(|cell| cell.get().is_none()));
  let path = std::env::var_os("WOML_ISQ_CAPTURE").expect("explicit capture path");
  let contract: serde_json::Value = serde_json::from_str(include_str!(
    "../../../../../docs/incremental-scheduling-acceptance.v1.json"
  ))
  .unwrap();
  let mut rows = Vec::new();
  for count in contract["sizes"].as_array().unwrap() {
    let nodes = count.as_u64().unwrap() as usize;
    let (model, hash, events) = chain(nodes);
    let fixture = format!(
      "{:x}",
      Sha256::digest(serde_json::to_vec(&(&model, &hash, &events)).unwrap())
    );
    let projection = crate::fold_events(&events[..1 + 2 * (nodes / 2)]).unwrap();
    let now = events[0].occurred_at;
    let node = format!("node{}", nodes / 2);
    let graph = ReadinessGraph::new(&model);
    assert!(membership(&graph, &hash, &projection, now, &node).unwrap());
    crate::engine::validate_event_history_against_definition(&model, &hash, &events).unwrap();
    let queries = contract["queriesPerSample"].as_u64().unwrap();
    for sample in
      0..contract["warmupSamples"].as_u64().unwrap() + contract["measuredSamples"].as_u64().unwrap()
    {
      let (result, history) =
        timed(|| crate::engine::validate_event_history_against_definition(&model, &hash, &events));
      assert_eq!(result, Ok(()));
      let (_, member) = timed(|| {
        for _ in 0..queries {
          std::hint::black_box(membership(&graph, &hash, &projection, now, &node).unwrap());
        }
      });
      let (_, full) = timed(|| {
        for _ in 0..queries {
          std::hint::black_box(
            super::ready_node_ids_for_projection_at(&model, &hash, &projection, now).unwrap(),
          );
        }
      });
      let (_, topology) = timed(|| {
        for _ in 0..queries {
          std::hint::black_box(ReadinessGraph::new(&model));
        }
      });
      let per_query = |mut value: serde_json::Value| {
        for name in ["wallMs", "threadCpuMs"] {
          if let Some(time) = value[name].as_f64() {
            value[name] = json!(time / queries as f64);
          }
        }
        value
      };
      rows.push(json!({"nodes":nodes,"events":events.len(),"sample":sample,"warmup":sample < contract["warmupSamples"].as_u64().unwrap(),"fixtureSha256":fixture,
        "historyValidation":history,"membership":per_query(member),"fullQueryIncludingPreparation":per_query(full),"topologyPreparation":per_query(topology)}));
    }
  }
  std::fs::write(path, serde_json::to_vec_pretty(&json!({"profile":"woml.incremental-scheduling-capture/v1","allocationMeasurement":null,"peakLiveBytes":null,"rows":rows})).unwrap()).unwrap();
}
