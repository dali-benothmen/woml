# WOML Production Trigger Contracts v1

Status: frozen in T0. Webhook frontend lowering is implemented in T1, the Rust
occurrence authority in T2, and the Rust HTTP ingress plus background durable
execution path in T3. T4 exposes that runtime through long-lived `woml run`,
one-shot `woml test`, and durable `woml runs get` inspection.

This document pins the boundary shared by the TypeScript WOML frontend, Rust
core, CLI, HTTP listener, provider adapters, and durable store. The normative
machine-readable artifacts are:

- `docs/schemas/compiled-workflow-model.v7.schema.json`
- `docs/schemas/run-event.v7.schema.json`
- `docs/schemas/trigger-occurrence.v1.schema.json`
- `docs/schemas/trigger-ingress.v1.schema.json`
- `docs/schemas/webhook-http.v1.schema.json`
- `docs/schemas/slack-trigger-protocol.v1.schema.json`
- `docs/schemas/trigger-progress.v1.schema.json`

Model v1–v6 and Event v1–v6 remain immutable. A Model v7 run uses Event v7 for
its entire history.

## Compiled triggers

Model v7 keeps the existing DAG and defines a strict trigger union:

| WOML trigger | Handler |
|---|---|
| `<manual>` | `trigger.manual` |
| `<webhook>` | `trigger.webhook` |
| `<slack>` | `trigger.slack` |
| `<schedule>` | `trigger.schedule` |
| `<interval>` | `trigger.interval` |
| `<event>` | `trigger.event` |

Trigger IDs are unique within a workflow and every trigger starts the same DAG.
Only manual and webhook syntax is accepted by the T1 frontend. The remaining
shapes are frozen fixtures but stay rejected until their activation phases.
Resolved credentials never appear in a compiled definition; only
`secretReference` expressions are permitted.

## Occurrence admission

Every listener, scheduler, CLI publisher, and provider adapter submits
`woml.trigger-ingress` v1. Rust alone decides whether the occurrence is new, a
same-payload duplicate, a changed-payload conflict, or invalid history.

The atomic acceptance boundary is:

```text
insert or resolve occurrence
  + create run bound to definitionHash
  + append run_started v7
  + commit
```

Only after commit may execution be dispatched or an external delivery be
acknowledged. Recovery after a commit but before dispatch must continue the
existing run.

`sourceIdentity` crosses the local ingress boundary but its SHA-256 hash is the
only durable representation. The exact digest inputs are:

```text
sourceIdentityHash = "sha256:" + hex(SHA-256(UTF-8(sourceIdentity)))
payloadHash        = "sha256:" + hex(SHA-256(UTF-8(RFC8785(payload))))
```

The durable uniqueness key is `(workflowId, triggerId, sourceIdentityHash)`.
The occurrence also records the immutable definition hash, but that hash is not
part of its uniqueness key. Repeating one source identity with the same payload
returns the
original occurrence and run with `duplicate: true`. Repeating it with a
different payload returns `WOML_TRIGGER_IDEMPOTENCY_CONFLICT` and creates
nothing.

The trigger occurrence identity deliberately excludes the current definition
hash. A redelivery after a definition update still resolves to its original
run and original recorded definition hash.

For a new row, Rust generates opaque `occ_...` and `run_...` IDs. A duplicate
never generates replacement IDs; it returns the values already stored. Manual
and webhook deliveries without an external idempotency key receive a fresh
cryptographically random source identity before admission.

### T2 durable implementation

Rust validates every frozen Model v7 trigger shape before admission. SQLite
store schema v4 adds an immutable occurrence table with database-enforced
uniqueness for both the source identity and the one-to-one run binding. Rust
then commits the occurrence, immutable run binding, and sole `run_started` v7
event in one immediate transaction.

Payload hashes use an RFC 8785 implementation rather than ordinary JSON
serialization, so semantically identical objects deduplicate even when their
properties arrive in a different order. The raw source identity is accepted
only at the in-process admission boundary and is never written to occurrences,
events, context, or diagnostics.

Concurrent identical admission calls resolve to the same occurrence and run.
A transaction failure rolls back all three durable records. On restart, Rust
reports a valid occurrence whose run still contains only `run_started` as
undispatched recovery work; contradictory occurrence/run/event history fails
closed.

## Event and context projection

`run_started` v7 binds:

- `workflowId`
- `definitionHash`
- `triggerId`
- `triggerHandler`
- `triggerOccurrenceId`
- the direct public `trigger` object

Folding the final field produces `context.trigger`. Engine metadata,
credentials, request headers, Slack envelopes, idempotency keys, and scheduler
cursors are not added to workflow context. `context.run` remains unavailable.

## Trigger source identities

| Trigger | External identity material |
|---|---|
| Manual | A newly generated identity for every invocation |
| Webhook | `Idempotency-Key`, or a newly generated identity when omitted |
| Slack | Workspace ID + provider event ID + workflow ID + trigger ID |
| Schedule | Workflow ID + trigger ID + planned UTC instant |
| Interval | Workflow ID + trigger ID + durable anchor sequence |
| Event | Publisher event ID + workflow ID + trigger ID |

This deduplicates runs. Step-effect idempotency remains the separate Retry and
Idempotency v1 contract.

## Webhook HTTP mapping

Webhook responses are asynchronous; acceptance does not mean workflow success.

| Outcome | Status | Body code/status |
|---|---:|---|
| New or duplicate durable occurrence | 202 | `status: accepted` |
| Malformed/non-object JSON | 400 | `WOML_TRIGGER_PAYLOAD_INVALID` |
| Schema mismatch | 400 | `WOML_TRIGGER_SCHEMA_INVALID` |
| Missing/invalid bearer token | 401 | `WOML_TRIGGER_UNAUTHORIZED` |
| Unknown route | 404 | `WOML_TRIGGER_NOT_FOUND` |
| Wrong method | 405 | `WOML_TRIGGER_METHOD_NOT_ALLOWED` |
| Changed payload for an existing key | 409 | `WOML_TRIGGER_IDEMPOTENCY_CONFLICT` |
| Body exceeds 1 MiB | 413 | `WOML_TRIGGER_PAYLOAD_TOO_LARGE` |
| Durable authority unavailable | 503 | `WOML_TRIGGER_UNAVAILABLE` |

Rejected requests create no occurrence, run, or run event. Error bodies may
contain bounded JSON Pointer issues but never raw payloads or credentials.

### T3 HTTP implementation

The WOML Rust runtime binds the listener directly; it does not reuse the legacy
Cronflow webhook server. Registration resolves symbolic bearer secrets before
binding, rejects inactive trigger kinds and route conflicts, compiles inline
schemas once as Draft 2020-12 validators, and disables external schema
retrieval.

For each request, Rust resolves the exact static route and POST method, compares
bearer credentials through fixed-length SHA-256 digests in constant time,
requires `application/json`, enforces the 1 MiB limit while streaming, parses a
top-level object, and applies the compiled schema. Only then does it call the T2
atomic occurrence authority.

A new or duplicate occurrence returns the frozen `202` response immediately.
Only a new occurrence is dispatched, and it continues the already-created run
through the same durable DAG runtime as manual execution. Startup performs
crash recovery once and dispatches committed occurrences that contain only
their valid `run_started` event. Per-request dispatch deliberately does not run
global recovery, because another webhook run may be actively executing in the
same long-lived process.

## Slack adapter boundary

Slack Trigger Protocol v1 is separate from the approval Notification Provider
Protocol. They may share credential resolution, channel lookup, Web API, Socket
Mode, reconnect, and diagnostic transport primitives, but they do not share
domain messages.

The trigger adapter admits only normalized human app mentions and direct
messages. Every event identifies its workflow, immutable definition, and
workflow-local trigger, so registrations cannot collide across workflows. It
excludes bot/self messages, edits, deletes, unsupported subtypes, tokens, outer
provider envelopes, and unreviewed fields from `context.trigger`.
It acknowledges a Slack envelope only after Rust returns a durable acceptance
or duplicate result. When Rust is unavailable, the adapter leaves the envelope
unacknowledged so Slack can redeliver it.

### T7 Slack implementation

`woml run` keeps a shared Slack Socket Mode transport active beside the Rust
runtime. Bun resolves symbolic Slack credentials without placing token values
in the compiled registration sent to Rust, validates and filters provider
envelopes, and lowers only the frozen normalized payload through Trigger
Ingress v1. Rust atomically records the occurrence and run before Bun
acknowledges Slack. Source identity combines workspace, Slack `event_id`,
workflow, and trigger, so redelivery returns the original run and does not
dispatch again.

App mentions honor configured channel filters. Direct messages are accepted
independently of mention-channel filters when enabled. Bot/self messages,
unsupported subtypes, edits, deletes, malformed events, and unreviewed outer
Slack fields never enter `context.trigger`. Slack approval actions and trigger
events are routed as separate protocol messages, even when their adapters
share one Socket connection.

## Progress boundary

Trigger Progress v1 is operational output for long-lived CLI processes. It
distinguishes readiness, occurrence acceptance (including duplicate), run
start, terminal run status, and safe rejection summaries. Progress never
changes the final workflow result contract and never contains resolved secrets
or rejected payload bodies.

### T4 CLI implementation

`woml run <workflow.woml|directory>` compiles every definition, resolves
symbolic secrets in memory, preflights workflow IDs and webhook routes, starts
the Rust listener, and remains active until SIGINT or SIGTERM. A selected
manual trigger fires once at activation; completing that run does not stop the
runtime. Each subsequent webhook occurrence is admitted and executed
independently through the same Rust authority.

Rust emits Trigger Progress v1 for readiness, acceptance, duplicate
recognition, run start, terminal status, and safe rejection summaries. Bun
validates those messages and formats them for the terminal; it does not infer
run state. After a succeeded terminal message, Bun reads the folded durable
projection and prints the final workflow JSON; this is a presentation read and
does not change Trigger Progress v1. Startup also prints a schema-informed
`curl` example for each webhook. `woml runs get <runId> --state <path>` reads
the same folded durable projection. `woml test <workflow.woml>` retains the
explicit one-manual-run journey and exits after printing its result.

The N-API bridge owns a dedicated Actix runtime thread for each activated local
webhook runtime. Graceful shutdown stops admission and joins that thread. The
public product has no separate `woml serve` command, and source hot reload is
not part of T4.

### T5 production profile

Webhook is publishable after the T5 hardening gate. The gate exercises
concurrent requests, SQLite lock contention, slow clients, malformed HTTP
framing, actual streamed and declared oversize bodies, route/port conflicts,
deduplication, recovery, and host failure. Host startup failure is represented
as a durable attempt plus terminal `WOML_SCRIPT_HOST_CRASHED` failure rather
than leaving an active run behind.

Bearer registration hashes the resolved token once and discards the raw route
credential. Every request hashes the presented token to the same fixed width
before a `subtle` constant-time comparison. Tests and the release verifier scan
progress, SQLite state/WAL files, packaged artifacts, and public output for raw
credentials and idempotency identities. Cross-feature tests admit webhook runs
that execute retry, branch, parallel, approval waiting, and the existing Slack
approval notification journey without changing any of those contracts.

## Frozen fixtures

Reviewed source/model fixtures live under `woml/tests/fixtures/triggers-*`.
Occurrence, ingress, HTTP, Slack, and progress messages live under
`woml/tests/fixtures/trigger-contracts/`. The fixtures explicitly pin new
acceptance, same-payload duplication, duplication after restart,
changed-payload conflict, and corrupt-history rejection.

Any incompatible change to these shapes requires a new contract version rather
than silently changing v1 or Model/Event v7.
