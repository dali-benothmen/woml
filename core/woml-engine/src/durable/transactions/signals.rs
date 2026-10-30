//! Authenticated signal identity/deduplication, bounded early receipts, and exact-wait settlement. Admission never invents a second run.

use crate::durable::append_to_history;
use crate::durable::definition_by_hash;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::load_run_binding_optional;
use crate::durable::DurableEventStore;
use crate::durable::DurableStoreError;
use crate::durable::ExternalSignalAdmissionOutcome;
use crate::durable::ExternalSignalAdmissionRequest;
use crate::durable::ExternalSignalAdmissionStatus;
use crate::durable::ExternalSignalResolution;
use crate::durable::EXTERNAL_SIGNAL_ADMISSION_PROFILE;
use crate::durable::EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN;
use crate::durable::EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES;
use crate::durable::EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH;
use crate::durable::EXTERNAL_SIGNAL_RESULT_PROFILE;
use crate::engine::resolve_context_reference;
use crate::engine::validate_event_history_against_definition;
use crate::fold_events;
use crate::model::ValueExpression;
use crate::CompiledWorkflowDefinition;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;

pub(in crate::durable) fn sha256_prefixed(bytes: &[u8]) -> String {
  format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn valid_external_signal_name(value: &str) -> bool {
  if value.is_empty() || value.len() > 128 {
    return false;
  }
  let mut characters = value.chars();
  if !matches!(characters.next(), Some(first) if first.is_ascii_lowercase()) {
    return false;
  }
  let mut saw_separator = false;
  let mut previous_was_separator = false;
  for character in characters {
    if matches!(character, '.' | '_' | '-') {
      if previous_was_separator {
        return false;
      }
      saw_separator = true;
      previous_was_separator = true;
    } else if character.is_ascii_lowercase() || character.is_ascii_digit() {
      previous_was_separator = false;
    } else {
      return false;
    }
  }
  saw_separator && !previous_was_separator
}

fn valid_bounded_identity(value: &str, maximum: usize) -> bool {
  !value.is_empty()
    && value.len() <= maximum
    && value.trim() == value
    && !value.chars().any(char::is_control)
}

fn valid_public_wait_identity(value: &str) -> bool {
  value.len() <= 128 && {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
      && characters.all(|character| character.is_ascii_alphanumeric())
  }
}

fn valid_bounded_signal_value(value: &str, maximum: usize) -> bool {
  valid_bounded_identity(value, maximum)
}

fn json_depth(value: &Value) -> usize {
  match value {
    Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
    Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
    _ => 1,
  }
}

pub(in crate::durable) fn resolve_signal_correlation(
  expression: &ValueExpression,
  context: &crate::WorkflowContext,
) -> Option<String> {
  let value = match expression {
    ValueExpression::Literal { value } => value.clone(),
    ValueExpression::ContextReference { .. } => {
      resolve_context_reference(expression, context).ok()?
    }
    _ => return None,
  };
  value
    .as_str()
    .filter(|value| valid_bounded_signal_value(value, 256))
    .map(str::to_string)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::durable) fn append_signal_resolution(
  transaction: &Transaction<'_>,
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  wait_id: &str,
  signal_id: &str,
  signal: &str,
  correlation: &str,
  idempotency_digest: &str,
  authentication_class: &str,
  received_at: DateTime<Utc>,
  resolved_at: DateTime<Utc>,
  payload: Value,
) -> Result<RunProjection, DurableStoreError> {
  let event_schema_version = events
    .first()
    .map(|event| event.event_schema_version)
    .ok_or_else(|| DurableStoreError::Contract("Signal target has no run history.".to_string()))?;
  append_to_history(
    transaction,
    events,
    run_id,
    generated_event_id(),
    received_at,
    event_schema_version,
    RunEventPayload::ExternalSignalReceived(crate::ExternalSignalReceivedData {
      signal_id: signal_id.to_string(),
      wait_id: wait_id.to_string(),
      signal: signal.to_string(),
      received_at,
      idempotency_digest: idempotency_digest.to_string(),
      payload: payload.clone(),
      correlation: Some(correlation.to_string()),
      authentication_class: Some(authentication_class.to_string()),
    }),
  )?;
  let resolution_event_id = generated_event_id();
  append_to_history(
    transaction,
    events,
    run_id,
    resolution_event_id.clone(),
    resolved_at,
    event_schema_version,
    RunEventPayload::WaitSatisfied(crate::WaitSatisfiedData {
      wait_id: wait_id.to_string(),
      winner: crate::DurableWaitWinner::Signal,
      result: serde_json::json!({
        "status": "signaled",
        "signal": signal,
        "signalId": signal_id,
        "correlation": correlation,
        "receivedAt": received_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "payload": payload,
      }),
      resolved_at,
    }),
  )?;
  validate_event_history_against_definition(workflow, definition_hash, events)
    .map_err(DurableStoreError::Contract)?;
  let changed = transaction.execute(
    "UPDATE woml_durable_waits
     SET state = 'satisfied', resolution_event_id = ?1, resolved_at = ?2
     WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = '' AND state = 'registered'",
    params![
      resolution_event_id,
      resolved_at.to_rfc3339(),
      run_id,
      wait_id
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Signal resolution lost the durable wait winner transaction.".to_string(),
    ));
  }
  transaction.execute(
    "UPDATE woml_external_signals SET consumed_at = ?1
     WHERE signal_id = ?2 AND consumed_at IS NULL",
    params![resolved_at.to_rfc3339(), signal_id],
  )?;
  let projection = fold_events(events)?;
  if projection.status != RunStatus::Running || !projection.context.steps.contains_key(wait_id) {
    return Err(DurableStoreError::Contract(
      "Signal settlement did not publish the wait result into context.".to_string(),
    ));
  }
  Ok(projection)
}

impl DurableEventStore {
  pub fn admit_external_signal_atomically(
    &mut self,
    request: ExternalSignalAdmissionRequest,
    known_secrets: &[String],
  ) -> Result<ExternalSignalAdmissionOutcome, DurableStoreError> {
    let idempotency_digest = sha256_prefixed(request.idempotency_key.as_bytes());
    let signal_id = format!(
      "signal_{}",
      &idempotency_digest
        .strip_prefix("sha256:")
        .unwrap_or(&idempotency_digest)[..32]
    );
    let outcome = |status: ExternalSignalAdmissionStatus,
                   resolution: Option<ExternalSignalResolution>,
                   code: Option<&str>| ExternalSignalAdmissionOutcome {
      profile: EXTERNAL_SIGNAL_RESULT_PROFILE,
      status,
      signal_id: signal_id.clone(),
      run_id: request.run_id.clone(),
      wait_id: request.wait_id.clone(),
      resolution,
      code: code.map(str::to_string),
    };
    if request.profile != EXTERNAL_SIGNAL_ADMISSION_PROFILE
      || !valid_external_signal_name(&request.signal)
      || !valid_bounded_identity(&request.run_id, 320)
      || !valid_public_wait_identity(&request.wait_id)
      || !valid_bounded_signal_value(&request.correlation, 256)
      || !valid_bounded_signal_value(&request.idempotency_key, 256)
      || !valid_bounded_signal_value(&request.authentication_class, 64)
      || json_depth(&request.payload) > EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_INVALID"),
      ));
    }
    let payload_json = canonical_json(&request.payload)?;
    if payload_json.len() > EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_TOO_LARGE"),
      ));
    }
    if known_secrets.iter().any(|secret| {
      secret.len() >= 8
        && payload_json
          .windows(secret.len())
          .any(|window| window == secret.as_bytes())
    }) {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_SECRET"),
      ));
    }
    let request_digest = sha256_prefixed(&canonical_json(&serde_json::json!({
      "signal": request.signal,
      "runId": request.run_id,
      "waitId": request.wait_id,
      "correlation": request.correlation,
      "payload": request.payload,
    }))?);
    let correlation_digest = sha256_prefixed(request.correlation.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let prior: Option<(String, String)> = transaction
      .query_row(
        "SELECT request_digest, signal_id FROM woml_signal_idempotency
         WHERE idempotency_digest = ?1",
        [&idempotency_digest],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((prior_digest, prior_signal_id)) = prior {
      let mut duplicate = outcome(
        if prior_digest == request_digest {
          ExternalSignalAdmissionStatus::Duplicate
        } else {
          ExternalSignalAdmissionStatus::IdentityConflict
        },
        None,
        (prior_digest != request_digest).then_some("WOML_SIGNAL_IDENTITY_CONFLICT"),
      );
      duplicate.signal_id = prior_signal_id;
      return Ok(duplicate);
    }
    let Some(binding) = load_run_binding_optional(&transaction, &request.run_id)? else {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::TargetNotFound,
        None,
        Some("WOML_SIGNAL_TARGET_NOT_FOUND"),
      ));
    };
    let workflow = definition_by_hash(&transaction, &binding.definition_hash)?;
    let Some(crate::CompiledWait::Signal {
      signals,
      correlation,
      ..
    }) = workflow.durable_wait(&request.wait_id)
    else {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::TargetNotFound,
        None,
        Some("WOML_SIGNAL_TARGET_NOT_FOUND"),
      ));
    };
    let Some(signal) = signals
      .iter()
      .find(|signal| signal.signal == request.signal)
    else {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::TargetNotFound,
        None,
        Some("WOML_SIGNAL_TARGET_NOT_FOUND"),
      ));
    };
    let mut events = load_events(&transaction, &request.run_id)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Finalizing
    ) || projection.business_outcome.is_some()
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::AlreadyResolved,
        None,
        Some("WOML_SIGNAL_ALREADY_RESOLVED"),
      ));
    }
    let expected_correlation = resolve_signal_correlation(correlation, &projection.context)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Signal correlation is unavailable before its dominating predecessor completes."
            .to_string(),
        )
      })?;
    if expected_correlation != request.correlation {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_CORRELATION_MISMATCH"),
      ));
    }
    let validator = jsonschema::draft202012::options()
      .should_validate_formats(true)
      .build(&signal.schema)
      .map_err(|error| {
        DurableStoreError::Contract(format!("Invalid stored signal schema: {error}"))
      })?;
    if !validator.is_valid(&request.payload) {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_INVALID"),
      ));
    }
    let active_wait: Option<(String, Option<String>)> = transaction
      .query_row(
        "SELECT state, correlation_digest FROM woml_durable_waits
         WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''",
        params![request.run_id, request.wait_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if active_wait
      .as_ref()
      .is_some_and(|(state, _)| state != "registered")
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::AlreadyResolved,
        None,
        Some("WOML_SIGNAL_ALREADY_RESOLVED"),
      ));
    }
    if active_wait
      .as_ref()
      .is_some_and(|(_, digest)| digest.as_deref() != Some(correlation_digest.as_str()))
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_CORRELATION_MISMATCH"),
      ));
    }
    if active_wait.is_none() {
      let pending: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM woml_external_signals
         WHERE run_id = ?1 AND consumed_at IS NULL",
        [&request.run_id],
        |row| row.get(0),
      )?;
      if pending >= i64::try_from(EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN).unwrap_or(64) {
        return Ok(outcome(
          ExternalSignalAdmissionStatus::Rejected,
          None,
          Some("WOML_SIGNAL_EARLY_LIMIT_REACHED"),
        ));
      }
    }
    transaction.execute(
      "INSERT INTO woml_external_signals(
         signal_id, run_id, wait_id, iteration_identity, signal, received_at,
         idempotency_digest, request_digest, correlation_digest,
         payload_json, authentication_class, consumed_at
       ) VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
      params![
        signal_id,
        request.run_id,
        request.wait_id,
        request.signal,
        request.received_at.to_rfc3339(),
        idempotency_digest,
        request_digest,
        correlation_digest,
        String::from_utf8(payload_json).map_err(|_| DurableStoreError::Contract(
          "Canonical signal payload is not UTF-8.".to_string()
        ))?,
        request.authentication_class,
      ],
    )?;
    transaction.execute(
      "INSERT INTO woml_signal_idempotency(
         idempotency_digest, request_digest, signal_id, created_at
       ) VALUES (?1, ?2, ?3, ?4)",
      params![
        idempotency_digest,
        request_digest,
        signal_id,
        request.received_at.to_rfc3339(),
      ],
    )?;
    if active_wait.is_none() {
      transaction.commit()?;
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Accepted,
        Some(ExternalSignalResolution::Early),
        None,
      ));
    }
    append_signal_resolution(
      &transaction,
      &workflow,
      &binding.definition_hash,
      &mut events,
      &request.run_id,
      &request.wait_id,
      &signal_id,
      &request.signal,
      &request.correlation,
      &idempotency_digest,
      &request.authentication_class,
      request.received_at,
      request.received_at,
      request.payload,
    )?;
    transaction.commit()?;
    Ok(outcome(
      ExternalSignalAdmissionStatus::Accepted,
      Some(ExternalSignalResolution::Resolved),
      None,
    ))
  }
}
