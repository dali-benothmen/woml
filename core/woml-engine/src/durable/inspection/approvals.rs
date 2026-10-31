//! Bounded rich-review snapshots and decisions, without exposing credentials.

use crate::durable::{
  parse_approval_decision, parse_timestamp, rich_review_row, DurableEventStore, DurableStoreError,
  RichApprovalDecisionInspection, RichApprovalInspectionV1,
  RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
};
use rusqlite::params;

impl DurableEventStore {
  pub fn inspect_rich_approval(
    &self,
    run_id: &str,
    request_id: &str,
    decision_limit: usize,
  ) -> Result<RichApprovalInspectionV1, DurableStoreError> {
    let limit = decision_limit.clamp(1, RICH_APPROVAL_INSPECTION_DECISION_LIMIT);
    let row = rich_review_row(&self.connection, run_id, request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    let (review_json, directory_version, created_at, resolved_at): (
      String,
      String,
      String,
      Option<String>,
    ) = self.connection.query_row(
      "SELECT review_json, directory_version, created_at, resolved_at
       FROM woml_approval_reviews WHERE run_id = ?1 AND request_id = ?2",
      params![run_id, request_id],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let total_decisions: u32 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_approval_reviewer_decisions
       WHERE run_id = ?1 AND request_id = ?2",
      params![run_id, request_id],
      |row| row.get(0),
    )?;
    let mut statement = self.connection.prepare(
      "SELECT principal_id, decision, provider, provider_account_id, roles_json, decided_at
       FROM woml_approval_reviewer_decisions
       WHERE run_id = ?1 AND request_id = ?2
       ORDER BY decided_at, principal_id LIMIT ?3",
    )?;
    let decisions = statement
      .query_map(params![run_id, request_id, limit as u64], |row| {
        let decision: String = row.get(1)?;
        let roles_json: String = row.get(4)?;
        let decided_at: String = row.get(5)?;
        Ok((
          row.get::<_, String>(0)?,
          decision,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          roles_json,
          decided_at,
        ))
      })?
      .map(|row| {
        let (principal_id, decision, provider, provider_account_id, roles_json, decided_at) = row?;
        Ok(RichApprovalDecisionInspection {
          principal_id,
          decision: parse_approval_decision(&decision)?,
          provider,
          provider_account_id,
          roles: serde_json::from_str(&roles_json)?,
          decided_at: parse_timestamp(&decided_at)?,
        })
      })
      .collect::<Result<Vec<_>, DurableStoreError>>()?;
    Ok(RichApprovalInspectionV1 {
      run_id: run_id.to_string(),
      approval_id: row.approval_id,
      request_id: request_id.to_string(),
      review_digest: row.review_digest,
      review: serde_json::from_str(&review_json)?,
      required: row.required,
      required_roles: row.required_roles,
      directory_version,
      status: row.status,
      created_at: parse_timestamp(&created_at)?,
      resolved_at: resolved_at.as_deref().map(parse_timestamp).transpose()?,
      decisions_truncated: total_decisions as usize > decisions.len(),
      total_decisions,
      decisions,
    })
  }
}
