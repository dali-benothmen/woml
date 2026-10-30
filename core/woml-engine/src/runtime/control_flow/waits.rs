//! Durable wait/approval registration and waiting-outcome construction.
//!
//! Persist the wait before returning a waiting outcome; this module does not
//! turn a wait into success or grant a second authority to callbacks.

use crate::engine::node_is_complete;
use crate::engine::resolve_context_reference;
use crate::event::ApprovalRequestedData;
use crate::event::ApprovalTimeoutPolicy;
use crate::model::ApprovalDefinition;
use crate::model::ValueExpression;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::outcomes::WaitingRichApprovalAuthority;
use crate::runtime::outcomes::WaitingWorkflowApproval;
use crate::runtime::outcomes::WaitingWorkflowSignal;
use crate::runtime::outcomes::WaitingWorkflowTimer;
use crate::runtime::outcomes::WorkflowRuntimeOutcome;
use crate::runtime::outcomes::RUNTIME_OUTCOME_CONTRACT;
use crate::runtime::outcomes::RUNTIME_OUTCOME_VERSION;
use crate::runtime::{generated_id, RuntimeDagEngine};
use crate::BoundRichApprovalReview;
use crate::CompiledWorkflowDefinition;
use crate::IssuedApprovalToken;
use crate::RichApprovalArtifactVerifier;
use crate::RichApprovalBindingInput;
use crate::RichApprovalError;
use crate::RunProjection;
use crate::RunStatus;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

pub(in crate::runtime) enum DurableWaitNodeProgress {
  Resolved,
  Waiting(WorkflowRuntimeOutcome),
}

pub(in crate::runtime) async fn register_durable_wait_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<DurableWaitNodeProgress, RuntimeExecutionError> {
  let descriptor = engine
    .workflow()
    .durable_wait(node_id)
    .cloned()
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "durable wait node {node_id:?} has no Model v17 descriptor"
      ))
    })?;
  let registered_at = options.clock.now();
  let (kind, deadline, signals, correlation_digest) = match &descriptor {
    crate::CompiledWait::Duration { duration_ms, .. } => {
      let duration = chrono::Duration::milliseconds(i64::try_from(*duration_ms).map_err(|_| {
        RuntimeExecutionError::Stalled(
          "durable wait duration exceeds the runtime clock".to_string(),
        )
      })?);
      (
        crate::DurableWaitKind::Duration,
        Some(registered_at + duration),
        None,
        None,
      )
    }
    crate::CompiledWait::Until { at, .. } => {
      let value = match at {
        ValueExpression::Literal { value } => value.clone(),
        ValueExpression::ContextReference { .. } => {
          resolve_context_reference(at, &engine.projection(run_id)?.context).map_err(|error| {
            RuntimeExecutionError::Stalled(format!(
              "durable wait {node_id:?} cannot resolve context.{}",
              error.path.join(".")
            ))
          })?
        }
        _ => {
          return Err(RuntimeExecutionError::Stalled(format!(
            "durable wait {node_id:?} has an invalid until expression"
          )))
        }
      };
      let value = value.as_str().ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "durable wait {node_id:?} resolved at to a non-string value"
        ))
      })?;
      let deadline = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| {
          RuntimeExecutionError::Stalled(format!(
            "durable wait {node_id:?} resolved at to an invalid RFC 3339 instant"
          ))
        })?
        .with_timezone(&chrono::Utc);
      (crate::DurableWaitKind::Until, Some(deadline), None, None)
    }
    crate::CompiledWait::Signal {
      signals,
      correlation,
      timeout_ms,
      ..
    } => {
      let correlation = match correlation {
        ValueExpression::Literal { value } => value.clone(),
        ValueExpression::ContextReference { .. } => {
          resolve_context_reference(correlation, &engine.projection(run_id)?.context).map_err(
            |error| {
              RuntimeExecutionError::Stalled(format!(
                "signal wait {node_id:?} cannot resolve context.{}",
                error.path.join(".")
              ))
            },
          )?
        }
        _ => {
          return Err(RuntimeExecutionError::Stalled(format!(
            "signal wait {node_id:?} has an invalid correlation expression"
          )))
        }
      };
      let correlation = correlation.as_str().ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "signal wait {node_id:?} resolved correlation to a non-string value"
        ))
      })?;
      let deadline = timeout_ms
        .map(|timeout_ms| {
          i64::try_from(timeout_ms)
            .map(chrono::Duration::milliseconds)
            .map(|duration| registered_at + duration)
        })
        .transpose()
        .map_err(|_| {
          RuntimeExecutionError::Stalled(
            "signal wait timeout exceeds the runtime clock".to_string(),
          )
        })?;
      (
        crate::DurableWaitKind::Signal,
        deadline,
        Some(signals.iter().map(|signal| signal.signal.clone()).collect()),
        Some(format!(
          "sha256:{}",
          hex::encode(Sha256::digest(correlation.as_bytes()))
        )),
      )
    }
  };
  engine.register_durable_wait(
    run_id,
    registered_at,
    crate::WaitRegisteredData {
      wait_id: descriptor.wait_id().to_string(),
      node_id: descriptor.node_id().to_string(),
      kind,
      registered_at,
      deadline,
      signals,
      correlation_digest,
    },
  )?;
  let projection = engine.projection(run_id)?;
  if projection.status == RunStatus::Running
    && projection.context.steps.contains_key(descriptor.wait_id())
  {
    return Ok(DurableWaitNodeProgress::Resolved);
  }
  if deadline.is_some_and(|deadline| deadline <= registered_at)
    && kind != crate::DurableWaitKind::Signal
    && matches!(
      engine.settle_timer_wait(run_id, descriptor.wait_id(), registered_at)?,
      crate::TimerWaitSettlement::Resolved { .. }
        | crate::TimerWaitSettlement::AlreadyResolved { .. }
    )
  {
    return Ok(DurableWaitNodeProgress::Resolved);
  }
  options.suspend_policy_execution_slot().await?;
  if kind == crate::DurableWaitKind::Signal {
    return Ok(DurableWaitNodeProgress::Waiting(
      WorkflowRuntimeOutcome::WaitingSignal {
        contract: RUNTIME_OUTCOME_CONTRACT,
        version: RUNTIME_OUTCOME_VERSION,
        workflow_id: engine.workflow().workflow_id.clone(),
        run_id: run_id.to_string(),
        wait: WaitingWorkflowSignal {
          wait_id: descriptor.wait_id().to_string(),
          node_id: descriptor.node_id().to_string(),
          registered_at,
          signals: match descriptor {
            crate::CompiledWait::Signal { signals, .. } => {
              signals.iter().map(|signal| signal.signal.clone()).collect()
            }
            _ => Vec::new(),
          },
          deadline,
        },
      },
    ));
  }
  Ok(DurableWaitNodeProgress::Waiting(
    WorkflowRuntimeOutcome::WaitingTime {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      workflow_id: engine.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      wait: WaitingWorkflowTimer {
        wait_id: descriptor.wait_id().to_string(),
        node_id: descriptor.node_id().to_string(),
        kind,
        registered_at,
        deadline: deadline.expect("timer waits have a deadline"),
      },
    },
  ))
}

pub(in crate::runtime) fn completed_terminal_approval_result(
  workflow: &CompiledWorkflowDefinition,
  terminal_node_id: &str,
  projection: &RunProjection,
) -> Option<Value> {
  let terminal = workflow.node(terminal_node_id)?;
  if terminal.handler != "engine.approval-join" || !node_is_complete(workflow, terminal, projection)
  {
    return None;
  }
  let approval_id = terminal_node_id
    .strip_prefix("__woml_approval__")?
    .strip_suffix("__join")?;
  projection.context.steps.get(approval_id).cloned()
}

pub(in crate::runtime) fn request_approval_wait<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let approval = engine.workflow().approval(node_id).ok_or_else(|| {
    RuntimeExecutionError::Stalled(format!(
      "approval node {node_id:?} has invalid compiled inputs"
    ))
  })?;
  let occurred_at = options.clock.now();
  let expires_at = approval
    .timeout_ms
    .map(|milliseconds| {
      i64::try_from(milliseconds)
        .map(chrono::Duration::milliseconds)
        .map(|duration| occurred_at + duration)
    })
    .transpose()
    .map_err(|_| {
      RuntimeExecutionError::Stalled(format!(
        "approval {:?} timeout exceeds the clock range",
        approval.approval_id
      ))
    })?;
  let on_timeout = match approval.on_timeout.as_str() {
    "reject" => ApprovalTimeoutPolicy::Reject,
    "fail" => ApprovalTimeoutPolicy::Fail,
    _ => {
      return Err(RuntimeExecutionError::Stalled(format!(
        "approval {:?} has an unknown timeout policy",
        approval.approval_id
      )));
    }
  };
  let request_id = generated_id("aprreq");
  let workflow_deadline = engine.projection(run_id)?.timeout_at;
  let waiting_expires_at = match (expires_at, workflow_deadline) {
    (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
    (Some(approval), None) => Some(approval),
    (None, Some(workflow)) => Some(workflow),
    (None, None) => None,
  };
  let waiting_on_timeout = if workflow_deadline
    .is_some_and(|workflow| expires_at.is_none_or(|approval| workflow <= approval))
  {
    ApprovalTimeoutPolicy::Fail
  } else {
    on_timeout
  };
  if let Some(review_reference) = approval.review.clone() {
    let authority = options.reviewer_authority.as_ref().ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: rich approval requires an operator-owned Reviewer Directory v1".to_string(),
      )
    })?;
    let required = approval.required_approvals.ok_or_else(|| {
      RuntimeExecutionError::Stalled(
        "a rich approval has no compiled required approval count".to_string(),
      )
    })?;
    let directory_version = authority.active_version().map_err(|error| {
      RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
    })?;
    authority
      .validate_requirements(&directory_version, required, &approval.required_roles)
      .map_err(|error| {
        RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
      })?;
    let projection = engine.projection(run_id)?;
    let review = crate::resolve_rich_approval_review(&review_reference, &projection.context)
      .map_err(rich_approval_runtime_error)?;
    let workflow_version = engine
      .workflow()
      .metadata
      .as_ref()
      .and_then(|metadata| metadata.version.as_deref())
      .unwrap_or("0.0.0")
      .to_string();
    let reject_artifacts = RejectRichApprovalArtifacts;
    let artifact_verifier: &dyn RichApprovalArtifactVerifier = options
      .managed_artifact_store
      .as_deref()
      .map(|store| store as &dyn RichApprovalArtifactVerifier)
      .unwrap_or(&reject_artifacts);
    let known_secrets = options
      .resolved_secrets
      .values()
      .cloned()
      .collect::<Vec<_>>();
    let binding = crate::bind_rich_approval_review(
      RichApprovalBindingInput {
        definition_hash: engine.definition_hash().to_string(),
        workflow_id: engine.workflow().workflow_id.clone(),
        workflow_version,
        run_id: run_id.to_string(),
        approval_id: approval.approval_id.clone(),
        request_id: request_id.clone(),
        review,
        required_approvals: required,
        required_roles: approval.required_roles.clone(),
        directory_version,
      },
      artifact_verifier,
      &known_secrets,
    )
    .map_err(rich_approval_runtime_error)?;
    engine.request_rich_approval(run_id, occurred_at, &binding)?;
    return Ok(rich_waiting_outcome(
      engine.workflow().workflow_id.clone(),
      run_id.to_string(),
      approval,
      waiting_expires_at,
      waiting_on_timeout,
      binding,
      0,
      Vec::new(),
    ));
  }
  let token = engine.request_approval(
    run_id,
    occurred_at,
    ApprovalRequestedData {
      approval_id: approval.approval_id.clone(),
      request_id: request_id.clone(),
      expires_at,
      on_timeout,
    },
  )?;
  Ok(waiting_outcome(
    engine.workflow().workflow_id.clone(),
    run_id.to_string(),
    approval,
    request_id,
    waiting_expires_at,
    waiting_on_timeout,
    token,
  ))
}

pub(in crate::runtime) fn waiting_outcome(
  workflow_id: String,
  run_id: String,
  approval: ApprovalDefinition,
  request_id: String,
  expires_at: Option<chrono::DateTime<chrono::Utc>>,
  on_timeout: ApprovalTimeoutPolicy,
  token: IssuedApprovalToken,
) -> WorkflowRuntimeOutcome {
  WorkflowRuntimeOutcome::Waiting {
    contract: RUNTIME_OUTCOME_CONTRACT,
    version: RUNTIME_OUTCOME_VERSION,
    workflow_id,
    run_id,
    approval: WaitingWorkflowApproval {
      approval_id: approval.approval_id,
      request_id,
      name: approval.name,
      description: approval.description,
      expires_at,
      on_timeout,
      token: Some(token.token),
      credential_expires_at: Some(token.credential_expires_at),
      review_digest: None,
      review: None,
      authority: None,
    },
  }
}

pub(in crate::runtime) fn rich_waiting_outcome(
  workflow_id: String,
  run_id: String,
  approval: ApprovalDefinition,
  expires_at: Option<chrono::DateTime<chrono::Utc>>,
  on_timeout: ApprovalTimeoutPolicy,
  binding: BoundRichApprovalReview,
  received: u32,
  represented_roles: Vec<String>,
) -> WorkflowRuntimeOutcome {
  WorkflowRuntimeOutcome::Waiting {
    contract: RUNTIME_OUTCOME_CONTRACT,
    version: RUNTIME_OUTCOME_VERSION,
    workflow_id,
    run_id,
    approval: WaitingWorkflowApproval {
      approval_id: approval.approval_id,
      request_id: binding.request_id,
      name: approval.name,
      description: approval.description,
      expires_at,
      on_timeout,
      token: None,
      credential_expires_at: None,
      review_digest: Some(binding.review_digest),
      review: Some(binding.review),
      authority: Some(WaitingRichApprovalAuthority {
        required: binding.required_approvals,
        received,
        required_roles: binding.required_roles,
        represented_roles,
      }),
    },
  }
}

struct RejectRichApprovalArtifacts;

impl RichApprovalArtifactVerifier for RejectRichApprovalArtifacts {
  fn verify_review_artifact(
    &self,
    _run_id: &str,
    _artifact: &crate::ArtifactReferenceV1,
  ) -> Result<crate::ArtifactReferenceV1, RichApprovalError> {
    Err(RichApprovalError::ArtifactInvalid)
  }
}

fn rich_approval_runtime_error(error: RichApprovalError) -> RuntimeExecutionError {
  RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
}
