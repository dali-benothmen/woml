# WOML Native Runtime Outcome v1

Status: frozen and implemented in the durable Rust runtime through A5; N-API
and CLI delivery remain staged for A6

The normative schema is
`docs/schemas/approval-runtime-outcome.v1.schema.json`.

The N-API boundary returns one of two discriminated outcomes:

- `succeeded` contains the existing complete workflow execution result.
- `waiting` contains the durable run/request identity, reviewer-safe approval
  metadata, the newly issued plaintext token, credential expiry, and optional
  workflow deadline.

Rust returns `waiting`; it never holds an N-API Promise for the human wait.
Bun builds the loopback URL from the token and owns the terminal/HTTP
experience. Rust remains the authority for token issuance, verification,
resolution, timeout, event persistence, and DAG continuation.

The waiting outcome contains no URL or port because transport configuration is
not part of the workflow engine contract. The token is an intentional
one-delivery secret at this boundary and must be redacted from errors, access
logs, snapshots, and subsequent inspection responses.
