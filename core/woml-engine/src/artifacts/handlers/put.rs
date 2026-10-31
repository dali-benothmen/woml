//! Identity-bound put validation, transfer supervision, concurrency, and safe metadata. The handler does not trust authored invocation identity.

use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::artifact_failure;
use crate::artifacts::artifact_timed_out;
use crate::artifacts::backend_unavailable;
use crate::artifacts::operation_identity_conflict;
use crate::artifacts::parse_put;
use crate::artifacts::ArtifactPolicyError;
use crate::artifacts::ArtifactPolicyV1;
use crate::artifacts::ArtifactReadOperation;
use crate::artifacts::ArtifactRuntimeMode;
use crate::artifacts::ManagedArtifactReadHandler;
use crate::artifacts::ManagedArtifactStore;
use crate::artifacts::PreparedArtifactBackend;
use crate::artifacts::PreparedArtifactPolicy;
use crate::artifacts::ARTIFACTS_CONTRACT;
use crate::artifacts::ARTIFACTS_CONTRACT_VERSION;
use crate::capability::CapabilityIdentityMode;
use crate::CapabilityCallRequest;
use crate::CapabilityCancellationToken;
use crate::CapabilityDescriptor;
use crate::CapabilityEffect;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::CapabilityHandler;
use futures_util::future::BoxFuture;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

#[derive(Debug)]
pub struct ManagedArtifactPutHandler {
  pub(in crate::artifacts) store: Arc<ManagedArtifactStore>,
  pub(in crate::artifacts) policy: Arc<PreparedArtifactPolicy>,
  pub(in crate::artifacts) concurrency: Arc<Semaphore>,
  pub(in crate::artifacts) known_secrets: Arc<Vec<Vec<u8>>>,
}

impl CapabilityHandler for ManagedArtifactPutHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "artifacts".to_string(),
      operation: "put".to_string(),
      input_contract_version: ARTIFACTS_CONTRACT_VERSION,
      result_contract_version: ARTIFACTS_CONTRACT_VERSION,
      effect: CapabilityEffect::IdempotentWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    let parsed = parse_put(&request.input, &self.policy)?;
    if request.identity.mode != CapabilityIdentityMode::Named
      || request.identity.operation_name != parsed.input.name
    {
      return Err(operation_identity_conflict());
    }
    Ok(())
  }

  fn safe_request_metadata(
    &self,
    _request: &CapabilityCallRequest,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    Ok(Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.artifact-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!("put")),
      ("outcome".to_string(), json!("started")),
    ]))
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let mut metadata = Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.artifact-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!("put")),
      ("outcome".to_string(), json!("succeeded")),
    ]);
    let result = result.get("data").unwrap_or(result);
    for (from, to) in [("id", "artifactId"), ("size", "size")] {
      if let Some(value) = result.get(from) {
        metadata.insert(to.to_string(), value.clone());
      }
    }
    if let Some(digest) = result.get("sha256").and_then(Value::as_str) {
      metadata.insert(
        "contentDigest".to_string(),
        json!(format!("sha256:{digest}")),
      );
    }
    metadata
  }

  fn safe_failure_metadata(&self, failure: &CapabilityFailure) -> Map<String, Value> {
    Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.artifact-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!("put")),
      (
        "outcome".to_string(),
        json!(if failure.kind == CapabilityFailureKind::Cancelled {
          "cancelled"
        } else if failure.code == "WOML_ARTIFACT_INTERRUPTED" {
          "interrupted"
        } else {
          "failed"
        }),
      ),
      ("errorCode".to_string(), json!(failure.code)),
      ("ambiguous".to_string(), json!(failure.ambiguous)),
    ])
  }

  fn execute(
    &self,
    _input: Value,
    _cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    Box::pin(async { Err(backend_unavailable()) })
  }

  fn execute_request_scoped(
    &self,
    request: &CapabilityCallRequest,
    workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = request.clone();
    let parsed = match parse_put(&request.input, &self.policy) {
      Ok(parsed) => parsed,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let store = match self.store.configured() {
      Ok(store) => store,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let Some(workflow_scope) = workflow_scope else {
      return Box::pin(async { Err(backend_unavailable()) });
    };
    let concurrency = Arc::clone(&self.concurrency);
    let known_secrets = Arc::clone(&self.known_secrets);
    let timeout = Duration::from_millis(self.policy.limits.transfer_timeout_ms);
    Box::pin(async move {
      let permit = tokio::select! {
        permit = concurrency.acquire_owned() => permit.map_err(|_| backend_unavailable())?,
        _ = cancellation.cancelled() => return Err(artifact_cancelled()),
      };
      let task_cancellation = cancellation.clone();
      let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        store
          .put(
            &request,
            &workflow_scope,
            &parsed,
            &task_cancellation,
            &known_secrets,
          )
          .and_then(|reference| {
            Ok(json!({
              "contract": ARTIFACTS_CONTRACT,
              "contractVersion": ARTIFACTS_CONTRACT_VERSION,
              "kind": "result",
              "operation": "put",
              "data": serde_json::to_value(reference).map_err(|_| artifact_corrupt())?
            }))
          })
      });
      match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(artifact_failure(
          CapabilityFailureKind::HandlerCrashed,
          "WOML_ARTIFACT_BACKEND_UNAVAILABLE",
          "The artifact backend is unavailable.",
          true,
          true,
        )),
        Err(_) => {
          cancellation.cancel();
          Err(artifact_timed_out())
        }
      }
    })
  }
}

impl ManagedArtifactPutHandler {
  pub fn new(
    store: Arc<ManagedArtifactStore>,
    policy: ArtifactPolicyV1,
    mode: ArtifactRuntimeMode,
    resolved_secrets: &BTreeMap<String, String>,
  ) -> Result<Self, ArtifactPolicyError> {
    let policy = Arc::new(PreparedArtifactPolicy::prepare(
      policy,
      mode,
      resolved_secrets,
    )?);
    let concurrency = Arc::new(Semaphore::new(policy.limits.max_concurrent_transfers));
    let mut known_secrets = resolved_secrets
      .values()
      .map(|secret| secret.as_bytes().to_vec())
      .filter(|secret| secret.len() >= policy.minimum_secret_bytes)
      .collect::<Vec<_>>();
    if let PreparedArtifactBackend::S3(backend) = &policy.backend {
      for value in [
        Some(&backend.credentials.access_key),
        Some(&backend.credentials.secret_key),
        backend.credentials.session_token.as_ref(),
      ]
      .into_iter()
      .flatten()
      {
        if value.len() >= policy.minimum_secret_bytes {
          let bytes = value.as_bytes().to_vec();
          if !known_secrets.contains(&bytes) {
            known_secrets.push(bytes);
          }
        }
      }
    }
    Ok(Self {
      store,
      policy,
      concurrency,
      known_secrets: Arc::new(known_secrets),
    })
  }

  pub(crate) fn policy(&self) -> Arc<PreparedArtifactPolicy> {
    Arc::clone(&self.policy)
  }

  pub fn head_handler(&self) -> ManagedArtifactReadHandler {
    ManagedArtifactReadHandler {
      store: Arc::clone(&self.store),
      policy: Arc::clone(&self.policy),
      concurrency: Arc::clone(&self.concurrency),
      operation: ArtifactReadOperation::Head,
    }
  }

  pub fn materialize_handler(&self) -> ManagedArtifactReadHandler {
    ManagedArtifactReadHandler {
      store: Arc::clone(&self.store),
      policy: Arc::clone(&self.policy),
      concurrency: Arc::clone(&self.concurrency),
      operation: ArtifactReadOperation::Materialize,
    }
  }
}
