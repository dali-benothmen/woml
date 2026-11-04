//! Independent ordered-list/membership and full-replay checks, including fallback.
use super::{histories, HistoryPrefix};
use crate::engine::readiness::{observe_frontier, ReadinessGraph};
use crate::{CompiledWorkflowDefinition, RunEvent, RunEventPayload};
use chrono::{Duration, Utc};
use serde_json::json;

fn compare_queries(
  model: &CompiledWorkflowDefinition,
  hash: &str,
  events: &[RunEvent],
) -> [usize; 7] {
  let graph = ReadinessGraph::new(model);
  let scan = ReadinessGraph::reference(model);
  observe_frontier(|| {
    let mut cursor = HistoryPrefix::with_definition(events, model, hash);
    for end in 0..=events.len() {
      for _ in 0..2 {
        let prefix = cursor.query_at(end).unwrap();
        let fresh = crate::fold_events(&events[..end]).unwrap();
        assert_eq!(prefix.projection(), &fresh);
        let time = events
          .first()
          .map(|event| event.occurred_at)
          .unwrap_or_else(Utc::now);
        let mut clocks = vec![
          time - Duration::days(1),
          time,
          time + Duration::days(365),
          time,
        ];
        for schedule in fresh.pending_retries.values() {
          clocks.extend([
            schedule.scheduled_at - Duration::nanoseconds(1),
            schedule.scheduled_at,
            schedule.scheduled_at + Duration::nanoseconds(1),
            schedule.scheduled_at - Duration::days(1),
          ]);
        }
        for now in clocks {
          for binding in [hash, "wrong-hash"] {
            let expected = scan.ready_at(binding, &fresh, now);
            let actual = graph.ready_from_prefix_at(binding, &prefix, now);
            assert_eq!(actual, expected, "prefix {end} at {now}");
            assert_eq!(
              actual.as_ref().ok().and_then(|ids| ids.first()),
              expected.as_ref().ok().and_then(|ids| ids.first())
            );
            for id in model
              .graph
              .nodes
              .iter()
              .map(|node| node.id.as_str())
              .chain(["missing"])
            {
              assert_eq!(
                graph.is_ready_from_prefix_at(binding, &prefix, now, id),
                expected
                  .clone()
                  .map(|ids| ids.iter().any(|candidate| candidate == id)),
                "{end}:{id}"
              );
            }
          }
        }
      }
    }
  })
  .1
}

#[test]
fn all_recorded_scopes_match_full_replay_and_select_real_fallback() {
  for history in histories() {
    let work = compare_queries(&history.model, &history.definition_hash, &history.events);
    let events_ordinary = history
      .events
      .iter()
      .all(|event| crate::engine::readiness::changed_node(event).is_ok());
    if history
      .model
      .graph
      .nodes
      .iter()
      .any(|node| node.handler != "runtime.script")
      || history.model.lifecycle.is_some()
      || history.model.runtime_policy.is_some()
    {
      assert_eq!(
        work[0], 0,
        "{} unexpectedly prepared a frontier",
        history.label
      );
      assert!(work[5] > 0, "{} never exercised fallback", history.label);
    } else if events_ordinary && !history.model.graph.nodes.is_empty() {
      assert!(
        work[0] > 0,
        "{} did not exercise the ordinary frontier",
        history.label
      );
    }
  }
}

// Seeded valid DAGs include wide sets, fan-in/out, duplicate edges and source
// order different from topological/execution order. Direct disconnected inputs
// are compared separately, without claiming they pass model validation.
fn generated(seed: u64) -> (CompiledWorkflowDefinition, String, Vec<RunEvent>) {
  let (mut model, _, original) = crate::engine::readiness::chain_for_frontier_tests(12);
  let mut state = seed + 1;
  let mut random = || {
    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
    state
  };
  model.graph.edges.clear();
  for to in 1..10 {
    for from in 0..to {
      if random() % 4 == 0 {
        let mut edge = crate::model::CompiledWorkflowEdge {
          id: format!("edge{from}_{to}"),
          from: format!("node{from}"),
          to: format!("node{to}"),
          condition: crate::model::EdgeCondition::Always,
          branch_id: None,
          parallel_id: None,
          approval_id: None,
        };
        model.graph.edges.push(edge.clone());
        if random() % 5 == 0 {
          edge.id.push_str("_duplicate");
          model.graph.edges.push(edge);
        }
      }
    }
  }
  // The frozen executable profile requires reachable nodes, root entries and
  // exactly one terminal. Connect otherwise disconnected components and join
  // every leaf at node11, while retaining the generated fan-in/out shape.
  for to in 1..11 {
    let id = format!("node{to}");
    if !model.graph.edges.iter().any(|edge| edge.to == id) {
      model.graph.edges.push(crate::model::CompiledWorkflowEdge {
        id: format!("root{to}"),
        from: "node0".into(),
        to: id,
        condition: crate::model::EdgeCondition::Always,
        branch_id: None,
        parallel_id: None,
        approval_id: None,
      });
    }
  }
  for from in 0..11 {
    let id = format!("node{from}");
    if !model.graph.edges.iter().any(|edge| edge.from == id) {
      model.graph.edges.push(crate::model::CompiledWorkflowEdge {
        id: format!("join{from}"),
        from: id,
        to: "node11".into(),
        condition: crate::model::EdgeCondition::Always,
        branch_id: None,
        parallel_id: None,
        approval_id: None,
      });
    }
  }
  model.graph.entry_node_ids = vec!["node0".into()];
  model.graph.nodes.reverse();
  model.validate_structure().unwrap();
  use sha2::{Digest, Sha256};
  let hash = format!(
    "sha256:{:x}",
    Sha256::digest(serde_json_canonicalizer::to_vec(&model).unwrap())
  );
  let mut events = vec![original[0].clone()];
  let RunEventPayload::RunStarted(start) = &mut events[0].payload else {
    unreachable!();
  };
  start.definition_hash = hash.clone();
  let reference = ReadinessGraph::reference(&model);
  // Build valid work with the independent oracle, not the frontier under test.
  loop {
    let projection = crate::fold_events(&events).unwrap();
    let ready = reference
      .ready_at(&hash, &projection, original[0].occurred_at)
      .unwrap();
    if ready.is_empty() {
      break;
    }
    // Start all currently ready siblings before completing any, testing in-flight state.
    for node in &ready {
      let mut event = original[1].clone();
      event.payload = RunEventPayload::StepAttemptStarted(crate::StepAttemptStartedData {
        node_id: node.clone(),
        attempt: 1,
        invocation_id: format!("inv_{node}"),
        handler: "runtime.script".into(),
        idempotency_key: None,
      });
      event.sequence = events.len() as u64 + 1;
      event.event_id = format!("evt_generated_{}", event.sequence);
      events.push(event);
    }
    for node in ready.into_iter().rev() {
      let mut event = original[2].clone();
      event.payload = RunEventPayload::StepAttemptSucceeded(crate::StepAttemptSucceededData {
        node_id: node.clone(),
        attempt: 1,
        invocation_id: format!("inv_{node}"),
        output: json!({"seed":seed}),
      });
      event.sequence = events.len() as u64 + 1;
      event.event_id = format!("evt_generated_{}", event.sequence);
      events.push(event);
    }
  }
  (model, hash, events)
}

#[test]
fn seeded_dags_match_every_ordered_prefix_and_reject_mutated_input_identically() {
  for seed in 0..16 {
    let (model, hash, events) = generated(seed);
    let work = compare_queries(&model, &hash, &events);
    assert_eq!(work[0], 1);
    assert_eq!(work[5], 0);
    assert!(work[4] > 0);
    assert_eq!(
      crate::engine::validate_event_history_against_definition(&model, &hash, &events),
      Ok(())
    );
    for index in 1..events.len() {
      let mut corrupt = events.clone();
      if let RunEventPayload::StepAttemptStarted(data) = &mut corrupt[index].payload {
        data.node_id = "node11".into();
      } else {
        corrupt[index].sequence += 1;
      }
      assert_eq!(
        crate::engine::validate_event_history_against_definition(&model, &hash, &corrupt),
        super::super::validate_history_with_graph(
          &model,
          &hash,
          &corrupt,
          HistoryPrefix::replay,
          ReadinessGraph::reference
        )
      );
    }
  }
}

#[test]
fn rejected_candidate_poison_discards_frontier_and_foreign_definition_never_reuses_it() {
  let (model, hash, mut events) = generated(7);
  let other = model.clone();
  let mut cursor = HistoryPrefix::with_definition(&events, &model, &hash);
  let prefix = cursor.query_at(1).unwrap();
  assert!(prefix.frontier_for(&model).is_some());
  assert!(prefix.frontier_for(&other).is_none());
  drop(cursor);
  events[2].event_id = events[1].event_id.clone();
  let mut cursor = HistoryPrefix::with_definition(&events, &model, &hash);
  assert!(cursor.query_at(1).unwrap().frontier_for(&model).is_some());
  assert!(cursor.query_at(3).is_err());
  assert!(cursor.query_at(1).is_err());
  assert!(cursor.query_at(events.len()).is_err());
}

#[test]
fn unsupported_definition_fields_and_handlers_really_fall_back() {
  let (model, hash, events) = crate::engine::readiness::chain_for_frontier_tests(4);
  for mutation in 0..17 {
    let mut changed = model.clone();
    match mutation {
      0 => changed.graph.nodes[0].handler = "future.handler".into(),
      1 => changed.graph.choices = Some(Vec::new()),
      2 => changed.graph.forks = Some(Vec::new()),
      3 => changed.graph.for_each = Some(Vec::new()),
      4 => changed.graph.waits = Some(Vec::new()),
      5 => changed.graph.locks = Some(Vec::new()),
      6 => changed.graph.sagas = Some(Vec::new()),
      7 => changed.graph.context_visibility = Some(Vec::new()),
      8 => changed.graph.edges[0].branch_id = Some("branch".into()),
      9 => changed.graph.nodes.push(changed.graph.nodes[0].clone()),
      10 => changed.graph.edges[0].to = "missing".into(),
      11 => changed.graph.edges[0].from = "node3".into(), // cycle through nodes 1..3
      12 => changed.graph.entry_node_ids = vec!["missing".into()],
      13 => changed.schema_version = 999,
      14 => {
        changed.graph.nodes[0].execution = Some(crate::model::CompiledExecutionPlacement {
          profile_version: 1,
          target: "remote-test".into(),
          required_capabilities: Vec::new(),
        })
      }
      15 => changed.graph.edges[0].parallel_id = Some("parallel".into()),
      16 => changed.graph.edges[0].approval_id = Some("approval".into()),
      _ => unreachable!(),
    }
    let work = compare_queries(&changed, &hash, &events[..1]);
    assert_eq!(work[0], 0, "mutation {mutation}");
    assert!(work[5] > 0);
  }
}

#[test]
fn retry_deadlines_and_late_bootstrap_use_current_state_at_every_query() {
  let history = histories()
    .into_iter()
    .find(|history| history.label == "retry-complete")
    .unwrap();
  let work = compare_queries(&history.model, &history.definition_hash, &history.events);
  assert_eq!(work[0], 1);
  assert!(work[4] > 0);
  let graph = ReadinessGraph::new(&history.model);
  let reference = ReadinessGraph::reference(&history.model);
  for start in 0..=history.events.len() {
    let mut cursor =
      HistoryPrefix::with_definition(&history.events, &history.model, &history.definition_hash);
    for end in start..=history.events.len() {
      let prefix = cursor.query_at(end).unwrap();
      let fresh = crate::fold_events(&history.events[..end]).unwrap();
      for now in [
        history.events[0].occurred_at,
        history.events[0].occurred_at + Duration::days(365),
      ] {
        assert_eq!(
          graph.ready_from_prefix_at(&history.definition_hash, &prefix, now),
          reference.ready_at(&history.definition_hash, &fresh, now)
        );
      }
    }
  }
}

#[test]
fn independent_run_cursors_and_replacement_histories_do_not_share_candidates() {
  let (model, hash, events) = generated(3);
  let mut replacement = events.clone();
  for event in &mut replacement {
    event.run_id = "run_other_frontier".into();
  }
  let graph = ReadinessGraph::new(&model);
  let scan = ReadinessGraph::reference(&model);
  let mut a = HistoryPrefix::with_definition(&events, &model, &hash);
  let mut b = HistoryPrefix::with_definition(&replacement, &model, &hash);
  let aa = a.query_at(events.len()).unwrap();
  let bb = b.query_at(1).unwrap();
  assert_ne!(aa.run_id, bb.run_id);
  let now = events[0].occurred_at;
  assert_eq!(
    graph.ready_from_prefix_at(&hash, &aa, now),
    scan.ready_at(&hash, aa.projection(), now)
  );
  assert_eq!(
    graph.ready_from_prefix_at(&hash, &bb, now),
    scan.ready_at(&hash, bb.projection(), now)
  );
  assert_ne!(
    graph.ready_from_prefix_at(&hash, &aa, now),
    graph.ready_from_prefix_at(&hash, &bb, now)
  );
}

#[test]
fn disconnected_direct_query_and_terminal_invalidation_keep_original_semantics() {
  // Direct readiness inputs need not have passed model validation. Do not
  // manufacture readiness for unreachable nodes or use entry-list order.
  let (mut model, hash, events) = crate::engine::readiness::chain_for_frontier_tests(4);
  model.graph.edges.clear();
  model.graph.entry_node_ids = vec!["node3".into(), "node1".into()];
  let mut cursor = HistoryPrefix::with_definition(&events[..1], &model, &hash);
  let prefix = cursor.query_at(1).unwrap();
  assert_eq!(
    ReadinessGraph::new(&model).ready_from_prefix_at(&hash, &prefix, events[0].occurred_at),
    Ok(vec!["node1".into(), "node3".into()])
  );

  let history = histories()
    .into_iter()
    .find(|history| history.label == "hello")
    .unwrap();
  let mut cursor =
    HistoryPrefix::with_definition(&history.events, &history.model, &history.definition_hash);
  assert!(cursor
    .query_at(1)
    .unwrap()
    .frontier_for(&history.model)
    .is_some());
  assert!(cursor
    .query_at(history.events.len())
    .unwrap()
    .frontier_for(&history.model)
    .is_none());
}

#[test]
fn equal_length_histories_with_different_inflight_siblings_are_independent() {
  let (model, hash, all) = (0..16)
    .map(generated)
    .find(|(_, _, events)| {
      matches!(
        events.get(3).map(|event| &event.payload),
        Some(RunEventPayload::StepAttemptStarted(_))
      ) && matches!(
        events.get(4).map(|event| &event.payload),
        Some(RunEventPayload::StepAttemptStarted(_))
      )
    })
    .unwrap();
  let events = all[..4].to_vec();
  let mut other = events.clone();
  other[3].payload = all[4].payload.clone();
  for event in &mut other {
    event.run_id = "run_equal_length_other".into();
  }
  let mut a = HistoryPrefix::with_definition(&events, &model, &hash);
  let mut b = HistoryPrefix::with_definition(&other, &model, &hash);
  a.query_at(1).unwrap();
  b.query_at(1).unwrap();
  let graph = ReadinessGraph::new(&model);
  let scan = ReadinessGraph::reference(&model);
  let now = events[0].occurred_at;
  let expected_a = scan.ready_at(&hash, &crate::fold_events(&events).unwrap(), now);
  let expected_b = scan.ready_at(&hash, &crate::fold_events(&other).unwrap(), now);
  assert_ne!(expected_a, expected_b);
  assert_eq!(
    graph.ready_from_prefix_at(&hash, &a.query_at(4).unwrap(), now),
    expected_a
  );
  assert_eq!(
    graph.ready_from_prefix_at(&hash, &b.query_at(4).unwrap(), now),
    expected_b
  );
  assert!(a.query_at(3).is_err());
  assert!(a.query_at(5).is_err());
  assert_eq!(
    graph.ready_from_prefix_at(&hash, &a.query_at(4).unwrap(), now),
    expected_a
  );
}
