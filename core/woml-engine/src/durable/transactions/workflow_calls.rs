//! Parent/child workflow identity, lineage, deduplication, and child admission. Initial child history and its linkage keep the original writer boundary.

use crate::durable::append_to_history;
use crate::durable::definition_by_hash;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::load_run_binding_optional;
use crate::durable::DurableStoreError;
use crate::durable::{DurableEventStore, RUNTIME_POLICY_QUEUE_CEILING};
use crate::engine::validate_event_history_against_definition;
use crate::engine::validate_payload_against_definition;
use crate::event::is_definition_hash;
use crate::event::RunAdmissionQueue;
use crate::event::RunAdmissionTrigger;
use crate::event::RunAdmittedData;
use crate::event::RunIngress;
use crate::event::RunStartedData;
use crate::fold_events;
use crate::projection::AttemptStatus;
use crate::run_event_schema_version_for_model;
use crate::workflow_calls::WorkflowCallAdmission;
use crate::workflow_calls::WorkflowCallAdmissionOutcome;
use crate::workflow_calls::WorkflowCallAdmissionRequest;
use crate::workflow_calls::WorkflowCallIndexState;
use crate::workflow_calls::WorkflowCallRunRelations;
use crate::workflow_calls::WorkflowCallRunSummary;
use crate::workflow_calls::WorkflowTarget;
use crate::workflow_calls::MAX_WORKFLOW_CALL_DEPTH;
use crate::workflow_calls::MAX_WORKFLOW_CALL_INSPECTION_CHILDREN;
use crate::RunEventPayload;
use crate::RUN_EVENT_SCHEMA_VERSION_V11;
use crate::RUN_EVENT_SCHEMA_VERSION_V9;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::Map;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;

fn validate_workflow_call_admission_request(
  request: &WorkflowCallAdmissionRequest,
) -> Result<(), DurableStoreError> {
  let expected_child = format!(
    "run_call_{}",
    request
      .call_key
      .strip_prefix("sha256:")
      .unwrap_or(&request.call_key)
  );
  if !is_definition_hash(&request.call_key)
    || !is_definition_hash(&request.target_definition_hash)
    || request.child_run_id != expected_child
    || request.parent_run_id.is_empty()
    || request.parent_run_id.len() > 256
    || request.parent_node_id.is_empty()
    || request.parent_node_id.len() > 256
    || !(1..=10).contains(&request.parent_attempt)
    || request.target_workflow_id.is_empty()
    || request.target_workflow_id.len() > 256
  {
    return Err(DurableStoreError::Contract(
      "Workflow call admission identity is invalid.".to_string(),
    ));
  }
  durable_workflow_call_payload_digest(&request.payload)?;
  Ok(())
}

fn durable_workflow_call_payload_digest(
  payload: &Map<String, Value>,
) -> Result<String, DurableStoreError> {
  let encoded = canonical_json(payload).map_err(|_| {
    DurableStoreError::Contract("Workflow call payload is not canonical JSON.".to_string())
  })?;
  if encoded.len() > 1_048_576 {
    return Err(DurableStoreError::Contract(
      "Workflow call payload exceeds the 1 MiB durable admission limit.".to_string(),
    ));
  }
  Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn load_workflow_call(
  connection: &Connection,
  call_key: &str,
) -> Result<Option<WorkflowCallAdmission>, DurableStoreError> {
  let stored: Option<(
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
  )> = connection
    .query_row(
      "SELECT parent_run_id, parent_node_id, parent_attempt,
                target_workflow_id, target_definition_hash, child_run_id,
                payload_digest, depth, state, admitted_at
         FROM woml_workflow_calls WHERE call_key = ?1",
      [call_key],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
          row.get(7)?,
          row.get(8)?,
          row.get(9)?,
        ))
      },
    )
    .optional()?;
  let Some(stored) = stored else {
    return Ok(None);
  };
  if !is_definition_hash(call_key) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored call key is invalid".to_string(),
    ));
  }
  let expected_child_run_id = format!(
    "run_call_{}",
    call_key.strip_prefix("sha256:").unwrap_or(call_key)
  );
  if stored.5 != expected_child_run_id {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored child run identity does not match its call key".to_string(),
    ));
  }
  let parent_attempt = u32::try_from(stored.2).map_err(|_| {
    DurableStoreError::WorkflowCallHistoryInvalid("stored parent attempt is invalid".to_string())
  })?;
  if !(1..=10).contains(&parent_attempt) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored parent attempt is invalid".to_string(),
    ));
  }
  let depth = u32::try_from(stored.7).map_err(|_| {
    DurableStoreError::WorkflowCallHistoryInvalid("stored lineage depth is invalid".to_string())
  })?;
  if !(1..=MAX_WORKFLOW_CALL_DEPTH).contains(&depth) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored lineage depth is invalid".to_string(),
    ));
  }
  if stored.1.is_empty()
    || stored.1.len() > 128
    || stored.3.is_empty()
    || stored.3.len() > 128
    || !is_definition_hash(&stored.4)
    || !is_definition_hash(&stored.6)
  {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored workflow call identity fields are invalid".to_string(),
    ));
  }
  let parent_binding = load_run_binding_optional(connection, &stored.0)?.ok_or_else(|| {
    DurableStoreError::WorkflowCallHistoryInvalid(
      "stored workflow call parent run is missing".to_string(),
    )
  })?;
  let child_binding = load_run_binding_optional(connection, &stored.5)?.ok_or_else(|| {
    DurableStoreError::WorkflowCallHistoryInvalid(
      "stored workflow call child run is missing".to_string(),
    )
  })?;
  if parent_binding.run_id != stored.0
    || child_binding.run_id != stored.5
    || child_binding.workflow_id != stored.3
    || child_binding.definition_hash != stored.4
  {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored child run binding does not match its workflow call".to_string(),
    ));
  }
  let target_definition_workflow_id: Option<String> = connection
    .query_row(
      "SELECT workflow_id FROM woml_definitions WHERE definition_hash = ?1",
      [&stored.4],
      |row| row.get(0),
    )
    .optional()?;
  if target_definition_workflow_id.as_deref() != Some(stored.3.as_str()) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored target definition does not match its workflow call".to_string(),
    ));
  }
  let state = match stored.8.as_str() {
    "admitted" => WorkflowCallIndexState::Admitted,
    "running" => WorkflowCallIndexState::Running,
    "succeeded" => WorkflowCallIndexState::Succeeded,
    "failed" => WorkflowCallIndexState::Failed,
    _ => {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "stored call state is invalid".to_string(),
      ))
    }
  };
  let admitted_at = DateTime::parse_from_rfc3339(&stored.9)
    .map_err(|_| {
      DurableStoreError::WorkflowCallHistoryInvalid(
        "stored admission timestamp is invalid".to_string(),
      )
    })?
    .with_timezone(&Utc);
  Ok(Some(WorkflowCallAdmission {
    call_key: call_key.to_string(),
    parent_run_id: stored.0,
    parent_node_id: stored.1,
    parent_attempt,
    target_workflow_id: stored.3,
    target_definition_hash: stored.4,
    child_run_id: stored.5,
    payload_digest: stored.6,
    depth,
    state,
    admitted_at,
  }))
}

fn workflow_call_would_cycle(
  connection: &Connection,
  parent_run_id: &str,
  target_workflow_id: &str,
) -> Result<bool, DurableStoreError> {
  connection
    .query_row(
      "WITH RECURSIVE lineage(run_id, workflow_id) AS (
         SELECT run_id, workflow_id FROM woml_runs WHERE run_id = ?1
         UNION
         SELECT parent.parent_run_id, runs.workflow_id
         FROM lineage current
         JOIN woml_workflow_calls parent ON parent.child_run_id = current.run_id
         JOIN woml_runs runs ON runs.run_id = parent.parent_run_id
       )
       SELECT EXISTS(SELECT 1 FROM lineage WHERE workflow_id = ?2)",
      params![parent_run_id, target_workflow_id],
      |row| row.get(0),
    )
    .map_err(DurableStoreError::from)
}

fn workflow_call_state_name(state: WorkflowCallIndexState) -> &'static str {
  match state {
    WorkflowCallIndexState::Admitted => "admitted",
    WorkflowCallIndexState::Running => "running",
    WorkflowCallIndexState::Succeeded => "succeeded",
    WorkflowCallIndexState::Failed => "failed",
  }
}

fn validate_workflow_call_child(
  connection: &Connection,
  admission: &WorkflowCallAdmission,
  expected_trigger: &Map<String, Value>,
) -> Result<(), DurableStoreError> {
  let binding =
    load_run_binding_optional(connection, &admission.child_run_id)?.ok_or_else(|| {
      DurableStoreError::WorkflowCallHistoryInvalid("child run is missing".to_string())
    })?;
  if binding.workflow_id != admission.target_workflow_id
    || binding.definition_hash != admission.target_definition_hash
  {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "child run binding differs from its call index".to_string(),
    ));
  }
  let workflow = definition_by_hash(connection, &binding.definition_hash)?;
  let events = load_events(connection, &binding.run_id)?;
  let Some(first) = events.first() else {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "child run has no run_started event".to_string(),
    ));
  };
  let valid = match &first.payload {
    RunEventPayload::RunStarted(started) => {
      let valid_ingress = matches!(
        started.ingress.as_ref(),
        Some(RunIngress::WorkflowCall { call_key }) if call_key == &admission.call_key
      );
      matches!(
        first.event_schema_version,
        RUN_EVENT_SCHEMA_VERSION_V9 | crate::RUN_EVENT_SCHEMA_VERSION_V10
      ) && started.trigger == *expected_trigger
        && started.trigger_id.is_none()
        && started.trigger_handler.is_none()
        && started.trigger_occurrence_id.is_none()
        && valid_ingress
    }
    RunEventPayload::RunAdmitted(admitted) => {
      first.event_schema_version == run_event_schema_version_for_model(workflow.schema_version)
        && first.event_schema_version >= RUN_EVENT_SCHEMA_VERSION_V11
        && admitted.payload == *expected_trigger
        && admitted.trigger.id == "workflow-call"
        && admitted.trigger.handler == "trigger.event"
    }
    _ => false,
  };
  if !valid {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "child admission identity differs from its call index".to_string(),
    ));
  }
  validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
    .map_err(DurableStoreError::WorkflowCallHistoryInvalid)
}

impl DurableEventStore {
  /// Returns the durable lineage depth of a run. Top-level trigger runs have
  /// depth zero; an admitted workflow-call child stores its assigned depth.
  pub fn workflow_call_depth_for_parent(&self, run_id: &str) -> Result<u32, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let depth: Option<i64> = self
      .connection
      .query_row(
        "SELECT depth FROM woml_workflow_calls WHERE child_run_id = ?1",
        [run_id],
        |row| row.get(0),
      )
      .optional()?;
    match depth {
      None => Ok(0),
      Some(depth) if (1..=i64::from(MAX_WORKFLOW_CALL_DEPTH)).contains(&depth) => Ok(depth as u32),
      Some(_) => Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "stored lineage depth is invalid".to_string(),
      )),
    }
  }

  pub fn workflow_call(
    &self,
    call_key: &str,
  ) -> Result<Option<WorkflowCallAdmission>, DurableStoreError> {
    load_workflow_call(&self.connection, call_key)
  }

  pub fn workflow_call_for_child(
    &self,
    child_run_id: &str,
  ) -> Result<Option<WorkflowCallAdmission>, DurableStoreError> {
    let call_key: Option<String> = self
      .connection
      .query_row(
        "SELECT call_key FROM woml_workflow_calls WHERE child_run_id = ?1",
        [child_run_id],
        |row| row.get(0),
      )
      .optional()?;
    call_key
      .map(|call_key| load_workflow_call(&self.connection, &call_key))
      .transpose()
      .map(Option::flatten)
  }

  /// Returns a bounded, non-sensitive view of the parent/child relationships
  /// for one run. Payloads, digests, definition hashes, and call keys are
  /// deliberately excluded from this operator-facing surface.
  pub fn workflow_call_relations_for_run(
    &self,
    run_id: &str,
  ) -> Result<WorkflowCallRunRelations, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let parent_call = self
      .workflow_call_for_child(run_id)?
      .map(WorkflowCallRunSummary::from);
    let mut statement = self.connection.prepare(
      "SELECT call_key FROM woml_workflow_calls
       WHERE parent_run_id = ?1
       ORDER BY admitted_at, call_key
       LIMIT ?2",
    )?;
    let limit = i64::try_from(MAX_WORKFLOW_CALL_INSPECTION_CHILDREN + 1)
      .expect("workflow call inspection limit fits in SQLite integer");
    let call_keys = statement
      .query_map(params![run_id, limit], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let child_calls_truncated = call_keys.len() > MAX_WORKFLOW_CALL_INSPECTION_CHILDREN;
    let child_calls = call_keys
      .into_iter()
      .take(MAX_WORKFLOW_CALL_INSPECTION_CHILDREN)
      .map(|call_key| {
        load_workflow_call(&self.connection, &call_key)?
          .map(WorkflowCallRunSummary::from)
          .ok_or_else(|| {
            DurableStoreError::WorkflowCallHistoryInvalid(
              "workflow call disappeared during run inspection".to_string(),
            )
          })
      })
      .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkflowCallRunRelations {
      parent_call,
      child_calls,
      child_calls_truncated,
    })
  }

  pub fn admitted_workflow_calls_for_target(
    &self,
    target: &WorkflowTarget,
  ) -> Result<Vec<WorkflowCallAdmission>, DurableStoreError> {
    let mut statement = self.connection.prepare(
      "SELECT call_key FROM woml_workflow_calls
       WHERE target_workflow_id = ?1 AND target_definition_hash = ?2 AND state = 'admitted'
       ORDER BY admitted_at, call_key",
    )?;
    let call_keys = statement
      .query_map([&target.workflow_id, &target.definition_hash], |row| {
        row.get::<_, String>(0)
      })?
      .collect::<Result<Vec<_>, _>>()?;
    call_keys
      .into_iter()
      .map(|call_key| {
        load_workflow_call(&self.connection, &call_key)?.ok_or_else(|| {
          DurableStoreError::WorkflowCallHistoryInvalid(
            "pending workflow call disappeared during routing scan".to_string(),
          )
        })
      })
      .collect()
  }

  /// Advances only the mutable execution state of a durable workflow call.
  /// All identity-bearing columns remain protected by the schema trigger.
  pub fn transition_workflow_call_state(
    &mut self,
    call_key: &str,
    expected: WorkflowCallIndexState,
    next: WorkflowCallIndexState,
  ) -> Result<(), DurableStoreError> {
    let valid = matches!(
      (expected, next),
      (
        WorkflowCallIndexState::Admitted,
        WorkflowCallIndexState::Running
      ) | (
        WorkflowCallIndexState::Running,
        WorkflowCallIndexState::Succeeded
      ) | (
        WorkflowCallIndexState::Running,
        WorkflowCallIndexState::Failed
      )
    );
    if !valid {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "workflow call state transition is invalid".to_string(),
      ));
    }
    let changed = self.connection.execute(
      "UPDATE woml_workflow_calls SET state = ?1 WHERE call_key = ?2 AND state = ?3",
      params![
        workflow_call_state_name(next),
        call_key,
        workflow_call_state_name(expected),
      ],
    )?;
    if changed != 1 {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "workflow call state changed unexpectedly".to_string(),
      ));
    }
    Ok(())
  }

  /// Claims an admitted child for execution. Concurrent callers race through
  /// one conditional update, so exactly one caller becomes the executor and
  /// every other caller observes the same child run.
  pub fn claim_workflow_call_execution(
    &mut self,
    call_key: &str,
  ) -> Result<bool, DurableStoreError> {
    let changed = self.connection.execute(
      "UPDATE woml_workflow_calls SET state = 'running'
       WHERE call_key = ?1 AND state = 'admitted'",
      [call_key],
    )?;
    if changed > 1 {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "more than one workflow call row was claimed".to_string(),
      ));
    }
    Ok(changed == 1)
  }

  /// Atomically binds one stable parent call identity to exactly one child run.
  /// Repeating the same request returns the existing child; changing any
  /// identity-bearing input fails closed.
  pub fn admit_workflow_call(
    &mut self,
    request: WorkflowCallAdmissionRequest,
  ) -> Result<WorkflowCallAdmissionOutcome, DurableStoreError> {
    validate_workflow_call_admission_request(&request)?;
    let payload_digest = durable_workflow_call_payload_digest(&request.payload)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, &request.parent_run_id)?;
    let parent_workflow = definition_for_run(&transaction, &request.parent_run_id)?;
    if parent_workflow.node(&request.parent_node_id).is_none() {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "parent node is not present in its bound definition".to_string(),
      ));
    }
    let parent_events = load_events(&transaction, &request.parent_run_id)?;
    let parent_projection = fold_events(&parent_events)?;
    let active_parent_attempt = parent_projection
      .latest_attempt(&request.parent_node_id)
      .is_some_and(|attempt| {
        attempt.identity.attempt == request.parent_attempt
          && attempt.status == AttemptStatus::Started
      });
    if !active_parent_attempt {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "parent step attempt is not active".to_string(),
      ));
    }

    if workflow_call_would_cycle(
      &transaction,
      &request.parent_run_id,
      &request.target_workflow_id,
    )? {
      return Err(DurableStoreError::WorkflowCallCycle);
    }

    let workflow = definition_by_hash(&transaction, &request.target_definition_hash)?;
    if workflow.workflow_id != request.target_workflow_id {
      return Err(DurableStoreError::WorkflowCallDefinitionMismatch);
    }

    let parent_depth: Option<i64> = transaction
      .query_row(
        "SELECT depth FROM woml_workflow_calls WHERE child_run_id = ?1",
        [&request.parent_run_id],
        |row| row.get(0),
      )
      .optional()?;
    let parent_depth = parent_depth.unwrap_or(0);
    let depth = parent_depth
      .checked_add(1)
      .ok_or(DurableStoreError::WorkflowCallDepthExceeded)?;
    if !(1..=i64::from(MAX_WORKFLOW_CALL_DEPTH)).contains(&depth) {
      return Err(DurableStoreError::WorkflowCallDepthExceeded);
    }

    if let Some(existing) = load_workflow_call(&transaction, &request.call_key)? {
      let identical = existing.parent_run_id == request.parent_run_id
        && existing.parent_node_id == request.parent_node_id
        && existing.target_workflow_id == request.target_workflow_id
        && existing.target_definition_hash == request.target_definition_hash
        && existing.child_run_id == request.child_run_id
        && existing.payload_digest == payload_digest
        && existing.depth == depth as u32;
      if !identical {
        return Err(DurableStoreError::WorkflowCallIdempotencyConflict);
      }
      validate_workflow_call_child(&transaction, &existing, &request.payload)?;
      transaction.commit()?;
      return Ok(WorkflowCallAdmissionOutcome {
        admission: existing,
        duplicate: true,
      });
    }

    if load_run_binding_optional(&transaction, &request.child_run_id)?.is_some() {
      return Err(DurableStoreError::WorkflowCallIdempotencyConflict);
    }

    if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      let queued: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM woml_runtime_policy_queue",
        [],
        |row| row.get(0),
      )?;
      if queued >= RUNTIME_POLICY_QUEUE_CEILING {
        return Err(DurableStoreError::RuntimePolicyQueueFull);
      }
    }

    transaction.execute(
      "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
       VALUES (?1, ?2, ?3, ?4)",
      params![
        request.child_run_id,
        request.target_workflow_id,
        request.target_definition_hash,
        request.admitted_at.to_rfc3339(),
      ],
    )?;
    let payload = if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      let policy_hash = workflow.runtime_policy_hash().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy identity.".to_string())
      })?;
      let queue_name = workflow.runtime_policy_queue_name().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy queue.".to_string())
      })?;
      let conflict: bool = transaction.query_row(
        "SELECT EXISTS(
         SELECT 1
         FROM woml_runtime_policy_bindings AS bindings
         JOIN woml_run_summaries AS summaries ON summaries.run_id = bindings.run_id
         WHERE bindings.workflow_id = ?1
           AND summaries.status IN ('queued', 'running', 'waiting', 'cancelling', 'finalizing')
           AND (bindings.definition_hash != ?2 OR bindings.policy_hash != ?3)
       )",
        params![
          request.target_workflow_id,
          request.target_definition_hash,
          policy_hash
        ],
        |row| row.get(0),
      )?;
      if conflict {
        return Err(DurableStoreError::RuntimePolicyDefinitionConflict(
          request.target_workflow_id.clone(),
        ));
      }
      RunEventPayload::RunAdmitted(RunAdmittedData {
        definition_hash: request.target_definition_hash.clone(),
        policy_hash,
        trigger: RunAdmissionTrigger {
          id: "workflow-call".to_string(),
          handler: "trigger.event".to_string(),
        },
        payload: request.payload.clone(),
        queue: RunAdmissionQueue {
          name: queue_name,
          discipline: crate::model::QueueDiscipline::WorkConservingFifo,
        },
        admitted_at: request.admitted_at,
        occurrence_sequence: 0,
      })
    } else {
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: request.target_workflow_id.clone(),
        definition_hash: request.target_definition_hash.clone(),
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        ingress: Some(RunIngress::WorkflowCall {
          call_key: request.call_key.clone(),
        }),
        trigger: request.payload.clone(),
      })
    };
    validate_payload_against_definition(&workflow, &request.target_definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let mut events = Vec::new();
    append_to_history(
      &transaction,
      &mut events,
      &request.child_run_id,
      generated_event_id(),
      request.admitted_at,
      run_event_schema_version_for_model(workflow.schema_version),
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &request.target_definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;

    transaction.execute(
      "INSERT INTO woml_workflow_calls(
         call_key, parent_run_id, parent_node_id, parent_attempt,
         target_workflow_id, target_definition_hash, child_run_id,
         payload_digest, depth, state, admitted_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'admitted', ?10)",
      params![
        request.call_key,
        request.parent_run_id,
        request.parent_node_id,
        i64::from(request.parent_attempt),
        request.target_workflow_id,
        request.target_definition_hash,
        request.child_run_id,
        payload_digest,
        depth,
        request.admitted_at.to_rfc3339(),
      ],
    )?;
    let admission = load_workflow_call(&transaction, &request.call_key)?.ok_or_else(|| {
      DurableStoreError::WorkflowCallHistoryInvalid(
        "new admission disappeared before commit".to_string(),
      )
    })?;
    transaction.commit()?;
    Ok(WorkflowCallAdmissionOutcome {
      admission,
      duplicate: false,
    })
  }
}
