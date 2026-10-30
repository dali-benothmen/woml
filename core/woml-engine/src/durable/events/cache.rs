//! Bounded active-run projection caching, validated reload, and output hydration.
//!
//! Freshness needs both history length/sequence and last event identity. Tentative
//! writes remove the old entry; rollback cannot leave speculative state published.

use crate::durable::{load_events, DurableEventStore, DurableStoreError};
use crate::engine::validate_event_history_against_definition;
use crate::projection::ProjectionFoldState;
use crate::ReviewPayloadV1;
use crate::RunEvent;
use crate::RunProjection;
use crate::RunStatus;
use crate::RUN_EVENT_SCHEMA_VERSION_V11;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;

const MAX_ACTIVE_PROJECTION_CACHE_ENTRIES: usize = 256;

fn hydrate_rich_approval_public_outputs(
  connection: &Connection,
  run_id: &str,
  projection: &mut RunProjection,
) -> Result<(), DurableStoreError> {
  for request in projection.rich_approval_requests.values() {
    if request.status == crate::RichApprovalRequestStatus::Waiting {
      continue;
    }
    let review_json: Option<String> = connection
      .query_row(
        "SELECT review_json FROM woml_approval_reviews
         WHERE run_id = ?1 AND request_id = ?2",
        params![run_id, request.request_id],
        |row| row.get(0),
      )
      .optional()?;
    let Some(review_json) = review_json else {
      return Err(DurableStoreError::Contract(
        "A settled rich approval has no immutable review snapshot.".to_string(),
      ));
    };
    let review: ReviewPayloadV1 = serde_json::from_str(&review_json)?;
    let output = projection
      .context
      .steps
      .get_mut(&request.approval_id)
      .and_then(Value::as_object_mut)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "A settled rich approval has no public context output.".to_string(),
        )
      })?;
    output.insert(
      "review".to_string(),
      serde_json::json!({
        "digest": request.review_digest,
        "title": review.title,
        "risk": review.risk,
        "artifacts": review.artifacts,
      }),
    );
  }
  Ok(())
}

pub(in crate::durable) fn projection_state_for_events(
  cache: &RefCell<HashMap<String, ProjectionFoldState>>,
  run_id: &str,
  events: &[RunEvent],
) -> Result<ProjectionFoldState, DurableStoreError> {
  let cached = cache.borrow_mut().remove(run_id);
  if let Some(cached) = cached {
    let sequence_matches = cached.projection().last_sequence == events.len() as u64;
    let event_matches =
      cached.last_event_id() == events.last().map(|event| event.event_id.as_str());
    if sequence_matches && event_matches {
      return Ok(cached);
    }
  }
  Ok(ProjectionFoldState::from_events(events)?)
}

pub(in crate::durable) fn cache_projection_state(
  cache: &RefCell<HashMap<String, ProjectionFoldState>>,
  run_id: &str,
  state: ProjectionFoldState,
) {
  let mut cache = cache.borrow_mut();
  if matches!(
    state.projection().status,
    RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
  ) {
    cache.remove(run_id);
    return;
  }
  if !cache.contains_key(run_id) && cache.len() >= MAX_ACTIVE_PROJECTION_CACHE_ENTRIES {
    if let Some(evicted) = cache.keys().next().cloned() {
      cache.remove(&evicted);
    }
  }
  cache.insert(run_id.to_string(), state);
}

impl DurableEventStore {
  pub fn projection(&self, run_id: &str) -> Result<RunProjection, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.project_run");
    span.run_id(run_id.to_string());
    let binding = self.run_binding(run_id)?;
    let mut statement = self.connection.prepare_cached(
      "SELECT COUNT(*), (
         SELECT event_id FROM woml_run_events
         WHERE run_id = ?1 ORDER BY sequence DESC LIMIT 1
       )
       FROM woml_run_events WHERE run_id = ?1",
    )?;
    let (event_count, last_event_id): (i64, Option<String>) =
      statement.query_row([run_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let event_count = u64::try_from(event_count)
      .map_err(|_| DurableStoreError::Contract("run event count is invalid".to_string()))?;
    span.count("events", event_count as usize);
    if let Some(cached) = self.projection_cache.borrow().get(run_id) {
      if cached.projection().last_sequence == event_count
        && cached.last_event_id() == last_event_id.as_deref()
      {
        span.count("projection_cache_hits", 1);
        let mut projection = cached.projection().clone();
        if projection
          .event_schema_version
          .is_some_and(|version| version >= RUN_EVENT_SCHEMA_VERSION_V11)
          && projection.workflow_id.is_none()
        {
          projection.workflow_id = Some(binding.workflow_id);
        }
        hydrate_rich_approval_public_outputs(&self.connection, run_id, &mut projection)?;
        span.succeed();
        return Ok(projection);
      }
    }
    span.count("projection_cache_misses", 1);
    let events = load_events(&self.connection, run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let state = ProjectionFoldState::from_events(&events)?;
    let mut projection = state.projection().clone();
    if projection
      .event_schema_version
      .is_some_and(|version| version >= RUN_EVENT_SCHEMA_VERSION_V11)
      && projection.workflow_id.is_none()
    {
      projection.workflow_id = Some(binding.workflow_id);
    }
    hydrate_rich_approval_public_outputs(&self.connection, run_id, &mut projection)?;
    cache_projection_state(&self.projection_cache, run_id, state);
    span.succeed();
    Ok(projection)
  }
}
