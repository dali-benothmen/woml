use super::{index::LatestAttempts, reference, ReadinessGraph};
use crate::projection::{AttemptIdentity, AttemptProjection, AttemptStatus};
use crate::{RunProjection, RunStatus};

fn attempt(node: &str, number: u32, invocation: &str) -> AttemptProjection {
  AttemptProjection {
    identity: AttemptIdentity {
      node_id: node.to_string(),
      attempt: number,
      invocation_id: invocation.to_string(),
    },
    handler: "runtime.script".to_string(),
    idempotency_key: None,
    status: AttemptStatus::Started,
  }
}

#[test]
fn latest_lookup_uses_vector_order_and_exact_projection_not_attempt_number() {
  let mut first = RunProjection::default();
  first.attempts = vec![
    attempt("a", 9, "old"),
    attempt("b", 1, "other"),
    attempt("a", 1, "new"),
  ];
  {
    let lookup = LatestAttempts::new(&first);
    for id in ["a", "b", "missing"] {
      assert_eq!(lookup.get(id), first.latest_attempt(id));
    }
    assert_eq!(lookup.get("a").unwrap().identity.invocation_id, "new");
    let second = RunProjection {
      attempts: vec![attempt("a", 2, "different-run-or-scope")],
      ..RunProjection::default()
    };
    let independent = LatestAttempts::new(&second);
    assert_ne!(lookup.get("a"), independent.get("a"));
  }
  // The borrow ends before an append/rollback/replacement. The next query must
  // construct its own view, not keep the former last attempt.
  first.attempts.pop();
  assert_eq!(
    LatestAttempts::new(&first).get("a"),
    first.latest_attempt("a")
  );
  first.attempts.clear();
  assert!(LatestAttempts::new(&first).get("a").is_none());
}

#[test]
fn graph_indexes_preserve_source_order_duplicates_and_missing_endpoints() {
  let (mut model, hash, events) = super::investigation::chain(8);
  model.graph.nodes.reverse();
  model.graph.edges.reverse();
  let graph = ReadinessGraph::new(&model);
  for node in &model.graph.nodes {
    assert_eq!(graph.node(&node.id), model.node(&node.id));
    assert_eq!(
      graph.incoming(&node.id).collect::<Vec<_>>(),
      model
        .graph
        .edges
        .iter()
        .filter(|edge| edge.to == node.id)
        .collect::<Vec<_>>()
    );
    assert_eq!(
      graph.outgoing(&node.id).collect::<Vec<_>>(),
      model
        .graph
        .edges
        .iter()
        .filter(|edge| edge.from == node.id)
        .collect::<Vec<_>>()
    );
  }
  for end in 0..=events.len() {
    let projection = crate::fold_events(&events[..end]).unwrap();
    assert_eq!(
      graph.ready_at(&hash, &projection, events[0].occurred_at),
      reference::ready_node_ids_for_projection_at(
        &model,
        &hash,
        &projection,
        events[0].occurred_at
      )
    );
  }
  // Preparation is not a substitute for model validation. Even malformed direct
  // caller inputs retain first-node and edge-list semantics; no index panic.
  model.graph.nodes.push(model.graph.nodes[0].clone());
  model.graph.edges.push(model.graph.edges[0].clone());
  model.graph.edges[0].from = "missing".to_string();
  let graph = ReadinessGraph::new(&model);
  let projection = crate::fold_events(&events[..1]).unwrap();
  assert_eq!(
    graph.node(&model.graph.nodes[0].id),
    model.node(&model.graph.nodes[0].id)
  );
  assert_eq!(
    graph.ready_at(&hash, &projection, events[0].occurred_at),
    reference::ready_node_ids_for_projection_at(&model, &hash, &projection, events[0].occurred_at)
  );
}

#[test]
fn source_node_order_is_not_hash_iteration_order() {
  let (mut model, hash, events) = super::investigation::chain(16);
  model.graph.edges.clear();
  model.graph.nodes.reverse();
  model.graph.entry_node_ids = model
    .graph
    .nodes
    .iter()
    .map(|node| node.id.clone())
    .collect();
  let projection = crate::fold_events(&events[..1]).unwrap();
  let graph = ReadinessGraph::new(&model);
  for _ in 0..8 {
    assert_eq!(
      graph
        .ready_at(&hash, &projection, events[0].occurred_at)
        .unwrap(),
      model.graph.entry_node_ids
    );
  }
  let mut wrong_binding = projection.clone();
  wrong_binding.workflow_id = Some("different".to_string());
  assert!(graph
    .ready_at(&hash, &wrong_binding, events[0].occurred_at)
    .is_err());
  wrong_binding.status = RunStatus::Cancelled;
  assert_eq!(
    graph.ready_at("wrong-hash", &wrong_binding, events[0].occurred_at),
    Ok(Vec::new())
  );
}

fn compare_snapshot(model: &crate::CompiledWorkflowDefinition, projection: &RunProjection) {
  let graph = ReadinessGraph::new(model);
  let now = chrono::DateTime::parse_from_rfc3339("2026-11-03T00:00:00Z")
    .unwrap()
    .with_timezone(&chrono::Utc);
  assert_eq!(
    graph.active_node_ids(projection),
    reference::active_node_ids(model, projection)
  );
  for node in &model.graph.nodes {
    assert_eq!(
      graph.complete(node, projection),
      reference::node_is_complete(model, node, projection),
      "completion {}",
      node.id
    );
  }
  assert_eq!(
    graph.settlement_can_decide(projection, &LatestAttempts::new(projection)),
    reference::workflow_settlement_can_decide(model, projection)
  );
  assert_eq!(
    graph.ready_at("hash", projection, now),
    reference::ready_node_ids_for_projection_at(model, "hash", projection, now)
  );
  let expected = reference::ready_node_ids_for_projection_at(model, "hash", projection, now);
  let owner = crate::engine::ReadinessDefinition::new(model.clone());
  assert_eq!(owner.ready_at("hash", projection, now), expected);
  for node in &model.graph.nodes {
    assert_eq!(
      graph.is_ready_at("hash", projection, now, &node.id),
      expected.clone().map(|ids| ids.contains(&node.id))
    );
  }
}

#[test]
fn choices_and_approval_joins_match_scans_for_both_routes_and_completion_subsets() {
  // Predicate-level snapshots deliberately vary completion/selection separately;
  // they are not substitutes for the structurally validated history corpus.
  for source in [
    include_str!("../../../../../woml/tests/fixtures/fork-branch/control-choice.compiled.v13.json"),
    include_str!("../../../../../woml/tests/fixtures/approval.compiled.v4.json"),
  ] {
    let model: crate::CompiledWorkflowDefinition = serde_json::from_str(source).unwrap();
    model.validate_structure().unwrap();
    for mask in 0..(1usize << model.graph.nodes.len()) {
      for selected in [None, Some(false), Some(true)] {
        let mut projection = RunProjection {
          status: RunStatus::Running,
          workflow_id: Some(model.workflow_id.clone()),
          definition_hash: Some("hash".to_string()),
          ..RunProjection::default()
        };
        for (index, node) in model.graph.nodes.iter().enumerate() {
          if mask & (1 << index) != 0 {
            projection
              .context
              .steps
              .insert(node.id.clone(), serde_json::json!({"value":1}));
          }
          if node.handler == "engine.approval-wait" {
            if let Some(approved) = selected {
              projection.context.steps.insert(
                node.id.clone(),
                serde_json::json!({"decision":if approved {"approved"} else {"rejected"}}),
              );
            }
          }
        }
        for choice in model.graph.choices.as_deref().unwrap_or_default() {
          if let Some(first) = selected {
            projection.choice_selections.insert(
              choice.choice_id.clone(),
              choice.arm_ids[usize::from(!first)].clone(),
            );
          }
        }
        compare_snapshot(&model, &projection);
      }
    }
  }
}

#[test]
fn settlement_respects_latest_failures_retry_state_and_joined_fork_ownership() {
  use crate::projection::{
    ForkBranchProjection, ForkJoinStatus, ForkProjection, RetryScheduleProjection,
  };
  let mut model: crate::CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json"
  ))
  .unwrap();
  model.validate_structure().unwrap();
  let mut projection = RunProjection {
    status: RunStatus::Running,
    workflow_id: Some(model.workflow_id.clone()),
    definition_hash: Some("hash".to_string()),
    ..RunProjection::default()
  };
  for status in [
    ForkJoinStatus::Pending,
    ForkJoinStatus::Succeeded,
    ForkJoinStatus::Failed,
    ForkJoinStatus::Cancelled,
  ] {
    let fork = &model.graph.forks.as_ref().unwrap()[0];
    projection.forks.insert(
      fork.fork_id.clone(),
      ForkProjection {
        fork_id: fork.fork_id.clone(),
        join_status: status,
        blocking_branch_id: None,
        branches: fork
          .branches
          .iter()
          .map(|branch| {
            (
              branch.branch_id.clone(),
              ForkBranchProjection {
                branch_id: branch.branch_id.clone(),
                terminal_node_id: Some(branch.terminal_node_id.clone()),
                outcome: Some(crate::event::ForkBranchOutcome::Succeeded),
              },
            )
          })
          .collect(),
      },
    );
    compare_snapshot(&model, &projection);
  }
  let mut failure = attempt("publishInstagram", 3, "failed");
  failure.status = AttemptStatus::Failed {
    failure: crate::AttemptFailure {
      kind: crate::AttemptFailureKind::ScriptThrew,
      code: "WOML_SCRIPT_THROWN".to_string(),
      message: "test".to_string(),
      details: None,
      ..crate::AttemptFailure::legacy_defaults()
    },
  };
  projection.attempts.push(failure);
  assert!(ReadinessGraph::new(&model)
    .settlement_can_decide(&projection, &LatestAttempts::new(&projection)));
  compare_snapshot(&model, &projection);
  model.graph.forks.as_mut().unwrap()[0]
    .joined_branch_ids
    .retain(|id| id != "instagram");
  assert!(!ReadinessGraph::new(&model)
    .settlement_can_decide(&projection, &LatestAttempts::new(&projection)));
  compare_snapshot(&model, &projection);
  model.graph.forks.as_mut().unwrap()[0]
    .joined_branch_ids
    .push("instagram".to_string());
  projection.pending_retries.insert(
    "publishInstagram".to_string(),
    RetryScheduleProjection {
      node_id: "publishInstagram".to_string(),
      failed_attempt: 3,
      next_attempt: 4,
      scheduled_at: chrono::Utc::now(),
    },
  );
  compare_snapshot(&model, &projection);
  assert!(!ReadinessGraph::new(&model)
    .settlement_can_decide(&projection, &LatestAttempts::new(&projection)));
  projection.pending_retries.clear();
  projection
    .attempts
    .push(attempt("publishInstagram", 1, "newer-vector-entry"));
  compare_snapshot(&model, &projection);
  assert!(!ReadinessGraph::new(&model)
    .settlement_can_decide(&projection, &LatestAttempts::new(&projection)));
}

#[test]
#[ignore = "explicit optimized paired capture; not a CI timing assertion"]
fn capture_indexed() {
  use super::investigation::{chain, timed};
  use crate::engine::history::{
    validate_event_history_against_definition, validate_history_scan_reference,
  };
  use serde_json::json;
  use sha2::{Digest, Sha256};
  assert!(
    !cfg!(debug_assertions),
    "Use an explicit optimized build receipt"
  );
  assert!(std::env::var_os("WOML_PROFILE").is_none());
  let output = std::env::var_os("WOML_READINESS_INDEX_CAPTURE").expect("explicit output path");
  let mut rows = Vec::new();
  for nodes in [1, 4, 16, 64, 256, 1024] {
    let (model, hash, events) = chain(nodes);
    let fixture = format!("{:x}", Sha256::digest(serde_json::to_vec(&events).unwrap()));
    let projection = crate::fold_events(&events[..1 + 2 * (nodes / 2)]).unwrap();
    let now = events[0].occurred_at;
    let scan = || validate_history_scan_reference(&model, &hash, &events);
    let indexed = || validate_event_history_against_definition(&model, &hash, &events);
    // Validate both independently outside timers. Failures are not timing data.
    assert_eq!(scan(), Ok(()));
    assert_eq!(indexed(), Ok(()));
    for sample in 0..4 {
      let (scan_result, scan_time, indexed_result, indexed_time) = if sample % 2 == 0 {
        let (a, at) = timed(scan);
        let (b, bt) = timed(indexed);
        (a, at, b, bt)
      } else {
        let (b, bt) = timed(indexed);
        let (a, at) = timed(scan);
        (a, at, b, bt)
      };
      assert_eq!(scan_result, Ok(()));
      assert_eq!(indexed_result, scan_result);
      let original_query =
        || reference::ready_node_ids_for_projection_at(&model, &hash, &projection, now);
      let indexed_query =
        || super::ready_node_ids_for_projection_at(&model, &hash, &projection, now);
      let (a, at, b, bt) = if sample % 2 == 0 {
        let (a, at) = timed(original_query);
        let (b, bt) = timed(indexed_query);
        (a, at, b, bt)
      } else {
        let (b, bt) = timed(indexed_query);
        let (a, at) = timed(original_query);
        (a, at, b, bt)
      };
      assert_eq!(a, b);
      assert_eq!(b.unwrap(), vec![format!("node{}", nodes / 2)]);
      let (graph, preparation) = timed(|| ReadinessGraph::new(&model));
      assert_eq!(graph.workflow.graph.nodes.len(), nodes);
      drop(graph);
      rows.push(
        json!({ "nodes": nodes, "events": events.len(), "sample": sample, "warmup": sample == 0,
        "fixtureSha256":fixture, "scanValidation":scan_time, "indexedValidation":indexed_time,
        "scanQuery":at, "indexedQueryIncludingPreparation":bt, "graphPreparation":preparation }),
      );
    }
  }
  std::fs::write(output, serde_json::to_vec_pretty(&json!({
    "profile":"woml.readiness-indexes/v1", "allocationMeasurement":null, "peakLiveBytes":null, "rows":rows,
  })).unwrap()).unwrap();
}
