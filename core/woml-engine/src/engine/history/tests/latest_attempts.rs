use super::{histories, HistoryPrefix};
use crate::engine::readiness::{observe_prefix_visits, ReadinessGraph};

#[test]
fn cursor_lookup_matches_reverse_vector_at_every_prefix_status_scope_and_repeat() {
  for history in histories() {
    let graph = ReadinessGraph::new(&history.model);
    let reference = ReadinessGraph::reference(&history.model);
    let mut cursor = HistoryPrefix::new(&history.events);
    for end in 0..=history.events.len() {
      for _ in 0..2 {
        let prefix = cursor.query_at(end).unwrap();
        let expected = crate::fold_events(&history.events[..end]).unwrap();
        assert_eq!(prefix.projection(), &expected, "{}:{end}", history.label);
        for id in history
          .model
          .graph
          .nodes
          .iter()
          .map(|node| node.id.as_str())
          .chain(["missing"])
        {
          assert_eq!(
            prefix.latest_attempt(id),
            expected.latest_attempt(id),
            "{}:{end}:{id}",
            history.label
          );
        }
        for execution in expected
          .for_each
          .values()
          .flat_map(|state| state.executions.values())
        {
          for attempt in &execution.attempts {
            let id = &attempt.identity.node_id;
            assert_eq!(
              prefix.latest_attempt(id),
              expected.latest_attempt(id),
              "scoped attempt leaked into root lookup"
            );
          }
        }
        let time = history.events[end.saturating_sub(1)].occurred_at;
        for now in [
          time - chrono::Duration::days(1),
          time,
          time + chrono::Duration::days(365),
        ] {
          assert_eq!(
            graph.ready_from_prefix_at(&history.definition_hash, &prefix, now),
            reference.ready_at(&history.definition_hash, &expected, now)
          );
        }
      }
    }
  }
}

#[test]
fn legacy_latest_means_last_vector_entry_not_greatest_attempt_number() {
  use crate::RunEventPayload;
  let history = histories()
    .into_iter()
    .find(|history| history.label == "hello")
    .unwrap();
  let mut events = history.events[..3].to_vec();
  let RunEventPayload::StepAttemptStarted(start) = &mut events[1].payload else {
    panic!("start");
  };
  start.attempt = 9;
  let node = start.node_id.clone();
  let RunEventPayload::StepAttemptSucceeded(done) = &mut events[2].payload else {
    panic!("done");
  };
  done.attempt = 9;
  let mut next = events[1].clone();
  next.sequence = 4;
  next.event_id = "evt_latest_legacy_again".into();
  let RunEventPayload::StepAttemptStarted(start) = &mut next.payload else {
    unreachable!();
  };
  start.attempt = 1;
  start.invocation_id = "inv_latest_legacy_again".into();
  events.push(next);
  let expected = crate::fold_events(&events).unwrap();
  assert_eq!(expected.latest_attempt(&node).unwrap().identity.attempt, 1);
  let mut cursor = HistoryPrefix::new(&events);
  cursor.query_at(2).unwrap();
  let prefix = cursor.query_at(4).unwrap();
  assert_eq!(prefix.latest_attempt(&node), expected.latest_attempt(&node));
}

#[test]
fn lazy_index_bootstraps_once_then_visits_only_new_root_attempts() {
  let history = histories()
    .into_iter()
    .find(|history| history.label == "sequential-32")
    .unwrap();
  let (model, hash, events) = (history.model, history.definition_hash, history.events);
  let graph = ReadinessGraph::new(&model);
  let mut cursor = HistoryPrefix::new(&events);
  let initial = cursor.at(17).unwrap().attempts.len();
  let (_, visits) = observe_prefix_visits(|| {
    let prefix = cursor.query_at(17).unwrap();
    assert_eq!(
      graph.ready_from_prefix_at(&hash, &prefix, events[0].occurred_at),
      ReadinessGraph::reference(&model).ready_at(&hash, prefix.projection(), events[0].occurred_at)
    );
  });
  assert_eq!(visits, initial);
  let (_, visits) = observe_prefix_visits(|| {
    for end in 17..=events.len() {
      let prefix = cursor.query_at(end).unwrap();
      for attempt in &prefix.attempts {
        assert_eq!(
          prefix.latest_attempt(&attempt.identity.node_id),
          prefix
            .projection()
            .latest_attempt(&attempt.identity.node_id)
        );
      }
    }
  });
  assert_eq!(
    visits,
    crate::fold_events(&events).unwrap().attempts.len() - initial
  );
}

#[test]
fn failed_apply_poison_discards_tentative_projection_and_index() {
  let history = histories()
    .into_iter()
    .find(|history| history.label == "hello")
    .unwrap();
  let mut events = history.events;
  let node = &history.model.graph.entry_node_ids[0];
  events[3].event_id = events[2].event_id.clone();
  let mut cursor = HistoryPrefix::new(&events);
  assert!(cursor.query_at(3).unwrap().latest_attempt(node).is_some());
  let error = match cursor.query_at(4) {
    Ok(_) => panic!("duplicate event accepted"),
    Err(error) => error,
  };
  for end in [3, 4, events.len()] {
    assert_eq!(cursor.query_at(end).err(), Some(error.clone()));
  }
}

#[test]
fn cursors_with_equal_lengths_or_ids_never_share_dynamic_state() {
  let history = histories()
    .into_iter()
    .find(|history| history.label == "hello")
    .unwrap();
  let events = history.events;
  let node = &history.model.graph.entry_node_ids[0];
  let mut other = events.clone();
  for event in &mut other {
    event.run_id = "other-run".into();
  }
  let mut a = HistoryPrefix::new(&events);
  let mut b = HistoryPrefix::new(&other);
  let aa = a.query_at(3).unwrap();
  let bb = b.query_at(2).unwrap();
  assert_ne!(aa.run_id, bb.run_id);
  assert_ne!(aa.latest_attempt(node), bb.latest_attempt(node));
  assert_eq!(
    aa.latest_attempt(node),
    aa.projection().latest_attempt(node)
  );
}
