//! Ordered historical migrations over the startup-owned connection.
//!
//! Each original immediate transaction groups its DDL and version-marker update.
//! Failed migration work must not publish a newer version or partial layout.

use crate::durable::{
  rebuild_correction_indexes, rebuild_run_summary_index, rebuild_runtime_policy_indexes,
  validate_store_v13_schema, validate_store_v14_schema, validate_store_v15_schema,
  validate_store_v16_schema, validate_store_v17_schema, validate_store_v18_schema,
  validate_store_v19_schema, DurableStoreError, CREATE_APPROVAL_SCHEMA_V2,
  CREATE_CORRECTION_SCHEMA_V17, CREATE_DURABLE_STATE_SCHEMA_V13, CREATE_DURABLE_WAIT_SCHEMA_V15,
  CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19, CREATE_INTERNAL_EVENT_SCHEMA_V7,
  CREATE_INTERVAL_SCHEMA_V6, CREATE_LOCK_SCHEMA_V16, CREATE_MODULE_ARTIFACT_SCHEMA_V8,
  CREATE_NOTIFICATION_SCHEMA_V3, CREATE_PRODUCTION_RUNTIME_SCHEMA_V14,
  CREATE_RICH_APPROVAL_SCHEMA_V18, CREATE_RUN_SUMMARY_SCHEMA_V11, CREATE_SCHEDULE_SCHEMA_V5,
  CREATE_TRIGGER_SCHEMA_V4, CREATE_WORKFLOW_CALL_SCHEMA_V9, CREATE_WORKFLOW_RUNTIME_SCHEMA_V10,
  MIGRATE_RUNTIME_POLICY_SCHEMA_V12, STORE_SCHEMA_VERSION_V1, STORE_SCHEMA_VERSION_V10,
  STORE_SCHEMA_VERSION_V11, STORE_SCHEMA_VERSION_V12, STORE_SCHEMA_VERSION_V13,
  STORE_SCHEMA_VERSION_V14, STORE_SCHEMA_VERSION_V15, STORE_SCHEMA_VERSION_V16,
  STORE_SCHEMA_VERSION_V17, STORE_SCHEMA_VERSION_V18, STORE_SCHEMA_VERSION_V19,
  STORE_SCHEMA_VERSION_V2, STORE_SCHEMA_VERSION_V20, STORE_SCHEMA_VERSION_V3,
  STORE_SCHEMA_VERSION_V4, STORE_SCHEMA_VERSION_V5, STORE_SCHEMA_VERSION_V6,
  STORE_SCHEMA_VERSION_V7, STORE_SCHEMA_VERSION_V8, STORE_SCHEMA_VERSION_V9,
};
use crate::remote_dispatch::validate_remote_dispatch_schema;
use crate::remote_dispatch::CREATE_REMOTE_DISPATCH_SCHEMA_V20;
use rusqlite::Connection;
use rusqlite::TransactionBehavior;

pub(in crate::durable) fn migrate_store_v1_to_v2(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_APPROVAL_SCHEMA_V2)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V2, STORE_SCHEMA_VERSION_V1],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v1-to-v2 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v2_to_v3(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_NOTIFICATION_SCHEMA_V3)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V3, STORE_SCHEMA_VERSION_V2],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v2-to-v3 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v3_to_v4(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_TRIGGER_SCHEMA_V4)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V4, STORE_SCHEMA_VERSION_V3],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v3-to-v4 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v4_to_v5(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_SCHEDULE_SCHEMA_V5)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V5, STORE_SCHEMA_VERSION_V4],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v4-to-v5 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v5_to_v6(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INTERVAL_SCHEMA_V6)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V6, STORE_SCHEMA_VERSION_V5],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v5-to-v6 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v6_to_v7(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INTERNAL_EVENT_SCHEMA_V7)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V7, STORE_SCHEMA_VERSION_V6],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v6-to-v7 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v7_to_v8(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_MODULE_ARTIFACT_SCHEMA_V8)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V8, STORE_SCHEMA_VERSION_V7],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v7-to-v8 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v8_to_v9(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_WORKFLOW_CALL_SCHEMA_V9)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V9, STORE_SCHEMA_VERSION_V8],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v8-to-v9 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v9_to_v10(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_WORKFLOW_RUNTIME_SCHEMA_V10)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V10, STORE_SCHEMA_VERSION_V9],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v9-to-v10 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v10_to_v11(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_RUN_SUMMARY_SCHEMA_V11)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V11, STORE_SCHEMA_VERSION_V10],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v10-to-v11 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v11_to_v12(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(MIGRATE_RUNTIME_POLICY_SCHEMA_V12)?;
  rebuild_run_summary_index(&transaction)?;
  rebuild_runtime_policy_indexes(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V12, STORE_SCHEMA_VERSION_V11],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v11-to-v12 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v12_to_v13(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_DURABLE_STATE_SCHEMA_V13)?;
  validate_store_v13_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V13, STORE_SCHEMA_VERSION_V12],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v12-to-v13 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v13_to_v14(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_PRODUCTION_RUNTIME_SCHEMA_V14)?;
  validate_store_v14_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V14, STORE_SCHEMA_VERSION_V13],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v13-to-v14 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v14_to_v15(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_DURABLE_WAIT_SCHEMA_V15)?;
  validate_store_v15_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V15, STORE_SCHEMA_VERSION_V14],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v14-to-v15 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v15_to_v16(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_LOCK_SCHEMA_V16)?;
  validate_store_v16_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V16, STORE_SCHEMA_VERSION_V15],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v15-to-v16 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v16_to_v17(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_CORRECTION_SCHEMA_V17)?;
  validate_store_v17_schema(&transaction)?;
  rebuild_correction_indexes(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V17, STORE_SCHEMA_VERSION_V16],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v16-to-v17 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v17_to_v18(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_RICH_APPROVAL_SCHEMA_V18)?;
  validate_store_v18_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V18, STORE_SCHEMA_VERSION_V17],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v17-to-v18 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v18_to_v19(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19)?;
  validate_store_v19_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V19, STORE_SCHEMA_VERSION_V18],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v18-to-v19 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

pub(in crate::durable) fn migrate_store_v19_to_v20(
  connection: &mut Connection,
) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_REMOTE_DISPATCH_SCHEMA_V20)?;
  validate_remote_dispatch_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V20, STORE_SCHEMA_VERSION_V19],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v19-to-v20 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}
