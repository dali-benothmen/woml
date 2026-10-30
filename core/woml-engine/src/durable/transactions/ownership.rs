//! Exact runtime ownership and leased routing. Identity checks, expiry, and handoff remain in their original transactions.

use crate::durable::DurableEventStore;
use crate::durable::DurableStoreError;
use crate::durable::RuntimeOwnerLease;
use crate::event::is_definition_hash;
use crate::workflow_calls::WorkflowRuntimeRoute;
use crate::workflow_calls::WorkflowTarget;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;

fn validate_runtime_route_fields(
  runtime_id: &str,
  targets: &[WorkflowTarget],
  endpoint: &str,
  session_credential_hash: &str,
  now: DateTime<Utc>,
  lease_expires_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  let port = endpoint
    .strip_prefix("http://127.0.0.1:")
    .and_then(|value| value.parse::<u16>().ok())
    .filter(|port| *port > 0);
  if runtime_id.is_empty()
    || runtime_id.len() > 256
    || targets.is_empty()
    || port.is_none()
    || !is_definition_hash(session_credential_hash)
    || lease_expires_at <= now
    || targets.iter().any(|target| {
      target.runtime_id != runtime_id
        || target.workflow_id.is_empty()
        || !is_definition_hash(&target.definition_hash)
    })
  {
    return Err(DurableStoreError::Contract(
      "Workflow runtime route registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

impl DurableEventStore {
  pub fn register_workflow_runtime_routes(
    &mut self,
    runtime_id: &str,
    targets: &[WorkflowTarget],
    endpoint: &str,
    session_credential_hash: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<(), DurableStoreError> {
    validate_runtime_route_fields(
      runtime_id,
      targets,
      endpoint,
      session_credential_hash,
      now,
      lease_expires_at,
    )?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    for target in targets {
      let existing: Option<(String, String)> = transaction
        .query_row(
          "SELECT runtime_id, lease_expires_at FROM woml_workflow_runtime_routes
           WHERE workflow_id = ?1",
          [&target.workflow_id],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
      if let Some((owner, expires_at)) = existing {
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
          .map_err(|_| {
            DurableStoreError::Contract(
              "A stored workflow runtime lease has an invalid timestamp.".to_string(),
            )
          })?
          .with_timezone(&Utc);
        if owner != runtime_id && expires_at > now {
          return Err(DurableStoreError::WorkflowRuntimeDuplicateOwner(
            target.workflow_id.clone(),
          ));
        }
      }
    }
    for target in targets {
      transaction.execute(
        "INSERT INTO woml_workflow_runtime_routes(
           workflow_id, definition_hash, runtime_id, endpoint,
           session_credential_hash, lease_expires_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(workflow_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           runtime_id = excluded.runtime_id,
           endpoint = excluded.endpoint,
           session_credential_hash = excluded.session_credential_hash,
           lease_expires_at = excluded.lease_expires_at,
           updated_at = excluded.updated_at",
        params![
          target.workflow_id,
          target.definition_hash,
          runtime_id,
          endpoint,
          session_credential_hash,
          lease_expires_at.to_rfc3339(),
          now.to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    Ok(())
  }

  pub fn runtime_owner(&self) -> Result<Option<RuntimeOwnerLease>, DurableStoreError> {
    let stored: Option<(String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT deployment_id, activation_id, runtime_instance_id, heartbeat_at, lease_expires_at
         FROM woml_runtime_owner WHERE singleton = 1",
        [],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    stored
      .map(
        |(deployment_id, activation_id, runtime_instance_id, heartbeat, expires)| {
          let heartbeat_at = DateTime::parse_from_rfc3339(&heartbeat)
            .map_err(|_| {
              DurableStoreError::Contract("Stored runtime heartbeat is invalid.".to_string())
            })?
            .with_timezone(&Utc);
          let lease_expires_at = DateTime::parse_from_rfc3339(&expires)
            .map_err(|_| {
              DurableStoreError::Contract("Stored runtime lease expiry is invalid.".to_string())
            })?
            .with_timezone(&Utc);
          Ok(RuntimeOwnerLease {
            deployment_id,
            activation_id,
            runtime_instance_id,
            heartbeat_at,
            lease_expires_at,
          })
        },
      )
      .transpose()
  }

  pub fn acquire_runtime_owner(
    &mut self,
    deployment_id: &str,
    activation_id: &str,
    runtime_instance_id: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<(), DurableStoreError> {
    if deployment_id.is_empty()
      || deployment_id.len() > 320
      || runtime_instance_id.is_empty()
      || runtime_instance_id.len() > 320
      || !is_definition_hash(activation_id)
      || lease_expires_at <= now
    {
      return Err(DurableStoreError::Contract(
        "Runtime owner lease fields are invalid.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<(String, String, String)> = transaction
      .query_row(
        "SELECT deployment_id, runtime_instance_id, lease_expires_at
         FROM woml_runtime_owner WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .optional()?;
    if let Some((owner_deployment, owner_runtime, expires)) = existing {
      let expires = DateTime::parse_from_rfc3339(&expires)
        .map_err(|_| {
          DurableStoreError::Contract("Stored runtime lease expiry is invalid.".to_string())
        })?
        .with_timezone(&Utc);
      if owner_runtime != runtime_instance_id && expires > now {
        return Err(DurableStoreError::DeploymentRuntimeOwned {
          deployment_id: owner_deployment,
          runtime_instance_id: owner_runtime,
        });
      }
    }
    transaction.execute(
      "INSERT INTO woml_runtime_owner(
         singleton, deployment_id, activation_id, runtime_instance_id, heartbeat_at, lease_expires_at
       ) VALUES (1, ?1, ?2, ?3, ?4, ?5)
       ON CONFLICT(singleton) DO UPDATE SET
         deployment_id = excluded.deployment_id,
         activation_id = excluded.activation_id,
         runtime_instance_id = excluded.runtime_instance_id,
         heartbeat_at = excluded.heartbeat_at,
         lease_expires_at = excluded.lease_expires_at",
      params![
        deployment_id,
        activation_id,
        runtime_instance_id,
        now.to_rfc3339(),
        lease_expires_at.to_rfc3339(),
      ],
    )?;
    transaction.commit()?;
    Ok(())
  }

  pub fn renew_runtime_owner(
    &mut self,
    runtime_instance_id: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    if runtime_instance_id.is_empty() || lease_expires_at <= now {
      return Err(DurableStoreError::Contract(
        "Runtime owner renewal is invalid.".to_string(),
      ));
    }
    Ok(
      self.connection.execute(
        "UPDATE woml_runtime_owner SET heartbeat_at = ?1, lease_expires_at = ?2
       WHERE singleton = 1 AND runtime_instance_id = ?3",
        params![
          now.to_rfc3339(),
          lease_expires_at.to_rfc3339(),
          runtime_instance_id
        ],
      )? == 1,
    )
  }

  pub fn release_runtime_owner(
    &mut self,
    runtime_instance_id: &str,
  ) -> Result<bool, DurableStoreError> {
    Ok(
      self.connection.execute(
        "DELETE FROM woml_runtime_owner WHERE singleton = 1 AND runtime_instance_id = ?1",
        [runtime_instance_id],
      )? == 1,
    )
  }

  pub fn renew_workflow_runtime_routes(
    &mut self,
    runtime_id: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<usize, DurableStoreError> {
    if runtime_id.is_empty() || lease_expires_at <= now {
      return Err(DurableStoreError::Contract(
        "Workflow runtime lease renewal is invalid.".to_string(),
      ));
    }
    self
      .connection
      .execute(
        "UPDATE woml_workflow_runtime_routes
         SET lease_expires_at = ?1, updated_at = ?2 WHERE runtime_id = ?3",
        params![lease_expires_at.to_rfc3339(), now.to_rfc3339(), runtime_id],
      )
      .map_err(DurableStoreError::from)
  }

  pub fn unregister_workflow_runtime_routes(
    &mut self,
    runtime_id: &str,
  ) -> Result<usize, DurableStoreError> {
    self
      .connection
      .execute(
        "DELETE FROM woml_workflow_runtime_routes WHERE runtime_id = ?1",
        [runtime_id],
      )
      .map_err(DurableStoreError::from)
  }

  pub fn workflow_runtime_route(
    &self,
    workflow_id: &str,
    now: DateTime<Utc>,
  ) -> Result<Option<WorkflowRuntimeRoute>, DurableStoreError> {
    let stored: Option<(String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT runtime_id, definition_hash, endpoint,
                session_credential_hash, lease_expires_at
         FROM woml_workflow_runtime_routes
         WHERE workflow_id = ?1 AND lease_expires_at > ?2",
        params![workflow_id, now.to_rfc3339()],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    stored
      .map(
        |(runtime_id, definition_hash, endpoint, session_credential_hash, expires)| {
          let lease_expires_at = DateTime::parse_from_rfc3339(&expires)
            .map_err(|_| {
              DurableStoreError::Contract(
                "A stored workflow runtime lease has an invalid timestamp.".to_string(),
              )
            })?
            .with_timezone(&Utc);
          Ok(WorkflowRuntimeRoute {
            runtime_id,
            workflow_id: workflow_id.to_string(),
            definition_hash,
            endpoint,
            session_credential_hash,
            lease_expires_at,
          })
        },
      )
      .transpose()
  }

  pub fn has_live_workflow_runtime_routes(
    &self,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    self
      .connection
      .query_row(
        "SELECT EXISTS(
           SELECT 1 FROM woml_workflow_runtime_routes WHERE lease_expires_at > ?1
         )",
        [now.to_rfc3339()],
        |row| row.get(0),
      )
      .map_err(DurableStoreError::from)
  }
}
