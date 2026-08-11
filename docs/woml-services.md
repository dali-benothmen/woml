# WOML Services

WOML scripts currently execute with six built-in services without installing
an npm package:

```js
services.http
services.db
services.storage
services.cache
services.events
services.workflows
```

`services.workflows.call()` starts exactly one activated child workflow by ID,
passes a JSON object as its `context.trigger`, and resolves to its final JSON
result. WC4 safely reconnects retries and duplicate delivery to that same child,
requires stable names for repeated calls to one target in a step, rejects call
cycles, and fails ambiguous parent attempts closed. Workflows must currently be
loaded by the same `woml run` runtime; WC5 adds local cross-process routing.

Bun's native `fetch()` is also available. Bun executes JavaScript, while Rust
supervises managed service calls, records bounded operation events, applies
cancellation and limits, and keeps secrets out of durable operation metadata.

## Choose the smallest useful service

| Need | Use | Product meaning |
| --- | --- | --- |
| Standard Web API compatibility or streaming | `fetch()` | Bun's native Fetch API with redacted WOML observations |
| Supervised JSON/text/binary HTTP | `services.http.request()` | Rust-managed request, limits, timeout, and durable outcome |
| Application records and transactions | `services.db()` | Rust-managed SQLite or PostgreSQL |
| Files and larger durable values | `services.storage` | Checksummed objects stored outside workflow context |
| Reusable but disposable values | `services.cache` | Workflow-scoped expiring optimization data |
| Start every workflow interested in a fact | `services.events.emit()` | Durable internal named-event fan-out |
| Delegate work and use one workflow's answer | `services.workflows.call()` | One independently durable child run and direct JSON result |

Queue is deliberately unavailable. Durable triggers already create runs safely,
and internal events cover fan-out. A queue will be added only when WOML has a
complete producer, consumer trigger, recovery, and inspection experience.

## Reliability rules authors should know

- Put credentials in `woml secrets`; use only literal `secrets.NAME` access.
- Return only the data that later steps genuinely need. Returned values become
  durable workflow context.
- Give repeated effectful calls stable `{ name: "..." }` identities.
- Use provider idempotency keys for external APIs when they are available.
- Treat cache as disposable and database/storage as application-owned durable
  data.
- An interrupted external effect can be ambiguous. WOML fails closed instead
  of claiming an effect definitely did or did not happen.

All managed failures are catchable `WomlServiceError` values with stable
`code`, `service`, `operation`, `retryable`, and `ambiguous` fields.

## Run the examples

From the project root, use one-shot execution for standalone examples:

```bash
woml test examples/httpComparisonWorkflow.woml
woml test examples/sqliteWorkflow.woml
woml test examples/storageWorkflow.woml
woml test examples/cacheWorkflow.woml
```

The HTTP example needs internet access. PostgreSQL additionally needs
`POSTGRES_URL` configured with `woml secrets set POSTGRES_URL`:

```bash
woml test examples/postgresWorkflow.woml
```

The composition example runs a manual publisher and an internal event
subscriber in one active runtime:

```bash
woml run examples/servicesComposition
```

It writes an order to SQLite, object storage, and cache concurrently, reads the
values back, then emits `order.prepared` to the subscriber. No event secret or
HTTP endpoint is involved. Press Ctrl+C to stop.

The Workflow Calls example loads a triggered parent and a call-only child into
one runtime:

```bash
woml run examples/workflowCalls
```

The parent passes a customer ID to the child, receives its risk score, and
prints `{"message":"Customer risk score: 90","score":90}`. Press Ctrl+C to
stop the active runtime.

## Detailed references

- Outbound HTTP: `docs/woml-http-services.md`
- Database: `docs/woml-database.md`
- Durable object storage: `docs/woml-storage.md`
- Cache: `docs/woml-cache.md`
- Internal events: `docs/woml-events-service.md`
- Workflow Calls: `docs/protocols/workflow-calls-v1.md`
- SDK migration: `docs/woml-sdk-migration.md`
