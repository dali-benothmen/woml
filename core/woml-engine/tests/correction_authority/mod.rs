//! Real durable correction attempts, not fabricated ordinary step attempts.

use super::DEFINITION_HASH;
use chrono::Utc;
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use std::sync::{
  atomic::{AtomicUsize, Ordering},
  Arc,
};
use tokio::sync::Mutex;
use woml_engine::{
  capability, derive_operation_key, durable, prepare_infrastructure_policy,
  step_effect_idempotency_key, CapabilityCallIdentity, CapabilityCallLimits, CapabilityCallRequest,
  CapabilityCallResult, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityHandler, CapabilityRegistry, CompiledWorkflowDefinition,
  CorrectionActionStatus, CorrectionAttemptFailedData, CorrectionAttemptIdentityData,
  CorrectionInterruptedData, CorrectionRecoveryActionV1, CorrectionRecoveryRequestV1,
  CorrectionRetryScheduledData, CorrectiveActionKind, DurableCapabilityAuthority, DurableDagEngine,
  DurableEventStore, DurableStateStore, InfrastructurePolicyExecutionModeV1,
  InfrastructurePolicyTrustV1, ManagedLocksService, NativeFetchInvocationIdentity,
  NativeFetchObservation, OperationExecutionMode, OperationStartedData, OperationStatus,
  PreparedInfrastructurePolicyV1, RunEventPayload, RunStatus, SagaCorrectionReason,
  SagaCorrectionStartedData, SagaIdentityData, StepAttemptSucceededData, TestCapabilityHandler,
  TriggerAdmissionRequest, CORRECTION_RECOVERY_REQUEST_PROFILE,
};

const SAGA: &str = "provision";
const COMPENSATE: &str = "provision:compensate:createNetwork";
const CLEANUP: &str = "provision:cleanup:revokeCredentials";

fn append(store: &mut DurableEventStore, run: &str, payload: RunEventPayload) {
  store
    .append_payload(
      run,
      format!("evt_{}", uuid::Uuid::new_v4().simple()),
      Utc::now(),
      payload,
    )
    .unwrap();
}

fn active_correction(
  store: DurableEventStore,
  kind: CorrectiveActionKind,
) -> (DurableEventStore, String) {
  prepare_store(store, Some(kind))
}

fn prepare_store(
  store: DurableEventStore,
  kind: Option<CorrectiveActionKind>,
) -> (DurableEventStore, String) {
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/compensation-and-cleanup/model.v19.reviewed.json"
  ))
  .unwrap();
  prepare_workflow_store(store, kind, workflow)
}

fn prepare_workflow_store(
  store: DurableEventStore,
  kind: Option<CorrectiveActionKind>,
  workflow: CompiledWorkflowDefinition,
) -> (DurableEventStore, String) {
  let mut store = store;
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .unwrap();
  let run = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: DEFINITION_HASH.into(),
      trigger_id: "start".into(),
      trigger_handler: "trigger.manual".into(),
      source_identity: format!("f02_{}", uuid::Uuid::new_v4().simple()),
      payload: Map::new(),
      received_at: Utc::now(),
    })
    .unwrap()
    .run_id;
  store
    .claim_policy_run(
      &run,
      "f02-owner",
      Utc::now(),
      std::time::Duration::from_secs(30),
    )
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &run).unwrap();
  engine
    .start_step_attempt(&run, "prepare", 1, "inv_prepare", Utc::now())
    .unwrap();
  let Some(kind) = kind else {
    return (engine.into_store(), run);
  };
  engine
    .append_payload(
      "evt_prepare",
      &run,
      Utc::now(),
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: "prepare".into(),
        attempt: 1,
        invocation_id: "inv_prepare".into(),
        output: json!({"pullRequest": 42}),
      }),
    )
    .unwrap();
  if let Some(lock) = workflow
    .graph
    .locks
    .as_deref()
    .and_then(|locks| locks.first())
  {
    let now = Utc::now();
    engine
      .register_lock_waiter(
        &run,
        &lock.lock_id,
        "f02/releases",
        now,
        now + chrono::Duration::minutes(1),
      )
      .unwrap();
  }
  engine
    .append_payload(
      "evt_saga",
      &run,
      Utc::now(),
      RunEventPayload::SagaStarted(SagaIdentityData {
        saga_id: SAGA.into(),
      }),
    )
    .unwrap();
  engine
    .start_step_attempt(&run, "createNetwork", 1, "inv_network", Utc::now())
    .unwrap();
  engine
    .record_step_success_with_compensation(
      &run,
      StepAttemptSucceededData {
        node_id: "createNetwork".into(),
        attempt: 1,
        invocation_id: "inv_network".into(),
        output: json!({"networkId": "network-42"}),
      },
      SAGA,
      COMPENSATE,
    )
    .unwrap();
  let mut store = engine.into_store();
  append(
    &mut store,
    &run,
    RunEventPayload::SagaCorrectionStarted(SagaCorrectionStartedData {
      saga_id: SAGA.into(),
      reason: SagaCorrectionReason::Failure,
    }),
  );
  append(
    &mut store,
    &run,
    RunEventPayload::CorrectionAttemptStarted(identity(
      COMPENSATE,
      CorrectiveActionKind::Compensation,
      1,
    )),
  );
  if kind == CorrectiveActionKind::Cleanup {
    append(
      &mut store,
      &run,
      RunEventPayload::CorrectionAttemptSucceeded(identity(
        COMPENSATE,
        CorrectiveActionKind::Compensation,
        1,
      )),
    );
    append(
      &mut store,
      &run,
      RunEventPayload::CorrectionAttemptStarted(identity(CLEANUP, kind, 1)),
    );
  }
  (store, run)
}

fn identity(
  action: &str,
  kind: CorrectiveActionKind,
  attempt: u32,
) -> CorrectionAttemptIdentityData {
  CorrectionAttemptIdentityData {
    saga_id: SAGA.into(),
    action_id: action.into(),
    action_kind: kind,
    attempt,
  }
}

fn call(run: &str, action: &str, attempt: u32, call_id: &str) -> CapabilityCallRequest {
  let invocation = woml_engine::durable::correction_invocation_id(run, SAGA, action, attempt);
  let key = format!("sha256:{:x}", Sha256::digest(invocation.as_bytes()));
  let name = "process.run.corrective-tool";
  CapabilityCallRequest {
    contract: "woml.capability-call".into(),
    contract_version: 1,
    message_type: "request".into(),
    invocation_id: invocation,
    call_id: call_id.into(),
    run_id: run.into(),
    node_id: action.into(),
    attempt_number: attempt,
    capability: "process".into(),
    operation: "run".into(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: capability::CapabilityIdentityMode::Named,
      step_idempotency_key: key.clone(),
      operation_name: name.into(),
      operation_key: derive_operation_key(&key, name),
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits::default(),
    input: json!({"executable": "terraform", "args": ["apply"], "cwd": "."}),
  }
}

struct CountingHandler(Arc<AtomicUsize>);

impl CapabilityHandler for CountingHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "process".into(),
      operation: "run".into(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }
  fn execute(
    &self,
    _: serde_json::Value,
    _: CapabilityCancellationToken,
  ) -> futures_util::future::BoxFuture<'static, Result<serde_json::Value, CapabilityFailure>> {
    self.0.fetch_add(1, Ordering::SeqCst);
    Box::pin(async { Ok(json!({"exitCode": 0})) })
  }
}

fn authority(
  store: DurableEventStore,
) -> (
  Arc<Mutex<DurableEventStore>>,
  DurableCapabilityAuthority,
  Arc<AtomicUsize>,
) {
  let store = Arc::new(Mutex::new(store));
  let calls = Arc::new(AtomicUsize::new(0));
  let registry = Arc::new(CapabilityRegistry::default());
  registry
    .register(Arc::new(CountingHandler(calls.clone())))
    .unwrap();
  let authority = DurableCapabilityAuthority::new(registry, store.clone());
  (store, authority, calls)
}

#[tokio::test]
async fn compensation_and_cleanup_authorize_exact_attempts_and_commit_operations() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let (store, authority, calls) = authority(store);
    let request = call(&run, action, 1, "call_corrective");
    let result = authority
      .execute(request.clone(), CapabilityCancellationToken::default())
      .await;
    assert!(
      matches!(result, Ok(CapabilityCallResult::Succeeded(_))),
      "{kind:?}: {result:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut store = store.lock().await;
    let events = store.events(&run).unwrap();
    let operation = store
      .projection(&run)
      .unwrap()
      .operations
      .values()
      .next()
      .unwrap()
      .clone();
    assert_eq!(operation.node_id, action);
    assert_eq!(operation.identity.invocation_id, request.invocation_id);
    assert_eq!(operation.operation_key, request.identity.operation_key);
    assert!(matches!(
      operation.status,
      OperationStatus::Succeeded { .. }
    ));
    append(
      &mut store,
      &run,
      RunEventPayload::CorrectionAttemptSucceeded(identity(action, kind, 1)),
    );
    assert_eq!(
      events
        .iter()
        .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
        .count(),
      1
    );
  }
}

#[tokio::test]
async fn forged_corrective_identities_never_dispatch_or_append_history() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let (store, authority, calls) = authority(store);
    let valid = call(&run, action, 1, "call_forged");
    let before = store.lock().await.events(&run).unwrap();
    for mutation in 0..7 {
      let mut forged = valid.clone();
      match mutation {
        0 => forged.invocation_id = "inv_forged".into(),
        1 => forged.attempt_number = 2,
        2 => {
          forged.node_id = if action == COMPENSATE {
            CLEANUP
          } else {
            COMPENSATE
          }
          .into()
        }
        3 => {
          forged.identity.step_idempotency_key =
            step_effect_idempotency_key(&run, DEFINITION_HASH, action)
        }
        4 => forged.invocation_id = durable::correction_invocation_id("run_other", SAGA, action, 1),
        5 => {
          forged.invocation_id = durable::correction_invocation_id(&run, "other-saga", action, 1)
        }
        _ => {
          forged.identity.operation_key =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into()
        }
      }
      let result = authority
        .execute(forged, CapabilityCancellationToken::default())
        .await;
      assert!(
        !matches!(result, Ok(CapabilityCallResult::Succeeded(_))),
        "mutation {mutation}: {result:?}"
      );
      assert_eq!(store.lock().await.events(&run).unwrap(), before);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
  }
}

#[tokio::test]
async fn duplicate_and_settled_corrective_calls_cannot_repeat_the_effect() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let (store, authority, calls) = authority(store);
    let request = call(&run, action, 1, "call_once");
    assert!(matches!(
      authority
        .execute(request.clone(), CapabilityCancellationToken::default())
        .await
        .unwrap(),
      CapabilityCallResult::Succeeded(_)
    ));
    let before = store.lock().await.events(&run).unwrap();
    assert!(authority
      .execute(request.clone(), CapabilityCancellationToken::default())
      .await
      .is_err());
    let mut duplicate_key = request.clone();
    duplicate_key.call_id = "new_transport_same_logical_operation".into();
    assert!(authority
      .execute(duplicate_key, CapabilityCancellationToken::default())
      .await
      .is_err());
    assert_eq!(store.lock().await.events(&run).unwrap(), before);
    append(
      &mut *store.lock().await,
      &run,
      RunEventPayload::CorrectionAttemptSucceeded(identity(action, kind, 1)),
    );
    assert!(authority
      .execute(
        call(&run, action, 1, "call_after_settlement"),
        CapabilityCancellationToken::default()
      )
      .await
      .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
  }
}

#[tokio::test]
async fn corrective_retries_use_distinct_exact_attempt_and_operation_identities() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let (store, authority, calls) = authority(store);
    let first = call(&run, action, 1, "call_retry_1");
    assert!(matches!(
      authority
        .execute(first.clone(), CapabilityCancellationToken::default())
        .await
        .unwrap(),
      CapabilityCallResult::Succeeded(_)
    ));
    {
      let mut store = store.lock().await;
      append(
        &mut store,
        &run,
        RunEventPayload::CorrectionAttemptFailed(CorrectionAttemptFailedData {
          saga_id: SAGA.into(),
          action_id: action.into(),
          action_kind: kind,
          attempt: 1,
          code: "WOML_SCRIPT_THROWN".into(),
          retryable: true,
          ambiguous: false,
        }),
      );
      let retry_at = Utc::now();
      store
        .append_payload(
          &run,
          "evt_retry",
          retry_at,
          RunEventPayload::CorrectionRetryScheduled(CorrectionRetryScheduledData {
            saga_id: SAGA.into(),
            action_id: action.into(),
            action_kind: kind,
            attempt: 1,
            retry_at,
          }),
        )
        .unwrap();
      append(
        &mut store,
        &run,
        RunEventPayload::CorrectionAttemptStarted(identity(action, kind, 2)),
      );
    }
    assert!(authority
      .execute(
        call(&run, action, 1, "stale_retry"),
        CapabilityCancellationToken::default()
      )
      .await
      .is_err());
    let second = call(&run, action, 2, "call_retry_2");
    assert_ne!(first.invocation_id, second.invocation_id);
    assert_ne!(first.identity.operation_key, second.identity.operation_key);
    assert!(matches!(
      authority
        .execute(second, CapabilityCancellationToken::default())
        .await
        .unwrap(),
      CapabilityCallResult::Succeeded(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
  }
}

fn policy(decision: &str) -> PreparedInfrastructurePolicyV1 {
  prepare_infrastructure_policy(serde_json::from_value(json!({
    "contract": "woml.infrastructure-policy", "contractVersion": 1, "id": "f02-policy", "version": "1", "environment": "test",
    "defaults": {"read": decision, "idempotent-write": decision, "unsafe-write": decision}, "rules": []
  })).unwrap(), &InfrastructurePolicyTrustV1::default()).unwrap()
}

#[tokio::test]
async fn corrective_effects_keep_policy_decisions_and_execution_modes() {
  for (kind, action, mode) in [
    (
      CorrectiveActionKind::Compensation,
      COMPENSATE,
      InfrastructurePolicyExecutionModeV1::Compensation,
    ),
    (
      CorrectiveActionKind::Cleanup,
      CLEANUP,
      InfrastructurePolicyExecutionModeV1::Cleanup,
    ),
  ] {
    for decision in ["allow", "deny"] {
      let (mut store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
      store
        .bind_infrastructure_policy(&run, &policy(decision), Utc::now())
        .unwrap();
      let (store, authority, calls) = authority(store);
      let result = authority
        .execute(
          call(&run, action, 1, "call_policy"),
          CapabilityCancellationToken::default(),
        )
        .await
        .unwrap();
      if decision == "deny" {
        assert!(
          matches!(result, CapabilityCallResult::Failed(result) if result.error.code == "WOML_POLICY_DENIED")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(store
          .lock()
          .await
          .projection(&run)
          .unwrap()
          .operations
          .is_empty());
      } else {
        assert!(matches!(result, CapabilityCallResult::Succeeded(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
      }
      let projection = store.lock().await.projection(&run).unwrap();
      assert_eq!(projection.infrastructure_policy_decisions.len(), 1);
      assert_eq!(
        projection.infrastructure_policy_decisions[0]
          .decision
          .operation
          .execution_mode,
        mode
      );
    }
  }
}

#[tokio::test]
async fn cancellation_allows_corrective_effects_but_not_new_forward_effects() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (mut store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    store
      .request_run_cancellation(&run, "f02-cancel", Utc::now())
      .unwrap();
    let (store, authority, calls) = authority(store);
    assert!(matches!(
      authority
        .execute(
          call(&run, action, 1, "call_cancel_cleanup"),
          CapabilityCancellationToken::default()
        )
        .await
        .unwrap(),
      CapabilityCallResult::Succeeded(_)
    ));
    append(
      &mut *store.lock().await,
      &run,
      RunEventPayload::CorrectionAttemptSucceeded(identity(action, kind, 1)),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
  }
  let (mut store, run) = prepare_store(DurableEventStore::open_in_memory().unwrap(), None);
  store
    .request_run_cancellation(&run, "cancel_forward", Utc::now())
    .unwrap();
  assert_eq!(
    store.projection(&run).unwrap().status,
    RunStatus::Cancelling
  );
  let (store, authority, calls) = authority(store);
  let mut forward = call(&run, "prepare", 1, "call_forward");
  forward.invocation_id = "inv_prepare".into();
  forward.identity.step_idempotency_key =
    step_effect_idempotency_key(&run, DEFINITION_HASH, "prepare");
  forward.identity.operation_name = "process.run.forward".into();
  forward.identity.operation_key = derive_operation_key(
    &forward.identity.step_idempotency_key,
    &forward.identity.operation_name,
  );
  let result = authority
    .execute(forward, CapabilityCancellationToken::default())
    .await;
  assert!(result.is_err(), "cancelled forward call: {result:?}");
  assert_eq!(calls.load(Ordering::SeqCst), 0);
  assert!(store
    .lock()
    .await
    .projection(&run)
    .unwrap()
    .operations
    .is_empty());
}

fn operation_start(request: &CapabilityCallRequest) -> RunEventPayload {
  RunEventPayload::OperationStarted(OperationStartedData {
    node_id: request.node_id.clone(),
    attempt_number: request.attempt_number,
    invocation_id: request.invocation_id.clone(),
    call_id: request.call_id.clone(),
    operation_key: request.identity.operation_key.clone(),
    capability: request.capability.clone(),
    operation: request.operation.clone(),
    execution_mode: OperationExecutionMode::Managed,
    metadata: Map::new(),
  })
}

#[test]
fn corrective_history_rejects_forged_operations_and_premature_attempt_settlement() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (mut store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let mut request = call(&run, action, 1, "call_open");
    request.invocation_id = "inv_forged".into();
    let before = store.events(&run).unwrap();
    assert!(store
      .append_payload(
        &run,
        "evt_forged_operation",
        Utc::now(),
        operation_start(&request)
      )
      .is_err());
    assert_eq!(store.events(&run).unwrap(), before);
    let request = call(&run, action, 1, "call_open");
    append(&mut store, &run, operation_start(&request));
    let before = store.events(&run).unwrap();
    for payload in [
      RunEventPayload::CorrectionAttemptSucceeded(identity(action, kind, 1)),
      RunEventPayload::CorrectionAttemptFailed(CorrectionAttemptFailedData {
        saga_id: SAGA.into(),
        action_id: action.into(),
        action_kind: kind,
        attempt: 1,
        code: "WOML_SCRIPT_THROWN".into(),
        retryable: false,
        ambiguous: false,
      }),
      RunEventPayload::CorrectionInterrupted(CorrectionInterruptedData {
        saga_id: SAGA.into(),
        action_id: action.into(),
        action_kind: kind,
        attempt: 1,
        ambiguous: true,
      }),
    ] {
      assert!(store
        .append_payload(
          &run,
          format!("evt_bad_{}", uuid::Uuid::new_v4().simple()),
          Utc::now(),
          payload
        )
        .is_err());
      assert_eq!(store.events(&run).unwrap(), before);
    }
  }
}

struct TestDirectory(std::path::PathBuf);
impl TestDirectory {
  fn new() -> Self {
    let root = std::env::temp_dir().join(format!("woml-f02-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&root).unwrap();
    Self(root)
  }
}
impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

#[tokio::test]
async fn restart_settles_corrective_operations_before_manual_recovery_without_replay() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let directory = TestDirectory::new();
    let database = directory.0.join("state.sqlite");
    let (mut store, run) = active_correction(DurableEventStore::open(&database).unwrap(), kind);
    append(
      &mut store,
      &run,
      operation_start(&call(&run, action, 1, "call_interrupted")),
    );
    drop(store);
    let mut store = DurableEventStore::open(&database).unwrap();
    assert_eq!(store.recover_interrupted_runs().unwrap().recovered_runs, 1);
    let events = store.events(&run).unwrap();
    let operation_terminal = events.iter().find(|event| matches!(&event.payload,
      RunEventPayload::OperationFailed(data) if data.node_id == action && data.failure.ambiguous && !data.failure.retryable)).expect("unknown operation fails closed");
    let interrupted = events.iter().find(|event| matches!(&event.payload, RunEventPayload::CorrectionInterrupted(data) if data.action_id == action)).unwrap();
    assert!(operation_terminal.sequence < interrupted.sequence);
    let projection = store.projection(&run).unwrap();
    assert_eq!(
      projection.sagas[SAGA].actions[action].status,
      CorrectionActionStatus::ManualRecovery
    );
    assert!(!projection
      .operations
      .values()
      .any(|op| op.status == OperationStatus::Started));
    store.recover_interrupted_runs().unwrap();
    assert_eq!(store.events(&run).unwrap(), events);
    let (store, authority, calls) = authority(store);
    assert!(authority
      .execute(
        call(&run, action, 1, "late_after_restart"),
        CapabilityCancellationToken::default()
      )
      .await
      .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.lock().await.events(&run).unwrap(), events);
    {
      let mut store = store.lock().await;
      store
        .recover_correction_v1(
          CorrectionRecoveryRequestV1 {
            profile: CORRECTION_RECOVERY_REQUEST_PROFILE.into(),
            run_id: run.clone(),
            action_id: action.into(),
            action: CorrectionRecoveryActionV1::Retry,
            note: Some("provider outcome reviewed before retry".into()),
          },
          Utc::now(),
        )
        .unwrap();
      append(
        &mut store,
        &run,
        RunEventPayload::CorrectionAttemptStarted(identity(action, kind, 2)),
      );
    }
    assert!(matches!(
      authority
        .execute(
          call(&run, action, 2, "explicit_operator_retry"),
          CapabilityCancellationToken::default()
        )
        .await
        .unwrap(),
      CapabilityCallResult::Succeeded(_)
    ));
    assert_eq!(
      calls.load(Ordering::SeqCst),
      1,
      "only an explicit new attempt can execute after recovery"
    );
  }
}

#[tokio::test]
async fn corrective_calls_cannot_bypass_a_required_policy_or_pre_dispatch_cancellation() {
  let (store, run) = active_correction(
    DurableEventStore::open_in_memory().unwrap(),
    CorrectiveActionKind::Cleanup,
  );
  let (store, authority, calls) = authority(store);
  let required = DurableCapabilityAuthority::new_with_locks_and_infrastructure_policy(
    authority.registry().clone(),
    store.clone(),
    Arc::new(ManagedLocksService::default()),
    true,
  );
  let result = required
    .execute(
      call(&run, CLEANUP, 1, "missing_policy"),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert!(
    matches!(result, CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_REQUIRED")
  );
  let cancellation = CapabilityCancellationToken::default();
  cancellation.cancel();
  let result = authority
    .execute(call(&run, CLEANUP, 1, "cancelled_call"), cancellation)
    .await
    .unwrap();
  assert!(matches!(result, CapabilityCallResult::Cancelled(_)));
  assert_eq!(calls.load(Ordering::SeqCst), 0);
  assert!(store
    .lock()
    .await
    .projection(&run)
    .unwrap()
    .operations
    .is_empty());
}

#[tokio::test]
async fn native_fetch_tracking_uses_the_same_exact_corrective_identity() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let (store, authority, _) = authority(store);
    let request = call(&run, action, 1, "fetch_owner");
    let identity = NativeFetchInvocationIdentity {
      run_id: run.clone(),
      node_id: action.into(),
      attempt_number: 1,
      invocation_id: request.invocation_id.clone(),
      step_idempotency_key: request.identity.step_idempotency_key,
    };
    let started = NativeFetchObservation::Started {
      contract: "woml.native-fetch-observation".into(),
      contract_version: 1,
      invocation_id: request.invocation_id.clone(),
      request_id: "fetch_corrective".into(),
      method: "GET".into(),
      origin: "https://example.test".into(),
      path: "/health".into(),
      request_body_bytes: None,
      started_at: Utc::now(),
    };
    let mut forged = identity.clone();
    forged.invocation_id = "inv_forged".into();
    assert!(authority
      .observe_native_fetch(&forged, started.clone())
      .await
      .is_err());
    authority
      .observe_native_fetch(&identity, started)
      .await
      .unwrap();
    authority
      .observe_native_fetch(
        &identity,
        NativeFetchObservation::Completed {
          contract: "woml.native-fetch-observation".into(),
          contract_version: 1,
          invocation_id: request.invocation_id,
          request_id: "fetch_corrective".into(),
          status: 200,
          response_body_bytes: json!(0),
          duration_ms: 1.0,
          completed_at: Utc::now(),
        },
      )
      .await
      .unwrap();
    assert!(matches!(
      store
        .lock()
        .await
        .projection(&run)
        .unwrap()
        .operations
        .values()
        .next()
        .unwrap()
        .status,
      OperationStatus::Succeeded { .. }
    ));
  }
}

#[test]
fn a_committed_state_mutation_does_not_prove_its_corrective_script_completed() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let directory = TestDirectory::new();
    let database = directory.0.join("state.sqlite");
    let (mut store, run) = active_correction(DurableEventStore::open(&database).unwrap(), kind);
    let mut request = call(&run, action, 1, "call_proven_state_mutation");
    request.capability = "state".into();
    request.operation = "increment".into();
    request.identity.operation_name = "state.increment.corrective-count".into();
    request.identity.operation_key = derive_operation_key(
      &request.identity.step_idempotency_key,
      &request.identity.operation_name,
    );
    request.input = json!({"contract": "woml.state", "contractVersion": 1, "kind": "request", "operation": "increment", "input": {"key": "private-corrective-counter", "amount": 1}});
    let state = DurableStateStore::open(&database).unwrap();
    let workflow_id = store.run_binding(&run).unwrap().workflow_id;
    let mut metadata = state
      .execute(&workflow_id, &request)
      .unwrap()
      .safe_metadata();
    for field in [
      "outcome",
      "resultDigest",
      "durationMs",
      "version",
      "valueBytes",
    ] {
      metadata.remove(field);
    }
    let RunEventPayload::OperationStarted(mut start) = operation_start(&request) else {
      unreachable!()
    };
    start.metadata = metadata;
    append(&mut store, &run, RunEventPayload::OperationStarted(start));
    drop(store);
    let mut store = DurableEventStore::open(&database).unwrap();
    store.recover_interrupted_runs().unwrap();
    let projection = store.projection(&run).unwrap();
    assert!(matches!(
      projection.operations.values().next().unwrap().status,
      OperationStatus::Succeeded { .. }
    ));
    assert_eq!(
      projection.sagas[SAGA].actions[action].status,
      CorrectionActionStatus::ManualRecovery
    );
    let events = store.events(&run).unwrap();
    assert!(!serde_json::to_string(&events)
      .unwrap()
      .contains("private-corrective-counter"));
    store.recover_interrupted_runs().unwrap();
    assert_eq!(store.events(&run).unwrap(), events);
  }
}

#[tokio::test]
async fn handler_rejection_is_durably_settled_before_corrective_failure() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let store = Arc::new(Mutex::new(store));
    let registry = Arc::new(CapabilityRegistry::default());
    registry.register(Arc::new(TestCapabilityHandler)).unwrap();
    let authority = DurableCapabilityAuthority::new(registry, store.clone());
    let mut request = call(&run, action, 1, "call_rejected");
    request.capability = "test".into();
    request.operation = "control".into();
    request.input = json!({"mode": "fail"});
    request.identity.operation_name = "test.control.reject".into();
    request.identity.operation_key = derive_operation_key(
      &request.identity.step_idempotency_key,
      &request.identity.operation_name,
    );
    let result = authority
      .execute(request, CapabilityCancellationToken::default())
      .await
      .unwrap();
    assert!(
      matches!(result, CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_TEST_FAILURE")
    );
    let mut store = store.lock().await;
    assert!(matches!(
      store
        .projection(&run)
        .unwrap()
        .operations
        .values()
        .next()
        .unwrap()
        .status,
      OperationStatus::Failed { .. }
    ));
    append(
      &mut store,
      &run,
      RunEventPayload::CorrectionAttemptFailed(CorrectionAttemptFailedData {
        saga_id: SAGA.into(),
        action_id: action.into(),
        action_kind: kind,
        attempt: 1,
        code: "WOML_TEST_FAILURE".into(),
        retryable: false,
        ambiguous: false,
      }),
    );
    assert_eq!(
      store.projection(&run).unwrap().sagas[SAGA].actions[action].status,
      CorrectionActionStatus::Failed
    );
  }
}

#[test]
fn failed_corrective_recovery_rolls_back_operation_and_action_changes_together() {
  let directory = TestDirectory::new();
  let database = directory.0.join("state.sqlite");
  let (mut store, run) = active_correction(
    DurableEventStore::open(&database).unwrap(),
    CorrectiveActionKind::Compensation,
  );
  append(
    &mut store,
    &run,
    operation_start(&call(&run, COMPENSATE, 1, "call_rollback")),
  );
  let before = store.events(&run).unwrap();
  let connection = rusqlite::Connection::open(&database).unwrap();
  connection
    .execute_batch(
      "CREATE TRIGGER reject_correction_recovery BEFORE INSERT ON woml_run_events
    WHEN json_extract(NEW.event_json, '$.type') = 'correction_interrupted'
    BEGIN SELECT RAISE(ABORT, 'injected correction recovery failure'); END;",
    )
    .unwrap();
  assert!(store.recover_interrupted_runs().is_err());
  assert_eq!(store.events(&run).unwrap(), before);
  assert_eq!(
    store
      .projection(&run)
      .unwrap()
      .operations
      .values()
      .next()
      .unwrap()
      .status,
    OperationStatus::Started
  );
  assert_eq!(
    store.projection(&run).unwrap().sagas[SAGA].actions[COMPENSATE].status,
    CorrectionActionStatus::Started
  );
  connection
    .execute_batch("DROP TRIGGER reject_correction_recovery;")
    .unwrap();
  assert_eq!(store.recover_interrupted_runs().unwrap().recovered_runs, 1);
  assert_eq!(
    store.projection(&run).unwrap().sagas[SAGA].actions[COMPENSATE].status,
    CorrectionActionStatus::ManualRecovery
  );
}

#[test]
fn programmatic_leases_accept_only_exact_active_corrective_invocations() {
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (mut store, run) = active_correction(DurableEventStore::open_in_memory().unwrap(), kind);
    let request = call(&run, action, 1, "call_lease");
    let now = Utc::now();
    assert!(store
      .acquire_programmatic_lock_atomically(
        &run,
        action,
        "inv_forged",
        "session_bad",
        "f02/releases",
        1_000,
        5_000,
        now
      )
      .is_err());
    let lease = store
      .acquire_programmatic_lock_atomically(
        &run,
        action,
        &request.invocation_id,
        "session_corrective",
        "f02/releases",
        1_000,
        5_000,
        now,
      )
      .unwrap();
    assert_eq!(lease.state, woml_engine::ProgrammaticLockAcquireState::Held);
    assert_eq!(lease.lease.unwrap().fencing_token, "1");
    store
      .release_programmatic_lock_atomically(&run, "session_corrective", 5_000, now)
      .unwrap();
    append(
      &mut store,
      &run,
      RunEventPayload::CorrectionAttemptSucceeded(identity(action, kind, 1)),
    );
    assert!(store
      .acquire_programmatic_lock_atomically(
        &run,
        action,
        &request.invocation_id,
        "session_late",
        "f02/releases",
        1_000,
        5_000,
        now
      )
      .is_err());
  }
}

#[tokio::test]
async fn corrective_calls_cannot_escape_their_enclosing_markup_lock() {
  if !std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success())
  {
    return;
  }
  let source = include_str!("../../../../woml/tests/fixtures/compensation-and-cleanup/model.reviewed.woml")
    .replace("<saga id=\"provision\"", "<lock id=\"release\" key=\"f02/releases\" acquire-timeout=\"1m\" lease=\"30s\"><saga id=\"provision\"")
    .replace("</saga>", "</saga></lock>");
  let repository = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
  let output = std::process::Command::new("bun").current_dir(repository).env("F02_SOURCE", source)
    .args(["-e", "import {parseWoml, compileWoml} from './woml/src/index.ts'; process.stdout.write(JSON.stringify(compileWoml(parseWoml(process.env.F02_SOURCE))));"])
    .output().unwrap();
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  let workflow: CompiledWorkflowDefinition = serde_json::from_slice(&output.stdout).unwrap();
  for (kind, action) in [
    (CorrectiveActionKind::Compensation, COMPENSATE),
    (CorrectiveActionKind::Cleanup, CLEANUP),
  ] {
    let (mut store, run) = prepare_workflow_store(
      DurableEventStore::open_in_memory().unwrap(),
      Some(kind),
      workflow.clone(),
    );
    let request = call(&run, action, 1, "call_stale_owner");
    assert!(store
      .managed_effect_has_current_lock(&run, action, &request.invocation_id, Utc::now())
      .unwrap());
    let nested = store
      .acquire_programmatic_lock_atomically(
        &run,
        action,
        &request.invocation_id,
        "nested_session",
        "other/resource",
        1_000,
        5_000,
        Utc::now(),
      )
      .unwrap_err();
    assert!(nested.to_string().contains("cannot run inside a markup"));
    assert!(!store
      .managed_effect_has_current_lock(
        &run,
        action,
        &request.invocation_id,
        Utc::now() + chrono::Duration::seconds(31)
      )
      .unwrap());
    let (store, authority, calls) = authority(store);
    let result = authority
      .execute(request, CapabilityCancellationToken::default())
      .await
      .unwrap();
    assert!(
      matches!(result, CapabilityCallResult::Failed(failure) if failure.error.code == "WOML_LOCK_STALE_OWNER")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(store
      .lock()
      .await
      .projection(&run)
      .unwrap()
      .operations
      .is_empty());
  }
}
