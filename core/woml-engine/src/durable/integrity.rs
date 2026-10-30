//! Explicit store integrity audit over the existing connection and admitted policy evidence, without schema repair or workflow execution.

use crate::durable::validate_store_v19_schema;
use crate::durable::DurableEventStore;
use crate::durable::DurableStoreError;
use crate::remote_dispatch::validate_remote_dispatch_schema;

impl DurableEventStore {
  pub fn audit_integrity(&self) -> Result<(), DurableStoreError> {
    let result: String = self
      .connection
      .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if result != "ok" {
      return Err(DurableStoreError::Contract(format!(
        "The durable WOML store failed its integrity audit: {result}."
      )));
    }
    validate_store_v19_schema(&self.connection)?;
    validate_remote_dispatch_schema(&self.connection)?;
    let mut statement = self
      .connection
      .prepare("SELECT run_id FROM woml_run_infrastructure_policies ORDER BY run_id")?;
    let run_ids = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for run_id in run_ids {
      self
        .infrastructure_policy_for_run(&run_id)?
        .ok_or_else(|| {
          DurableStoreError::Contract(
            "WOML_POLICY_UNAVAILABLE: an admitted run lost its canonical policy bundle."
              .to_string(),
          )
        })?;
    }
    Ok(())
  }
}
