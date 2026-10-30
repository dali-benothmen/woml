//! Shared iteration adapter over one borrowed runtime engine.
//!
//! Clones share the original Arc<Mutex<&mut E>>. Short with_engine operations
//! serialize projection/event access; immutable workflow/hash are shared
//! separately. This is not a database, scheduler, or event log per item.

use crate::event::ApprovalRequestedData;
use crate::event::ForEachIterationScope;
use crate::event::LifecycleFailure;
use crate::event::StepAttemptFailedData;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::outcomes::WorkflowRuntimeOutcome;
use crate::runtime::RuntimeDagEngine;
use crate::BoundRichApprovalReview;
use crate::CompiledWorkflowDefinition;
use crate::IssuedApprovalToken;
use crate::RichApprovalResolutionReason;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::StepFailureDisposition;
use serde_json::Map;
use serde_json::Value;
use std::sync::Arc;

/// Serializes durable event/projection access while allowing iteration futures
/// to wait on the multiplexed Bun host concurrently. The compiled definition is
/// immutable and is shared without taking the engine lock.
pub(in crate::runtime) struct SharedForEachEngine<'a, E> {
  pub(in crate::runtime) inner: Arc<std::sync::Mutex<&'a mut E>>,
  pub(in crate::runtime) workflow: Arc<CompiledWorkflowDefinition>,
  pub(in crate::runtime) definition_hash: Arc<String>,
}

impl<E> Clone for SharedForEachEngine<'_, E> {
  fn clone(&self) -> Self {
    Self {
      inner: Arc::clone(&self.inner),
      workflow: Arc::clone(&self.workflow),
      definition_hash: Arc::clone(&self.definition_hash),
    }
  }
}

impl<'a, E: RuntimeDagEngine> SharedForEachEngine<'a, E> {
  pub(in crate::runtime) fn new(engine: &'a mut E) -> Self {
    Self {
      workflow: Arc::new(engine.workflow().clone()),
      definition_hash: Arc::new(engine.definition_hash().to_string()),
      inner: Arc::new(std::sync::Mutex::new(engine)),
    }
  }

  pub(in crate::runtime) fn with_engine<T>(
    &self,
    operation: impl FnOnce(&mut E) -> Result<T, RuntimeExecutionError>,
  ) -> Result<T, RuntimeExecutionError> {
    let mut engine = self.inner.lock().map_err(|_| {
      RuntimeExecutionError::Stalled("the shared for-each event authority was poisoned".to_string())
    })?;
    operation(&mut **engine)
  }
}

impl<E: RuntimeDagEngine> RuntimeDagEngine for SharedForEachEngine<'_, E> {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow.as_ref()
  }

  fn definition_hash(&self) -> &str {
    self.definition_hash.as_str()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.start_run(run_id, trigger))
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.append_payload(run_id, payload))
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.append_scoped_payload(run_id, iteration, payload))
  }

  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.append_scoped_payloads(run_id, iteration, payloads))
  }

  fn settle_workflow_timeout(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    self.with_engine(|engine| engine.settle_workflow_timeout(run_id, occurred_at))
  }

  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| {
      engine.decide_run_cancelled(run_id, cancellation_request_id, occurred_at)
    })
  }

  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.decide_run_failed(run_id, failure, occurred_at))
  }

  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    self.with_engine(|engine| engine.record_step_attempt_failure(run_id, failed_at, failure))
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    self.with_engine(|engine| engine.projection(run_id))
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    self.with_engine(|engine| engine.events(run_id))
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    self.with_engine(|engine| engine.ready_node_ids(run_id))
  }

  fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    self.with_engine(|engine| {
      engine.correction_action_externally_resolved(run_id, saga_id, action_id)
    })
  }

  fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    self.with_engine(|engine| engine.request_approval(run_id, occurred_at, request))
  }

  fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.request_rich_approval(run_id, occurred_at, binding))
  }

  fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    self.with_engine(|engine| engine.settle_pending_rich_approval(run_id, reason, settled_at))
  }

  fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.register_durable_wait(run_id, occurred_at, registration))
  }

  fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    self.with_engine(|engine| engine.settle_timer_wait(run_id, wait_id, now))
  }

  fn reissue_waiting_outcome(
    &mut self,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    self.with_engine(|engine| engine.reissue_waiting_outcome(run_id, now))
  }
}
