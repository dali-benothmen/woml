# WOML For-Each Contracts v1

Status: frozen for FE0  
Compiled model: v16  
Run events: v15  
Script-host protocol: v9

This document freezes the interfaces that the TypeScript frontend, Rust core,
Bun script host, durable event fold, terminal UI, and later definition-package
work use to implement `<for-each>`. It does not make loops executable; runtime
implementation begins in FE2.

## Authoring contract

```xml
<for-each
  id="processItems"
  name="Process items"
  description="Normalize every input item."
  items="{{context.steps.loadItems.items}}"
  concurrency="4"
>
  <step id="normalize">
    <script>
      return { value: context.item, index: context.iteration.index };
    </script>
  </step>

  <result value="{{context.steps.normalize}}" />
</for-each>
```

- `id` and `items` are required.
- `items` is exactly one `context.payload...` or earlier
  `context.steps.<id>...` reference. Runtime requires the resolved value to be
  an array.
- `name` and `description` are optional display metadata.
- `concurrency` defaults to `1` and is an integer from `1` through `64`.
- The first body profile accepts `step`, `choose`, `switch`, and `parallel`.
  Expanded reusable steps are ordinary steps before lowering.
- Nested `for-each`, `fork`, and `approval` are rejected in v1.
- One optional `<result value="{{...}}" />` may appear only as the final child.
- A body must contain at least one executable step on every accepted path.

## Context and visibility

For a loop-owned script and its matching step lifecycle action:

- `context.item` is the current JSON item;
- `context.iteration.index` is its stable zero-based position; and
- `context.iteration.total` is the captured input count.

The body can read outer steps that were visible when the loop opened. Body
outputs are local to one iteration and never overwrite outputs from another
iteration. After settlement, only `context.steps.<forEachId>` becomes visible
to the outer workflow. Authored structural IDs remain unique across the full
workflow, including the loop body; visibility isolation does not create a
second authoring namespace.

Without `<result>`, the public loop output is:

```json
{ "total": 3, "succeeded": 3 }
```

With `<result>`, it additionally contains `results`. Result slots always follow
input order, even when iterations finish out of order.

## Compiled Model v16

The root graph represents a loop as an open boundary and one public settlement
node. The `forEach` descriptor owns the immutable body DAG template, normalized
items reference, metadata, concurrency, outer visibility, optional result
reference, and public node identities.

Rust instantiates the body template per item. It must independently reject an
invalid descriptor, cyclic or unreachable body, identity collision, forbidden
nesting, invalid continuation, or visibility escape. Rust never parses WOML or
the original `items` string.

Models v1 through v15 keep their frozen behavior.

## Event v15

The durable loop vocabulary is:

- `for_each_opened`
- `for_each_iteration_started`
- `for_each_iteration_succeeded`
- `for_each_iteration_failed`
- `for_each_iteration_skipped`
- `for_each_succeeded`
- `for_each_failed`
- `for_each_cancelled`

`for_each_opened` captures the item count, canonical array digest, and chosen
concurrency. Iteration events carry `{ forEachId, index }` in the event's
bounded `iteration` scope. Step-attempt, retry, operation, and lifecycle events
for loop-owned work use the same scope rather than encoding iteration meaning
only in a generated node ID.

The event fold is authoritative. Succeeded iterations are not replayed after a
restart. An attempt with a start but no terminal event follows the existing
fail-closed `interrupted` rule. Cancellation stops new admissions, settles
active work, records remaining indexes as skipped, then emits one terminal loop
event before workflow settlement.

Loop-control events never store credentials, secrets, raw context, or item
previews. An iteration result may be stored only through the same bounded,
JSON-safe result policy already used by executable steps.

## Script Host Protocol v9

Protocol v9 keeps the v8 Content-Length framing, multiplexing, capability
supervision, cancellation, and module bindings. Its `execute.context` envelope
adds `item` and `iteration` as a pair:

- both are required for a loop-owned step or matching lifecycle invocation;
- both are forbidden for root workflow invocations;
- `iteration` contains bounded `index` and `total` integers; and
- the Worker exposes both values as deeply read-only JSON.

The schema validates that the two fields occur together. Rust additionally
validates that `index < total` and that their presence matches the owning Model
v16 node instance. Bun executes one invocation; it never admits iterations,
chooses concurrency, or aggregates loop results.

## Frozen limits and policies

| Contract | Decision |
| --- | --- |
| Default concurrency | `1` |
| Maximum authored concurrency | `64` |
| Maximum captured items | `10,000` |
| Iteration identity | `(runId, forEachId, index)` |
| Result ordering | Original input order |
| Failure policy | Fail fast, then settle owned work |
| Recovery | Fold Event v15; never replay succeeded work |
| Empty input | Succeeds immediately with zero counts |
| Nested loop/fork/approval | Rejected in v1 |
| Event store bump | None unless FE5 proves non-event state is required |

## Deferred from FE0 and FE1

- Rust Model v16 deserialization and execution;
- definition-package advancement;
- Bun Worker v9 runtime bindings;
- persistence, folding, inspection, and terminal progress;
- nested loops, approval or fork bodies, streaming iterables, dynamic
  concurrency, unordered aggregates, and continue-on-error authoring.

FE1 accepts and validates the source language but deliberately refuses to
compile a loop into an older executable model. Until FE2 exists, compilation
fails with `WOML_FOR_EACH_EXECUTION_UNAVAILABLE`.
