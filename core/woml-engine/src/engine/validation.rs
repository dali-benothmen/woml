//! engine validation.

use super::{validate_for_each_id, validate_operation_node, validate_reusable_lifecycle_binding};
use crate::event::ApprovalTimeoutPolicy;
use crate::event::ParallelFailurePolicy;
use crate::event::RunEventPayload;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV2;
use crate::event::RunFailedDataV3;
use crate::event::RunFailedDataV4;
use crate::event::RunFailedDataV5;
use crate::model::CompiledWorkflowNode;
use crate::CompiledWorkflowDefinition;

pub(crate) fn validate_payload_against_definition(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  payload: &RunEventPayload,
) -> Result<(), String> {
  match payload {
    RunEventPayload::RunAdmitted(data) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) || data.definition_hash != definition_hash
        || workflow.runtime_policy_hash().as_deref() != Some(data.policy_hash.as_str())
        || workflow.runtime_policy_queue_name().as_deref() != Some(data.queue.name.as_str())
      {
        return Err(
          "run_admitted does not match the compiled definition and immutable policy identity."
            .to_string(),
        );
      }
      let workflow_call_ingress =
        data.trigger.id == "workflow-call" && data.trigger.handler == "trigger.event";
      if !workflow_call_ingress {
        let trigger = workflow.trigger(&data.trigger.id).ok_or_else(|| {
          format!(
            "run_admitted references unknown trigger {:?}.",
            data.trigger.id
          )
        })?;
        if trigger.handler != data.trigger.handler {
          return Err("run_admitted trigger identity does not match the definition.".to_string());
        }
      }
    }
    RunEventPayload::RunExecutionStarted(data) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) {
        return Err("Event v11+ runtime-policy events require compiled Model v12+.".to_string());
      }
      let expected_timeout = workflow
        .runtime_policy
        .as_ref()
        .and_then(|policy| policy.timeout_ms)
        .map(|timeout_ms| {
          data.started_at
            + chrono::Duration::milliseconds(i64::try_from(timeout_ms).unwrap_or(i64::MAX))
        });
      if data.timeout_at != expected_timeout {
        return Err(
          "run_execution_started timeoutAt does not match the compiled workflow timeout."
            .to_string(),
        );
      }
    }
    RunEventPayload::RunTimeoutReached(_) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) {
        return Err("Event v11+ runtime-policy events require compiled Model v12+.".to_string());
      }
    }
    RunEventPayload::RunStarted(data) => {
      if data.workflow_id != workflow.workflow_id || data.definition_hash != definition_hash {
        return Err(
          "run_started does not match the engine's workflow ID and definition hash.".to_string(),
        );
      }
      if matches!(
        data.ingress.as_ref(),
        Some(crate::RunIngress::WorkflowCall { .. })
      ) {
        if data.trigger_id.is_some()
          || data.trigger_handler.is_some()
          || data.trigger_occurrence_id.is_some()
        {
          return Err("Workflow-call run_started requires no source trigger identity.".to_string());
        }
      } else if workflow.schema_version == crate::COMPILED_MODEL_SCHEMA_VERSION_V10 {
        return Err("Call-only Model v10 requires workflow_call ingress.".to_string());
      } else if workflow.schema_version == crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        && data.ingress.is_some()
      {
        if data.trigger_id.is_some()
          || data.trigger_handler.is_some()
          || data.trigger_occurrence_id.is_some()
        {
          return Err("Model v11 workflow-call ingress cannot carry trigger identity.".to_string());
        }
      } else if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V7 {
        let trigger_id = data
          .trigger_id
          .as_deref()
          .ok_or_else(|| "run_started v7 is missing its triggerId.".to_string())?;
        let trigger = workflow
          .trigger(trigger_id)
          .ok_or_else(|| format!("run_started references unknown trigger {trigger_id:?}."))?;
        if data.trigger_handler.as_deref() != Some(trigger.handler.as_str())
          || data.trigger_occurrence_id.is_none()
        {
          return Err(
            "run_started trigger identity does not match the compiled definition.".to_string(),
          );
        }
        if data.ingress.is_some() {
          return Err(
            "Source-triggered run_started cannot carry workflow-call ingress.".to_string(),
          );
        }
      } else if data.trigger_id.is_some()
        || data.trigger_handler.is_some()
        || data.trigger_occurrence_id.is_some()
        || data.ingress.is_some()
      {
        return Err("Pre-v7 run_started cannot carry production-trigger identity.".to_string());
      }
    }
    RunEventPayload::StepAttemptStarted(data) => {
      let node = workflow
        .node(&data.node_id)
        .ok_or_else(|| format!("Attempt references unknown node {:?}.", data.node_id))?;
      let maximum_attempts = node
        .retry_policy
        .as_ref()
        .map_or(1, |policy| policy.max_attempts);
      if data.attempt == 0
        || data.attempt > maximum_attempts
        || (workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 && data.attempt != 1)
      {
        return Err(format!(
          "Node {:?} attempt {} exceeds its compiled maximum of {maximum_attempts}.",
          data.node_id, data.attempt
        ));
      }
      if data.handler != node.handler {
        return Err(format!(
          "Attempt handler {:?} does not match node {:?} handler {:?}.",
          data.handler, data.node_id, node.handler
        ));
      }
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      if workflow.node(&data.node_id).is_none() {
        return Err(format!(
          "Attempt references unknown node {:?}.",
          data.node_id
        ));
      }
    }
    RunEventPayload::StepAttemptFailed(data) => {
      if workflow.node(&data.node_id).is_none() {
        return Err(format!(
          "Attempt references unknown node {:?}.",
          data.node_id
        ));
      }
    }
    RunEventPayload::StepRetryScheduled(data) => {
      if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
        return Err("step_retry_scheduled requires compiled model v6.".to_string());
      }
      let node = workflow
        .node(&data.node_id)
        .ok_or_else(|| format!("Retry schedule references unknown node {:?}.", data.node_id))?;
      let policy = node.retry_policy.as_ref().ok_or_else(|| {
        format!(
          "Retry schedule references node {:?} without a compiled retry policy.",
          data.node_id
        )
      })?;
      if node.handler != "runtime.script"
        || data.next_attempt != data.failed_attempt + 1
        || data.next_attempt > policy.max_attempts
      {
        return Err(format!(
          "Retry schedule for node {:?} exceeds its compiled policy.",
          data.node_id
        ));
      }
    }
    RunEventPayload::BranchSelected(data) => {
      let selector_id = format!("__woml_branch__{}__select", data.branch_id);
      let selector = workflow.node(&selector_id).ok_or_else(|| {
        format!(
          "branch_selected references unknown branch {:?}.",
          data.branch_id
        )
      })?;
      if selector.handler != "engine.branch-select" {
        return Err(format!(
          "Branch {:?} does not have its canonical selector.",
          data.branch_id
        ));
      }
      let valid_arm = workflow.graph.edges.iter().any(|edge| {
        edge.from == selector_id
          && edge.id == data.arm_id
          && edge.branch_id.as_deref() == Some(data.branch_id.as_str())
      });
      if !valid_arm {
        return Err(format!(
          "Arm {:?} is not selectable for branch {:?}.",
          data.arm_id, data.branch_id
        ));
      }
    }
    RunEventPayload::ChoiceSelected(data) => {
      let choice = workflow
        .graph
        .choices
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|choice| choice.choice_id == data.choice_id)
        .ok_or_else(|| {
          format!(
            "choice_selected references unknown choice {:?}.",
            data.choice_id
          )
        })?;
      if !choice.arm_ids.contains(&data.arm_id) {
        return Err(format!(
          "Arm {:?} is not selectable for choice {:?}.",
          data.arm_id, data.choice_id
        ));
      }
    }
    RunEventPayload::ForkOpened(data) => {
      if workflow
        .graph
        .forks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .all(|fork| fork.fork_id != data.fork_id)
      {
        return Err(format!(
          "fork_opened references unknown fork {:?}.",
          data.fork_id
        ));
      }
    }
    RunEventPayload::ForkBranchSettled(data) => {
      let fork = workflow
        .graph
        .forks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|fork| fork.fork_id == data.fork_id)
        .ok_or_else(|| {
          format!(
            "fork_branch_settled references unknown fork {:?}.",
            data.fork_id
          )
        })?;
      let branch = fork
        .branches
        .iter()
        .find(|branch| branch.branch_id == data.branch_id)
        .ok_or_else(|| {
          format!(
            "fork_branch_settled references unknown branch {:?} in fork {:?}.",
            data.branch_id, data.fork_id
          )
        })?;
      if branch.terminal_node_id != data.terminal_node_id {
        return Err(format!(
          "Fork branch {:?}.{:?} terminal does not match Model v13.",
          data.fork_id, data.branch_id
        ));
      }
    }
    RunEventPayload::ForkJoinSettled(data) => {
      let fork = workflow
        .graph
        .forks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|fork| fork.fork_id == data.fork_id)
        .ok_or_else(|| {
          format!(
            "fork_join_settled references unknown fork {:?}.",
            data.fork_id
          )
        })?;
      if data
        .blocking_branch_id
        .as_ref()
        .is_some_and(|branch_id| !fork.joined_branch_ids.contains(branch_id))
      {
        return Err(format!(
          "Fork {:?} join is blocked by an unjoined branch.",
          data.fork_id
        ));
      }
    }
    RunEventPayload::ParallelGroupStarted(data) => {
      if workflow.parallel_group(&data.parallel_id).is_none() {
        return Err(format!(
          "parallel_group_started references unknown parallel group {:?}.",
          data.parallel_id
        ));
      }
    }
    RunEventPayload::ParallelGroupCompleted(data) => {
      let group = workflow.parallel_group(&data.parallel_id).ok_or_else(|| {
        format!(
          "parallel_group_completed references unknown parallel group {:?}.",
          data.parallel_id
        )
      })?;
      if data
        .failed_node_ids
        .iter()
        .chain(&data.cancelled_node_ids)
        .any(|node_id| !group.child_node_ids.contains(node_id))
      {
        return Err(format!(
          "parallel_group_completed for {:?} references a node outside that group.",
          data.parallel_id
        ));
      }
    }
    RunEventPayload::ApprovalRequested(data) => {
      let approval = workflow.approval(&data.approval_id).ok_or_else(|| {
        format!(
          "approval_requested references unknown approval {:?}.",
          data.approval_id
        )
      })?;
      let expected_policy = match approval.on_timeout.as_str() {
        "reject" => ApprovalTimeoutPolicy::Reject,
        "fail" => ApprovalTimeoutPolicy::Fail,
        _ => {
          return Err(format!(
            "Approval {:?} has an invalid policy.",
            data.approval_id
          ))
        }
      };
      if data.on_timeout != expected_policy
        || approval.timeout_ms.is_some() != data.expires_at.is_some()
      {
        return Err(format!(
          "approval_requested does not match approval {:?} timeout inputs.",
          data.approval_id
        ));
      }
    }
    RunEventPayload::ApprovalReviewRequested(data) => {
      let approval = workflow.approval(&data.approval_id).ok_or_else(|| {
        format!(
          "approval_review_requested references unknown approval {:?}.",
          data.approval_id
        )
      })?;
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20
        && workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V21
        || approval.review.is_none()
        || approval.required_approvals != Some(data.required)
        || approval.required_roles != data.required_roles
      {
        return Err(
          "approval_review_requested does not match its Model v20 authority policy.".to_string(),
        );
      }
    }
    RunEventPayload::ApprovalReviewerDecisionRecorded(data) => {
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20
        && workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V21
        || workflow
          .approval(&data.approval_id)
          .is_none_or(|approval| approval.review.is_none())
      {
        return Err(
          "approval_reviewer_decision_recorded requires a Model v20 rich approval.".to_string(),
        );
      }
    }
    RunEventPayload::ApprovalQuorumProgressed(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "approval_quorum_progressed references an unknown approval.".to_string())?;
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20
        && workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V21
        || approval.review.is_none()
        || approval.required_approvals != Some(data.required)
        || approval.required_roles != data.required_roles
      {
        return Err(
          "approval_quorum_progressed does not match its Model v20 authority policy.".to_string(),
        );
      }
    }
    RunEventPayload::ApprovalResolved(data) => {
      if workflow.approval(&data.approval_id).is_none() {
        return Err(format!(
          "approval_resolved references unknown approval {:?}.",
          data.approval_id
        ));
      }
    }
    RunEventPayload::ApprovalInvalidated(data) => {
      if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20
        && workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V21
        || workflow
          .approval(&data.approval_id)
          .is_none_or(|approval| approval.review.is_none())
      {
        return Err("approval_invalidated requires a Model v20 rich approval.".to_string());
      }
    }
    RunEventPayload::NotificationDeliveryRequested(data) => {
      let approval = workflow.approval(&data.approval_id).ok_or_else(|| {
        format!(
          "Notification references unknown approval {:?}.",
          data.approval_id
        )
      })?;
      let delivery = approval
        .notifications
        .iter()
        .find(|delivery| delivery.delivery_id == data.delivery_id)
        .ok_or_else(|| {
          format!(
            "Notification references unknown delivery {:?}.",
            data.delivery_id
          )
        })?;
      if data.provider != delivery.provider || data.destination != delivery.destination {
        return Err(
          "Notification delivery intent does not match the compiled definition.".to_string(),
        );
      }
    }
    RunEventPayload::NotificationDeliveryAttemptStarted(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification attempt references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification attempt references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationDeliverySucceeded(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification success references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification success references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationDeliveryFailed(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification failure references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification failure references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationDecisionAccepted(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification decision references an unknown approval.".to_string())?;
      if !approval.notifications.iter().any(|delivery| {
        delivery.delivery_id == data.delivery_id && delivery.provider == data.provider
      }) {
        return Err("Notification decision references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationMessageUpdateRequested(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification update references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err("Notification update references an unknown compiled delivery.".to_string());
      }
    }
    RunEventPayload::NotificationMessageUpdateAttemptStarted(data)
    | RunEventPayload::NotificationMessageUpdated(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification update attempt references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err(
          "Notification update attempt references an unknown compiled delivery.".to_string(),
        );
      }
    }
    RunEventPayload::NotificationMessageUpdateFailed(data) => {
      let approval = workflow
        .approval(&data.approval_id)
        .ok_or_else(|| "Notification update failure references an unknown approval.".to_string())?;
      if !approval
        .notifications
        .iter()
        .any(|delivery| delivery.delivery_id == data.delivery_id)
      {
        return Err(
          "Notification update failure references an unknown compiled delivery.".to_string(),
        );
      }
    }
    RunEventPayload::WaitRegistered(data) => {
      let wait = workflow
        .durable_wait(&data.wait_id)
        .ok_or_else(|| "wait_registered references an unknown durable wait.".to_string())?;
      let kind_matches = matches!(
        (wait, data.kind),
        (
          crate::CompiledWait::Duration { .. },
          crate::DurableWaitKind::Duration
        ) | (
          crate::CompiledWait::Until { .. },
          crate::DurableWaitKind::Until
        ) | (
          crate::CompiledWait::Signal { .. },
          crate::DurableWaitKind::Signal
        )
      );
      if data.node_id != wait.node_id() || !kind_matches {
        return Err(
          "wait_registered does not match its compiled Model v17 descriptor.".to_string(),
        );
      }
    }
    RunEventPayload::ExternalSignalReceived(data) => {
      let Some(crate::CompiledWait::Signal { signals, .. }) = workflow.durable_wait(&data.wait_id)
      else {
        return Err("external_signal_received references an unknown signal wait.".to_string());
      };
      if !signals.iter().any(|signal| signal.signal == data.signal) {
        return Err(
          "external_signal_received names a signal not accepted by the wait.".to_string(),
        );
      }
    }
    RunEventPayload::WaitSatisfied(data) => {
      if workflow.durable_wait(&data.wait_id).is_none() {
        return Err("wait_satisfied references an unknown durable wait.".to_string());
      }
    }
    RunEventPayload::WaitTimedOut(data) => {
      let Some(crate::CompiledWait::Signal { on_timeout, .. }) =
        workflow.durable_wait(&data.wait_id)
      else {
        return Err("wait_timed_out references an unknown signal wait.".to_string());
      };
      let behavior_matches = matches!(
        (on_timeout.as_str(), data.behavior),
        ("fail", crate::DurableWaitTimeoutBehavior::Fail)
          | ("continue", crate::DurableWaitTimeoutBehavior::Continue)
      );
      if !behavior_matches {
        return Err("wait_timed_out behavior differs from the compiled wait policy.".to_string());
      }
    }
    RunEventPayload::WaitCancelled(data) => {
      if workflow.durable_wait(&data.wait_id).is_none() {
        return Err("wait_cancelled references an unknown durable wait.".to_string());
      }
    }
    RunEventPayload::LockWaitRegistered(data) => {
      if workflow.durable_lock(&data.lock_id).is_none() {
        return Err("lock_wait_registered references an unknown Model v18 lock.".to_string());
      }
    }
    RunEventPayload::LockAcquired(data) => {
      if workflow.durable_lock(&data.lock_id).is_none() {
        return Err("lock_acquired references an unknown Model v18 lock.".to_string());
      }
    }
    RunEventPayload::LockReleased(data) => {
      if workflow.durable_lock(&data.lock_id).is_none() {
        return Err("lock_released references an unknown Model v18 lock.".to_string());
      }
    }
    RunEventPayload::LockAcquireTimedOut(data) => {
      if workflow.durable_lock(&data.lock_id).is_none() {
        return Err("lock_acquire_timed_out references an unknown Model v18 lock.".to_string());
      }
    }
    RunEventPayload::LockAcquisitionCancelled(data) => {
      if workflow.durable_lock(&data.lock_id).is_none() {
        return Err("lock_acquisition_cancelled references an unknown Model v18 lock.".to_string());
      }
    }
    RunEventPayload::LockLeaseLost(data) => {
      if workflow.durable_lock(&data.lock_id).is_none() {
        return Err("lock_lease_lost references an unknown Model v18 lock.".to_string());
      }
    }
    RunEventPayload::RunSucceeded(data) => {
      if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V11 {
        return Err("Model v11+ finalizes with run_outcome_decided and run_finalized.".to_string());
      }
      if workflow.terminal_node_id() != Some(data.terminal_node_id.as_str()) {
        return Err(format!(
          "run_succeeded names {:?}, which is not the workflow terminal node.",
          data.terminal_node_id
        ));
      }
    }
    RunEventPayload::OperationStarted(data) => {
      validate_operation_node(workflow, &data.node_id)?;
    }
    RunEventPayload::OperationSucceeded(data) => {
      validate_operation_node(workflow, &data.node_id)?;
    }
    RunEventPayload::OperationFailed(data) => {
      validate_operation_node(workflow, &data.node_id)?;
    }
    RunEventPayload::RunCancellationRequested(_)
    | RunEventPayload::RunOutcomeDecided(_)
    | RunEventPayload::RunFinalized(_) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) {
        return Err("Lifecycle and run-control events require compiled Model v11+.".to_string());
      }
    }
    RunEventPayload::LifecycleHookRequested(data) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) {
        return Err("Lifecycle hook events require compiled Model v11+.".to_string());
      }
      let hook = workflow.lifecycle_hook(&data.hook_id).ok_or_else(|| {
        format!(
          "Lifecycle request references unknown hook {:?}.",
          data.hook_id
        )
      })?;
      if hook.event != data.event
        || (data.event.is_step()
          && workflow
            .lifecycle_hook_for_step_event(data.event, &data.subject.id)
            .is_none())
      {
        return Err("Lifecycle hook request does not match its compiled binding.".to_string());
      }
    }
    RunEventPayload::LifecycleActionAttemptStarted(data)
    | RunEventPayload::LifecycleActionSucceeded(data) => {
      let action_exists = workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
        lifecycle.hooks.iter().any(|hook| {
          hook.hook_id == data.action_id.split(":action:").next().unwrap_or_default()
            && hook
              .actions
              .iter()
              .any(|action| action.action_id == data.action_id)
        })
      });
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) || !action_exists
      {
        return Err("Lifecycle action event references an unknown Model v11+ action.".to_string());
      }
    }
    RunEventPayload::LifecycleActionFailed(data) => {
      let action_exists = workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
        lifecycle
          .hooks
          .iter()
          .flat_map(|hook| &hook.actions)
          .any(|action| action.action_id == data.action_id)
      });
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) || !action_exists
      {
        return Err(
          "Lifecycle action failure references an unknown Model v11+ action.".to_string(),
        );
      }
    }
    RunEventPayload::LifecycleHookCompleted(data) => {
      if !matches!(
        workflow.schema_version,
        crate::COMPILED_MODEL_SCHEMA_VERSION_V11
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
          | crate::COMPILED_MODEL_SCHEMA_VERSION_V21
      ) || workflow.lifecycle.is_none()
        || !crate::event::is_definition_hash(&data.hook_invocation_id)
      {
        return Err(
          "Lifecycle hook completion requires a Model v11+ lifecycle binding.".to_string(),
        );
      }
    }
    RunEventPayload::ReusableLifecycleRequested(data) => {
      validate_reusable_lifecycle_binding(
        workflow,
        &data.invocation_id,
        &data.definition_digest,
        None,
      )?;
    }
    RunEventPayload::ReusableLifecycleActionStarted(data) => {
      validate_reusable_lifecycle_binding(
        workflow,
        &data.invocation_id,
        &data.definition_digest,
        Some(&data.action_id),
      )?;
    }
    RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
      validate_reusable_lifecycle_binding(
        workflow,
        &data.invocation_id,
        &data.definition_digest,
        Some(&data.action_id),
      )?;
    }
    RunEventPayload::ReusableLifecycleActionFailed(data) => {
      validate_reusable_lifecycle_binding(
        workflow,
        &data.invocation_id,
        &data.definition_digest,
        Some(&data.action_id),
      )?;
    }
    RunEventPayload::ReusableLifecycleCompleted(data) => {
      validate_reusable_lifecycle_binding(
        workflow,
        &data.invocation_id,
        &data.definition_digest,
        None,
      )?;
    }
    RunEventPayload::RunFailed(data) => {
      if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V11 {
        return Err("Model v11+ finalizes with run_outcome_decided and run_finalized.".to_string());
      }
      if let RunFailedData::V5(RunFailedDataV5::Notification {
        approval_id,
        failed_delivery_ids,
        ..
      }) = data
      {
        let approval = workflow
          .approval(approval_id)
          .ok_or_else(|| format!("run_failed references unknown approval {approval_id:?}."))?;
        if failed_delivery_ids.iter().any(|id| {
          !approval
            .notifications
            .iter()
            .any(|delivery| delivery.delivery_id == *id)
        }) {
          return Err(
            "run_failed references a delivery outside its compiled approval.".to_string(),
          );
        }
      }
      let (node_id, branch_identity, parallel_identity, approval_identity) = match data {
        RunFailedData::V5(_) => (None, None, None, None),
        RunFailedData::V1(data) => (data.node_id.as_deref(), None, None, None),
        RunFailedData::V2(RunFailedDataV2::Attempt { node_id, .. }) => {
          (Some(node_id.as_str()), None, None, None)
        }
        RunFailedData::V2(RunFailedDataV2::Branch {
          branch_id, arm_id, ..
        }) => (
          None,
          Some((branch_id.as_str(), arm_id.as_deref())),
          None,
          None,
        ),
        RunFailedData::V3(RunFailedDataV3::Parallel {
          parallel_id,
          policy,
          primary_node_id,
          failed_node_ids,
          cancelled_node_ids,
          ..
        }) => (
          None,
          None,
          Some((
            parallel_id.as_str(),
            *policy,
            primary_node_id.as_str(),
            failed_node_ids,
            cancelled_node_ids,
          )),
          None,
        ),
        RunFailedData::V4(RunFailedDataV4::Approval {
          approval_id,
          request_id,
          ..
        }) => (
          None,
          None,
          None,
          Some((approval_id.as_str(), request_id.as_str())),
        ),
      };
      if node_id.is_some_and(|node_id| workflow.node(node_id).is_none()) {
        return Err(format!("run_failed references unknown node {node_id:?}."));
      }
      if let Some((branch_id, arm_id)) = branch_identity {
        let selector_id = format!("__woml_branch__{branch_id}__select");
        if workflow.node(&selector_id).is_none() {
          return Err(format!(
            "run_failed references unknown branch {branch_id:?}."
          ));
        }
        if arm_id.is_some_and(|arm_id| {
          !workflow.graph.edges.iter().any(|edge| {
            edge.from == selector_id
              && edge.id == arm_id
              && edge.branch_id.as_deref() == Some(branch_id)
          })
        }) {
          return Err(format!(
            "run_failed references an unknown arm for branch {branch_id:?}."
          ));
        }
      }
      if let Some((parallel_id, policy, primary_node_id, failed, cancelled)) = parallel_identity {
        let group = workflow.parallel_group(parallel_id).ok_or_else(|| {
          format!("run_failed references unknown parallel group {parallel_id:?}.")
        })?;
        let expected_policy = match group.on_error.as_str() {
          "fail-fast" => ParallelFailurePolicy::FailFast,
          "wait-all" => ParallelFailurePolicy::WaitAll,
          _ => {
            return Err(format!(
              "Parallel group {parallel_id:?} has an invalid policy."
            ))
          }
        };
        if policy != expected_policy
          || !failed.iter().any(|node_id| node_id == primary_node_id)
          || failed
            .iter()
            .chain(cancelled)
            .any(|node_id| !group.child_node_ids.contains(node_id))
        {
          return Err(format!(
            "run_failed does not match parallel group {parallel_id:?} and its compiled policy."
          ));
        }
      }
      if let Some((approval_id, _request_id)) = approval_identity {
        if workflow.approval(approval_id).is_none() {
          return Err(format!(
            "run_failed references unknown approval {approval_id:?}."
          ));
        }
      }
    }
    RunEventPayload::ForEachOpened(data) => validate_for_each_id(workflow, &data.for_each_id)?,
    RunEventPayload::ForEachIterationStarted(data) => {
      validate_for_each_id(workflow, &data.for_each_id)?
    }
    RunEventPayload::ForEachIterationSucceeded(data) => {
      validate_for_each_id(workflow, &data.for_each_id)?
    }
    RunEventPayload::ForEachIterationFailed(data) => {
      validate_for_each_id(workflow, &data.for_each_id)?
    }
    RunEventPayload::ForEachIterationSkipped(data) => {
      validate_for_each_id(workflow, &data.for_each_id)?
    }
    RunEventPayload::ForEachSucceeded(data)
    | RunEventPayload::ForEachFailed(data)
    | RunEventPayload::ForEachCancelled(data) => validate_for_each_id(workflow, &data.for_each_id)?,
    RunEventPayload::SagaStarted(data)
    | RunEventPayload::SagaForwardSucceeded(data)
    | RunEventPayload::SagaCorrectionCompleted(data) => {
      if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V19
        || workflow
          .graph
          .sagas
          .as_deref()
          .unwrap_or_default()
          .iter()
          .all(|saga| saga.saga_id != data.saga_id)
      {
        return Err(format!(
          "Correction event references unknown saga {:?}.",
          data.saga_id
        ));
      }
    }
    RunEventPayload::CompensationRegistered(data) => {
      let valid = workflow
        .graph
        .sagas
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|saga| saga.saga_id == data.saga_id)
        .is_some_and(|saga| {
          saga.compensations.iter().any(|action| {
            action.action_id == data.action_id
              && action.for_step_id.as_deref() == Some(data.for_step_id.as_str())
          })
        });
      if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V19 || !valid {
        return Err("Compensation registration does not match Model v19.".to_string());
      }
    }
    RunEventPayload::SagaCorrectionStarted(data) => {
      if workflow
        .graph
        .sagas
        .as_deref()
        .unwrap_or_default()
        .iter()
        .all(|saga| saga.saga_id != data.saga_id)
      {
        return Err("Saga correction references an unknown Model v19 saga.".to_string());
      }
    }
    RunEventPayload::CorrectionAttemptStarted(data)
    | RunEventPayload::CorrectionAttemptSucceeded(data) => {
      validate_correction_action(workflow, &data.saga_id, &data.action_id, data.action_kind)?;
    }
    RunEventPayload::CorrectionAttemptFailed(data) => {
      validate_correction_action(workflow, &data.saga_id, &data.action_id, data.action_kind)?;
    }
    RunEventPayload::CorrectionRetryScheduled(data) => {
      validate_correction_action(workflow, &data.saga_id, &data.action_id, data.action_kind)?;
    }
    RunEventPayload::CorrectionInterrupted(data) => {
      validate_correction_action(workflow, &data.saga_id, &data.action_id, data.action_kind)?;
    }
    RunEventPayload::ManualRecoveryRequired(data) => {
      validate_correction_action(workflow, &data.saga_id, &data.action_id, data.action_kind)?;
    }
    RunEventPayload::InfrastructurePolicyDecisionRecorded(_) => {}
    RunEventPayload::RemoteDispatchTransitioned(data) => {
      let node = workflow.node(&data.node_id).or_else(|| {
        workflow
          .graph
          .sagas
          .as_deref()
          .unwrap_or_default()
          .iter()
          .flat_map(|saga| saga.compensations.iter().chain(&saga.cleanup))
          .filter(|action| {
            matches!(
              &action.identity,
              crate::model::CompiledCorrectionIdentity::Static { .. }
            )
          })
          .find(|action| action.node.id == data.node_id)
          .map(|action| &action.node)
      });
      validate_remote_dispatch_placement(workflow, node, data)?;
    }
  }

  Ok(())
}

fn validate_correction_action(
  workflow: &CompiledWorkflowDefinition,
  saga_id: &str,
  action_id: &str,
  kind: crate::CorrectiveActionKind,
) -> Result<(), String> {
  let saga = workflow
    .graph
    .sagas
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|saga| saga.saga_id == saga_id)
    .ok_or_else(|| format!("Correction action references unknown saga {saga_id:?}."))?;
  let exists = match kind {
    crate::CorrectiveActionKind::Compensation => saga.compensations.iter(),
    crate::CorrectiveActionKind::Cleanup => saga.cleanup.iter(),
  }
  .any(|action| match &action.identity {
    crate::model::CompiledCorrectionIdentity::Static {
      action_id: compiled_action_id,
    } => compiled_action_id == action_id,
    crate::model::CompiledCorrectionIdentity::ForEachIteration {
      action_id_template, ..
    } => {
      let Some((prefix, suffix)) = action_id_template.split_once("{iterationKey}") else {
        return false;
      };
      action_id
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
        .is_some_and(|key| {
          !key.is_empty() && key.chars().all(|character| character.is_ascii_digit())
        })
    }
  });
  if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V19 || !exists {
    return Err(format!(
      "Correction action {action_id:?} does not match saga {saga_id:?}."
    ));
  }
  Ok(())
}

fn validate_remote_dispatch_placement(
  workflow: &CompiledWorkflowDefinition,
  node: Option<&CompiledWorkflowNode>,
  data: &crate::RemoteDispatchTransitionedData,
) -> Result<(), String> {
  let node = node.ok_or_else(|| {
    format!(
      "Remote dispatch references unknown executable node {:?}.",
      data.node_id
    )
  })?;
  let maximum_attempts = node
    .retry_policy
    .as_ref()
    .map_or(1, |retry| retry.max_attempts);
  if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V21
    || node.handler != "runtime.script"
    || node
      .execution
      .as_ref()
      .is_none_or(|execution| execution.target == "local" || execution.target != data.target)
    || data.attempt == 0
    || data.attempt > maximum_attempts
  {
    return Err(format!(
      "Remote dispatch for node {:?} does not match its Model v21 placement and retry policy.",
      data.node_id
    ));
  }
  Ok(())
}

pub(crate) fn validate_scoped_payload_against_definition(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  run_id: &str,
  scope: &crate::event::ForEachIterationScope,
  payload: &RunEventPayload,
) -> Result<(), String> {
  let descriptor = workflow
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|descriptor| descriptor.for_each_id == scope.for_each_id)
    .ok_or_else(|| format!("Unknown for-each scope {:?}.", scope.for_each_id))?;
  let body_node = |node_id: &str| descriptor.body.nodes.iter().find(|node| node.id == node_id);
  match payload {
    RunEventPayload::StepAttemptStarted(data) => {
      let node = body_node(&data.node_id).ok_or_else(|| {
        format!(
          "Iteration attempt references unknown node {:?}.",
          data.node_id
        )
      })?;
      let maximum = node
        .retry_policy
        .as_ref()
        .map_or(1, |policy| policy.max_attempts);
      if data.handler != node.handler || data.attempt == 0 || data.attempt > maximum {
        return Err(format!(
          "Iteration attempt for node {:?} does not match its compiled handler and retry policy.",
          data.node_id
        ));
      }
      Ok(())
    }
    RunEventPayload::CompensationRegistered(data) => {
      let valid = workflow.saga(&data.saga_id).is_some_and(|saga| {
        saga.compensations.iter().any(|action| {
          action.for_step_id.as_deref() == Some(data.for_step_id.as_str())
            && matches!(
              &action.identity,
              crate::model::CompiledCorrectionIdentity::ForEachIteration {
                for_each_id,
                action_id_template,
              } if for_each_id == &scope.for_each_id
                && action_id_template.replace("{iterationKey}", &scope.index.to_string())
                  == data.action_id
            )
        })
      });
      valid
        .then_some(())
        .ok_or_else(|| "Iteration compensation registration does not match Model v19.".to_string())
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      body_node(&data.node_id).map(|_| ()).ok_or_else(|| {
        format!(
          "Iteration attempt references unknown node {:?}.",
          data.node_id
        )
      })
    }
    RunEventPayload::StepAttemptFailed(data) => {
      body_node(&data.node_id).map(|_| ()).ok_or_else(|| {
        format!(
          "Iteration attempt references unknown node {:?}.",
          data.node_id
        )
      })
    }
    RunEventPayload::StepRetryScheduled(data) => {
      let node = body_node(&data.node_id).ok_or_else(|| {
        format!(
          "Iteration retry references unknown node {:?}.",
          data.node_id
        )
      })?;
      let policy = node
        .retry_policy
        .as_ref()
        .ok_or_else(|| format!("Iteration node {:?} has no retry policy.", data.node_id))?;
      if data.next_attempt != data.failed_attempt + 1 || data.next_attempt > policy.max_attempts {
        return Err(format!(
          "Iteration retry for node {:?} exceeds its compiled policy.",
          data.node_id
        ));
      }
      Ok(())
    }
    RunEventPayload::ChoiceSelected(data) => {
      let choice = descriptor
        .body
        .choices
        .iter()
        .find(|choice| choice.choice_id == data.choice_id)
        .ok_or_else(|| format!("Unknown iteration choice {:?}.", data.choice_id))?;
      if choice.arm_ids.contains(&data.arm_id) {
        Ok(())
      } else {
        Err(format!(
          "Arm {:?} is not selectable for iteration choice {:?}.",
          data.arm_id, data.choice_id
        ))
      }
    }
    RunEventPayload::BranchSelected(data) => {
      let selector_id = format!("__woml_branch__{}__select", data.branch_id);
      let selector = body_node(&selector_id)
        .ok_or_else(|| format!("Unknown iteration conditional branch {:?}.", data.branch_id))?;
      let valid_arm = selector.handler == "engine.branch-select"
        && descriptor.body.edges.iter().any(|edge| {
          edge.from == selector_id
            && edge.id == data.arm_id
            && edge.branch_id.as_deref() == Some(data.branch_id.as_str())
        });
      if valid_arm {
        Ok(())
      } else {
        Err(format!(
          "Arm {:?} is not selectable for iteration branch {:?}.",
          data.arm_id, data.branch_id
        ))
      }
    }
    RunEventPayload::ParallelGroupStarted(data) => {
      let start = format!("__woml_parallel__{}__start", data.parallel_id);
      if body_node(&start).is_some()
        && body_node(&data.parallel_id).is_some_and(|node| node.handler == "engine.parallel-join")
      {
        Ok(())
      } else {
        Err(format!(
          "Unknown iteration parallel group {:?}.",
          data.parallel_id
        ))
      }
    }
    RunEventPayload::ParallelGroupCompleted(data) => {
      let start = format!("__woml_parallel__{}__start", data.parallel_id);
      if body_node(&start).is_some()
        && body_node(&data.parallel_id).is_some_and(|node| node.handler == "engine.parallel-join")
      {
        Ok(())
      } else {
        Err(format!(
          "Unknown iteration parallel group {:?}.",
          data.parallel_id
        ))
      }
    }
    RunEventPayload::OperationStarted(data) => {
      body_node(&data.node_id).map(|_| ()).ok_or_else(|| {
        format!(
          "Iteration operation references unknown node {:?}.",
          data.node_id
        )
      })
    }
    RunEventPayload::OperationSucceeded(data) => {
      body_node(&data.node_id).map(|_| ()).ok_or_else(|| {
        format!(
          "Iteration operation references unknown node {:?}.",
          data.node_id
        )
      })
    }
    RunEventPayload::OperationFailed(data) => {
      body_node(&data.node_id).map(|_| ()).ok_or_else(|| {
        format!(
          "Iteration operation references unknown node {:?}.",
          data.node_id
        )
      })
    }
    RunEventPayload::LifecycleHookRequested(data) => {
      let prefix = format!("{}[{}].", scope.for_each_id, scope.index);
      let node_id = data.subject.id.strip_prefix(&prefix).ok_or_else(|| {
        "Iteration lifecycle subject does not match its durable scope.".to_string()
      })?;
      let hook = workflow
        .lifecycle_hook_for_step_event(data.event, node_id)
        .ok_or_else(|| {
          "Iteration lifecycle request does not match a compiled step hook.".to_string()
        })?;
      let expected_invocation_id = crate::derive_lifecycle_hook_invocation_id(
        run_id,
        &data.hook_id,
        data.subject.kind,
        &data.subject.id,
      );
      if data.subject.kind != crate::LifecycleSubjectKind::Step
        || hook.hook_id != data.hook_id
        || expected_invocation_id != data.hook_invocation_id
      {
        return Err("Iteration lifecycle request does not match its compiled binding.".to_string());
      }
      Ok(())
    }
    RunEventPayload::LifecycleActionAttemptStarted(data)
    | RunEventPayload::LifecycleActionSucceeded(data) => {
      let action_exists = workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
        lifecycle
          .hooks
          .iter()
          .flat_map(|hook| &hook.actions)
          .any(|action| action.action_id == data.action_id)
      });
      if !action_exists || data.attempt == 0 {
        Err("Iteration lifecycle action does not match its compiled binding.".to_string())
      } else {
        Ok(())
      }
    }
    RunEventPayload::LifecycleActionFailed(data) => {
      let action_exists = workflow.lifecycle.as_ref().is_some_and(|lifecycle| {
        lifecycle
          .hooks
          .iter()
          .flat_map(|hook| &hook.actions)
          .any(|action| action.action_id == data.action_id)
      });
      if !action_exists || data.attempt == 0 {
        Err("Iteration lifecycle action does not match its compiled binding.".to_string())
      } else {
        Ok(())
      }
    }
    RunEventPayload::LifecycleHookCompleted(_) => Ok(()),
    RunEventPayload::ReusableLifecycleRequested(_)
    | RunEventPayload::ReusableLifecycleActionStarted(_)
    | RunEventPayload::ReusableLifecycleActionSucceeded(_)
    | RunEventPayload::ReusableLifecycleActionFailed(_)
    | RunEventPayload::ReusableLifecycleCompleted(_) => {
      validate_payload_against_definition(workflow, definition_hash, payload)
    }
    RunEventPayload::RemoteDispatchTransitioned(data) => {
      let corrective_node = workflow
        .graph
        .sagas
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|saga| saga.compensations.iter().chain(&saga.cleanup))
        .filter(|action| {
          matches!(
            &action.identity,
            crate::model::CompiledCorrectionIdentity::ForEachIteration { for_each_id, .. }
              if for_each_id == &scope.for_each_id
          )
        })
        .find(|action| action.node.id == data.node_id)
        .map(|action| &action.node);
      validate_remote_dispatch_placement(
        workflow,
        body_node(&data.node_id).or(corrective_node),
        data,
      )
    }
    _ => validate_payload_against_definition(workflow, definition_hash, payload),
  }
}
