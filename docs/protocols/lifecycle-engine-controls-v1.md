# Lifecycle and Engine Controls v1 Contracts

Status: frozen by LEC0 on 2026-08-11. This document fixes protocol shape;
LEC1 compiles lifecycle source but does not execute it.

## Frozen boundaries

- Compiled Workflow Model v11 stores one optional lifecycle definition outside
  the business DAG.
- Run Event v10 separates `run_outcome_decided` from `run_finalized`.
- Store v11 stores immutable Model v11 definitions and Event v10 streams, plus
  a bounded rebuildable run-summary index. Events remain authoritative.
- Script Runtime Bindings v2 adds a deeply read-only `lifecycle` binding only
  for lifecycle scripts. Normal steps retain Bindings v1.
- Notification Provider Host v2 is informational only. It has no approval
  decision capability, button, interaction message, or callback authority.
- WOML Template v1 accepts literal text and bounded scalar references rooted at
  `context` or `lifecycle`. Secrets are forbidden in message templates.

## Hook order and identity

Source order is `on-start`, the four step hooks, the three outcome hooks, then
`on-complete`. Every hook is a singleton. Actions execute in source order and a
failed action does not suppress later actions.

Hook invocation identity is the SHA-256 identity of:

```text
run_id + hook_id + subject_kind + subject_id
```

An action attempt adds `action_id + attempt`. Lifecycle v1 has one automatic
script attempt; managed operations retain their own reviewed retry policy.

## Business truth and lifecycle health

Business outcome is one of succeeded, failed, or cancelled. Lifecycle health is
independent. A failed notification or observer script creates a warning and
cannot rewrite business truth, branch selection, DAG output, or retry state.
`on-complete` runs after the matching outcome hook. A run is externally terminal
only after `run_finalized`.

## Cancellation race authority

The committed event sequence decides races. Work committed before cancellation
remains committed. After `run_cancellation_requested`, the engine schedules no
new nodes or retries. A failed step that caused fail-fast remains a failure;
engine-cancelled siblings are cancelled, not failed. Waiting approval decisions
are invalidated. Workflow Call and Workflow Start children remain independent.
Fetch and external side effects may be ambiguous and are never described as
rolled back.

## Store v11

Store v11 adds a run-summary projection containing only run ID, workflow ID,
public status, started time, and updated time. It contains no context, result,
failure message, notification text, provider payload, secret, operation key, or
idempotency key. Deleting and rebuilding the summary from events must produce
the same canonical rows.

## Compatibility and staging

Models v1-v10 and Events v1-v9 remain immutable. During LEC1, only source that
declares lifecycle emits Model v11, because the Event v10 Rust authority begins
in LEC2. This deliberate staging keeps existing workflows executable between
phases. LEC2 moves newly admitted definitions onto the v11/v10 boundary.

Public cancellation of pre-v11 runs returns
`WOML_RUN_CONTROL_VERSION_UNSUPPORTED`; it never manufactures a legacy failure.
`context.run`, durable user state, workflow policy `<config>`, remote control
authorization, and cancellation propagation to child workflows remain deferred.
