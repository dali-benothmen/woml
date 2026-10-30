//! Definition-bound durable DAG execution adapter. Preserve specialized atomic failure/retry and success/compensation paths instead of generic per-event substitutes.

use crate::durable::expand_model_v11_payload;
use crate::durable::generated_event_id;
use crate::durable::DurableStoreError;
use crate::durable::IssuedApprovalToken;
use crate::durable::LockLeaseMaintenance;
use crate::durable::RecoveryReport;
use crate::durable::RichApprovalInspectionV1;
use crate::durable::RunTimeoutSettlement;
use crate::durable::TimerWaitSettlement;
use crate::durable::RICH_APPROVAL_INSPECTION_DECISION_LIMIT;
use crate::durable::{attempt_run_failed_data, DurableEventStore};
use crate::engine::ready_node_ids_for_projection;
use crate::engine::ready_node_ids_for_projection_at;
use crate::engine::step_effect_idempotency_key;
use crate::engine::validate_payload_against_definition;
use crate::engine::validate_scoped_payload_against_definition;
use crate::event::ApprovalRequestedData;
use crate::event::ForEachIterationScope;
use crate::event::RichApprovalResolutionReason;
use crate::event::StepAttemptFailedData;
use crate::event::StepRetryScheduledData;
use crate::projection::AttemptStatus;
use crate::run_event_schema_version_for_model;
use crate::AttemptFailureKind;
use crate::BoundRichApprovalReview;
use crate::CompiledWorkflowDefinition;
use crate::ModelValidationError;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DurableEngineError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error("{0}")]
  Contract(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepFailureDisposition {
  RetryScheduled {
    next_attempt: u32,
    scheduled_at: DateTime<Utc>,
  },
  StepFailed,
  RunFailureDeferred,
  RunFailed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepFailureCommit {
  pub disposition: StepFailureDisposition,
  pub projection: RunProjection,
}

#[derive(Debug)]
pub struct DurableDagEngine {
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  store: DurableEventStore,
}

impl DurableDagEngine {
  pub fn new(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
    mut store: DurableEventStore,
  ) -> Result<Self, DurableEngineError> {
    workflow.validate_for_durable_execution()?;
    let definition_hash = definition_hash.into();
    store.register_definition(&workflow, &definition_hash)?;
    Ok(Self {
      workflow,
      definition_hash,
      store,
    })
  }

  pub fn new_for_event_history(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
    mut store: DurableEventStore,
  ) -> Result<Self, DurableEngineError> {
    workflow.validate_structure()?;
    let definition_hash = definition_hash.into();
    store.register_definition(&workflow, &definition_hash)?;
    Ok(Self {
      workflow,
      definition_hash,
      store,
    })
  }

  pub fn resume(store: DurableEventStore, run_id: &str) -> Result<Self, DurableEngineError> {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    if workflow.workflow_id != binding.workflow_id {
      return Err(DurableEngineError::Contract(
        "Stored run binding does not match its compiled definition.".to_string(),
      ));
    }
    Ok(Self {
      workflow,
      definition_hash: binding.definition_hash,
      store,
    })
  }

  pub fn workflow(&self) -> &CompiledWorkflowDefinition {
    &self.workflow
  }

  pub fn definition_hash(&self) -> &str {
    &self.definition_hash
  }

  pub fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, DurableEngineError> {
    Ok(
      self
        .store
        .correction_action_externally_resolved(run_id, saga_id, action_id)?,
    )
  }

  pub fn decide_run_outcome(
    &mut self,
    run_id: &str,
    outcome: crate::event::RunOutcomeDecidedData,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .decide_run_outcome(run_id, outcome, occurred_at)?,
    )
  }

  pub fn settle_run_timeout(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunTimeoutSettlement, DurableEngineError> {
    Ok(self.store.settle_run_timeout(run_id, occurred_at)?)
  }

  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    trigger: Map<String, Value>,
  ) -> Result<RunProjection, DurableEngineError> {
    let (_, projection) = self.store.start_run(
      event_id,
      run_id,
      occurred_at,
      self.workflow.workflow_id.clone(),
      self.definition_hash.clone(),
      trigger,
    )?;
    Ok(projection)
  }

  pub fn append_payload(
    &mut self,
    event_id: impl Into<String>,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    payload: RunEventPayload,
  ) -> Result<RunProjection, DurableEngineError> {
    let event_id = event_id.into();
    if matches!(
      run_event_schema_version_for_model(self.workflow.schema_version),
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) && matches!(
      &payload,
      RunEventPayload::RunSucceeded(_) | RunEventPayload::RunFailed(_)
    ) {
      let translated = expand_model_v11_payload(&self.workflow, run_id, payload)?;
      return Ok(
        self.store.append_payloads_atomically(
          run_id,
          translated
            .into_iter()
            .enumerate()
            .map(|(index, payload)| {
              (
                if index == 0 {
                  event_id.clone()
                } else {
                  generated_event_id()
                },
                occurred_at,
                payload,
              )
            })
            .collect(),
        )?,
      );
    }
    validate_payload_against_definition(&self.workflow, &self.definition_hash, &payload)
      .map_err(DurableEngineError::Contract)?;
    if let RunEventPayload::StepAttemptStarted(data) = &payload {
      let ready = self.ready_node_ids(run_id)?;
      if !ready.iter().any(|node_id| node_id == &data.node_id) {
        return Err(DurableEngineError::Contract(format!(
          "Node {:?} is not ready for execution.",
          data.node_id
        )));
      }
    }
    if self.workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6
      && matches!(
        payload,
        RunEventPayload::StepAttemptFailed(_) | RunEventPayload::StepRetryScheduled(_)
      )
    {
      return Err(DurableEngineError::Contract(
        "Model v6 step failures and retry schedules must use the atomic failure API.".to_string(),
      ));
    }
    let (_, projection) = self
      .store
      .append_payload(run_id, event_id, occurred_at, payload)?;
    Ok(projection)
  }

  pub fn append_scoped_payload(
    &mut self,
    event_id: impl Into<String>,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    iteration: crate::event::ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<RunProjection, DurableEngineError> {
    validate_scoped_payload_against_definition(
      &self.workflow,
      &self.definition_hash,
      run_id,
      &iteration,
      &payload,
    )
    .map_err(DurableEngineError::Contract)?;
    let (_, projection) =
      self
        .store
        .append_payload_scoped(run_id, event_id, occurred_at, Some(iteration), payload)?;
    Ok(projection)
  }

  pub fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: DateTime<Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureCommit, DurableEngineError> {
    if self.workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
      return Err(DurableEngineError::Contract(
        "The atomic retry failure API requires compiled model v6.".to_string(),
      ));
    }
    let node = self.workflow.node(&failure.node_id).ok_or_else(|| {
      DurableEngineError::Contract(format!(
        "Attempt failure references unknown node {:?}.",
        failure.node_id
      ))
    })?;
    let parallel_child = self
      .workflow
      .parallel_group_for_child(&failure.node_id)
      .is_some();
    let fork_child = self.workflow.fork_branch_owner(&failure.node_id).is_some();
    let reusable_lifecycle =
      self
        .workflow
        .reusable_definitions
        .iter()
        .flatten()
        .any(|definition| {
          matches!(
            definition,
            crate::model::CompiledReusableInvocation::Step {
              node_id,
              lifecycle: Some(_),
              ..
            } if node_id == &failure.node_id
          )
        });
    let lock_body = self
      .workflow
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .any(|lock| {
        let mut pending = vec![lock.body_entry_node_id.as_str()];
        let mut visited = HashSet::new();
        while let Some(candidate) = pending.pop() {
          if !visited.insert(candidate) {
            continue;
          }
          if candidate == failure.node_id {
            return true;
          }
          if candidate == lock.body_terminal_node_id {
            continue;
          }
          pending.extend(
            self
              .workflow
              .graph
              .edges
              .iter()
              .filter(|edge| edge.from == candidate)
              .map(|edge| edge.to.as_str()),
          );
        }
        false
      });
    let saga_body = self.workflow.saga_for_body_node(&failure.node_id).is_some();
    let projection = self.projection(run_id)?;
    let fork_coordinated = self.workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13
      && projection.forks.values().any(|fork| {
        fork.join_status == crate::projection::ForkJoinStatus::Pending
          || fork.branches.len()
            < self
              .workflow
              .graph
              .forks
              .as_deref()
              .unwrap_or_default()
              .iter()
              .find(|definition| definition.fork_id == fork.fork_id)
              .map_or(0, |definition| definition.branches.len())
      });
    if !matches!(
      projection.latest_attempt(&failure.node_id),
      Some(attempt)
        if attempt.identity.attempt == failure.attempt
          && attempt.identity.invocation_id == failure.invocation_id
          && attempt.status == AttemptStatus::Started
    ) {
      return Err(DurableEngineError::Contract(
        "Attempt failure does not close the active compiled step attempt.".to_string(),
      ));
    }

    let cancellation_settlement = projection.status == RunStatus::Cancelling
      && failure.failure.kind == AttemptFailureKind::InvocationCancelled;
    let retry = node.retry_policy.as_ref().filter(|policy| {
      !cancellation_settlement
        && failure.failure.kind == AttemptFailureKind::ScriptThrew
        && failure.attempt < policy.max_attempts
    });
    let (payloads, disposition) = if let Some(policy) = retry {
      let next_attempt = failure.attempt + 1;
      let delay_ms = policy.delay_before_attempt(next_attempt).ok_or_else(|| {
        DurableEngineError::Contract("Compiled retry delay is invalid.".to_string())
      })?;
      let delay = chrono::Duration::milliseconds(i64::try_from(delay_ms).map_err(|_| {
        DurableEngineError::Contract("Retry delay exceeds the clock range.".to_string())
      })?);
      let scheduled_at = failed_at + delay;
      (
        vec![
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepAttemptFailed(failure.clone()),
          ),
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepRetryScheduled(StepRetryScheduledData {
              node_id: failure.node_id.clone(),
              failed_attempt: failure.attempt,
              next_attempt,
              scheduled_at,
            }),
          ),
        ],
        StepFailureDisposition::RetryScheduled {
          next_attempt,
          scheduled_at,
        },
      )
    } else if parallel_child || fork_child || fork_coordinated || cancellation_settlement {
      (
        vec![(
          generated_event_id(),
          failed_at,
          RunEventPayload::StepAttemptFailed(failure.clone()),
        )],
        StepFailureDisposition::StepFailed,
      )
    } else if reusable_lifecycle || lock_body || saga_body {
      (
        vec![(
          generated_event_id(),
          failed_at,
          RunEventPayload::StepAttemptFailed(failure.clone()),
        )],
        StepFailureDisposition::RunFailureDeferred,
      )
    } else {
      (
        vec![
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepAttemptFailed(failure.clone()),
          ),
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              run_event_schema_version_for_model(self.workflow.schema_version),
              failure.node_id.clone(),
              failure.attempt,
              failure.invocation_id.clone(),
              failure.failure.clone(),
            )),
          ),
        ],
        StepFailureDisposition::RunFailed,
      )
    };
    let projection = self.store.append_payloads_atomically(run_id, payloads)?;
    Ok(StepFailureCommit {
      disposition,
      projection,
    })
  }

  pub fn start_step_attempt(
    &mut self,
    run_id: &str,
    node_id: &str,
    attempt: u32,
    invocation_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let node = self
      .workflow
      .node(node_id)
      .ok_or_else(|| DurableEngineError::Contract(format!("Unknown step node {node_id:?}.")))?;
    let idempotency_key = step_effect_idempotency_key(run_id, &self.definition_hash, node_id);
    self.append_payload(
      generated_event_id(),
      run_id,
      occurred_at,
      RunEventPayload::StepAttemptStarted(crate::event::StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt,
        invocation_id: invocation_id.into(),
        handler: node.handler.clone(),
        idempotency_key: Some(idempotency_key),
      }),
    )
  }

  /// Commits a successful forward effect and its compensation eligibility as
  /// one Store v17 transaction. A validation or SQLite failure rolls back both
  /// records, so replay can never observe success without registration.
  pub fn record_step_success_with_compensation(
    &mut self,
    run_id: &str,
    success: crate::StepAttemptSucceededData,
    saga_id: &str,
    action_id: &str,
  ) -> Result<RunProjection, DurableEngineError> {
    let completion_sequence = self.events(run_id)?.len() as u64 + 1;
    self.append_payloads_atomically(
      run_id,
      vec![
        RunEventPayload::StepAttemptSucceeded(success.clone()),
        RunEventPayload::CompensationRegistered(crate::CompensationRegisteredData {
          saga_id: saga_id.to_string(),
          action_id: action_id.to_string(),
          for_step_id: success.node_id,
          completion_sequence,
        }),
      ],
    )
  }

  pub fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &request.approval_id) {
      return Err(DurableEngineError::Contract(format!(
        "Approval {:?} is not ready to enter waiting state.",
        request.approval_id
      )));
    }
    let (_, token) =
      self
        .store
        .request_approval_atomically(run_id, generated_event_id(), occurred_at, request)?;
    Ok(token)
  }

  pub fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<RunProjection, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &binding.approval_id) {
      return Err(DurableEngineError::Contract(format!(
        "Rich approval {:?} is not ready to enter waiting state.",
        binding.approval_id
      )));
    }
    Ok(
      self
        .store
        .create_rich_approval_request(binding, occurred_at)?,
    )
  }

  pub fn inspect_rich_approval(
    &self,
    run_id: &str,
    request_id: &str,
  ) -> Result<RichApprovalInspectionV1, DurableEngineError> {
    Ok(self.store.inspect_rich_approval(
      run_id,
      request_id,
      RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
    )?)
  }

  pub fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: DateTime<Utc>,
  ) -> Result<bool, DurableEngineError> {
    let projection = self.projection(run_id)?;
    let Some(request) = projection
      .rich_approval_requests
      .values()
      .find(|request| request.status == crate::RichApprovalRequestStatus::Waiting)
      .cloned()
    else {
      return Ok(false);
    };
    self.store.settle_rich_approval_without_vote(
      run_id,
      &request.request_id,
      &request.review_digest,
      reason,
      settled_at,
    )?;
    Ok(true)
  }

  pub fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<RunProjection, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &registration.node_id) {
      return Err(DurableEngineError::Contract(format!(
        "Durable wait {:?} is not ready to enter waiting state.",
        registration.wait_id
      )));
    }
    Ok(self.store.register_durable_wait_atomically(
      run_id,
      generated_event_id(),
      occurred_at,
      registration,
    )?)
  }

  pub fn register_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    occurred_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let descriptor = self
      .workflow
      .durable_lock(lock_id)
      .ok_or_else(|| DurableEngineError::Contract("Unknown Model v18 lock.".to_string()))?;
    let ready = self.ready_node_ids(run_id)?;
    if !ready
      .iter()
      .any(|node_id| node_id == &descriptor.acquire_node_id)
    {
      return Err(DurableEngineError::Contract(format!(
        "Lock {lock_id:?} is not ready to acquire."
      )));
    }
    Ok(self.store.register_lock_waiter_atomically(
      run_id,
      lock_id,
      key,
      &descriptor.acquire_node_id,
      occurred_at,
      deadline,
    )?)
  }

  pub fn release_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .release_lock_atomically(run_id, lock_id, result, occurred_at)?,
    )
  }

  pub fn timeout_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .timeout_lock_waiter_atomically(run_id, lock_id, occurred_at)?,
    )
  }

  pub fn renew_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableEngineError> {
    Ok(
      self
        .store
        .renew_lock_lease_atomically(run_id, lock_id, now)?,
    )
  }

  pub fn renew_iteration_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableEngineError> {
    Ok(
      self
        .store
        .renew_lock_lease_atomically_scoped(run_id, lock_id, Some(iteration), now)?,
    )
  }

  pub fn cancel_lock_waiters(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .cancel_lock_waiters_atomically(run_id, cancellation_id, now)?,
    )
  }

  pub fn register_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    iteration: ForEachIterationScope,
    occurred_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let descriptor = self
      .workflow
      .durable_lock(lock_id)
      .ok_or_else(|| DurableEngineError::Contract("Unknown Model v18 lock.".to_string()))?;
    let execution_identity = format!(
      "{}:{}:{}",
      iteration.for_each_id, iteration.index, descriptor.acquire_node_id
    );
    Ok(self.store.register_lock_waiter_atomically_scoped(
      run_id,
      lock_id,
      key,
      &execution_identity,
      Some(iteration),
      occurred_at,
      deadline,
    )?)
  }

  pub fn release_iteration_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    result: Value,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.release_lock_atomically_scoped(
      run_id,
      lock_id,
      Some(iteration),
      result,
      occurred_at,
    )?)
  }

  pub fn timeout_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.timeout_lock_waiter_atomically_scoped(
      run_id,
      lock_id,
      Some(iteration),
      occurred_at,
    )?)
  }

  pub fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: DateTime<Utc>,
  ) -> Result<TimerWaitSettlement, DurableEngineError> {
    Ok(self.store.settle_timer_wait(run_id, wait_id, now)?)
  }

  pub fn reissue_waiting_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableEngineError> {
    Ok(
      self
        .store
        .reissue_approval_token(run_id, approval_id, request_id, issued_at)?,
    )
  }

  pub fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    output: Value,
  ) -> Result<RunProjection, DurableEngineError> {
    let node = self.workflow.node(node_id).ok_or_else(|| {
      DurableEngineError::Contract(format!("Unknown pure result node {node_id:?}."))
    })?;
    if !matches!(
      node.handler.as_str(),
      "engine.branch-result" | "engine.choice-result" | "engine.saga-result"
    ) {
      return Err(DurableEngineError::Contract(format!(
        "Node {node_id:?} is not a supported pure-result operation."
      )));
    }
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|ready_id| ready_id == node_id) {
      return Err(DurableEngineError::Contract(format!(
        "Node {node_id:?} is not ready for pure result publication."
      )));
    }
    Ok(self.store.append_payloads_atomically(
      run_id,
      vec![
        (
          generated_event_id(),
          Utc::now(),
          RunEventPayload::StepAttemptStarted(crate::event::StepAttemptStartedData {
            node_id: node_id.to_string(),
            attempt: 1,
            invocation_id: invocation_id.to_string(),
            handler: node.handler.clone(),
            idempotency_key: (self.workflow.schema_version
              >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6)
              .then(|| {
                step_effect_idempotency_key(run_id, &self.definition_hash, node_id)
              }),
          }),
        ),
        (
          generated_event_id(),
          Utc::now(),
          RunEventPayload::StepAttemptSucceeded(crate::event::StepAttemptSucceededData {
            node_id: node_id.to_string(),
            attempt: 1,
            invocation_id: invocation_id.to_string(),
            output,
          }),
        ),
      ],
    )?)
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.projection(run_id)?)
  }

  pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, DurableEngineError> {
    Ok(self.store.events(run_id)?)
  }

  pub fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, DurableEngineError> {
    let projection = self.projection(run_id)?;
    ready_node_ids_for_projection(&self.workflow, &self.definition_hash, &projection)
      .map_err(DurableEngineError::Contract)
  }

  pub fn ready_node_ids_at(
    &self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, DurableEngineError> {
    let projection = self.projection(run_id)?;
    ready_node_ids_for_projection_at(&self.workflow, &self.definition_hash, &projection, now)
      .map_err(DurableEngineError::Contract)
  }

  pub(crate) fn append_payloads_atomically(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self.store.append_payloads_atomically(
        run_id,
        payloads
          .into_iter()
          .map(|payload| (generated_event_id(), Utc::now(), payload))
          .collect(),
      )?,
    )
  }

  pub(crate) fn append_scoped_payloads_atomically(
    &mut self,
    run_id: &str,
    iteration: crate::event::ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self.store.append_payloads_scoped_atomically(
        run_id,
        Some(iteration),
        payloads
          .into_iter()
          .map(|payload| (generated_event_id(), Utc::now(), payload))
          .collect(),
      )?,
    )
  }

  pub fn recover_interrupted_runs(&mut self) -> Result<RecoveryReport, DurableEngineError> {
    Ok(self.store.recover_interrupted_runs()?)
  }

  pub fn into_store(self) -> DurableEventStore {
    self.store
  }
}
