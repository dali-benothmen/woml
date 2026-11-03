//! Actual converted boundaries: scoped fold counts, raw/public views and rollback.
use super::{running_store, DurableEventStore, DurableStoreError, HASH, MODEL, RUN};
use crate::projection::fold_counts::observe;
use crate::{CompiledWorkflowDefinition, RunEventPayload, RunStatus};
use chrono::{DateTime, Utc};
use serde_json::Map;
use std::time::Duration;

#[test]
fn cache_miss_reuses_one_complete_fold_and_hits_do_not_fold() {
  let store = running_store();
  let expected = store.projection(RUN).unwrap();
  store.projection_cache.borrow_mut().clear();
  let (actual, counts) = observe(|| store.projection(RUN));
  assert_eq!(actual.unwrap(), expected);
  assert_eq!(counts, vec![1]);
  let (hit, counts) = observe(|| store.projection(RUN));
  assert_eq!(hit.unwrap(), expected);
  assert!(counts.is_empty());
  assert_eq!(
    store.projection_cache.borrow()[RUN].last_event_id(),
    Some("evt_store_boundary_start")
  );
}

#[test]
fn terminal_checked_states_are_not_retained_in_the_active_cache() {
  let events: Vec<crate::RunEvent> = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/run-events/hello.events.v1.json"
  ))
  .unwrap();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let model: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  store.register_definition(&model, HASH).unwrap();
  let run = &events[0].run_id;
  store
    .start_run(
      &events[0].event_id,
      run,
      events[0].occurred_at,
      &model.workflow_id,
      HASH,
      Map::new(),
    )
    .unwrap();
  for event in &events[1..] {
    store
      .append_payload(
        run,
        &event.event_id,
        event.occurred_at,
        event.payload.clone(),
      )
      .unwrap();
  }
  for _ in 0..2 {
    let (projection, counts) = observe(|| store.projection(run));
    assert_eq!(projection.unwrap().status, RunStatus::Succeeded);
    assert_eq!(counts, vec![events.len()]);
    assert!(!store.projection_cache.borrow().contains_key(run));
  }
}

#[test]
fn failed_cache_reload_keeps_loader_errors_and_publishes_no_checked_state() {
  let store = running_store();
  store.projection_cache.borrow_mut().clear();
  // Offline-corruption injection in this isolated in-memory test database only.
  // Normal production writes must still be blocked by the append-only trigger.
  store
    .connection
    .execute_batch("DROP TRIGGER woml_run_events_no_update;")
    .unwrap();
  store
    .connection
    .execute(
      "UPDATE woml_run_events SET sequence = 99 WHERE run_id = ?1",
      [RUN],
    )
    .unwrap();
  let (result, counts) = observe(|| store.projection(RUN));
  assert!(
    matches!(result, Err(DurableStoreError::InvalidStoredEvent(ref error)) if error == "indexed event columns do not match event_json")
  );
  assert!(
    counts.is_empty(),
    "Loading must reject before the new constructor"
  );
  assert!(!store.projection_cache.borrow().contains_key(RUN));
  store
    .connection
    .execute(
      "UPDATE woml_run_events SET sequence = 1 WHERE run_id = ?1",
      [RUN],
    )
    .unwrap();
  assert_eq!(observe(|| store.projection(RUN)).1, vec![1]);
}

fn policy_store(model: CompiledWorkflowDefinition) -> (DurableEventStore, String, DateTime<Utc>) {
  let now = Utc::now();
  let mut store = DurableEventStore::open_in_memory().unwrap();
  store.register_definition(&model, HASH).unwrap();
  let admitted = store
    .admit_trigger_occurrence(crate::TriggerAdmissionRequest {
      workflow_id: model.workflow_id.clone(),
      definition_hash: HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "manual:validated-state".to_string(),
      payload: Map::from_iter([("orderId".to_string(), serde_json::json!("order-1"))]),
      received_at: now,
    })
    .unwrap();
  (store, admitted.run_id, now)
}

#[test]
fn policy_claim_reuses_only_the_initial_snapshot_and_rechecks_appended_start() {
  let model = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  let (mut store, run, now) = policy_store(model);
  let (claim, counts) =
    observe(|| store.try_claim_policy_run(&run, "owner", now, Duration::from_secs(30)));
  assert!(matches!(
    claim.unwrap(),
    crate::PolicyExecutionClaimResult::Claimed {
      first_start: true,
      ..
    }
  ));
  // Initial checked history once, then unchanged append/summary/policy-index
  // folds plus COMPLETE final validation of the new two-event history.
  assert_eq!(counts, vec![1, 2, 2, 2, 2]);
  let (claim, counts) =
    observe(|| store.try_claim_policy_run(&run, "owner", now, Duration::from_secs(30)));
  assert!(matches!(
    claim.unwrap(),
    crate::PolicyExecutionClaimResult::Claimed {
      first_start: false,
      ..
    }
  ));
  assert_eq!(counts, vec![2]);
  let (other, counts) =
    observe(|| store.try_claim_policy_run(&run, "other-owner", now, Duration::from_secs(30)));
  assert!(matches!(
    other.unwrap(),
    crate::PolicyExecutionClaimResult::Waiting { .. }
  ));
  assert_eq!(counts, vec![2]);
  store.projection_cache.borrow_mut().clear();
  let (public, counts) = observe(|| store.projection(&run));
  assert_eq!(counts, vec![2]);
  assert_eq!(public.unwrap().workflow_id.as_deref(), Some("policy-demo"));
  assert_eq!(
    store.projection_cache.borrow()[&run]
      .projection()
      .workflow_id,
    None,
    "Compatibility enrichment belongs to the public clone, not raw fold state"
  );
}

#[test]
fn wait_registration_reuses_resulting_history_and_late_sql_failure_rolls_back() {
  let model = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/durable-waits/model.v17.reviewed.json"
  ))
  .unwrap();
  let (mut store, run, now) = policy_store(model);
  store
    .try_claim_policy_run(&run, "owner", now, Duration::from_secs(30))
    .unwrap();
  let before = store.events(&run).unwrap();
  let previous = store.projection(&run).unwrap();
  let registration = crate::WaitRegisteredData {
    wait_id: "pause".to_string(),
    node_id: "pause".to_string(),
    kind: crate::DurableWaitKind::Duration,
    registered_at: now,
    deadline: Some(now + chrono::Duration::seconds(1)),
    signals: None,
    correlation_digest: None,
  };
  store.connection.execute_batch("CREATE TEMP TRIGGER reject_vsr_wait BEFORE INSERT ON woml_durable_waits BEGIN SELECT RAISE(ABORT, 'VSR late wait failure'); END;").unwrap();
  let (failed, counts) = observe(|| {
    store.register_durable_wait_atomically(&run, "evt_vsr_wait", now, registration.clone())
  });
  assert!(matches!(failed, Err(DurableStoreError::Sqlite(_))));
  assert_eq!(counts, vec![3, 3, 3, 3]);
  assert_eq!(store.events(&run).unwrap(), before);
  assert_eq!(store.projection(&run).unwrap(), previous);
  assert_eq!(
    store.projection_cache.borrow()[&run]
      .projection()
      .last_sequence,
    2
  );
  store
    .connection
    .execute_batch("DROP TRIGGER reject_vsr_wait;")
    .unwrap();
  let (waiting, counts) =
    observe(|| store.register_durable_wait_atomically(&run, "evt_vsr_wait", now, registration));
  assert_eq!(counts, vec![3, 3, 3, 3]);
  assert_eq!(waiting.unwrap().status, RunStatus::Waiting);
  let events = store.events(&run).unwrap();
  assert!(matches!(
    events.last().unwrap().payload,
    RunEventPayload::WaitRegistered(_)
  ));
  assert_eq!(store.projection(&run).unwrap().status, RunStatus::Waiting);
}

#[test]
fn hydrated_review_is_public_only_and_missing_review_blocks_cache_miss_publication() {
  struct NoArtifacts;
  impl crate::RichApprovalArtifactVerifier for NoArtifacts {
    fn verify_review_artifact(
      &self,
      _: &str,
      _: &crate::ArtifactReferenceV1,
    ) -> Result<crate::ArtifactReferenceV1, crate::RichApprovalError> {
      panic!("This fixture contains no artifact references");
    }
  }
  let model = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/rich-plan-approvals/model.v20.reviewed.json"
  ))
  .unwrap();
  let (mut store, run, now) = policy_store(model);
  store
    .try_claim_policy_run(&run, "owner", now, Duration::from_secs(30))
    .unwrap();
  let review: serde_json::Value = serde_json::from_str(include_str!(
    "../../../../../woml/tests/fixtures/rich-plan-approvals/review.v1.json"
  ))
  .unwrap();
  store
    .append_payload(
      &run,
      "evt_vsr_build",
      now,
      RunEventPayload::StepAttemptStarted(crate::StepAttemptStartedData {
        node_id: "buildReview".to_string(),
        attempt: 1,
        invocation_id: "inv_vsr_review".to_string(),
        handler: "runtime.script".to_string(),
        idempotency_key: Some(crate::step_effect_idempotency_key(
          &run,
          HASH,
          "buildReview",
        )),
      }),
    )
    .unwrap();
  store
    .append_payload(
      &run,
      "evt_vsr_built",
      now,
      RunEventPayload::StepAttemptSucceeded(crate::StepAttemptSucceededData {
        node_id: "buildReview".to_string(),
        attempt: 1,
        invocation_id: "inv_vsr_review".to_string(),
        output: serde_json::json!({"review":review}),
      }),
    )
    .unwrap();
  let binding = crate::bind_rich_approval_review(
    crate::RichApprovalBindingInput {
      definition_hash: HASH.to_string(),
      workflow_id: "production-deployment".to_string(),
      workflow_version: "1.0.0".to_string(),
      run_id: run.clone(),
      approval_id: "approveDeployment".to_string(),
      request_id: "aprreq_vsr_review".to_string(),
      review,
      required_approvals: 2,
      required_roles: vec!["production-owner".to_string(), "security-owner".to_string()],
      directory_version: "directory-2026-10-21".to_string(),
    },
    &NoArtifacts,
    &[],
  )
  .unwrap();
  store.create_rich_approval_request(&binding, now).unwrap();
  store
    .settle_rich_approval_without_vote(
      &run,
      &binding.request_id,
      &binding.review_digest,
      crate::RichApprovalResolutionReason::Cancellation,
      now,
    )
    .unwrap();
  let length = store.events(&run).unwrap().len();
  store.projection_cache.borrow_mut().clear();
  let (public, counts) = observe(|| store.projection(&run));
  assert_eq!(counts, vec![length]);
  let public = public.unwrap();
  assert_eq!(
    public.context.steps["approveDeployment"]["review"]["title"],
    "Deploy payments-api to production"
  );
  assert_eq!(store.projection_cache.borrow()[&run]
    .projection()
    .context
    .steps["approveDeployment"]["review"], serde_json::json!({"digest":binding.review_digest}),
    "The raw approval output retains only its event-recorded digest, not hydrated title/risk/artifacts");
  assert_eq!(observe(|| store.projection(&run)).1, Vec::<usize>::new());
  // Simulate offline corruption only in this isolated memory database. Normal
  // retention and immutable-review protections are not weakened in production.
  store
    .connection
    .execute_batch("DROP TRIGGER woml_approval_reviews_no_delete;")
    .unwrap();
  store
    .connection
    .execute(
      "DELETE FROM woml_approval_reviews WHERE run_id = ?1",
      [&run],
    )
    .unwrap();
  let missing = "A settled rich approval has no immutable review snapshot.";
  let (hit, counts) = observe(|| store.projection(&run));
  assert!(matches!(hit, Err(DurableStoreError::Contract(ref error)) if error == missing));
  assert!(counts.is_empty());
  store.projection_cache.borrow_mut().clear();
  let (miss, counts) = observe(|| store.projection(&run));
  assert!(matches!(miss, Err(DurableStoreError::Contract(ref error)) if error == missing));
  assert_eq!(counts, vec![length]);
  assert!(!store.projection_cache.borrow().contains_key(&run));
}
