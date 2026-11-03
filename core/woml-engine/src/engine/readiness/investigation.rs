//! Diagnostic copies/components only; never a production scheduler or fast path.
use super::{
  active_node_ids, edge_is_active, node_is_complete, ready_node_ids_for_projection_at,
  workflow_settlement_can_decide,
};
use crate::engine::{
  validate_event_history_against_definition, validate_payload_against_definition,
};
use crate::projection::ProjectionFoldState;
use crate::{CompiledWorkflowDefinition, RunProjection, RunStatus};
use crate::{RunEvent, RunEventPayload};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

#[cfg(unix)]
fn cpu_ms() -> Option<f64> {
  let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
  // SAFETY: a valid aligned timespec pointer, read only after a successful call.
  if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, value.as_mut_ptr()) } != 0 {
    return None;
  }
  let value = unsafe { value.assume_init() };
  Some(value.tv_sec as f64 * 1000.0 + value.tv_nsec as f64 / 1_000_000.0)
}

#[cfg(not(unix))]
fn cpu_ms() -> Option<f64> {
  None
}

fn timed<T>(operation: impl FnOnce() -> T) -> (T, Value) {
  let before = cpu_ms();
  let started = Instant::now();
  let value = operation();
  let wall = started.elapsed().as_secs_f64() * 1000.0;
  let cpu = before.zip(cpu_ms()).map(|(a, b)| b - a);
  (value, json!({"wallMs":wall,"threadCpuMs":cpu}))
}

// Exact readiness algorithm, separated at EXISTING component boundaries for
// attribution. Existing private predicates are reused, not optimized. Differential
// tests below guard this diagnostic copy against current production behavior.
fn staged(
  workflow: &CompiledWorkflowDefinition,
  hash: &str,
  projection: &RunProjection,
  now: DateTime<Utc>,
) -> Result<(Vec<String>, Value), String> {
  if projection.status != RunStatus::Running {
    return Ok((Vec::new(), json!({})));
  }
  let matches = projection.workflow_id.as_deref() == Some(workflow.workflow_id.as_str())
    || (projection
      .event_schema_version
      .is_some_and(|version| version >= crate::RUN_EVENT_SCHEMA_VERSION_V11)
      && projection.workflow_id.is_none());
  if !matches || projection.definition_hash.as_deref() != Some(hash) {
    return Err("The run is not bound to this compiled workflow definition.".to_string());
  }
  let (active, traversal) = timed(|| active_node_ids(workflow, projection));
  let (incoming, incoming_time) = timed(|| {
    let mut incoming: HashMap<&str, Vec<&crate::model::CompiledWorkflowEdge>> = workflow
      .graph
      .nodes
      .iter()
      .map(|node| (node.id.as_str(), Vec::new()))
      .collect();
    for edge in &workflow.graph.edges {
      if active.contains(edge.from.as_str()) && edge_is_active(edge, projection) {
        incoming.entry(edge.to.as_str()).or_default().push(edge);
      }
    }
    incoming
  });
  let (ready, filtering) = timed(|| {
    workflow
      .graph
      .nodes
      .iter()
      .filter(|node| {
        active.contains(node.id.as_str())
          && !node_is_complete(workflow, node, projection)
          && (node.handler != "engine.workflow-settlement"
            || workflow_settlement_can_decide(workflow, projection))
          && match projection.latest_attempt(&node.id) {
            None => true,
            Some(attempt)
              if matches!(attempt.status, crate::projection::AttemptStatus::Started) =>
            {
              false
            }
            Some(attempt)
              if matches!(
                attempt.status,
                crate::projection::AttemptStatus::Succeeded { .. }
              ) =>
            {
              false
            }
            Some(_) => projection
              .pending_retries
              .get(&node.id)
              .is_some_and(|schedule| schedule.scheduled_at <= now),
          }
          && incoming
            .get(node.id.as_str())
            .into_iter()
            .flatten()
            .all(|edge| {
              workflow
                .node(&edge.from)
                .is_some_and(|predecessor| node_is_complete(workflow, predecessor, projection))
            })
      })
      .map(|node| node.id.clone())
      .collect()
  });
  Ok((
    ready,
    json!({"activeTraversal":traversal,"incomingConstruction":incoming_time,"nodeFiltering":filtering}),
  ))
}

fn chain(nodes: usize) -> (CompiledWorkflowDefinition, String, Vec<RunEvent>) {
  let mut model: Value = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/hello.compiled.v1.json"
  ))
  .unwrap();
  model["workflowId"] = json!("readiness-investigation");
  let mut node = model["graph"]["nodes"][0].clone();
  node["inputs"] =
    json!({"kind":"object","fields":{"source":{"kind":"literal","value":"return { value: 1 };"}}});
  model["graph"]["nodes"] = json!((0..nodes)
    .map(|index| {
      let mut node = node.clone();
      node["id"] = json!(format!("node{index}"));
      node
    })
    .collect::<Vec<_>>());
  model["graph"]["entryNodeIds"] = json!(["node0"]);
  model["graph"]["edges"] = json!((1..nodes).map(|index| json!({"id":format!("edge{index}"),"from":format!("node{}",index-1),"to":format!("node{index}"),"condition":{"kind":"always"}})).collect::<Vec<_>>());
  let model: CompiledWorkflowDefinition = serde_json::from_value(model).unwrap();
  model.validate_structure().unwrap();
  let hash = format!(
    "sha256:{:x}",
    Sha256::digest(serde_json_canonicalizer::to_vec(&model).unwrap())
  );
  let events = (1..=nodes*2).map(|sequence| {
    let index = sequence.saturating_sub(2) / 2;
    let (kind, data) = if sequence == 1 {
      ("run_started", json!({"workflowId":model.workflow_id,"definitionHash":hash,"trigger":{}}))
    } else if sequence % 2 == 0 {
      ("step_attempt_started", json!({"nodeId":format!("node{index}"),"attempt":1,"invocationId":format!("inv_probe_{index}"),"handler":"runtime.script"}))
    } else {
      ("step_attempt_succeeded", json!({"nodeId":format!("node{index}"),"attempt":1,"invocationId":format!("inv_probe_{index}"),"output":{"value":1}}))
    };
    serde_json::from_value(json!({"eventSchemaVersion":1,"eventId":format!("evt_probe_{sequence}"),"runId":"run_readiness_probe","sequence":sequence,"occurredAt":"2026-11-03T00:00:00Z","type":kind,"data":data})).unwrap()
  }).collect();
  (model, hash, events)
}

// Separate untimed counted traversal for plain successful sequential chains.
// These are deterministic predicate/lookup inspections, NOT hardware counters,
// allocator counts or instrumentation inserted into production functions.
fn counted_chain(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
) -> (Vec<String>, Value) {
  assert!(workflow
    .graph
    .nodes
    .iter()
    .all(|node| node.handler == "runtime.script"));
  assert!(projection.pending_retries.is_empty());
  let mut edge_tests = 0_u64;
  let mut active = HashSet::new();
  let mut queue: VecDeque<&str> = workflow
    .graph
    .entry_node_ids
    .iter()
    .map(String::as_str)
    .collect();
  while let Some(id) = queue.pop_front() {
    if !active.insert(id) {
      continue;
    }
    for edge in &workflow.graph.edges {
      edge_tests += 1;
      if edge.from == id && edge_is_active(edge, projection) {
        queue.push_back(edge.to.as_str());
      }
    }
  }
  assert_eq!(active, active_node_ids(workflow, projection));
  let mut incoming: HashMap<&str, Vec<&crate::model::CompiledWorkflowEdge>> = HashMap::new();
  for edge in &workflow.graph.edges {
    if active.contains(edge.from.as_str()) && edge_is_active(edge, projection) {
      incoming.entry(edge.to.as_str()).or_default().push(edge);
    }
  }
  let mut attempt_tests = 0_u64;
  let mut node_tests = 0_u64;
  let mut ready = Vec::new();
  for node in &workflow.graph.nodes {
    if !active.contains(node.id.as_str()) || node_is_complete(workflow, node, projection) {
      continue;
    }
    let latest = projection.attempts.iter().rev().find(|attempt| {
      attempt_tests += 1;
      attempt.identity.node_id == node.id
    });
    if latest.is_some() {
      continue;
    }
    if incoming
      .get(node.id.as_str())
      .into_iter()
      .flatten()
      .all(|edge| {
        workflow
          .graph
          .nodes
          .iter()
          .find(|predecessor| {
            node_tests += 1;
            predecessor.id == edge.from
          })
          .is_some_and(|predecessor| node_is_complete(workflow, predecessor, projection))
      })
    {
      ready.push(node.id.clone());
    }
  }
  (
    ready,
    json!({"outgoingEdgeInspections":edge_tests,"latestAttemptInspections":attempt_tests,"predecessorNodeInspections":node_tests,"nodeCandidates":workflow.graph.nodes.len(),"incomingEdgeInspections":workflow.graph.edges.len()}),
  )
}

#[test]
fn staged_diagnostic_matches_production_for_recorded_features_and_binding_errors() {
  let recorded: Value =
    serde_json::from_str(include_str!("../history/tests/profile-histories.json")).unwrap();
  let now = DateTime::parse_from_rfc3339("2026-11-03T00:00:00Z")
    .unwrap()
    .with_timezone(&Utc);
  let mut cases = 0;
  let mut histories = recorded["histories"].as_array().unwrap().clone();
  let retry_model: Value = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/retry.compiled.v6.json"
  ))
  .unwrap();
  let mut retry_events: Value = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/run-events/retry-success.events.v6.json"
  ))
  .unwrap();
  // This frozen projection-only fixture omits the prerequisite `prepare`.
  // Complete an independent in-memory history; do not rewrite the golden file.
  let mut prepare_start = retry_events[0].clone();
  prepare_start["eventId"] = json!("evt_readiness_prepare_start");
  prepare_start["type"] = json!("step_attempt_started");
  prepare_start["data"] = json!({"nodeId":"prepare","attempt":1,"invocationId":"inv_readiness_prepare","handler":"runtime.script","idempotencyKey":crate::step_effect_idempotency_key(retry_events[0]["runId"].as_str().unwrap(),retry_events[0]["data"]["definitionHash"].as_str().unwrap(),"prepare")});
  let mut prepare_done = prepare_start.clone();
  prepare_done["eventId"] = json!("evt_readiness_prepare_done");
  prepare_done["type"] = json!("step_attempt_succeeded");
  prepare_done["data"] = json!({"nodeId":"prepare","attempt":1,"invocationId":"inv_readiness_prepare","output":{"name":"World"}});
  retry_events
    .as_array_mut()
    .unwrap()
    .splice(1..1, [prepare_start, prepare_done]);
  for (index, event) in retry_events.as_array_mut().unwrap().iter_mut().enumerate() {
    event["sequence"] = json!(index + 1);
  }
  validate_event_history_against_definition(
    &serde_json::from_value(retry_model.clone()).unwrap(),
    retry_events[0]["data"]["definitionHash"].as_str().unwrap(),
    &serde_json::from_value::<Vec<RunEvent>>(retry_events.clone()).unwrap(),
  )
  .unwrap();
  histories.push(json!({"model":retry_model,"definitionHash":retry_events[0]["data"]["definitionHash"],"events":retry_events}));
  let mut clock_sensitive = false;
  for history in &histories {
    let model = serde_json::from_value(history["model"].clone()).unwrap();
    let events: Vec<RunEvent> = serde_json::from_value(history["events"].clone()).unwrap();
    for end in 0..=events.len() {
      let Ok(projection) = crate::fold_events(&events[..end]) else {
        continue;
      };
      for hash in [
        history["definitionHash"].as_str().unwrap(),
        "wrong-definition",
      ] {
        let event_time = end
          .checked_sub(1)
          .map(|index| events[index].occurred_at)
          .unwrap_or(now);
        let at_event = ready_node_ids_for_projection_at(&model, hash, &projection, event_time);
        let later = ready_node_ids_for_projection_at(&model, hash, &projection, now);
        clock_sensitive |= at_event != later;
        for time in [event_time, now] {
          assert_eq!(
            staged(&model, hash, &projection, time).map(|(ready, _)| ready),
            ready_node_ids_for_projection_at(&model, hash, &projection, time)
          );
          cases += 1;
        }
      }
    }
  }
  assert!(cases > 400);
  assert!(
    clock_sensitive,
    "Retry deadlines must exercise different readiness results"
  );
}

#[test]
fn counted_chain_matches_production_and_closed_form_work() {
  let (model, hash, events) = chain(16);
  let now = events[0].occurred_at;
  for completed in 0..16 {
    let state = crate::fold_events(&events[..1 + 2 * completed]).unwrap();
    let (ready, counts) = counted_chain(&model, &state);
    assert_eq!(ready, vec![format!("node{completed}")]);
    assert_eq!(
      ready,
      ready_node_ids_for_projection_at(&model, &hash, &state, now).unwrap()
    );
    assert_eq!(counts["outgoingEdgeInspections"], json!(16 * 15));
    assert_eq!(
      counts["latestAttemptInspections"],
      json!((16 - completed) * completed)
    );
    assert_eq!(
      counts["predecessorNodeInspections"],
      json!((completed.max(1)..16).sum::<usize>())
    );
  }
}

#[test]
#[ignore = "explicit optimized diagnosis; no production fast path"]
fn capture() {
  assert_eq!(
    std::env::var("WOML_PROFILE").ok(),
    None,
    "Do not mix existing tracing with component clocks"
  );
  let output = std::env::var_os("WOML_READINESS_CAPTURE").expect("WOML_READINESS_CAPTURE required");
  let mut components = Vec::new();
  let mut histories = Vec::new();
  for nodes in [16, 64, 256, 1024] {
    let (model, hash, events) = chain(nodes);
    // Verify generated inputs outside measurements, never rewrite old fixtures.
    validate_event_history_against_definition(&model, &hash, &events).unwrap();
    let fixture_hash = format!(
      "{:x}",
      Sha256::digest(serde_json::to_vec(&(&model, &hash, &events)).unwrap())
    );
    for completed in [0, nodes / 2, nodes - 1] {
      let state = crate::fold_events(&events[..1 + 2 * completed]).unwrap();
      let now = events[0].occurred_at;
      let expected = ready_node_ids_for_projection_at(&model, &hash, &state, now).unwrap();
      let (counted, counts) = counted_chain(&model, &state);
      assert_eq!(counted, expected);
      for sample in 0..5 {
        let ((actual, whole), (staged_ready, stages)) = if sample % 2 == 0 {
          (
            timed(|| ready_node_ids_for_projection_at(&model, &hash, &state, now).unwrap()),
            staged(&model, &hash, &state, now).unwrap(),
          )
        } else {
          let staged = staged(&model, &hash, &state, now).unwrap();
          (
            timed(|| ready_node_ids_for_projection_at(&model, &hash, &state, now).unwrap()),
            staged,
          )
        };
        assert_eq!(actual, expected);
        assert_eq!(staged_ready, expected);
        components.push(json!({"nodes":nodes,"completed":completed,"historyEvents":1+2*completed,"sample":sample,"warmup":sample<2,"fixtureSha256":fixture_hash,"wholeQuery":whole,"stages":stages,"countedInspections":counts}));
      }
    }
    // Independent axes: full active history vs 17-event history with the same
    // increasingly wide graph. These are NEW exploratory cohorts, not a silent
    // replacement for VSR0's frozen acceptance cohort.
    for (name, length) in [("full-active", events.len()), ("fixed-17-events", 17)] {
      let events = &events[..length];
      validate_event_history_against_definition(&model, &hash, events).unwrap();
      let (_, whole) =
        timed(|| validate_event_history_against_definition(&model, &hash, events).unwrap());
      let (_, payloads) = timed(|| {
        for event in events {
          validate_payload_against_definition(&model, &hash, &event.payload).unwrap();
        }
      });
      let (_, fold) = timed(|| crate::fold_events(events).unwrap());
      let mut queries = 0;
      let mut visits = 0;
      let (_, readiness_replay) = timed(|| {
        let mut state = ProjectionFoldState::default();
        for event in events {
          if let RunEventPayload::StepAttemptStarted(data) = &event.payload {
            let ready = ready_node_ids_for_projection_at(
              &model,
              &hash,
              state.projection(),
              event.occurred_at,
            )
            .unwrap();
            assert!(ready.contains(&data.node_id));
            queries += 1;
          }
          state.apply(event).unwrap();
          visits += 1;
        }
      });
      histories.push(json!({"cohort":name,"nodes":nodes,"events":length,"queries":queries,"prefixVisits":visits,"fixtureSha256":fixture_hash,"wholeValidation":whole,"payloadChecks":payloads,"structuralFold":fold,"readinessAndPrefixReplay":readiness_replay}));
    }
  }
  std::fs::write(output, serde_json::to_vec_pretty(&json!({"profile":"woml.readiness-investigation/v1","components":components,"histories":histories,"scope":"Synthetic Model/Event v1 sequential chains; no candidate optimization, native execution or allocation measurement"})).unwrap()).unwrap();
}
