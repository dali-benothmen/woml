# Compensation and Cleanup v1

**Status:** frozen by CC0; authoring and Model v19 lowering are implemented by CC1. Durable execution begins in CC2.

This contract binds WOML authoring, Compiled Model v19, Run Event v18, Durable Store v17, Script Host Protocol v11, Correction Inspection v1, and operator recovery.

## Authoring

- `<saga id="...">` owns one non-empty forward flow and an optional final `<cleanup>` block.
- A forward `<step>` inside a saga may contain one trailing `<compensate>` with exactly one `<script>`.
- `<compensate>` accepts optional `timeout` and `retry` attributes.
- `<cleanup>` contains one or more sequential `<step>` elements; cleanup step `timeout` and retry attributes use the existing WOML grammars.
- Nested sagas, detached fork branches, corrective waits, corrective approvals, and multi-step compensations are rejected in v1.

## Fixed execution semantics

- Forward failure or cancellation stops new saga work and corrects only proven, successfully completed eligible actions.
- Registration and forward success are one durable transition.
- Compensation order is reverse durable completion sequence, never inferred source order.
- Cleanup always runs after compensation, or immediately after successful forward work, in source order.
- Every eligible corrective action is attempted even when another corrective action fails.
- The original failure or cancellation remains the primary run outcome.
- A committed saga is not reopened by a later failure outside its boundary.

## Exactly-once honesty

Corrective attempts use stable identities derived from the run, saga, forward execution, corrective action, and attempt. A disappearance after a side effect may have occurred is recorded as ambiguous and enters manual recovery; it is never replayed automatically.

### Managed operation ownership

A corrective script is not an ordinary DAG step. Managed-service authorization
matches its run, saga, runtime action ID (including an iteration suffix), active
attempt number, and derived corrective invocation ID against durable history.
The inherited Script Host attempt key is the SHA-256 digest of that invocation
ID; each managed operation key is derived from the attempt key and operation
name. Both keys are verified before dispatch. A retry receives its own exact
attempt identity; an old invocation cannot borrow authority from it.

Operation starts and terminals retain that corrective action and invocation
identity in the existing event shapes. An operation must settle before its
owning corrective attempt can settle. Duplicate transport calls or reused
logical operation keys within an invocation are rejected before another effect.

Cancellation stops forward dispatch but does not deny an active compensation or
cleanup the managed operations needed to settle safely. The existing lock,
capability-handler, infrastructure-policy, and lower-level service checks still
apply. Policy evidence labels these operations as compensation or cleanup;
unsupported policy adapters remain fail-closed.

The existing programmatic lease authority also requires the exact active
corrective invocation. Markup-lock protection for corrective actions resolves
through the enclosing saga's position in the compiled graph: a lock around the
saga remains mandatory, while a lock inside the saga is not implicitly
reacquired. The existing prohibition on nesting a callback lease inside a
markup lock still applies.

## Context

Compensation receives a bounded read-only `context.compensation` binding with `sagaId`, `forStepId`, and `reason`. Cleanup receives `context.cleanup` with `sagaId`, `reason`, and `compensationStatus`. These values are ephemeral Script Host inputs and never mutable authoritative workflow context.

## Recovery

The frozen command direction is `woml recover <runId> --retry <actionId>` or `woml recover <runId> --resolve <actionId> --note <text>`. External resolution records `externally_resolved`; it never fabricates a successful execution event. The command is implemented in CC6.

When recovering an interrupted corrective attempt, the same recovery transaction
first settles each open owned operation, then records interruption and manual
recovery for the action. Unknown effects become ambiguous failures. An existing
immutable StateService mutation receipt can prove that mutation committed, but
cannot prove that the rest of the corrective script completed; the action still
requires manual recovery and is not automatically replayed.

## Limits

The reviewed v1 limits are 64 sagas per workflow, 10,000 compensation registrations per run, 64 cleanup steps per saga, 10 attempts per corrective action, 10,000 unresolved recovery rows per store, 100 inspection rows, a 1 MiB inspection response, and 100 recovery actions per scheduler batch.
