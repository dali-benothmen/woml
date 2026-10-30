//! Frozen native-fetch observations and durable observation settlement. Bun performs the fetch; observation is not managed replay permission.

use crate::capability::derive_operation_key;
use crate::capability::generated_event_id;
use crate::capability::infrastructure_policy_execution_mode;
use crate::capability::iteration_attempt_identity;
use crate::capability::operation_with_iteration_scope;
use crate::capability::validate_safe_metadata;
use crate::capability::CapabilityEffect;
use crate::capability::CapabilityFailure;
use crate::capability::CapabilityFailureKind;
use crate::capability::DurableCapabilityAuthority;
use crate::capability::DurableCapabilityAuthorityError;
use crate::DurableStoreError;
use crate::InfrastructurePolicyDecisionRecordedData;
use crate::OperationExecutionMode;
use crate::OperationFailedData;
use crate::OperationStartedData;
use crate::OperationSucceededData;
use crate::RunEventPayload;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

pub const NATIVE_FETCH_OBSERVATION_CONTRACT: &str = "woml.native-fetch-observation";

pub const NATIVE_FETCH_OBSERVATION_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeFetchInvocationIdentity {
  pub run_id: String,
  pub node_id: String,
  pub attempt_number: u32,
  pub invocation_id: String,
  pub step_idempotency_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeFetchFailureKind {
  TrackingFailed,
  FetchRejected,
  TimedOut,
  Cancelled,
  WorkerCrashed,
  HostCrashed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeFetchFailure {
  pub kind: NativeFetchFailureKind,
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
  tag = "observationType",
  rename_all = "snake_case",
  rename_all_fields = "camelCase"
)]
pub enum NativeFetchObservation {
  Started {
    contract: String,
    contract_version: u32,
    invocation_id: String,
    request_id: String,
    method: String,
    origin: String,
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_body_bytes: Option<u64>,
    started_at: DateTime<Utc>,
  },
  Completed {
    contract: String,
    contract_version: u32,
    invocation_id: String,
    request_id: String,
    status: u16,
    response_body_bytes: Value,
    duration_ms: f64,
    completed_at: DateTime<Utc>,
  },
  Failed {
    contract: String,
    contract_version: u32,
    invocation_id: String,
    request_id: String,
    duration_ms: f64,
    failed_at: DateTime<Utc>,
    error: NativeFetchFailure,
  },
}

impl NativeFetchObservation {
  pub fn invocation_id(&self) -> &str {
    match self {
      Self::Started { invocation_id, .. }
      | Self::Completed { invocation_id, .. }
      | Self::Failed { invocation_id, .. } => invocation_id,
    }
  }

  pub fn request_id(&self) -> &str {
    match self {
      Self::Started { request_id, .. }
      | Self::Completed { request_id, .. }
      | Self::Failed { request_id, .. } => request_id,
    }
  }

  pub fn validate(&self) -> Result<(), String> {
    let (contract, contract_version, invocation_id, request_id) = match self {
      Self::Started {
        contract,
        contract_version,
        invocation_id,
        request_id,
        method,
        origin,
        path,
        ..
      } => {
        let origin_tail = origin
          .strip_prefix("https://")
          .or_else(|| origin.strip_prefix("http://"));
        if method.is_empty()
          || method.len() > 32
          || !method.bytes().all(|byte| byte.is_ascii_uppercase())
          || origin.len() > 2048
          || origin_tail.is_none_or(|tail| {
            tail.is_empty()
              || tail
                .bytes()
                .any(|byte| matches!(byte, b'/' | b'?' | b'#' | b'@'))
          })
          || path.len() > 2048
          || !path.starts_with('/')
          || path.bytes().any(|byte| matches!(byte, b'?' | b'#'))
        {
          return Err("Native Fetch start metadata is not safely redacted.".to_string());
        }
        (contract, contract_version, invocation_id, request_id)
      }
      Self::Completed {
        contract,
        contract_version,
        invocation_id,
        request_id,
        status,
        response_body_bytes,
        duration_ms,
        ..
      } => {
        if !(100..=599).contains(status)
          || !(response_body_bytes.is_null() || response_body_bytes.as_u64().is_some())
          || !duration_ms.is_finite()
          || *duration_ms < 0.0
        {
          return Err("Native Fetch completion metadata is invalid.".to_string());
        }
        (contract, contract_version, invocation_id, request_id)
      }
      Self::Failed {
        contract,
        contract_version,
        invocation_id,
        request_id,
        duration_ms,
        error,
        ..
      } => {
        if !duration_ms.is_finite()
          || *duration_ms < 0.0
          || error.code.is_empty()
          || !error
            .code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
          || error.message.is_empty()
          || error.message.len() > 2048
        {
          return Err("Native Fetch failure metadata is invalid.".to_string());
        }
        (contract, contract_version, invocation_id, request_id)
      }
    };
    if contract != NATIVE_FETCH_OBSERVATION_CONTRACT
      || *contract_version != NATIVE_FETCH_OBSERVATION_CONTRACT_VERSION
      || invocation_id.is_empty()
      || invocation_id.len() > 256
      || request_id.is_empty()
      || request_id.len() > 256
    {
      return Err("Message does not match Native Fetch Observation v1.".to_string());
    }
    Ok(())
  }
}

impl DurableCapabilityAuthority {
  pub async fn observe_native_fetch(
    &self,
    identity: &NativeFetchInvocationIdentity,
    observation: NativeFetchObservation,
  ) -> Result<(), DurableCapabilityAuthorityError> {
    observation
      .validate()
      .map_err(DurableStoreError::Contract)?;
    if observation.invocation_id() != identity.invocation_id {
      return Err(
        DurableStoreError::Contract(
          "Native Fetch observation crossed invocation boundaries.".to_string(),
        )
        .into(),
      );
    }
    let event_schema_version = {
      let store = self.store.lock().await;
      store
        .events(&identity.run_id)?
        .first()
        .map(|event| event.event_schema_version)
        .ok_or_else(|| {
          DurableStoreError::Contract(
            "Native Fetch observation references a run without durable history.".to_string(),
          )
        })?
    };
    if event_schema_version < crate::RUN_EVENT_SCHEMA_VERSION_V8 {
      // Models v1-v7 predate native-Fetch observations. Their stored behavior
      // remains executable; every newly compiled Fetch script targets Model v8.
      return Ok(());
    }

    match observation {
      NativeFetchObservation::Started {
        request_id,
        method,
        origin,
        path,
        request_body_bytes,
        ..
      } => {
        let operation_key = derive_operation_key(
          &identity.step_idempotency_key,
          &format!("native-fetch.{request_id}"),
        );
        {
          let mut store = self.store.lock().await;
          if self.infrastructure_policy_required
            || store
              .infrastructure_policy_binding(&identity.run_id)?
              .is_some()
          {
            if let Some(policy) = store.infrastructure_policy_for_run(&identity.run_id)? {
              let binding = store.run_binding(&identity.run_id)?;
              let projection = store.projection(&identity.run_id)?;
              let execution_mode = infrastructure_policy_execution_mode(
                &projection,
                &identity.node_id,
                identity.attempt_number,
              );
              let evaluated_at = Utc::now();
              let decision = self
                .infrastructure_policy_evaluators
                .prepare(policy)
                .input_unsupported_decision(
                  &identity.run_id,
                  &binding.workflow_id,
                  &binding.definition_hash,
                  &identity.node_id,
                  identity.attempt_number,
                  &identity.invocation_id,
                  &operation_key,
                  "http",
                  "fetch",
                  CapabilityEffect::UnsafeWrite,
                  execution_mode,
                  evaluated_at,
                );
              store.append_payload(
                identity.run_id.clone(),
                generated_event_id(),
                evaluated_at,
                RunEventPayload::InfrastructurePolicyDecisionRecorded(
                  InfrastructurePolicyDecisionRecordedData {
                    invocation_id: identity.invocation_id.clone(),
                    call_id: request_id.clone(),
                    operation_key: operation_key.clone(),
                    decision,
                  },
                ),
              )?;
            }
            return Err(
              DurableStoreError::Contract(
                "WOML_POLICY_INPUT_UNSUPPORTED: native fetch is blocked before network I/O while Infrastructure Policy enforcement is active; use services.http.request()."
                  .to_string(),
              )
              .into(),
            );
          }
        }
        let iteration_scope = {
          let store = self.store.lock().await;
          let projection = store.projection(&identity.run_id)?;
          let binding = store.run_binding(&identity.run_id)?;
          let attempt = projection.attempts.iter().find(|attempt| {
            attempt.identity.node_id == identity.node_id
              && attempt.identity.attempt == identity.attempt_number
              && attempt.identity.invocation_id == identity.invocation_id
              && attempt.status == crate::projection::AttemptStatus::Started
          });
          let lifecycle_key = projection.lifecycle_hooks.values().find_map(|hook| {
            hook.actions.get(&identity.node_id).and_then(|action| {
              (action.attempt == identity.attempt_number
                && action.status == crate::projection::LifecycleActionStatus::Started)
                .then(|| {
                  crate::step_effect_idempotency_key(
                    &identity.run_id,
                    &binding.definition_hash,
                    &identity.node_id,
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
            &identity.run_id,
            &binding.definition_hash,
            &identity.node_id,
            identity.attempt_number,
            &identity.invocation_id,
          );
          let (iteration_scope, iteration_key) = iteration
            .map(|(scope, key)| (Some(scope), Some(key)))
            .unwrap_or((None, None));
          if root_key.or(iteration_key).as_deref() != Some(identity.step_idempotency_key.as_str()) {
            return Err(
              DurableStoreError::Contract(
                "Native Fetch identity does not match its active durable attempt.".to_string(),
              )
              .into(),
            );
          }
          iteration_scope
        };
        let mut metadata = Map::new();
        metadata.insert("method".to_string(), Value::String(method));
        metadata.insert("origin".to_string(), Value::String(origin));
        metadata.insert("path".to_string(), Value::String(path));
        if let Some(bytes) = request_body_bytes {
          metadata.insert("requestBodyBytes".to_string(), Value::from(bytes));
        }
        validate_safe_metadata(&metadata).map_err(DurableStoreError::Contract)?;
        let payload = RunEventPayload::OperationStarted(OperationStartedData {
          node_id: identity.node_id.clone(),
          attempt_number: identity.attempt_number,
          invocation_id: identity.invocation_id.clone(),
          call_id: request_id,
          operation_key,
          capability: "http".to_string(),
          operation: "fetch".to_string(),
          execution_mode: OperationExecutionMode::Observed,
          metadata,
        });
        let mut store = self.store.lock().await;
        if let Some(scope) = iteration_scope {
          store.append_payload_scoped(
            identity.run_id.clone(),
            generated_event_id(),
            Utc::now(),
            Some(scope),
            payload,
          )?;
        } else {
          store.append_payload(
            identity.run_id.clone(),
            generated_event_id(),
            Utc::now(),
            payload,
          )?;
        }
      }
      NativeFetchObservation::Completed {
        request_id,
        status,
        response_body_bytes,
        duration_ms,
        ..
      } => {
        let (iteration_scope, operation) = {
          let store = self.store.lock().await;
          operation_with_iteration_scope(
            &store.projection(&identity.run_id)?,
            &crate::projection::OperationIdentity {
              invocation_id: identity.invocation_id.clone(),
              call_id: request_id.clone(),
            },
          )
          .ok_or_else(|| {
            DurableStoreError::Contract("Native Fetch completion has no durable start.".to_string())
          })?
        };
        let mut metadata = Map::new();
        metadata.insert("status".to_string(), Value::from(status));
        if let Some(bytes) = response_body_bytes.as_u64() {
          metadata.insert("responseBodyBytes".to_string(), Value::from(bytes));
        }
        let payload = RunEventPayload::OperationSucceeded(OperationSucceededData {
          node_id: operation.node_id,
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id,
          call_id: operation.identity.call_id,
          operation_key: operation.operation_key,
          capability: operation.capability,
          operation: operation.operation,
          execution_mode: operation.execution_mode,
          metadata,
          duration_ms,
          result_bytes: 0,
          result_digest: format!("sha256:{}", hex::encode(Sha256::digest([]))),
        });
        let mut store = self.store.lock().await;
        if let Some(scope) = iteration_scope {
          store.append_payload_scoped(
            identity.run_id.clone(),
            generated_event_id(),
            Utc::now(),
            Some(scope),
            payload,
          )?;
        } else {
          store.append_payload(
            identity.run_id.clone(),
            generated_event_id(),
            Utc::now(),
            payload,
          )?;
        }
      }
      NativeFetchObservation::Failed {
        request_id,
        duration_ms,
        error,
        ..
      } => {
        let (iteration_scope, operation) = {
          let store = self.store.lock().await;
          operation_with_iteration_scope(
            &store.projection(&identity.run_id)?,
            &crate::projection::OperationIdentity {
              invocation_id: identity.invocation_id.clone(),
              call_id: request_id.clone(),
            },
          )
          .ok_or_else(|| {
            DurableStoreError::Contract("Native Fetch failure has no durable start.".to_string())
          })?
        };
        let kind = match error.kind {
          NativeFetchFailureKind::TimedOut => CapabilityFailureKind::TimedOut,
          NativeFetchFailureKind::Cancelled => CapabilityFailureKind::Cancelled,
          NativeFetchFailureKind::WorkerCrashed => CapabilityFailureKind::WorkerCrashed,
          NativeFetchFailureKind::HostCrashed => CapabilityFailureKind::HostCrashed,
          NativeFetchFailureKind::TrackingFailed | NativeFetchFailureKind::FetchRejected => {
            CapabilityFailureKind::TransportFailed
          }
        };
        let payload = RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id,
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id,
          call_id: operation.identity.call_id,
          operation_key: operation.operation_key,
          capability: operation.capability,
          operation: operation.operation,
          execution_mode: operation.execution_mode,
          metadata: Map::new(),
          duration_ms,
          failure: CapabilityFailure {
            kind,
            code: error.code,
            message: error.message,
            retryable: false,
            ambiguous: true,
            details: None,
          },
        });
        let mut store = self.store.lock().await;
        if let Some(scope) = iteration_scope {
          store.append_payload_scoped(
            identity.run_id.clone(),
            generated_event_id(),
            Utc::now(),
            Some(scope),
            payload,
          )?;
        } else {
          store.append_payload(
            identity.run_id.clone(),
            generated_event_id(),
            Utc::now(),
            payload,
          )?;
        }
      }
    }
    Ok(())
  }

  pub async fn close_active_native_fetches(
    &self,
    identity: &NativeFetchInvocationIdentity,
    failure: CapabilityFailure,
  ) -> Result<(), DurableCapabilityAuthorityError> {
    failure.validate().map_err(DurableStoreError::Contract)?;
    let operations = {
      let store = self.store.lock().await;
      let projection = store.projection(&identity.run_id)?;
      let mut operations = projection
        .operations
        .values()
        .filter(|operation| {
          operation.identity.invocation_id == identity.invocation_id
            && operation.execution_mode == OperationExecutionMode::Observed
            && operation.status == crate::projection::OperationStatus::Started
        })
        .cloned()
        .map(|operation| (None, operation))
        .collect::<Vec<_>>();
      for loop_ in projection.for_each.values() {
        for (index, execution) in &loop_.executions {
          operations.extend(
            execution
              .operations
              .values()
              .filter(|operation| {
                operation.identity.invocation_id == identity.invocation_id
                  && operation.execution_mode == OperationExecutionMode::Observed
                  && operation.status == crate::projection::OperationStatus::Started
              })
              .cloned()
              .map(|operation| {
                (
                  Some(crate::event::ForEachIterationScope {
                    for_each_id: loop_.for_each_id.clone(),
                    index: *index,
                  }),
                  operation,
                )
              }),
          );
        }
      }
      operations
    };
    for (iteration_scope, operation) in operations {
      let payload = RunEventPayload::OperationFailed(OperationFailedData {
        node_id: operation.node_id,
        attempt_number: operation.attempt_number,
        invocation_id: operation.identity.invocation_id,
        call_id: operation.identity.call_id,
        operation_key: operation.operation_key,
        capability: operation.capability,
        operation: operation.operation,
        execution_mode: operation.execution_mode,
        metadata: operation.metadata,
        duration_ms: 0.0,
        failure: failure.clone(),
      });
      let mut store = self.store.lock().await;
      if let Some(scope) = iteration_scope {
        store.append_payload_scoped(
          identity.run_id.clone(),
          generated_event_id(),
          Utc::now(),
          Some(scope),
          payload,
        )?;
      } else {
        store.append_payload(
          identity.run_id.clone(),
          generated_event_id(),
          Utc::now(),
          payload,
        )?;
      }
    }
    Ok(())
  }
}
