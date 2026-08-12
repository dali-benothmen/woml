# WOML Services

## Durable workflow memory (`services.state`)

`services.state` stores small, permanent, workflow-owned JSON values:

```js
const previous = await services.state.get('previous-sales');
await services.state.set('previous-sales', 700, {
  name: 'remember-sales',
  ifVersion: previous.found ? previous.version : 0
});
```

Normal `woml check`/`woml run` preparation generates the `StateService` editor
types automatically, including for local modules. DS3 routes every call from
Bun through Capability Call v1 to Rust's workflow-scoped Store v13 authority;
it never substitutes Cache v1 or process memory. The frozen boundary is
documented in [Durable User State v1](protocols/durable-state-v1.md).
Concurrency, backup, recovery, permissions, encryption, and corruption handling
are covered in [Durable User State Operations](woml-durable-state.md).

WOML scripts expose seven built-in services without installing
an npm package:

```js
services.http
services.db
services.storage
services.cache
services.events
services.workflows
services.state
```

`services.workflows.call()` starts exactly one activated child workflow by ID,
passes a JSON object as its `context.payload`, and resolves to its final JSON
result. WC4 safely reconnects retries and duplicate delivery to that same child,
requires stable names for repeated calls to one target in a step, rejects call
cycles, and fails ambiguous parent attempts closed. The target may be loaded by
the same `woml run` runtime or owned by another local `woml run` process sharing
the same state database. Cross-process routing is automatic; it requires no
author-managed URL or secret.

`services.workflows.start()` uses the same exact targeting and durable child
admission, but returns `{ workflowId, runId, duplicate }` after dispatch rather
than waiting for completion. The parent continues while the child runs; a later
child failure does not retroactively fail it.

The child uses the same normal engine as any triggered workflow, so branches,
parallel groups, retries, modules, native Fetch, and all current services keep
their normal behavior. Human Approval targets are rejected before admission in
v1. The CLI prints both run IDs, while `woml get` exposes each run's bounded,
redacted lifecycle status; see `docs/woml-workflow-calls.md`.
WC7 hardens migration, corrupted-store rejection, clean packaging, benchmark
coverage, and the documented one-machine production boundary in
`docs/woml-workflow-calls-production.md`.

Bun's native `fetch()` is also available. Bun executes JavaScript, while Rust
supervises managed service calls, records bounded operation events, applies
cancellation and limits, and keeps secrets out of durable operation metadata.
Lifecycle hooks can call the same services for observational work, but a hook
failure becomes a lifecycle warning and never rewrites the business outcome.
Keep any service operation required for correctness as an ordinary workflow
step. See [Lifecycle and Local Run Control](woml-lifecycle-and-run-control.md).

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
| Start exact background workflow work | `services.workflows.start()` | Durable child run ID without waiting for completion |
| Small workflow-owned correctness data | `services.state` | Versioned durable JSON state shared by future runs of the same workflow ID |

The expanded decision tree, limits, ownership, and migration signals are in
[Choosing Where Workflow Data Lives](woml-data-guide.md).

Queue is deliberately unavailable. Durable triggers already create runs safely,
and internal events cover fan-out. A queue will be added only when WOML has a
complete producer, consumer trigger, recovery, and inspection experience.

## Reliability rules authors should know

- Put credentials in `woml secrets`; use only literal `secrets.NAME` access.
- Return only the data that later steps genuinely need. Returned values become
  durable workflow context.
- Give repeated effectful calls stable `{ name: "..." }` identities.
- Use `ifVersion` when concurrent runs may update the same state key.
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

The same runtime unit can be selected explicitly:

```bash
woml run examples/workflowCalls/request-risk.woml \
  examples/workflowCalls/calculate-risk.woml
```

For separate local processes, point both commands at the same durable state:

```bash
woml run examples/workflowCalls/calculate-risk.woml --state .woml/state.sqlite
woml run examples/workflowCalls/request-risk.woml --state .woml/state.sqlite
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
- Workflow Calls author guide: `docs/woml-workflow-calls.md`
- Durable User State operations: `docs/woml-durable-state.md`
- Data choice guide: `docs/woml-data-guide.md`
- Local data security: `docs/woml-data-security.md`
- SDK migration: `docs/woml-sdk-migration.md`
