//! Provider-neutral handler interface and defaults. Handler selection cannot authorize a workflow attempt.

use crate::capability::CapabilityCallRequest;
use crate::capability::CapabilityCancellationToken;
use crate::capability::CapabilityDescriptor;
use crate::capability::CapabilityEffect;
use crate::capability::CapabilityFailure;
use crate::infrastructure_policy_runtime::adapt_infrastructure_policy_metadata;
use crate::infrastructure_policy_runtime::InfrastructurePolicyOperationMetadataV1;
use futures_util::future::BoxFuture;
use serde_json::Map;
use serde_json::Value;

pub trait CapabilityHandler: Send + Sync + 'static {
  fn descriptor(&self) -> CapabilityDescriptor;

  fn validate_request(&self, _request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    Ok(())
  }

  fn effect(&self, _input: &Value) -> CapabilityEffect {
    self.descriptor().effect
  }

  fn safe_metadata(&self, _input: &Value) -> Map<String, Value> {
    Map::new()
  }

  /// Produces safe start metadata with the verified engine call identity.
  /// Existing handlers remain input-only; identity-sensitive handlers such as
  /// Workflow Calls can bind a deterministic child before dispatch.
  fn safe_request_metadata(
    &self,
    request: &CapabilityCallRequest,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    Ok(self.safe_metadata(&request.input))
  }

  /// Produces bounded policy-only facts after the request has passed the
  /// registered handler's normal validation. Policy facts are deliberately
  /// separate from presentation and durable operation metadata.
  fn infrastructure_policy_metadata(
    &self,
    request: &CapabilityCallRequest,
  ) -> Result<InfrastructurePolicyOperationMetadataV1, CapabilityFailure> {
    adapt_infrastructure_policy_metadata(&self.descriptor(), request)
  }

  fn safe_result_metadata(&self, _result: &Value) -> Map<String, Value> {
    Map::new()
  }

  fn safe_failure_metadata(&self, _failure: &CapabilityFailure) -> Map<String, Value> {
    Map::new()
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>>;

  /// Executes with engine-owned identity that is deliberately absent from the
  /// frozen Capability Call v1 wire protocol. Most handlers do not need it.
  fn execute_scoped(
    &self,
    input: Value,
    workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let _ = workflow_scope;
    self.execute(input, cancellation)
  }

  /// Executes with the complete engine-verified call identity. This remains an
  /// internal Rust boundary: it does not add fields to Capability Call v1.
  /// Handlers whose correctness depends on the originating run or stable
  /// operation identity may override it; all other handlers keep the scoped
  /// behavior above.
  fn execute_request_scoped(
    &self,
    request: &CapabilityCallRequest,
    workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    self.execute_scoped(request.input.clone(), workflow_scope, cancellation)
  }
}
