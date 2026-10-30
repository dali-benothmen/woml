//! Durable capability authority: active identity, locks, policy, operation start, handler invocation, and terminal settlement in their original order.

use crate::capability::failed_fields;
use crate::capability::failed_result;
use crate::capability::failure;
use crate::capability::infrastructure_policy_evidence;
use crate::capability::infrastructure_policy_execution_mode;
use crate::capability::iteration_attempt_identity;
use crate::capability::CapabilityCallRequest;
use crate::capability::CapabilityCallResult;
use crate::capability::CapabilityCancellationToken;
use crate::capability::CapabilityEffect;
use crate::capability::CapabilityFailureKind;
use crate::capability::CapabilityRegistry;
use crate::infrastructure_policy_runtime::apply_emergency_deny_overlay;
use crate::infrastructure_policy_runtime::InfrastructurePolicyDecisionOutcomeV1;
use crate::infrastructure_policy_runtime::InfrastructurePolicyDescriptorContextV1;
use crate::infrastructure_policy_runtime::InfrastructurePolicyEvaluatorCacheV1;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::InfrastructurePolicyDecisionRecordedData;
use crate::OperationExecutionMode;
use crate::OperationFailedData;
use crate::OperationStartedData;
use crate::OperationSucceededData;
use crate::PreparedInfrastructurePolicyV1;
use crate::RunEventPayload;
use chrono::Utc;
use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DurableCapabilityAuthorityError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
}

pub struct DurableCapabilityAuthority {
  pub(in crate::capability) registry: Arc<CapabilityRegistry>,
  pub(in crate::capability) store: Arc<AsyncMutex<DurableEventStore>>,
  pub(in crate::capability) locks: Arc<crate::ManagedLocksService>,
  pub(in crate::capability) infrastructure_policy_required: bool,
  pub(in crate::capability) infrastructure_policy_evaluators:
    Arc<InfrastructurePolicyEvaluatorCacheV1>,
  pub(in crate::capability) artifact_store: Option<Arc<crate::ManagedArtifactStore>>,
  pub(in crate::capability) emergency_deny_overlay: Option<Arc<PreparedInfrastructurePolicyV1>>,
  pub(in crate::capability) operational_output_store:
    Option<Arc<crate::LazyOperationalOutputStore>>,
  pub(in crate::capability) operational_redactions: Arc<std::collections::BTreeMap<String, String>>,
}

pub(in crate::capability) fn generated_event_id() -> String {
  format!("event_{}", Uuid::new_v4().simple())
}

impl DurableCapabilityAuthority {
  pub fn new(registry: Arc<CapabilityRegistry>, store: Arc<AsyncMutex<DurableEventStore>>) -> Self {
    Self {
      registry,
      store,
      locks: Arc::new(crate::ManagedLocksService::default()),
      infrastructure_policy_required: false,
      infrastructure_policy_evaluators: Arc::new(InfrastructurePolicyEvaluatorCacheV1::default()),
      artifact_store: None,
      emergency_deny_overlay: None,
      operational_output_store: None,
      operational_redactions: Arc::new(std::collections::BTreeMap::new()),
    }
  }

  pub fn new_with_locks(
    registry: Arc<CapabilityRegistry>,
    store: Arc<AsyncMutex<DurableEventStore>>,
    locks: Arc<crate::ManagedLocksService>,
  ) -> Self {
    Self {
      registry,
      store,
      locks,
      infrastructure_policy_required: false,
      infrastructure_policy_evaluators: Arc::new(InfrastructurePolicyEvaluatorCacheV1::default()),
      artifact_store: None,
      emergency_deny_overlay: None,
      operational_output_store: None,
      operational_redactions: Arc::new(std::collections::BTreeMap::new()),
    }
  }

  pub fn new_with_locks_and_infrastructure_policy(
    registry: Arc<CapabilityRegistry>,
    store: Arc<AsyncMutex<DurableEventStore>>,
    locks: Arc<crate::ManagedLocksService>,
    infrastructure_policy_required: bool,
  ) -> Self {
    Self::new_with_infrastructure_policy_authorities(
      registry,
      store,
      locks,
      infrastructure_policy_required,
      None,
      None,
    )
  }

  pub fn new_with_infrastructure_policy_authorities(
    registry: Arc<CapabilityRegistry>,
    store: Arc<AsyncMutex<DurableEventStore>>,
    locks: Arc<crate::ManagedLocksService>,
    infrastructure_policy_required: bool,
    artifact_store: Option<Arc<crate::ManagedArtifactStore>>,
    emergency_deny_overlay: Option<Arc<PreparedInfrastructurePolicyV1>>,
  ) -> Self {
    Self {
      registry,
      store,
      locks,
      infrastructure_policy_required,
      infrastructure_policy_evaluators: Arc::new(InfrastructurePolicyEvaluatorCacheV1::default()),
      artifact_store,
      emergency_deny_overlay,
      operational_output_store: None,
      operational_redactions: Arc::new(std::collections::BTreeMap::new()),
    }
  }

  pub fn with_operational_output(
    mut self,
    store: Arc<crate::LazyOperationalOutputStore>,
    redactions: Arc<std::collections::BTreeMap<String, String>>,
  ) -> Self {
    self.operational_output_store = Some(store);
    self.operational_redactions = redactions;
    self
  }

  pub fn registry(&self) -> &Arc<CapabilityRegistry> {
    &self.registry
  }

  pub fn store(&self) -> &Arc<AsyncMutex<DurableEventStore>> {
    &self.store
  }

  /// Called only after the host has reported the invocation terminal or the
  /// host connection itself has ended. At that point Bun can no longer issue
  /// protected work, so Rust may safely stop renewal and release the session.
  pub fn close_programmatic_lock_sessions(&self, invocation_id: &str) {
    self.locks.close_invocation(invocation_id);
  }

  pub async fn execute(
    &self,
    request: CapabilityCallRequest,
    cancellation: CapabilityCancellationToken,
  ) -> Result<CapabilityCallResult, DurableCapabilityAuthorityError> {
    let (workflow_scope, definition_hash, iteration_scope, policy_execution_mode) = {
      let mut store = self.store.lock().await;
      let projection = store.projection(&request.run_id)?;
      let binding = store.run_binding(&request.run_id)?;
      let attempt = projection.attempts.iter().find(|attempt| {
        attempt.identity.node_id == request.node_id
          && attempt.identity.attempt == request.attempt_number
          && attempt.identity.invocation_id == request.invocation_id
          && attempt.status == crate::projection::AttemptStatus::Started
      });
      let lifecycle_key = projection.lifecycle_hooks.values().find_map(|hook| {
        hook.actions.get(&request.node_id).and_then(|action| {
          (action.attempt == request.attempt_number
            && action.status == crate::projection::LifecycleActionStatus::Started)
            .then(|| {
              crate::step_effect_idempotency_key(
                &request.run_id,
                &binding.definition_hash,
                &request.node_id,
              )
            })
        })
      });
      let root_key = attempt
        .and_then(|attempt| attempt.idempotency_key.as_deref())
        .map(str::to_string)
        .or(lifecycle_key);
      let iteration = iteration_attempt_identity(
        &projection,
        &request.run_id,
        &binding.definition_hash,
        &request.node_id,
        request.attempt_number,
        &request.invocation_id,
      );
      let (iteration_scope, iteration_key) = iteration
        .map(|(scope, key)| (Some(scope), Some(key)))
        .unwrap_or((None, None));
      if root_key.or(iteration_key).as_deref()
        != Some(request.identity.step_idempotency_key.as_str())
      {
        return Err(
          DurableStoreError::Contract(
            "Capability call step identity does not match its active durable attempt.".to_string(),
          )
          .into(),
        );
      }
      if !store.managed_effect_has_current_lock(
        &request.run_id,
        &request.node_id,
        &request.invocation_id,
        Utc::now(),
      )? {
        return Ok(failed_result(
          &request,
          Instant::now(),
          failure(
            CapabilityFailureKind::ServiceRejected,
            "WOML_LOCK_STALE_OWNER",
            "The managed effect was rejected because its lock lease is no longer current.",
            false,
            false,
          ),
        ));
      }
      let policy_execution_mode =
        infrastructure_policy_execution_mode(&projection, &request.node_id, request.attempt_number);
      (
        binding.workflow_id,
        binding.definition_hash,
        iteration_scope,
        policy_execution_mode,
      )
    };
    match self.locks.managed_effect_is_current(&request.invocation_id) {
      Ok(true) => {}
      Ok(false) => {
        return Ok(failed_result(
          &request,
          Instant::now(),
          failure(
            CapabilityFailureKind::ServiceRejected,
            "WOML_LOCK_STALE_OWNER",
            "The managed effect was rejected because its programmatic lease is no longer current.",
            false,
            false,
          ),
        ));
      }
      Err(error) => {
        return Ok(failed_result(&request, Instant::now(), error));
      }
    }
    let metadata = match self.registry.safe_metadata(&request) {
      Ok(metadata) => metadata,
      Err(error) => {
        return Ok(CapabilityCallResult::Failed(failed_fields(
          &request,
          Instant::now(),
          "failed",
          error,
        )))
      }
    };
    let policy_decision = {
      let mut store = self.store.lock().await;
      let admitted_policy = match store.infrastructure_policy_for_run(&request.run_id) {
        Ok(policy) => policy,
        Err(error) => {
          return Ok(failed_result(
            &request,
            Instant::now(),
            failure(
              CapabilityFailureKind::ServiceRejected,
              "WOML_POLICY_UNAVAILABLE",
              format!("The admitted infrastructure policy is unavailable: {error}"),
              false,
              false,
            ),
          ));
        }
      };
      if admitted_policy.is_none() && self.infrastructure_policy_required {
        return Ok(failed_result(
          &request,
          Instant::now(),
          failure(
            CapabilityFailureKind::ServiceRejected,
            "WOML_POLICY_REQUIRED",
            "This runtime requires an admitted infrastructure policy before protected effects may start.",
            false,
            false,
          ),
        ));
      }
      if let Some(policy) = admitted_policy {
        let evaluated_at = Utc::now();
        let (review_digest, evidence) = match infrastructure_policy_evidence(
          &store,
          self.artifact_store.as_ref(),
          &request.run_id,
          evaluated_at,
        ) {
          Ok(evidence) => evidence,
          Err(error) => {
            return Ok(failed_result(
              &request,
              Instant::now(),
              failure(
                CapabilityFailureKind::ServiceRejected,
                "WOML_POLICY_DECISION_UNAVAILABLE",
                format!("Trusted infrastructure-policy evidence is unavailable: {error}"),
                false,
                false,
              ),
            ));
          }
        };
        let descriptor = match self.registry.infrastructure_policy_descriptor(
          &request,
          InfrastructurePolicyDescriptorContextV1 {
            workflow_id: workflow_scope.clone(),
            definition_hash: definition_hash.clone(),
            environment: policy.bundle.environment.clone(),
            effective_target: None,
            execution_mode: policy_execution_mode,
            review_digest,
          },
        ) {
          Ok(descriptor) => descriptor,
          Err(error) => {
            let effect = self
              .registry
              .descriptor(&request.capability, &request.operation)
              .map_or(CapabilityEffect::UnsafeWrite, |descriptor| {
                descriptor.effect
              });
            let evaluator = self.infrastructure_policy_evaluators.prepare(policy);
            let decision = evaluator.input_unsupported_decision(
              &request.run_id,
              &workflow_scope,
              &definition_hash,
              &request.node_id,
              request.attempt_number,
              &request.invocation_id,
              &request.identity.operation_key,
              &request.capability,
              &request.operation,
              effect,
              policy_execution_mode,
              evaluated_at,
            );
            store.append_payload(
              request.run_id.clone(),
              generated_event_id(),
              evaluated_at,
              RunEventPayload::InfrastructurePolicyDecisionRecorded(
                InfrastructurePolicyDecisionRecordedData {
                  invocation_id: request.invocation_id.clone(),
                  call_id: request.call_id.clone(),
                  operation_key: request.identity.operation_key.clone(),
                  decision,
                },
              ),
            )?;
            return Ok(failed_result(&request, Instant::now(), error));
          }
        };
        let evaluator = self.infrastructure_policy_evaluators.prepare(policy);
        let base = evaluator.evaluate(&descriptor, &evidence, evaluated_at);
        Some(
          if let Some(overlay) = self.emergency_deny_overlay.as_ref() {
            let overlay = self
              .infrastructure_policy_evaluators
              .prepare(overlay.as_ref().clone());
            let overlay_decision = overlay.evaluate(&descriptor, &[], evaluated_at);
            apply_emergency_deny_overlay(base, overlay_decision)
          } else {
            base
          },
        )
      } else {
        None
      }
    };
    if cancellation.is_cancelled() {
      return Ok(CapabilityCallResult::Cancelled(failed_fields(
        &request,
        Instant::now(),
        "cancelled",
        failure(
          CapabilityFailureKind::Cancelled,
          "WOML_CAPABILITY_CANCELLED",
          "The capability call was cancelled before policy-authorized dispatch.",
          false,
          false,
        ),
      )));
    }
    {
      let mut store = self.store.lock().await;
      let payload = RunEventPayload::OperationStarted(OperationStartedData {
        node_id: request.node_id.clone(),
        attempt_number: request.attempt_number,
        invocation_id: request.invocation_id.clone(),
        call_id: request.call_id.clone(),
        operation_key: request.identity.operation_key.clone(),
        capability: request.capability.clone(),
        operation: request.operation.clone(),
        execution_mode: OperationExecutionMode::Managed,
        metadata: metadata.clone(),
      });
      if let Some(decision) = policy_decision.as_ref() {
        store.append_payload(
          request.run_id.clone(),
          generated_event_id(),
          decision.evaluated_at,
          RunEventPayload::InfrastructurePolicyDecisionRecorded(
            InfrastructurePolicyDecisionRecordedData {
              invocation_id: request.invocation_id.clone(),
              call_id: request.call_id.clone(),
              operation_key: request.identity.operation_key.clone(),
              decision: decision.clone(),
            },
          ),
        )?;
        if decision.outcome != InfrastructurePolicyDecisionOutcomeV1::Allow {
          let (code, message) = match decision.outcome {
            InfrastructurePolicyDecisionOutcomeV1::Deny => (
              "WOML_POLICY_DENIED",
              "The admitted infrastructure policy denied this protected effect.",
            ),
            InfrastructurePolicyDecisionOutcomeV1::RequirementsUnsatisfied => (
              "WOML_POLICY_REQUIREMENTS_UNSATISFIED",
              "Trusted evidence required by the infrastructure policy is missing or insufficient.",
            ),
            InfrastructurePolicyDecisionOutcomeV1::Allow => unreachable!(),
          };
          return Ok(failed_result(
            &request,
            Instant::now(),
            failure(
              CapabilityFailureKind::ServiceRejected,
              code,
              message,
              false,
              false,
            ),
          ));
        }
      }
      if let Some(scope) = iteration_scope.clone() {
        store.append_payload_scoped(
          request.run_id.clone(),
          generated_event_id(),
          Utc::now(),
          Some(scope),
          payload,
        )?;
      } else {
        store.append_payload(
          request.run_id.clone(),
          generated_event_id(),
          Utc::now(),
          payload,
        )?;
      }
    }

    let result = self
      .registry
      .execute_scoped(request.clone(), Some(workflow_scope), cancellation)
      .await;
    let payload = match &result {
      CapabilityCallResult::Succeeded(success) => {
        let encoded = serde_json::to_vec(&success.result).unwrap_or_default();
        let mut result_metadata = if matches!(request.capability.as_str(), "state" | "process") {
          metadata.clone()
        } else {
          Map::new()
        };
        result_metadata.extend(
          self
            .registry
            .safe_result_metadata(&request, &success.result)
            .unwrap_or_default(),
        );
        if matches!(request.capability.as_str(), "state" | "artifacts") {
          result_metadata.insert("durationMs".to_string(), Value::from(success.duration_ms));
        }
        RunEventPayload::OperationSucceeded(OperationSucceededData {
          node_id: request.node_id.clone(),
          attempt_number: request.attempt_number,
          invocation_id: request.invocation_id.clone(),
          call_id: request.call_id.clone(),
          operation_key: request.identity.operation_key.clone(),
          capability: request.capability.clone(),
          operation: request.operation.clone(),
          execution_mode: OperationExecutionMode::Managed,
          metadata: result_metadata,
          duration_ms: success.duration_ms,
          result_bytes: success.result_bytes,
          result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&encoded))),
        })
      }
      CapabilityCallResult::Failed(failed) | CapabilityCallResult::Cancelled(failed) => {
        let mut metadata = metadata;
        metadata.extend(
          self
            .registry
            .safe_failure_metadata(&request, &failed.error)
            .unwrap_or_default(),
        );
        if request.capability == "process" {
          metadata.insert("durationMs".to_string(), Value::from(failed.duration_ms));
        }
        if request.capability == "state" {
          let outcome = if failed.error.kind == CapabilityFailureKind::Cancelled {
            "cancelled"
          } else if failed.error.code == "WOML_STATE_CONFLICT" {
            "conflict"
          } else if failed.error.code == "WOML_STATE_QUOTA_EXCEEDED" {
            "quota_exceeded"
          } else {
            "failed"
          };
          metadata.insert("outcome".to_string(), Value::String(outcome.to_string()));
          metadata.insert("durationMs".to_string(), Value::from(failed.duration_ms));
        }
        if request.capability == "artifacts" {
          metadata.insert("durationMs".to_string(), Value::from(failed.duration_ms));
        }
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: request.node_id.clone(),
          attempt_number: request.attempt_number,
          invocation_id: request.invocation_id.clone(),
          call_id: request.call_id.clone(),
          operation_key: request.identity.operation_key.clone(),
          capability: request.capability.clone(),
          operation: request.operation.clone(),
          execution_mode: OperationExecutionMode::Managed,
          metadata,
          duration_ms: failed.duration_ms,
          failure: failed.error.clone(),
        })
      }
    };
    {
      let mut store = self.store.lock().await;
      if let Some(scope) = iteration_scope {
        store.append_payload_scoped(
          request.run_id.clone(),
          generated_event_id(),
          Utc::now(),
          Some(scope),
          payload,
        )?;
      } else {
        store.append_payload(
          request.run_id.clone(),
          generated_event_id(),
          Utc::now(),
          payload,
        )?;
      }
    }
    Ok(result)
  }
}
