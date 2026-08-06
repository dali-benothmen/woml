# WOML Run Events v1

Status: frozen for the Rust `hello.woml` integration slice

This document is the normative semantic contract for the first Rust run-event
vocabulary and fold. The accompanying JSON Schema is normative for individual
event shapes.

## 1. Authority

The ordered event log is the authoritative record of a workflow run. Run
status, attempt state, and script context are derived projections. A cached
projection may be discarded and rebuilt without changing its meaning.

Events are immutable and append-only. Updating or deleting an earlier event is
not a valid state transition.

## 2. Schema and Envelope

The schema identifier is:

```text
https://cronflow.dev/schemas/run-event/v1
```

Every event contains:

- `eventSchemaVersion`: exactly `1`.
- `eventId`: globally unique event identity.
- `runId`: the run to which the event belongs.
- `sequence`: one-based, monotonically increasing run-local order.
- `occurredAt`: RFC 3339 timestamp.
- `type`: one executable v1 event type.
- `data`: payload selected by `type`.

The pair `(runId, sequence)` is unique. Fold order is `sequence`, never
`occurredAt`.

## 3. Workflow Definition Binding

`run_started` binds the immutable run to one compiled workflow definition.

`definitionHash` is calculated as:

1. Encode the complete Compiled Workflow Model JSON with RFC 8785 JSON
   Canonicalization Scheme (JCS).
2. Hash those UTF-8 bytes with SHA-256.
3. Render `sha256:` followed by 64 lowercase hexadecimal characters.

Example:

```text
sha256:74d4a6799119042d1cdcf2ed3e1e8e30228b3fbb80ad6750c1256ebd335b03ae
```

That example is the hash of the reviewed `hello.compiled.v1.json` fixture.
Compiler provenance and the hash itself are not injected into the semantic
model before hashing.

## 4. Executable Event Types

### 4.1 `run_started`

Must be sequence `1` and the first event for a run. Its data contains
`workflowId`, `definitionHash`, and the strict-JSON trigger object.

Folding it initializes:

```json
{
  "status": "running",
  "context": {
    "trigger": {},
    "steps": {}
  }
}
```

### 4.2 `step_attempt_started`

Records `nodeId`, one-based `attempt`, `invocationId`, and compiled `handler`.
It proves only that the engine began the attempt. It does not prove that the
script executed or that any side effect occurred.

### 4.3 `step_attempt_succeeded`

Records the same attempt identity and strict-JSON `output`. Folding this event
publishes the output at `context.steps.<nodeId>`.

No other event may publish a step output.

### 4.4 `step_attempt_failed`

Records the same attempt identity and one canonical attempt failure. Failed
outputs are never added to context.

### 4.5 `run_succeeded`

Records `terminalNodeId` and strict-JSON `result`, and folds the run to
`succeeded`. It is terminal.

### 4.6 `run_failed`

Records one canonical failure and optional attempt identity, and folds the run
to `failed`. It is terminal.

## 5. Attempt Identity and Legal Ordering

For the first slice an attempt is identified by:

```text
(runId, nodeId, attempt, invocationId)
```

Semantic validation requires:

1. One `run_started` event first.
2. A `step_attempt_started` before its succeeded or failed event.
3. At most one terminal event for an attempt.
4. A node output becomes visible only after success.
5. A run has at most one terminal run event.
6. No execution event follows a terminal run event.

These cross-event rules cannot be expressed completely by the single-event
JSON Schema and must be enforced by Rust plus conformance tests.

## 6. Minimal Fold

Conceptually:

```text
projection = empty

for event in events ordered by sequence:
  run_started:
    status = running
    context.trigger = event.data.trigger
    context.steps = {}

  step_attempt_started:
    attempts[identity] = running

  step_attempt_succeeded:
    attempts[identity] = succeeded
    context.steps[nodeId] = output

  step_attempt_failed:
    attempts[identity] = failed

  run_succeeded:
    status = succeeded
    result = event.data.result

  run_failed:
    status = failed
    failure = event.data.failure
```

The initial in-memory store and the later SQLite store must use the same fold.

## 7. Crash Recovery

After persistence exists, recovery folds all durable events in sequence. A
`step_attempt_started` without a matching succeeded or failed event is
ambiguous. Recovery appends `step_attempt_failed` with failure kind
`interrupted`, then the initial runtime fails the run.

The engine never automatically replays an ambiguous attempt and makes no
exactly-once side-effect guarantee.

If Rust directly observes its Bun child process exit while an invocation is in
flight, it records `host_crashed`. `interrupted` is used when recovery cannot
prove the cause.

## 8. Reserved Vocabulary

The following names are reserved for future reviewed schemas but are rejected
by the executable v1 event schema:

- `step_retry_scheduled`
- `run_paused`
- `run_resumed`
- `branch_selected`
- `parallel_group_started`
- `parallel_group_completed`

Their payloads are not frozen. Making one executable requires explicit schema
review and, when incompatible with existing readers, a new event schema
version.

## 9. Data and Secret Rules

All event data is strict JSON. Non-finite numbers and non-JSON runtime values
are rejected before append.

Resolved secrets must never appear in trigger data, outputs, failure messages,
event data, or diagnostics. Internal event fields do not implicitly become a
public `context.run` contract; `context.run` remains unavailable in the first
slice.

## 10. Conformance

The reviewed hello event log must:

- Validate every event against schema v1.
- Use contiguous sequence values from 1 through 6.
- Bind to the reviewed compiled-model definition hash.
- Start attempts before terminating them.
- Fold step `a` to `{ "x": "World" }`.
- Expose that output before step `b` succeeds.
- Fold the final result to `{ "message": "Hello World" }`.

Separate fixtures prove that `host_crashed` and `interrupted` remain distinct.
Any incompatible envelope, payload, ordering, or fold change requires a
reviewed schema version and migration strategy.
