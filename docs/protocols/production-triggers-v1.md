# WOML Production Trigger Contracts v1

Status: frozen in T0. Webhook frontend lowering is implemented in T1. The Rust
occurrence and run-start authority is implemented in T2; HTTP ingress begins in
T3.

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

## Progress boundary

Trigger Progress v1 is operational output for long-lived CLI processes. It
distinguishes readiness, occurrence acceptance (including duplicate), run
start, terminal run status, and safe rejection summaries. Progress never
changes the final workflow result contract and never contains resolved secrets
or rejected payload bodies.

## Frozen fixtures

Reviewed source/model fixtures live under `woml/tests/fixtures/triggers-*`.
Occurrence, ingress, HTTP, Slack, and progress messages live under
`woml/tests/fixtures/trigger-contracts/`. The fixtures explicitly pin new
acceptance, same-payload duplication, duplication after restart,
changed-payload conflict, and corrupt-history rejection.

Any incompatible change to these shapes requires a new contract version rather
than silently changing v1 or Model/Event v7.
