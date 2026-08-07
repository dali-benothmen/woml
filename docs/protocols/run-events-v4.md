# WOML Run Events v4

Status: frozen and implemented through the A6 local approval runtime

The normative schema is `docs/schemas/run-event.v4.schema.json`. Event schemas
v1 through v3 remain immutable, and one run never mixes schema versions.

Version 4 adds two durable control events:

- `approval_requested` records the approval ID, stable request ID, optional
  deadline, and timeout policy before a capability token or URL is exposed.
- `approval_resolved` records either a human decision, a timeout rejection, or
  a timeout failure. The event timestamp is the public `decidedAt` value.

A decision resolution publishes this folded output at
`context.steps.<approvalId>`:

```json
{
  "decision": "approved",
  "source": "human",
  "decidedAt": "2026-08-07T10:05:00.000Z"
}
```

A timeout failure publishes no approval output. Its `approval_resolved` event
and approval-scoped `run_failed` event are committed atomically.

Human decisions and timeout settlement compete under one immediate SQLite
transaction. An identical repeated human decision is an idempotent transport
result and appends no second event. A different human decision conflicts. A
decision arriving after timeout loses to the durable timeout resolution.

Tokens are credential-index data rather than workflow events. Token plaintext,
token IDs, hashes, URLs, ports, and reviewer credentials are forbidden from the
event vocabulary.

The reviewed approved, rejected, timeout-rejected, and timeout-failed histories
live under `woml/tests/fixtures/run-events/`.
