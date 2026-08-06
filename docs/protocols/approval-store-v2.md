# WOML Durable Store v2 — Approval Credentials

Status: frozen for the Human Approval profile

Store v2 preserves the immutable v1 definition, run-binding, and event tables
and adds this append-only credential index:

```sql
CREATE TABLE woml_approval_tokens (
  token_id TEXT PRIMARY KEY,
  secret_hash BLOB NOT NULL CHECK (length(secret_hash) = 32),
  request_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  approval_id TEXT NOT NULL,
  issued_at TEXT NOT NULL,
  credential_expires_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_approval_tokens_request
  ON woml_approval_tokens(run_id, approval_id, request_id);
```

Update and delete triggers make token rows append-only. The v1-to-v2 migration
runs in one immediate transaction, creates the table/index/triggers, and changes
`woml_store_metadata.schema_version` from `1` to `2`. It never rewrites a
definition, run binding, or event.

## Token contract

The external token is `apr_<tokenId>.<secret>`. The ID and secret are generated
from a cryptographically secure random source. The secret has at least 256 bits
of entropy. SQLite stores `SHA-256(secret)` as 32 bytes and compares a candidate
digest in constant time after lookup by token ID.

The default credential lifetime is 24 hours and is capped by the workflow
approval deadline when that deadline arrives sooner. Credential expiry does not
resolve an approval. A recovered unresolved request may receive another
append-only token row; earlier unexpired tokens remain valid and converge on the
same atomic resolution.

Plaintext tokens are never stored in definitions, events, context, outputs,
errors, or SQLite. Resolved/expired credential rows remain until an explicitly
versioned retention contract is implemented.

## Atomic decision and timeout

Decision and timeout operations use `BEGIN IMMEDIATE`, load and validate the
bound model/history, compare the engine clock to the durable request deadline,
and append one resolution. An identical durable human decision returns an
idempotent transport result without another event. A different decision
conflicts. Timeout failure appends `approval_resolved` and `run_failed` in the
same transaction.
