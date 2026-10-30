//! Domain-separated operation keys and exact ordinary, iteration, corrective, and lifecycle invocation checks.

use crate::capability::NativeFetchInvocationIdentity;
use sha2::Digest;
use sha2::Sha256;

/// Rust's durable authority for managed capability operations. It records a
/// start before dispatch and a terminal event before exposing the result.
pub(in crate::capability) fn iteration_attempt_identity(
  projection: &crate::RunProjection,
  run_id: &str,
  definition_hash: &str,
  node_id: &str,
  attempt_number: u32,
  invocation_id: &str,
) -> Option<(crate::event::ForEachIterationScope, String)> {
  projection.for_each.values().find_map(|loop_| {
    loop_.executions.iter().find_map(|(index, execution)| {
      let attempt = execution.attempts.iter().find_map(|attempt| {
        (attempt.identity.node_id == node_id
          && attempt.identity.attempt == attempt_number
          && attempt.identity.invocation_id == invocation_id
          && attempt.status == crate::projection::AttemptStatus::Started)
          .then(|| {
            attempt.idempotency_key.clone().map(|key| {
              (
                crate::event::ForEachIterationScope {
                  for_each_id: loop_.for_each_id.clone(),
                  index: *index,
                },
                key,
              )
            })
          })
          .flatten()
      });
      attempt.or_else(|| {
        let lifecycle = execution.lifecycle_hooks.values().find_map(|hook| {
          hook.actions.get(node_id).and_then(|action| {
            (action.attempt == attempt_number
              && action.status == crate::projection::LifecycleActionStatus::Started)
              .then(|| {
                (
                  crate::event::ForEachIterationScope {
                    for_each_id: loop_.for_each_id.clone(),
                    index: *index,
                  },
                  crate::runtime::for_each_effect_key(
                    run_id,
                    definition_hash,
                    &loop_.for_each_id,
                    *index,
                    node_id,
                  ),
                )
              })
          })
        });
        lifecycle.or_else(|| {
          execution
            .reusable_lifecycle_hooks
            .values()
            .any(|hook| hook.active_action_id.as_deref() == Some(node_id))
            .then(|| {
              (
                crate::event::ForEachIterationScope {
                  for_each_id: loop_.for_each_id.clone(),
                  index: *index,
                },
                crate::runtime::for_each_effect_key(
                  run_id,
                  definition_hash,
                  &loop_.for_each_id,
                  *index,
                  node_id,
                ),
              )
            })
        })
      })
    })
  })
}

pub(in crate::capability) fn operational_invocation_is_active(
  projection: &crate::RunProjection,
  identity: &NativeFetchInvocationIdentity,
) -> bool {
  let root_attempt = projection.attempts.iter().any(|attempt| {
    attempt.identity.node_id == identity.node_id
      && attempt.identity.attempt == identity.attempt_number
      && attempt.identity.invocation_id == identity.invocation_id
      && attempt.status == crate::projection::AttemptStatus::Started
  });
  let root_lifecycle = projection.lifecycle_hooks.values().any(|hook| {
    hook.actions.get(&identity.node_id).is_some_and(|action| {
      action.attempt == identity.attempt_number
        && action.status == crate::projection::LifecycleActionStatus::Started
    })
  });
  let root_reusable = projection
    .reusable_lifecycle_hooks
    .values()
    .any(|hook| hook.active_action_id.as_deref() == Some(identity.node_id.as_str()));
  let iteration = projection.for_each.values().any(|loop_| {
    loop_.executions.values().any(|execution| {
      execution.attempts.iter().any(|attempt| {
        attempt.identity.node_id == identity.node_id
          && attempt.identity.attempt == identity.attempt_number
          && attempt.identity.invocation_id == identity.invocation_id
          && attempt.status == crate::projection::AttemptStatus::Started
      }) || execution.lifecycle_hooks.values().any(|hook| {
        hook.actions.get(&identity.node_id).is_some_and(|action| {
          action.attempt == identity.attempt_number
            && action.status == crate::projection::LifecycleActionStatus::Started
        })
      }) || execution
        .reusable_lifecycle_hooks
        .values()
        .any(|hook| hook.active_action_id.as_deref() == Some(identity.node_id.as_str()))
    })
  });
  let correction = projection.sagas.values().any(|saga| {
    saga.actions.get(&identity.node_id).is_some_and(|action| {
      action.active_attempt == Some(identity.attempt_number)
        && action.status == crate::projection::CorrectionActionStatus::Started
        && crate::durable::correction_invocation_id(
          &identity.run_id,
          &saga.saga_id,
          &identity.node_id,
          identity.attempt_number,
        ) == identity.invocation_id
    })
  });
  root_attempt || root_lifecycle || root_reusable || iteration || correction
}

pub(in crate::capability) fn operation_with_iteration_scope(
  projection: &crate::RunProjection,
  identity: &crate::projection::OperationIdentity,
) -> Option<(
  Option<crate::event::ForEachIterationScope>,
  crate::projection::OperationProjection,
)> {
  if let Some(operation) = projection.operations.get(identity) {
    return Some((None, operation.clone()));
  }
  projection.for_each.values().find_map(|loop_| {
    loop_.executions.iter().find_map(|(index, execution)| {
      execution.operations.get(identity).map(|operation| {
        (
          Some(crate::event::ForEachIterationScope {
            for_each_id: loop_.for_each_id.clone(),
            index: *index,
          }),
          operation.clone(),
        )
      })
    })
  })
}

pub fn derive_operation_key(step_idempotency_key: &str, operation_name: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.capability-operation\0v1\0");
  digest.update(step_idempotency_key.as_bytes());
  digest.update(b"\0");
  digest.update(operation_name.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}
