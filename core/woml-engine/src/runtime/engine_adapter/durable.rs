//! Runtime adaptation to the bound durable DAG engine. Preserve grouped scoped appends, specialized failure/retry disposition, waiting outcomes, and exact attempt identities.

use crate::event::ApprovalRequestedData;
use crate::event::ApprovalTimeoutPolicy;
use crate::event::ForEachIterationScope;
use crate::event::LifecycleFailure;
use crate::event::RunOutcomeDecidedData;
use crate::event::StepAttemptFailedData;
use crate::projection::ApprovalRequestStatus;
use crate::runtime::control_flow::rich_waiting_outcome;
use crate::runtime::control_flow::waiting_outcome;
use crate::runtime::failure::attempt_run_failed_data;
use crate::runtime::RuntimeExecutionError;
use crate::runtime::WaitingWorkflowSignal;
use crate::runtime::WaitingWorkflowTimer;
use crate::runtime::WorkflowRuntimeOutcome;
use crate::runtime::RUNTIME_OUTCOME_CONTRACT;
use crate::runtime::RUNTIME_OUTCOME_VERSION;
use crate::runtime::{generated_id, RuntimeDagEngine};
use crate::BoundRichApprovalReview;
use crate::CompiledWorkflowDefinition;
use crate::DurableDagEngine;
use crate::IssuedApprovalToken;
use crate::RichApprovalResolutionReason;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunTimeoutSettlement;
use crate::StepFailureDisposition;
use serde_json::Map;
use serde_json::Value;

impl RuntimeDagEngine for DurableDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow()
  }

  fn definition_hash(&self) -> &str {
    self.definition_hash()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V7 {
      let trigger_definition = self
        .workflow()
        .triggers
        .iter()
        .find(|candidate| candidate.handler == "trigger.manual")
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "a direct Model v7+ run requires a manual trigger".to_string(),
          )
        })?;
      DurableDagEngine::append_payload(
        self,
        generated_id("evt"),
        run_id,
        chrono::Utc::now(),
        RunEventPayload::RunStarted(crate::RunStartedData {
          workflow_id: self.workflow().workflow_id.clone(),
          definition_hash: self.definition_hash().to_string(),
          trigger_id: Some(trigger_definition.id.clone()),
          trigger_handler: Some(trigger_definition.handler.clone()),
          trigger_occurrence_id: Some(generated_id("occ")),
          ingress: None,
          trigger,
        }),
      )?;
      return Ok(());
    }
    self.start_run(generated_id("evt"), run_id, chrono::Utc::now(), trigger)?;
    Ok(())
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(generated_id("evt"), run_id, chrono::Utc::now(), payload)?;
    Ok(())
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_scoped_payload(
      generated_id("evt"),
      run_id,
      chrono::Utc::now(),
      iteration,
      payload,
    )?;
    Ok(())
  }

  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_scoped_payloads_atomically(run_id, iteration, payloads)?;
    Ok(())
  }

  fn settle_workflow_timeout(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(matches!(
      DurableDagEngine::settle_run_timeout(self, run_id, occurred_at)?,
      RunTimeoutSettlement::TimedOut { .. }
    ))
  }

  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::decide_run_outcome(
      self,
      run_id,
      RunOutcomeDecidedData::Cancelled {
        cancellation_request_id,
      },
      occurred_at,
    )?;
    Ok(())
  }

  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::decide_run_outcome(
      self,
      run_id,
      RunOutcomeDecidedData::Failed { failure },
      occurred_at,
    )?;
    Ok(())
  }

  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
      return Ok(
        DurableDagEngine::record_step_attempt_failure(self, run_id, failed_at, failure)?
          .disposition,
      );
    }
    if self
      .workflow()
      .parallel_group_for_child(&failure.node_id)
      .is_some()
    {
      DurableDagEngine::append_payload(
        self,
        generated_id("evt"),
        run_id,
        failed_at,
        RunEventPayload::StepAttemptFailed(failure),
      )?;
      return Ok(StepFailureDisposition::StepFailed);
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

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    Ok(self.projection(run_id)?)
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    Ok(self.events(run_id)?)
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    Ok(self.ready_node_ids(run_id)?)
  }

  fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(DurableDagEngine::correction_action_externally_resolved(
      self, run_id, saga_id, action_id,
    )?)
  }

  fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    Ok(DurableDagEngine::request_approval(
      self,
      run_id,
      occurred_at,
      request,
    )?)
  }

  fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::request_rich_approval(self, run_id, occurred_at, binding)?;
    Ok(())
  }

  fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(DurableDagEngine::settle_pending_rich_approval(
      self, run_id, reason, settled_at,
    )?)
  }

  fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_durable_wait(self, run_id, occurred_at, registration)?;
    Ok(())
  }

  fn register_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_lock_waiter(self, run_id, lock_id, key, occurred_at, deadline)?;
    Ok(())
  }

  fn release_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::release_lock(self, run_id, lock_id, result, occurred_at)?;
    Ok(())
  }

  fn timeout_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::timeout_lock_waiter(self, run_id, lock_id, occurred_at)?;
    Ok(())
  }

  fn renew_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Ok(DurableDagEngine::renew_lock_lease(
      self, run_id, lock_id, now,
    )?)
  }

  fn renew_iteration_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Ok(DurableDagEngine::renew_iteration_lock_lease(
      self, run_id, lock_id, iteration, now,
    )?)
  }

  fn cancel_lock_waiters(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::cancel_lock_waiters(self, run_id, cancellation_id, now)?;
    Ok(())
  }

  fn register_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_iteration_lock_waiter(
      self, run_id, lock_id, key, iteration, now, deadline,
    )?;
    Ok(())
  }

  fn release_iteration_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    result: Value,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::release_iteration_lock(self, run_id, lock_id, iteration, result, now)?;
    Ok(())
  }

  fn timeout_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::timeout_iteration_lock_waiter(self, run_id, lock_id, iteration, now)?;
    Ok(())
  }

  fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    Ok(DurableDagEngine::settle_timer_wait(
      self, run_id, wait_id, now,
    )?)
  }

  fn reissue_waiting_outcome(
    &mut self,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    let projection = self.projection(run_id)?;
    if let Some(request) = projection
      .rich_approval_requests
      .values()
      .find(|request| matches!(request.status, crate::RichApprovalRequestStatus::Waiting))
      .cloned()
    {
      let approval = self
        .workflow()
        .approval(&request.approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "waiting run references an unknown rich approval definition".to_string(),
          )
        })?;
      let inspection = DurableDagEngine::inspect_rich_approval(self, run_id, &request.request_id)?;
      let expires_at = approval
        .timeout_ms
        .map(|milliseconds| {
          i64::try_from(milliseconds)
            .map(chrono::Duration::milliseconds)
            .map(|duration| request.requested_at + duration)
        })
        .transpose()
        .map_err(|_| {
          RuntimeExecutionError::Stalled(
            "rich approval timeout exceeds the clock range".to_string(),
          )
        })?;
      let waiting_expires_at = match (expires_at, projection.timeout_at) {
        (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
        (Some(approval), None) => Some(approval),
        (None, Some(workflow)) => Some(workflow),
        (None, None) => None,
      };
      if waiting_expires_at.is_some_and(|deadline| now >= deadline) {
        return Err(RuntimeExecutionError::Stalled(
          "rich approval deadline must be settled before wait reissue".to_string(),
        ));
      }
      let on_timeout = if projection
        .timeout_at
        .is_some_and(|workflow| expires_at.is_none_or(|approval| workflow <= approval))
      {
        ApprovalTimeoutPolicy::Fail
      } else if approval.on_timeout == "reject" {
        ApprovalTimeoutPolicy::Reject
      } else {
        ApprovalTimeoutPolicy::Fail
      };
      let workflow_version = self
        .workflow()
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.version.as_deref())
        .unwrap_or("0.0.0")
        .to_string();
      return Ok(rich_waiting_outcome(
        self.workflow().workflow_id.clone(),
        run_id.to_string(),
        approval,
        waiting_expires_at,
        on_timeout,
        BoundRichApprovalReview {
          profile: crate::RICH_PLAN_APPROVAL_PROFILE.to_string(),
          definition_hash: self.definition_hash().to_string(),
          workflow_id: self.workflow().workflow_id.clone(),
          workflow_version,
          run_id: run_id.to_string(),
          approval_id: request.approval_id,
          request_id: request.request_id,
          artifacts: inspection.review.artifacts.clone(),
          review: inspection.review,
          required_approvals: inspection.required,
          required_roles: inspection.required_roles,
          directory_version: inspection.directory_version,
          review_digest: inspection.review_digest,
        },
        request
          .decisions
          .values()
          .filter(|decision| decision.decision == crate::ApprovalDecision::Approved)
          .count() as u32,
        request.represented_roles,
      ));
    }
    if let Some(request) = projection
      .approval_requests
      .values()
      .find(|request| matches!(request.status, ApprovalRequestStatus::Waiting))
      .cloned()
    {
      let approval = self
        .workflow()
        .approval(&request.approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "waiting run references an unknown approval definition".to_string(),
          )
        })?;
      let token = DurableDagEngine::reissue_waiting_approval_token(
        self,
        run_id,
        &request.approval_id,
        &request.request_id,
        now,
      )?;
      return Ok(waiting_outcome(
        self.workflow().workflow_id.clone(),
        run_id.to_string(),
        approval,
        request.request_id,
        request.expires_at,
        request.on_timeout,
        token,
      ));
    }
    let wait = projection
      .durable_waits
      .values()
      .find(|wait| wait.status == crate::DurableWaitStatus::Registered)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "waiting run has no unresolved approval or durable wait".to_string(),
        )
      })?;
    if wait.kind == crate::DurableWaitKind::Signal {
      return Ok(WorkflowRuntimeOutcome::WaitingSignal {
        contract: RUNTIME_OUTCOME_CONTRACT,
        version: RUNTIME_OUTCOME_VERSION,
        workflow_id: self.workflow().workflow_id.clone(),
        run_id: run_id.to_string(),
        wait: WaitingWorkflowSignal {
          wait_id: wait.wait_id.clone(),
          node_id: wait.node_id.clone(),
          registered_at: wait.registered_at,
          signals: wait.signals.clone(),
          deadline: wait.deadline,
        },
      });
    }
    let deadline = wait.deadline.ok_or_else(|| {
      RuntimeExecutionError::Stalled("durable timer wait has no deadline".to_string())
    })?;
    Ok(WorkflowRuntimeOutcome::WaitingTime {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      workflow_id: self.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      wait: WaitingWorkflowTimer {
        wait_id: wait.wait_id.clone(),
        node_id: wait.node_id.clone(),
        kind: wait.kind,
        registered_at: wait.registered_at,
        deadline,
      },
    })
  }

  fn append_payloads(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payloads_atomically(run_id, payloads)?;
    Ok(())
  }

  fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    output: Value,
  ) -> Result<(), RuntimeExecutionError> {
    let invocation_id = generated_id("inv");
    self.publish_pure_result(run_id, node_id, &invocation_id, output)?;
    Ok(())
  }
}
