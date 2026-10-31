//! Narrow immutable state-mutation settlement proof validation. A matching receipt proves only the managed mutation, not the interrupted script or unrelated effects.

use crate::durable::DurableStoreError;
use crate::projection::OperationProjection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use serde_json::Map;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;

pub(in crate::durable) struct RecoveredStateMutationSettlement {
  pub(in crate::durable) metadata: Map<String, Value>,
  pub(in crate::durable) result_bytes: u64,
  pub(in crate::durable) result_digest: String,
}

/// A state mutation and its immutable result are committed together. If the
/// process dies before appending operation_succeeded, recovery may use that
/// settlement proof instead of incorrectly classifying the mutation itself as
/// ambiguous. The script attempt still fails closed and is never replayed.
pub(in crate::durable) fn recovered_state_mutation_settlement(
  transaction: &Transaction<'_>,
  operation: &OperationProjection,
) -> Result<Option<RecoveredStateMutationSettlement>, DurableStoreError> {
  if operation.capability != "state"
    || operation.execution_mode != crate::event::OperationExecutionMode::Managed
    || !matches!(
      operation.operation.as_str(),
      "set" | "delete" | "increment" | "set_if_absent"
    )
  {
    return Ok(None);
  }
  let stored = transaction
    .query_row(
      "SELECT operation, key_digest, input_digest, result_json, result_digest,
              committed_version
       FROM woml_state_mutations WHERE operation_key = ?1",
      [&operation.operation_key],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, Option<u64>>(5)?,
        ))
      },
    )
    .optional()?;
  let Some((stored_operation, key_digest, input_digest, result_json, result_digest, version)) =
    stored
  else {
    return Ok(None);
  };
  let expected_key_digest = operation.metadata.get("keyDigest").and_then(Value::as_str);
  let expected_input_digest = operation
    .metadata
    .get("inputDigest")
    .and_then(Value::as_str);
  let result: Value = serde_json::from_str(&result_json).map_err(|_| {
    DurableStoreError::Contract("A settled state mutation contains invalid JSON.".to_string())
  })?;
  let canonical = canonical_json(&result)?;
  let stored_result_digest = format!(
    "sha256:{}",
    hex::encode({
      let mut hasher = Sha256::new();
      hasher.update(b"woml.state-result\0v1\0");
      hasher.update(&canonical);
      hasher.finalize()
    })
  );
  if stored_operation != operation.operation
    || expected_key_digest != Some(key_digest.as_str())
    || expected_input_digest != Some(input_digest.as_str())
    || result_digest != stored_result_digest
    || result.get("contract") != Some(&Value::String("woml.state".to_string()))
    || result.get("contractVersion") != Some(&Value::from(1))
    || result.get("kind") != Some(&Value::String("result".to_string()))
    || result.get("operation") != Some(&Value::String(operation.operation.clone()))
    || !crate::durable_state::valid_mutation_result(&operation.operation, &result, version)
  {
    return Err(DurableStoreError::Contract(
      "A settled state mutation does not match its interrupted managed operation.".to_string(),
    ));
  }
  let encoded = serde_json::to_vec(&result)?;
  let mut metadata = operation.metadata.clone();
  metadata.insert(
    "outcome".to_string(),
    Value::String("succeeded".to_string()),
  );
  metadata.insert("resultDigest".to_string(), Value::String(result_digest));
  metadata.insert("durationMs".to_string(), Value::from(0.0));
  if let Some(version) = version {
    metadata.insert("version".to_string(), Value::from(version));
  }
  Ok(Some(RecoveredStateMutationSettlement {
    metadata,
    result_bytes: encoded.len() as u64,
    result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&encoded))),
  }))
}
