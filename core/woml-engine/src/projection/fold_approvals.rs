//! projection fold approvals.

use super::{
  require_running, ApprovalRequestProjection, ApprovalRequestStatus, FoldError,
  RichApprovalRequestProjection, RichApprovalRequestStatus, RichApprovalReviewerDecisionProjection,
  RunProjection, RunStatus,
};
use crate::event::ApprovalDecision;
use crate::event::ApprovalDecisionSource;
use crate::event::ApprovalResolution;
use crate::event::ApprovalTimeoutPolicy;
use crate::event::RunEvent;
use chrono::SecondsFormat;
use serde_json::json;
use std::collections::BTreeMap;

pub(super) fn apply_approval_requested(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ApprovalRequestedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection.approval_requests.contains_key(&data.approval_id) {
    return Err(FoldError::InvalidHistory(format!(
      "Approval {:?} was requested more than once.",
      data.approval_id
    )));
  }
  if projection
    .approval_requests
    .values()
    .any(|request| request.request_id == data.request_id)
  {
    return Err(FoldError::InvalidHistory(format!(
      "Approval request ID {:?} appears more than once.",
      data.request_id
    )));
  }
  projection.approval_requests.insert(
    data.approval_id.clone(),
    ApprovalRequestProjection {
      approval_id: data.approval_id.clone(),
      request_id: data.request_id.clone(),
      requested_at: event.occurred_at,
      expires_at: data.expires_at,
      on_timeout: data.on_timeout,
      status: ApprovalRequestStatus::Waiting,
    },
  );
  projection.status = RunStatus::Waiting;
  Ok(())
}

pub(super) fn apply_approval_review_requested(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ApprovalReviewRequestedData,
) -> Result<(), FoldError> {
  require_running(projection)?;
  if projection
    .rich_approval_requests
    .contains_key(&data.approval_id)
    || projection
      .rich_approval_requests
      .values()
      .any(|request| request.request_id == data.request_id)
  {
    return Err(FoldError::InvalidHistory(
      "A rich approval request identity appears more than once.".to_string(),
    ));
  }
  projection.rich_approval_requests.insert(
    data.approval_id.clone(),
    RichApprovalRequestProjection {
      approval_id: data.approval_id.clone(),
      request_id: data.request_id.clone(),
      review_digest: data.review_digest.clone(),
      directory_version: data.directory_version.clone(),
      required: data.required,
      required_roles: data.required_roles.clone(),
      represented_roles: Vec::new(),
      requested_at: event.occurred_at,
      resolved_at: None,
      status: RichApprovalRequestStatus::Waiting,
      decisions: BTreeMap::new(),
    },
  );
  projection.status = RunStatus::Waiting;
  Ok(())
}

pub(super) fn apply_approval_reviewer_decision_recorded(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ApprovalReviewerDecisionRecordedData,
) -> Result<(), FoldError> {
  let request = projection
    .rich_approval_requests
    .get_mut(&data.approval_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory(
        "A reviewer decision references an unknown rich approval.".to_string(),
      )
    })?;
  if projection.status != RunStatus::Waiting
    || request.status != RichApprovalRequestStatus::Waiting
    || request.request_id != data.request_id
    || request.review_digest != data.review_digest
    || request.decisions.contains_key(&data.principal_id)
  {
    return Err(FoldError::InvalidHistory(
      "A reviewer decision does not match one pending immutable review.".to_string(),
    ));
  }
  request.decisions.insert(
    data.principal_id.clone(),
    RichApprovalReviewerDecisionProjection {
      principal_id: data.principal_id.clone(),
      decision: data.decision,
      decided_at: event.occurred_at,
    },
  );
  Ok(())
}

pub(super) fn apply_approval_quorum_progressed(
  projection: &mut RunProjection,
  data: &crate::event::ApprovalQuorumProgressedData,
) -> Result<(), FoldError> {
  let request = projection
    .rich_approval_requests
    .get_mut(&data.approval_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("Quorum progress references an unknown rich approval.".to_string())
    })?;
  let approved = request
    .decisions
    .values()
    .filter(|decision| decision.decision == ApprovalDecision::Approved)
    .count() as u32;
  if request.status != RichApprovalRequestStatus::Waiting
    || request.request_id != data.request_id
    || request.review_digest != data.review_digest
    || request.required != data.required
    || request.required_roles != data.required_roles
    || approved != data.received
  {
    return Err(FoldError::InvalidHistory(
      "Quorum progress is not derived from the durable rich approval history.".to_string(),
    ));
  }
  request.represented_roles = data.represented_roles.clone();
  Ok(())
}

pub(super) fn apply_approval_resolved(
  projection: &mut RunProjection,
  event: &RunEvent,
  data: &crate::event::ApprovalResolvedData,
) -> Result<(), FoldError> {
  if data.is_rich() {
    let request = projection
      .rich_approval_requests
      .get_mut(&data.approval_id)
      .ok_or_else(|| {
        FoldError::InvalidHistory(
          "A rich approval resolved before its review was requested.".to_string(),
        )
      })?;
    let decision = data.decision.ok_or_else(|| {
      FoldError::InvalidHistory("A rich approval resolution has no decision.".to_string())
    })?;
    let cancellation_resolution = data.reason
      == Some(crate::RichApprovalResolutionReason::Cancellation)
      && projection.status == RunStatus::Cancelling;
    if (projection.status != RunStatus::Waiting && !cancellation_resolution)
      || request.status != RichApprovalRequestStatus::Waiting
      || request.request_id != data.request_id
      || data.review_digest.as_deref() != Some(request.review_digest.as_str())
      || data.required != Some(request.required)
      || data.required_roles.as_deref() != Some(request.required_roles.as_slice())
    {
      return Err(FoldError::InvalidHistory(
        "A rich approval resolution does not match its pending immutable review.".to_string(),
      ));
    }
    request.represented_roles = data.represented_roles.clone().unwrap_or_default();
    request.status = match data.reason {
      Some(crate::RichApprovalResolutionReason::Quorum) => RichApprovalRequestStatus::Approved,
      Some(crate::RichApprovalResolutionReason::Rejection) => RichApprovalRequestStatus::Rejected,
      Some(crate::RichApprovalResolutionReason::Timeout) => RichApprovalRequestStatus::Expired,
      Some(crate::RichApprovalResolutionReason::Cancellation) => {
        RichApprovalRequestStatus::Cancelled
      }
      Some(crate::RichApprovalResolutionReason::ArtifactInvalidated) => {
        RichApprovalRequestStatus::Invalidated
      }
      None => {
        return Err(FoldError::InvalidHistory(
          "A rich approval resolution has no reason.".to_string(),
        ))
      }
    };
    request.resolved_at = Some(event.occurred_at);
    projection.context.steps.insert(
      data.approval_id.clone(),
      json!({
        "decision": match decision {
          ApprovalDecision::Approved => "approved",
          ApprovalDecision::Rejected => "rejected",
        },
        "source": "human",
        "decidedAt": event.occurred_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        "review": {
          "digest": request.review_digest.clone(),
        },
        "authority": {
          "required": request.required,
          "received": data.received.unwrap_or_default(),
          "requiredRoles": request.required_roles.clone(),
        }
      }),
    );
    projection.status = if cancellation_resolution {
      RunStatus::Cancelling
    } else {
      RunStatus::Running
    };
  } else {
    if projection.status != RunStatus::Waiting {
      return Err(FoldError::InvalidHistory(
        "approval_resolved requires a waiting run.".to_string(),
      ));
    }
    let request = projection
      .approval_requests
      .get_mut(&data.approval_id)
      .ok_or_else(|| {
        FoldError::InvalidHistory(format!(
          "Approval {:?} resolved before it was requested.",
          data.approval_id
        ))
      })?;
    if request.request_id != data.request_id {
      return Err(FoldError::InvalidHistory(format!(
        "Approval {:?} resolved with a mismatched request ID.",
        data.approval_id
      )));
    }
    if !matches!(request.status, ApprovalRequestStatus::Waiting) {
      return Err(FoldError::InvalidHistory(format!(
        "Approval {:?} was resolved more than once.",
        data.approval_id
      )));
    }
    let resolution = data.resolution.as_ref().ok_or_else(|| {
      FoldError::InvalidHistory("A basic approval resolution is missing.".to_string())
    })?;
    match resolution {
      ApprovalResolution::Decision {
        source: ApprovalDecisionSource::Human,
        ..
      } if request
        .expires_at
        .is_some_and(|deadline| event.occurred_at >= deadline) =>
      {
        return Err(FoldError::InvalidHistory(format!(
          "Human decision for approval {:?} occurred at or after its deadline.",
          data.approval_id
        )));
      }
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        source: ApprovalDecisionSource::Timeout,
      } if request.on_timeout == ApprovalTimeoutPolicy::Reject
        && request
          .expires_at
          .is_some_and(|deadline| event.occurred_at >= deadline) => {}
      ApprovalResolution::TimeoutFailure
        if request.on_timeout == ApprovalTimeoutPolicy::Fail
          && request
            .expires_at
            .is_some_and(|deadline| event.occurred_at >= deadline) => {}
      ApprovalResolution::Decision {
        source: ApprovalDecisionSource::Human,
        ..
      } => {}
      _ => {
        return Err(FoldError::InvalidHistory(format!(
          "Approval {:?} resolution does not match its deadline and timeout policy.",
          data.approval_id
        )));
      }
    }
    if let ApprovalResolution::Decision { decision, source } = resolution {
      projection.context.steps.insert(
        data.approval_id.clone(),
        json!({
          "decision": match *decision {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Rejected => "rejected",
          },
          "source": match *source {
            ApprovalDecisionSource::Human => "human",
            ApprovalDecisionSource::Timeout => "timeout",
          },
          "decidedAt": event
            .occurred_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        }),
      );
    }
    request.status = ApprovalRequestStatus::Resolved {
      resolution: resolution.clone(),
      resolved_at: event.occurred_at,
    };
    projection.status = RunStatus::Running;
  }
  Ok(())
}

pub(super) fn apply_approval_invalidated(
  projection: &mut RunProjection,
  data: &crate::event::ApprovalInvalidatedData,
) -> Result<(), FoldError> {
  let request = projection
    .rich_approval_requests
    .get_mut(&data.approval_id)
    .ok_or_else(|| {
      FoldError::InvalidHistory("An invalidation references an unknown rich approval.".to_string())
    })?;
  if projection.status != RunStatus::Waiting
    || request.status != RichApprovalRequestStatus::Waiting
    || request.request_id != data.request_id
    || request.review_digest != data.review_digest
  {
    return Err(FoldError::InvalidHistory(
      "An invalidation does not match one pending immutable review.".to_string(),
    ));
  }
  // The invalidation fact records why the immutable review can no longer
  // be authorized. The following approval_resolved fact wins the shared
  // terminal race and closes the request.
  Ok(())
}
