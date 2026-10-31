//! Private runtime engine interface with unchanged defaults and supported-operation boundaries. The interface is not an independent scheduler or persistence guarantee.

mod durable;
mod in_memory;

use crate::event::ApprovalRequestedData;
use crate::event::ForEachIterationScope;
use crate::event::LifecycleFailure;
use crate::event::RunOutcomeDecidedData;
use crate::event::StepAttemptFailedData;
use crate::event::StepAttemptStartedData;
use crate::event::StepAttemptSucceededData;
use crate::run_event_schema_version_for_model;
use crate::runtime::failure::attempt_run_failed_data;
use crate::runtime::generated_id;
use crate::runtime::RuntimeExecutionError;
use crate::runtime::WorkflowRuntimeOutcome;
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

pub(super) trait RuntimeDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition;
  fn definition_hash(&self) -> &str;
  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    for payload in payloads {
      self.append_scoped_payload(run_id, iteration.clone(), payload)?;
    }
    Ok(())
  }
  fn settle_workflow_timeout(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(
      run_id,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Cancelled {
        cancellation_request_id,
      }),
    )
  }
  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(
      run_id,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Failed { failure }),
    )
  }
  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    _failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    if self
      .workflow()
      .parallel_group_for_child(&failure.node_id)
      .is_some()
    {
      self.append_payload(run_id, RunEventPayload::StepAttemptFailed(failure))?;
      return Ok(StepFailureDisposition::StepFailed);
    }
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
      return Err(RuntimeExecutionError::Stalled(
        "Model v6 retry failures require the durable runtime".to_string(),
      ));
    }
    let run_failed = attempt_run_failed_data(self.event_schema_version(), &failure);
    self.append_payloads(
      run_id,
      vec![
        RunEventPayload::StepAttemptFailed(failure),
        RunEventPayload::RunFailed(run_failed),
      ],
    )?;
    Ok(StepFailureDisposition::RunFailed)
  }
  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError>;
  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError>;
  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError>;
  fn correction_action_externally_resolved(
    &self,
    _run_id: &str,
    _saga_id: &str,
    _action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn request_approval(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "human approval requires the durable runtime".to_string(),
    ))
  }
  fn request_rich_approval(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "rich plan approval requires the durable runtime".to_string(),
    ))
  }
  fn settle_pending_rich_approval(
    &mut self,
    _run_id: &str,
    _reason: RichApprovalResolutionReason,
    _settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn register_durable_wait(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable waits require the durable runtime".to_string(),
    ))
  }
  fn register_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _key: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn release_lock(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _result: Value,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn timeout_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn renew_lock_lease(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable lock renewal requires the durable runtime".to_string(),
    ))
  }
  fn renew_iteration_lock_lease(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock renewal requires the durable runtime".to_string(),
    ))
  }
  fn cancel_lock_waiters(
    &mut self,
    _run_id: &str,
    _cancellation_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable lock cancellation requires the durable runtime".to_string(),
    ))
  }
  fn register_iteration_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _key: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
    _deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock acquisition requires the durable runtime".to_string(),
    ))
  }
  fn release_iteration_lock(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _result: Value,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock release requires the durable runtime".to_string(),
    ))
  }
  fn timeout_iteration_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock timeout requires the durable runtime".to_string(),
    ))
  }
  fn settle_timer_wait(
    &mut self,
    _run_id: &str,
    _wait_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable timer waits require the durable runtime".to_string(),
    ))
  }
  fn reissue_waiting_outcome(
    &mut self,
    _run_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "human approval requires the durable runtime".to_string(),
    ))
  }
  fn append_payloads(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    for payload in payloads {
      self.append_payload(run_id, payload)?;
    }
    Ok(())
  }
  fn event_schema_version(&self) -> u32 {
    run_event_schema_version_for_model(self.workflow().schema_version)
  }
  fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    output: Value,
  ) -> Result<(), RuntimeExecutionError> {
    let invocation_id = generated_id("inv");
    let handler = self
      .workflow()
      .node(node_id)
      .map(|node| node.handler.clone())
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("pure result node {node_id:?} disappeared"))
      })?;
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id: invocation_id.clone(),
        handler,
        idempotency_key: None,
      }),
    )?;
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id,
        output,
      }),
    )?;
    Ok(())
  }
}
