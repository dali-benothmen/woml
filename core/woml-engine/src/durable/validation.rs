//! Historical/current layout validation before startup accepts a durable store.
//!
//! Required objects and column contracts retain their existing validation order
//! and first error. The prepared request-opening path does not call this layer.

use crate::durable::DurableStoreError;
use rusqlite::Connection;

pub(in crate::durable) fn validate_store_v2_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  for (object_type, name) in [
    ("table", "woml_approval_tokens"),
    ("index", "woml_approval_tokens_request"),
    ("trigger", "woml_approval_tokens_no_update"),
    ("trigger", "woml_approval_tokens_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2
       )",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v2 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v3_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v2_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_notification_capabilities"),
    ("index", "woml_notification_capabilities_delivery"),
    ("trigger", "woml_notification_capabilities_no_update"),
    ("trigger", "woml_notification_capabilities_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v3 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v4_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v3_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_trigger_occurrences"),
    ("index", "woml_trigger_occurrences_run"),
    ("trigger", "woml_trigger_occurrences_no_update"),
    ("trigger", "woml_trigger_occurrences_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v4 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v5_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v4_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_schedule_cursors"),
    ("index", "woml_schedule_cursors_definition"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v5 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v6_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v5_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_interval_cursors"),
    ("index", "woml_interval_cursors_definition"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v6 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v7_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v6_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_internal_event_publications"),
    ("table", "woml_internal_event_deliveries"),
    ("index", "woml_internal_event_deliveries_run"),
    ("trigger", "woml_internal_event_publications_no_update"),
    ("trigger", "woml_internal_event_publications_no_delete"),
    ("trigger", "woml_internal_event_deliveries_no_update"),
    ("trigger", "woml_internal_event_deliveries_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v7 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v8_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v7_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_definition_module_artifacts"),
    ("index", "woml_definition_module_artifacts_bundle"),
    ("trigger", "woml_definition_module_artifacts_no_update"),
    ("trigger", "woml_definition_module_artifacts_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v8 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v9_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v8_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_workflow_calls"),
    ("index", "woml_workflow_calls_parent"),
    ("index", "woml_workflow_calls_target"),
    ("trigger", "woml_workflow_calls_identity_no_update"),
    ("trigger", "woml_workflow_calls_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v9 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v10_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v9_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_workflow_runtime_routes"),
    ("index", "woml_workflow_runtime_routes_runtime"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v10 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v11_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v10_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_run_summaries"),
    ("index", "woml_run_summaries_updated"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v11 is missing required {object_type} {name:?}."
      )));
    }
  }

  let mut statement = connection.prepare("PRAGMA table_info(woml_run_summaries)")?;
  let actual_columns = statement
    .query_map([], |row| row.get::<_, String>(1))?
    .collect::<Result<Vec<_>, _>>()?;
  let expected_columns = [
    "run_id",
    "workflow_id",
    "status",
    "started_at",
    "updated_at",
  ];
  if actual_columns != expected_columns {
    return Err(DurableStoreError::Contract(format!(
      "Store v11 woml_run_summaries columns do not match the frozen schema: expected {expected_columns:?}, found {actual_columns:?}."
    )));
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v12_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v10_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_run_summaries"),
    ("index", "woml_run_summaries_updated"),
    ("table", "woml_runtime_policy_bindings"),
    ("index", "woml_runtime_policy_bindings_workflow"),
    ("table", "woml_runtime_policy_queue"),
    ("index", "woml_runtime_policy_queue_order"),
    ("table", "woml_runtime_policy_starts"),
    ("index", "woml_runtime_policy_starts_window"),
    ("table", "woml_scheduler_claims"),
    ("index", "woml_scheduler_claims_expiry"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v12 is missing required {object_type} {name:?}."
      )));
    }
  }
  let mut statement = connection.prepare("PRAGMA table_info(woml_run_summaries)")?;
  let actual_columns = statement
    .query_map([], |row| row.get::<_, String>(1))?
    .collect::<Result<Vec<_>, _>>()?;
  let expected_columns = [
    "run_id",
    "workflow_id",
    "status",
    "admitted_at",
    "started_at",
    "updated_at",
    "queue_name",
    "waiting_for",
    "eligible_at",
  ];
  if actual_columns != expected_columns {
    return Err(DurableStoreError::Contract(format!(
      "Store v12 woml_run_summaries columns do not match the frozen schema: expected {expected_columns:?}, found {actual_columns:?}."
    )));
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v13_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v12_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_state_entries"),
    ("index", "woml_state_entries_scope"),
    ("table", "woml_state_mutations"),
    ("index", "woml_state_mutations_scope_key"),
    ("trigger", "woml_state_mutations_no_update"),
    ("trigger", "woml_state_mutations_no_delete"),
    ("table", "woml_state_quotas"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v13 is missing required {object_type} {name:?}."
      )));
    }
  }
  for (table, expected_columns) in [
    (
      "woml_state_entries",
      vec![
        "scope_digest",
        "key_digest",
        "key_text",
        "value_json",
        "value_bytes",
        "version",
        "updated_at",
      ],
    ),
    (
      "woml_state_mutations",
      vec![
        "operation_key",
        "scope_digest",
        "operation_name",
        "operation",
        "key_digest",
        "input_digest",
        "result_json",
        "result_digest",
        "committed_version",
        "committed_at",
      ],
    ),
    (
      "woml_state_quotas",
      vec!["scope_digest", "live_keys", "value_bytes"],
    ),
  ] {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let actual_columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if actual_columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v13 {table} columns do not match the frozen schema: expected {expected_columns:?}, found {actual_columns:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v14_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v13_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_runtime_owner",
      &[
        "singleton",
        "deployment_id",
        "activation_id",
        "runtime_instance_id",
        "heartbeat_at",
        "lease_expires_at",
      ][..],
    ),
    (
      "woml_maintenance_lease",
      &[
        "singleton",
        "lease_id",
        "operation",
        "owner_id",
        "expires_at",
      ][..],
    ),
    (
      "woml_last_verified_backup",
      &["singleton", "backup_id", "completed_at", "verified"][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v14 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v14 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v15_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v14_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_durable_waits",
      &[
        "run_id",
        "wait_id",
        "iteration_identity",
        "node_id",
        "kind",
        "registered_at",
        "deadline",
        "signals_json",
        "correlation_digest",
        "state",
        "resolution_event_id",
        "resolved_at",
      ][..],
    ),
    (
      "woml_external_signals",
      &[
        "signal_id",
        "run_id",
        "wait_id",
        "iteration_identity",
        "signal",
        "received_at",
        "idempotency_digest",
        "request_digest",
        "correlation_digest",
        "payload_json",
        "authentication_class",
        "consumed_at",
      ][..],
    ),
    (
      "woml_signal_idempotency",
      &[
        "idempotency_digest",
        "request_digest",
        "signal_id",
        "created_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v15 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v15 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for name in [
    "woml_durable_waits_due",
    "woml_durable_waits_active_target",
    "woml_external_signals_target",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v15 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v16_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v15_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_lock_resources",
      &[
        "key",
        "fencing_high_water",
        "owner_lease_id",
        "owner_run_id",
        "owner_node_execution_id",
        "owner_fencing_token",
        "acquired_at",
        "expires_at",
        "updated_at",
      ][..],
    ),
    (
      "woml_lock_waiters",
      &[
        "waiter_id",
        "key",
        "run_id",
        "node_execution_id",
        "sequence",
        "registered_at",
        "deadline",
        "status",
        "input_digest",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v16 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!("Store v16 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}.")));
    }
  }
  for name in [
    "woml_lock_resources_one_owner",
    "woml_lock_waiters_fair_queue",
    "woml_lock_waiters_recovery",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v16 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v17_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v16_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_saga_scopes",
      &[
        "run_id",
        "saga_id",
        "phase",
        "original_outcome",
        "started_sequence",
        "committed_sequence",
        "updated_at",
        "model_digest",
      ][..],
    ),
    (
      "woml_correction_actions",
      &[
        "run_id",
        "saga_id",
        "action_id",
        "action_kind",
        "for_step_execution_id",
        "completion_sequence",
        "status",
        "attempts",
        "next_attempt_at",
        "updated_at",
      ][..],
    ),
    (
      "woml_correction_attempts",
      &[
        "run_id",
        "saga_id",
        "action_id",
        "attempt",
        "invocation_id",
        "status",
        "started_at",
        "finished_at",
        "failure_code",
      ][..],
    ),
    (
      "woml_correction_manual_recovery",
      &[
        "run_id",
        "saga_id",
        "action_id",
        "status",
        "reason_code",
        "created_at",
        "resolved_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v17 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v17 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for name in [
    "woml_correction_actions_pending",
    "woml_correction_attempts_active",
    "woml_correction_manual_recovery_pending",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v17 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v18_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v17_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_approval_reviews",
      &[
        "run_id",
        "approval_id",
        "request_id",
        "definition_hash",
        "workflow_version",
        "review_json",
        "review_digest",
        "required_approvals",
        "required_roles_json",
        "directory_version",
        "status",
        "created_at",
        "resolved_at",
      ][..],
    ),
    (
      "woml_approval_reviewer_decisions",
      &[
        "run_id",
        "request_id",
        "principal_id",
        "decision",
        "provider",
        "provider_account_id",
        "provider_actor_id",
        "roles_json",
        "decided_at",
      ][..],
    ),
    (
      "woml_approval_artifact_pins",
      &[
        "run_id",
        "request_id",
        "artifact_id",
        "artifact_sha256",
        "released_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for name in [
    "woml_approval_reviews_pending",
    "woml_approval_decisions_request",
    "woml_approval_pins_live",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 is missing required index {name:?}."
      )));
    }
  }
  for name in [
    "woml_approval_reviews_no_delete",
    "woml_approval_reviews_immutable",
    "woml_approval_decisions_no_update",
    "woml_approval_decisions_no_delete",
    "woml_approval_pins_immutable",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 is missing required trigger {name:?}."
      )));
    }
  }
  Ok(())
}

pub(in crate::durable) fn validate_store_v19_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  validate_store_v18_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_infrastructure_policy_bundles",
      &[
        "policy_digest",
        "policy_id",
        "policy_version",
        "environment",
        "canonical_payload",
        "signature_status",
        "signing_key_id",
        "created_at",
      ][..],
    ),
    (
      "woml_run_infrastructure_policies",
      &["run_id", "policy_digest", "admitted_at"][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v19 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v19 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for (object_type, name) in [
    ("index", "woml_run_infrastructure_policies_digest"),
    ("trigger", "woml_infrastructure_policy_bundles_no_update"),
    ("trigger", "woml_infrastructure_policy_bundles_no_delete"),
    ("trigger", "woml_run_infrastructure_policies_no_update"),
    ("trigger", "woml_run_infrastructure_policies_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v19 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}
