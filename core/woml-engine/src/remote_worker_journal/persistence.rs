//! Private remote worker journal persistence.

use crate::remote_protocol::RemoteDispatchPackageV1;
use crate::remote_protocol::REMOTE_TERMINAL_RESULT_MAX_BYTES;
use crate::remote_worker_journal::sha256;
use crate::remote_worker_journal::PreparedRemoteDispatchV1;
use crate::remote_worker_journal::RemoteWorkerError;
use crate::remote_worker_journal::RemoteWorkerTerminalV1;
use crate::remote_worker_journal::WorkerAttemptRecordV1;
use crate::remote_worker_journal::WorkerAttemptStateV1;
use crate::remote_worker_journal::CREATE_SCHEMA;
use crate::remote_worker_journal::WORKER_JOURNAL_RECOVERY_LIMIT;
use crate::remote_worker_journal::WORKER_JOURNAL_SCHEMA_VERSION;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::json;
use serde_json_canonicalizer::to_vec as canonical_json;
use std::path::Path;

pub struct WorkerJournalV1 {
  pub(in crate::remote_worker_journal) connection: Connection,
}

impl WorkerJournalV1 {
  pub fn open(path: &Path) -> Result<Self, RemoteWorkerError> {
    let connection = Connection::open(path)?;
    Self::initialize(connection)
  }

  pub fn open_in_memory() -> Result<Self, RemoteWorkerError> {
    Self::initialize(Connection::open_in_memory()?)
  }

  pub(in crate::remote_worker_journal) fn initialize(
    connection: Connection,
  ) -> Result<Self, RemoteWorkerError> {
    connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    connection.execute_batch(CREATE_SCHEMA)?;
    let version: Option<String> = connection
      .query_row(
        "SELECT value FROM woml_worker_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
      )
      .optional()?;
    match version.as_deref() {
      None => {
        connection.execute(
          "INSERT INTO woml_worker_metadata(key,value) VALUES('schema_version',?1)",
          [WORKER_JOURNAL_SCHEMA_VERSION.to_string()],
        )?;
      }
      Some("1") => {}
      Some(_) => return Err(RemoteWorkerError::CorruptJournal),
    }
    let journal = Self { connection };
    journal.verify_chain()?;
    Ok(journal)
  }

  pub fn accept(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    worker_id: &str,
    session_id: &str,
    accepted_at: DateTime<Utc>,
  ) -> Result<WorkerAttemptRecordV1, RemoteWorkerError> {
    if prepared.worker_id != worker_id {
      return Err(RemoteWorkerError::Conflict);
    }
    if let Some(existing) = self.load(&prepared.package.dispatch_id)? {
      if existing.package_digest == prepared.package.package_digest
        && existing.worker_id == worker_id
        && existing.authority_digest == prepared.effective_authority.authority_digest
      {
        return Ok(existing);
      }
      return Err(RemoteWorkerError::Conflict);
    }
    let tx = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
      "INSERT INTO woml_worker_dispatches(
         dispatch_id,package_digest,worker_id,run_id,node_id,iteration_identity,attempt,
         invocation_id,target,authority_digest,state,accepted_at
       ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'accepted',?11)",
      params![
        prepared.package.dispatch_id,
        prepared.package.package_digest,
        worker_id,
        prepared.package.run_id,
        prepared.package.node_id,
        prepared.package.iteration_key.as_deref().unwrap_or(""),
        prepared.package.attempt,
        prepared.package.invocation_id,
        prepared.package.target,
        prepared.effective_authority.authority_digest,
        accepted_at.to_rfc3339(),
      ],
    )?;
    append_record(
      &tx,
      &prepared.package,
      worker_id,
      session_id,
      "accepted",
      &prepared.effective_authority.authority_digest,
      &prepared.package.package_digest,
      accepted_at,
    )?;
    tx.commit()?;
    self
      .load(&prepared.package.dispatch_id)?
      .ok_or(RemoteWorkerError::CorruptJournal)
  }

  pub fn start(
    &mut self,
    dispatch_id: &str,
    session_id: &str,
    started_at: DateTime<Utc>,
  ) -> Result<WorkerAttemptRecordV1, RemoteWorkerError> {
    let record = self.load(dispatch_id)?.ok_or(RemoteWorkerError::Conflict)?;
    if record.state != WorkerAttemptStateV1::Accepted {
      return if record.state == WorkerAttemptStateV1::Started {
        Ok(record)
      } else {
        Err(RemoteWorkerError::Conflict)
      };
    }
    let tx = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
      "UPDATE woml_worker_dispatches SET state='started',started_at=?2
       WHERE dispatch_id=?1 AND state='accepted'",
      params![dispatch_id, started_at.to_rfc3339()],
    )?;
    append_record_from_record(
      &tx,
      &record,
      session_id,
      "started",
      &sha256(b"started"),
      started_at,
    )?;
    tx.commit()?;
    self
      .load(dispatch_id)?
      .ok_or(RemoteWorkerError::CorruptJournal)
  }

  pub fn terminal(
    &mut self,
    dispatch_id: &str,
    session_id: &str,
    terminal: &RemoteWorkerTerminalV1,
  ) -> Result<WorkerAttemptRecordV1, RemoteWorkerError> {
    let record = self.load(dispatch_id)?.ok_or(RemoteWorkerError::Conflict)?;
    if let Some(existing) = &record.terminal {
      return if existing == terminal {
        Ok(record)
      } else {
        Err(RemoteWorkerError::Conflict)
      };
    }
    if record.state != WorkerAttemptStateV1::Started
      || !terminal.state.is_terminal()
      || terminal.dispatch_id != record.dispatch_id
      || terminal.invocation_id != record.invocation_id
    {
      return Err(RemoteWorkerError::Conflict);
    }
    let encoded = canonical_json(terminal).map_err(|_| RemoteWorkerError::InvalidResult)?;
    if encoded.len() > REMOTE_TERMINAL_RESULT_MAX_BYTES {
      return Err(RemoteWorkerError::InvalidResult);
    }
    let text = String::from_utf8(encoded).map_err(|_| RemoteWorkerError::InvalidResult)?;
    let tx = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
      "UPDATE woml_worker_dispatches
       SET state=?2,terminal_json=?3,terminal_digest=?4,terminal_at=?5
       WHERE dispatch_id=?1 AND state='started'",
      params![
        dispatch_id,
        terminal.state.as_str(),
        text,
        terminal.terminal_digest,
        terminal.completed_at.to_rfc3339(),
      ],
    )?;
    append_record_from_record(
      &tx,
      &record,
      session_id,
      terminal.state.as_str(),
      &terminal.terminal_digest,
      terminal.completed_at,
    )?;
    tx.commit()?;
    self
      .load(dispatch_id)?
      .ok_or(RemoteWorkerError::CorruptJournal)
  }

  pub fn recover(
    &mut self,
    now: DateTime<Utc>,
  ) -> Result<Vec<WorkerAttemptRecordV1>, RemoteWorkerError> {
    let count: usize = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_worker_dispatches WHERE state IN ('accepted','started')",
      [],
      |row| row.get(0),
    )?;
    if count > WORKER_JOURNAL_RECOVERY_LIMIT {
      return Err(RemoteWorkerError::CorruptJournal);
    }
    let started = {
      let mut statement = self.connection.prepare(
        "SELECT dispatch_id FROM woml_worker_dispatches WHERE state='started' ORDER BY dispatch_id",
      )?;
      let values = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
      values
    };
    for dispatch_id in started {
      let record = self
        .load(&dispatch_id)?
        .ok_or(RemoteWorkerError::CorruptJournal)?;
      let tx = self
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
      tx.execute(
        "UPDATE woml_worker_dispatches SET state='ambiguous' WHERE dispatch_id=?1 AND state='started'",
        [&dispatch_id],
      )?;
      append_record_from_record(
        &tx,
        &record,
        "recovery",
        "ambiguous",
        &sha256(b"ambiguous"),
        now,
      )?;
      tx.commit()?;
    }
    self.pending_or_ambiguous()
  }

  pub fn terminal_for_resend(
    &self,
    dispatch_id: &str,
  ) -> Result<Option<RemoteWorkerTerminalV1>, RemoteWorkerError> {
    Ok(self.load(dispatch_id)?.and_then(|record| record.terminal))
  }

  pub fn was_started_by_session(
    &self,
    dispatch_id: &str,
    session_id: &str,
  ) -> Result<bool, RemoteWorkerError> {
    let count: u64 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_worker_journal
       WHERE dispatch_id=?1 AND session_id=?2 AND record_type='started'",
      params![dispatch_id, session_id],
      |row| row.get(0),
    )?;
    Ok(count == 1)
  }

  pub fn pending_or_ambiguous(&self) -> Result<Vec<WorkerAttemptRecordV1>, RemoteWorkerError> {
    let mut statement = self.connection.prepare(
      "SELECT dispatch_id FROM woml_worker_dispatches
       WHERE state IN ('accepted','ambiguous') ORDER BY accepted_at,dispatch_id LIMIT ?1",
    )?;
    let ids = statement
      .query_map([WORKER_JOURNAL_RECOVERY_LIMIT], |row| {
        row.get::<_, String>(0)
      })?
      .collect::<Result<Vec<_>, _>>()?;
    ids
      .into_iter()
      .map(|id| self.load(&id)?.ok_or(RemoteWorkerError::CorruptJournal))
      .collect()
  }

  pub fn load(
    &self,
    dispatch_id: &str,
  ) -> Result<Option<WorkerAttemptRecordV1>, RemoteWorkerError> {
    let record = self
      .connection
      .query_row(
        "SELECT package_digest,worker_id,run_id,node_id,iteration_identity,attempt,
              invocation_id,target,authority_digest,state,terminal_json,terminal_digest
       FROM woml_worker_dispatches WHERE dispatch_id=?1",
        [dispatch_id],
        |row| {
          let state: String = row.get(9)?;
          let terminal_json: Option<String> = row.get(10)?;
          let terminal_digest: Option<String> = row.get(11)?;
          let terminal = terminal_json
            .map(|value| {
              serde_json::from_str::<RemoteWorkerTerminalV1>(&value)
                .map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .transpose()?;
          if terminal
            .as_ref()
            .map(|value| value.terminal_digest.as_str())
            != terminal_digest.as_deref()
            || terminal
              .as_ref()
              .is_some_and(|value| value.verify().is_err())
          {
            return Err(rusqlite::Error::InvalidQuery);
          }
          Ok((
            WorkerAttemptRecordV1 {
              dispatch_id: dispatch_id.to_string(),
              package_digest: row.get(0)?,
              worker_id: row.get(1)?,
              run_id: row.get(2)?,
              node_id: row.get(3)?,
              iteration_identity: row.get(4)?,
              attempt: row.get(5)?,
              invocation_id: row.get(6)?,
              target: row.get(7)?,
              authority_digest: row.get(8)?,
              state: WorkerAttemptStateV1::parse(&state)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
              terminal,
            },
            state,
          ))
        },
      )
      .optional()
      .map(|value| value.map(|(record, _)| record))
      .map_err(RemoteWorkerError::from)?;
    let Some(record) = record else {
      return Ok(None);
    };
    let accepted: Option<(String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT worker_id,package_digest,attempt_identity_digest,authority_digest
         FROM woml_worker_journal
         WHERE dispatch_id=?1 AND record_type='accepted'
         ORDER BY local_sequence LIMIT 1",
        [dispatch_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    let identity = sha256(
      &canonical_json(&json!({
        "runId": record.run_id,
        "nodeId": record.node_id,
        "iterationIdentity": record.iteration_identity,
        "attempt": record.attempt,
        "invocationId": record.invocation_id,
      }))
      .map_err(|_| RemoteWorkerError::CorruptJournal)?,
    );
    if accepted.as_ref().is_none_or(|accepted| {
      accepted.0 != record.worker_id
        || accepted.1 != record.package_digest
        || accepted.2 != identity
        || accepted.3 != record.authority_digest
    }) {
      return Err(RemoteWorkerError::CorruptJournal);
    }
    Ok(Some(record))
  }

  pub(in crate::remote_worker_journal) fn verify_chain(&self) -> Result<(), RemoteWorkerError> {
    let mut statement = self.connection.prepare(
      "SELECT local_sequence,dispatch_id,worker_id,session_id,package_digest,
              attempt_identity_digest,authority_digest,fact_digest,record_type,recorded_at,
              previous_record_digest,record_digest
       FROM woml_worker_journal ORDER BY local_sequence",
    )?;
    let mut rows = statement.query([])?;
    let mut previous =
      "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string();
    while let Some(row) = rows.next()? {
      let body = json!({
        "localSequence": row.get::<_, u64>(0)?,
        "dispatchId": row.get::<_, String>(1)?,
        "workerId": row.get::<_, String>(2)?,
        "sessionId": row.get::<_, String>(3)?,
        "packageDigest": row.get::<_, String>(4)?,
        "attemptIdentityDigest": row.get::<_, String>(5)?,
        "authorityDigest": row.get::<_, String>(6)?,
        "factDigest": row.get::<_, String>(7)?,
        "recordType": row.get::<_, String>(8)?,
        "recordedAt": row.get::<_, String>(9)?,
        "previousRecordDigest": row.get::<_, String>(10)?,
      });
      if body["previousRecordDigest"] != previous {
        return Err(RemoteWorkerError::CorruptJournal);
      }
      let expected = sha256(&canonical_json(&body).map_err(|_| RemoteWorkerError::CorruptJournal)?);
      let actual: String = row.get(11)?;
      if expected != actual {
        return Err(RemoteWorkerError::CorruptJournal);
      }
      previous = actual;
    }
    Ok(())
  }
}

fn append_record(
  tx: &rusqlite::Transaction<'_>,
  package: &RemoteDispatchPackageV1,
  worker_id: &str,
  session_id: &str,
  record_type: &str,
  authority_digest: &str,
  fact_digest: &str,
  recorded_at: DateTime<Utc>,
) -> Result<(), RemoteWorkerError> {
  let record = WorkerAttemptRecordV1 {
    dispatch_id: package.dispatch_id.clone(),
    package_digest: package.package_digest.clone(),
    worker_id: worker_id.to_string(),
    run_id: package.run_id.clone(),
    node_id: package.node_id.clone(),
    iteration_identity: package.iteration_key.clone().unwrap_or_default(),
    attempt: package.attempt,
    invocation_id: package.invocation_id.clone(),
    target: package.target.clone(),
    authority_digest: authority_digest.to_string(),
    state: WorkerAttemptStateV1::Accepted,
    terminal: None,
  };
  append_record_from_record(
    tx,
    &record,
    session_id,
    record_type,
    fact_digest,
    recorded_at,
  )
}

fn append_record_from_record(
  tx: &rusqlite::Transaction<'_>,
  record: &WorkerAttemptRecordV1,
  session_id: &str,
  record_type: &str,
  fact_digest: &str,
  recorded_at: DateTime<Utc>,
) -> Result<(), RemoteWorkerError> {
  let previous: String = tx
    .query_row(
      "SELECT record_digest FROM woml_worker_journal ORDER BY local_sequence DESC LIMIT 1",
      [],
      |row| row.get(0),
    )
    .optional()?
    .unwrap_or_else(|| {
      "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string()
    });
  let next: u64 = tx.query_row(
    "SELECT COALESCE(MAX(local_sequence),0)+1 FROM woml_worker_journal",
    [],
    |row| row.get(0),
  )?;
  let identity = sha256(
    &canonical_json(&json!({
      "runId": record.run_id,
      "nodeId": record.node_id,
      "iterationIdentity": record.iteration_identity,
      "attempt": record.attempt,
      "invocationId": record.invocation_id,
    }))
    .map_err(|_| RemoteWorkerError::CorruptJournal)?,
  );
  let body = json!({
    "localSequence": next,
    "dispatchId": record.dispatch_id,
    "workerId": record.worker_id,
    "sessionId": session_id,
    "packageDigest": record.package_digest,
    "attemptIdentityDigest": identity,
    "authorityDigest": record.authority_digest,
    "factDigest": fact_digest,
    "recordType": record_type,
    "recordedAt": recorded_at.to_rfc3339(),
    "previousRecordDigest": previous,
  });
  let digest = sha256(&canonical_json(&body).map_err(|_| RemoteWorkerError::CorruptJournal)?);
  tx.execute(
    "INSERT INTO woml_worker_journal(
       local_sequence,dispatch_id,worker_id,session_id,package_digest,attempt_identity_digest,
       authority_digest,fact_digest,record_type,recorded_at,previous_record_digest,record_digest
     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
    params![
      next,
      record.dispatch_id,
      record.worker_id,
      session_id,
      record.package_digest,
      identity,
      record.authority_digest,
      fact_digest,
      record_type,
      recorded_at.to_rfc3339(),
      previous,
      digest,
    ],
  )?;
  Ok(())
}
