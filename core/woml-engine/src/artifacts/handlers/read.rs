//! Identity-bound head/materialize capability handlers with existing timeout, cancellation, and safe-result rules.

use crate::artifacts::artifact_access_denied;
use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_failure;
use crate::artifacts::artifact_timed_out;
use crate::artifacts::backend_unavailable;
use crate::artifacts::parse_reference_request;
use crate::artifacts::ManagedArtifactStore;
use crate::artifacts::PreparedArtifactPolicy;
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
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::artifacts) enum ArtifactReadOperation {
  Head,
  Materialize,
}

impl ArtifactReadOperation {
  pub(in crate::artifacts) fn as_str(self) -> &'static str {
    match self {
      Self::Head => "head",
      Self::Materialize => "materialize",
    }
  }
}

#[derive(Debug)]
pub struct ManagedArtifactReadHandler {
  pub(in crate::artifacts) store: Arc<ManagedArtifactStore>,
  pub(in crate::artifacts) policy: Arc<PreparedArtifactPolicy>,
  pub(in crate::artifacts) concurrency: Arc<Semaphore>,
  pub(in crate::artifacts) operation: ArtifactReadOperation,
}

impl CapabilityHandler for ManagedArtifactReadHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "artifacts".to_string(),
      operation: self.operation.as_str().to_string(),
      input_contract_version: ARTIFACTS_CONTRACT_VERSION,
      result_contract_version: ARTIFACTS_CONTRACT_VERSION,
      effect: CapabilityEffect::Read,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse_reference_request(&request.input, self.operation.as_str())?;
    if request.identity.mode != CapabilityIdentityMode::Automatic {
      return Err(artifact_access_denied());
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
      ("operation".to_string(), json!(self.operation.as_str())),
      ("outcome".to_string(), json!("started")),
    ]))
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let mut metadata = Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.artifact-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!(self.operation.as_str())),
      ("outcome".to_string(), json!("succeeded")),
    ]);
    let artifact = result.get("data").and_then(|data| data.get("artifact"));
    if let Some(artifact) = artifact {
      for (from, to) in [("id", "artifactId"), ("size", "size")] {
        if let Some(value) = artifact.get(from) {
          metadata.insert(to.to_string(), value.clone());
        }
      }
      if let Some(digest) = artifact.get("sha256").and_then(Value::as_str) {
        metadata.insert(
          "contentDigest".to_string(),
          json!(format!("sha256:{digest}")),
        );
      }
    }
    metadata
  }

  fn safe_failure_metadata(&self, failure: &CapabilityFailure) -> Map<String, Value> {
    Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.artifact-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!(self.operation.as_str())),
      (
        "outcome".to_string(),
        json!(if failure.kind == CapabilityFailureKind::Cancelled {
          "cancelled"
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
    _workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = request.clone();
    let parsed = match parse_reference_request(&request.input, self.operation.as_str()) {
      Ok(parsed) => parsed,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let store = match self.store.configured() {
      Ok(store) => store,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let operation = self.operation;
    let concurrency = Arc::clone(&self.concurrency);
    let timeout = Duration::from_millis(self.policy.limits.transfer_timeout_ms);
    Box::pin(async move {
      let permit = tokio::select! {
        permit = concurrency.acquire_owned() => permit.map_err(|_| backend_unavailable())?,
        _ = cancellation.cancelled() => return Err(artifact_cancelled()),
      };
      let task_cancellation = cancellation.clone();
      let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match operation {
          ArtifactReadOperation::Head => store.head(&request, &parsed),
          ArtifactReadOperation::Materialize => {
            store.materialize(&request, &parsed, &task_cancellation)
          }
        }
      });
      match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(artifact_failure(
          CapabilityFailureKind::HandlerCrashed,
          "WOML_ARTIFACT_BACKEND_UNAVAILABLE",
          "The artifact backend is unavailable.",
          true,
          operation == ArtifactReadOperation::Materialize,
        )),
        Err(_) => {
          cancellation.cancel();
          Err(artifact_timed_out())
        }
      }
    })
  }
}
