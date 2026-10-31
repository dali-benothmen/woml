//! Runtime participation in held leases and root/iteration lock-key resolution.
//!
//! Renewal identities remain scope-specific. Durable lock authority still owns
//! FIFO grants and fencing; stopping renewal does not prove an external owner
//! stopped, and does not authorize releasing unproven ownership.

use crate::engine::resolve_context_reference;
use crate::event::ForEachIterationScope;
use crate::event::LifecycleFailure;
use crate::event::LifecycleFailureKind;
use crate::model::TemplatePart;
use crate::model::ValueExpression;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::{
  render_template_scalar, resolve_for_each_reference, resumed_failure, RuntimeDagEngine,
};
use crate::RunStatus;
use crate::WorkflowContext;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

impl RuntimeExecutionOptions {
  pub(in crate::runtime) fn ensure_lock_renewal(&self, run_id: &str, lock_id: &str, lease_ms: u64) {
    let Some(authority) = self.capability_authority.clone() else {
      return;
    };
    let identity = format!("{run_id}\0{lock_id}");
    let cancelled = Arc::new(AtomicBool::new(false));
    {
      let Ok(mut renewals) = self.lock_renewals.lock() else {
        return;
      };
      if renewals.contains_key(&identity) {
        return;
      }
      renewals.insert(identity.clone(), Arc::clone(&cancelled));
    }
    let renewals = Arc::clone(&self.lock_renewals);
    let run_id = run_id.to_string();
    let lock_id = lock_id.to_string();
    let interval = Duration::from_millis((lease_ms / 3).max(1));
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(interval).await;
        if cancelled.load(Ordering::SeqCst) {
          break;
        }
        let outcome = authority.store().lock().await.renew_lock_lease_atomically(
          &run_id,
          &lock_id,
          chrono::Utc::now(),
        );
        if !matches!(outcome, Ok(crate::LockLeaseMaintenance::Renewed)) {
          break;
        }
      }
      if let Ok(mut active) = renewals.lock() {
        active.remove(&identity);
      }
    });
  }
  pub(in crate::runtime) fn stop_lock_renewal(&self, run_id: &str, lock_id: &str) {
    let identity = format!("{run_id}\0{lock_id}");
    if let Ok(mut renewals) = self.lock_renewals.lock() {
      if let Some(cancelled) = renewals.remove(&identity) {
        cancelled.store(true, Ordering::SeqCst);
      }
    }
  }
  pub(in crate::runtime) fn ensure_iteration_lock_renewal(
    &self,
    run_id: &str,
    lock_id: &str,
    scope: ForEachIterationScope,
    lease_ms: u64,
  ) {
    let Some(authority) = self.capability_authority.clone() else {
      return;
    };
    let identity = format!(
      "{run_id}\0{}\0{}\0{lock_id}",
      scope.for_each_id, scope.index
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    {
      let Ok(mut renewals) = self.lock_renewals.lock() else {
        return;
      };
      if renewals.contains_key(&identity) {
        return;
      }
      renewals.insert(identity.clone(), Arc::clone(&cancelled));
    }
    let renewals = Arc::clone(&self.lock_renewals);
    let run_id = run_id.to_string();
    let lock_id = lock_id.to_string();
    let interval = Duration::from_millis((lease_ms / 3).max(1));
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(interval).await;
        if cancelled.load(Ordering::SeqCst) {
          break;
        }
        let outcome = authority
          .store()
          .lock()
          .await
          .renew_lock_lease_atomically_scoped(
            &run_id,
            &lock_id,
            Some(scope.clone()),
            chrono::Utc::now(),
          );
        if !matches!(outcome, Ok(crate::LockLeaseMaintenance::Renewed)) {
          break;
        }
      }
      if let Ok(mut active) = renewals.lock() {
        active.remove(&identity);
      }
    });
  }
  pub(in crate::runtime) fn stop_iteration_lock_renewal(
    &self,
    run_id: &str,
    lock_id: &str,
    scope: &ForEachIterationScope,
  ) {
    let identity = format!(
      "{run_id}\0{}\0{}\0{lock_id}",
      scope.for_each_id, scope.index
    );
    if let Ok(mut renewals) = self.lock_renewals.lock() {
      if let Some(cancelled) = renewals.remove(&identity) {
        cancelled.store(true, Ordering::SeqCst);
      }
    }
  }
}

pub(in crate::runtime) fn ensure_held_locks_current<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let mut lost = projection
    .locks
    .values()
    .filter(|lock| lock.status == crate::LockStatus::LeaseLost)
    .map(|lock| lock.lock_id.clone())
    .collect::<Vec<_>>();
  lost.extend(projection.for_each.values().flat_map(|loop_state| {
    loop_state.executions.values().flat_map(|execution| {
      execution
        .locks
        .values()
        .filter(|lock| lock.status == crate::LockStatus::LeaseLost)
        .map(|lock| lock.lock_id.clone())
    })
  }));
  if let Some(lock_id) = lost.first() {
    if projection.business_outcome.is_some() || projection.status == RunStatus::Cancelling {
      return Ok(());
    }
    engine.decide_run_failed(
      run_id,
      LifecycleFailure {
        kind: LifecycleFailureKind::Interrupted,
        code: "WOML_LOCK_LEASE_LOST".to_string(),
        message: format!(
          "Lock {lock_id:?} expired before its protected continuation could finish."
        ),
      },
      options.clock.now(),
    )?;
    return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
  }
  let held = projection
    .locks
    .values()
    .filter(|lock| lock.status == crate::LockStatus::Held)
    .map(|lock| lock.lock_id.clone())
    .collect::<Vec<_>>();
  for lock_id in held {
    let descriptor = engine
      .workflow()
      .durable_lock(&lock_id)
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled("A held lease has no compiled lock descriptor.".to_string())
      })?;
    match engine.renew_lock_lease(run_id, &lock_id, options.clock.now())? {
      crate::LockLeaseMaintenance::Renewed => {
        options.ensure_lock_renewal(run_id, &lock_id, descriptor.lease_ms);
      }
      crate::LockLeaseMaintenance::Lost | crate::LockLeaseMaintenance::NotHeld => {
        options.stop_lock_renewal(run_id, &lock_id);
        let projection = engine.projection(run_id)?;
        if projection.business_outcome.is_none() {
          engine.decide_run_failed(
            run_id,
            LifecycleFailure {
              kind: LifecycleFailureKind::Interrupted,
              code: "WOML_LOCK_LEASE_LOST".to_string(),
              message: format!(
                "Lock {lock_id:?} expired before WOML could renew its durable lease."
              ),
            },
            options.clock.now(),
          )?;
        }
        return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
      }
    }
  }
  Ok(())
}

pub(in crate::runtime) fn resolve_lock_key(
  expression: &ValueExpression,
  context: &WorkflowContext,
) -> Result<String, RuntimeExecutionError> {
  match expression {
    ValueExpression::Literal { value } => value.as_str().map(str::to_string),
    ValueExpression::ContextReference { .. } => resolve_context_reference(expression, context)
      .ok()
      .and_then(render_template_scalar),
    ValueExpression::Template { parts } => {
      let mut rendered = String::new();
      for part in parts {
        match part {
          TemplatePart::Text { text } => rendered.push_str(text),
          TemplatePart::ContextReference { path } => {
            let value = resolve_context_reference(
              &ValueExpression::ContextReference { path: path.clone() },
              context,
            )
            .ok()
            .and_then(render_template_scalar)
            .ok_or_else(|| {
              RuntimeExecutionError::InvalidConfiguration(
                "A lock-key reference is unavailable or is not scalar.".to_string(),
              )
            })?;
            rendered.push_str(&value);
          }
          TemplatePart::LifecycleReference { .. } => {
            return Err(RuntimeExecutionError::InvalidConfiguration(
              "Lock keys cannot use lifecycle references.".to_string(),
            ))
          }
        }
      }
      Some(rendered)
    }
    _ => None,
  }
  .filter(|key| {
    !key.is_empty()
      && key.len() <= 256
      && key.trim() == key
      && !key.chars().any(char::is_control)
      && !key.split('/').any(str::is_empty)
  })
  .ok_or_else(|| {
    RuntimeExecutionError::InvalidConfiguration(
      "The lock key did not resolve to a valid scalar key.".to_string(),
    )
  })
}

pub(in crate::runtime) fn resolve_for_each_lock_key(
  expression: &ValueExpression,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<String, RuntimeExecutionError> {
  let rendered = match expression {
    ValueExpression::Literal { value } => value.as_str().map(str::to_string),
    ValueExpression::ContextReference { .. } => render_template_scalar(resolve_for_each_reference(
      expression, context, item, index, total,
    )?),
    ValueExpression::Template { parts } => {
      let mut output = String::new();
      for part in parts {
        match part {
          TemplatePart::Text { text } => output.push_str(text),
          TemplatePart::ContextReference { path } => {
            let value = resolve_for_each_reference(
              &ValueExpression::ContextReference { path: path.clone() },
              context,
              item,
              index,
              total,
            )?;
            output.push_str(&render_template_scalar(value).ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock key contains a non-scalar context value.".to_string(),
              )
            })?);
          }
          TemplatePart::LifecycleReference { .. } => {
            return Err(RuntimeExecutionError::Stalled(
              "Lock keys cannot contain lifecycle references.".to_string(),
            ));
          }
        }
      }
      Some(output)
    }
    _ => None,
  }
  .ok_or_else(|| {
    RuntimeExecutionError::Stalled("Iteration lock key did not resolve to a string.".to_string())
  })?;
  if rendered.is_empty()
    || rendered.len() > 256
    || rendered.trim() != rendered
    || rendered.chars().any(char::is_control)
    || rendered.split('/').any(str::is_empty)
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "The iteration lock key did not resolve to a valid scalar key.".to_string(),
    ));
  }
  Ok(rendered)
}
