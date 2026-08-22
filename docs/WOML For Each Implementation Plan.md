# WOML `<for-each>` Implementation Plan

- Status: FE0 through FE8 completed; base `<for-each>` is release-ready
- Target language surface: WOML after v1.0
- Target compiled model: Model v16
- Target run events: Event v15
- Target script-host protocol: Protocol v9

## 1. Product outcome

WOML authors will be able to execute a workflow fragment once for every item
in a runtime array:

```xml
<for-each
  id="organize"
  name="Organize files"
  description="Move every file into the correct folder."
  items="{{context.steps.scan.files}}"
  concurrency="4"
>
  <step id="classify">
    <script>
      return { extension: context.item.ext };
    </script>
  </step>
</for-each>
```

Every iteration is visible, durable, retryable, recoverable, cancellable, and
inspectable. `<for-each>` is not syntax around a hidden JavaScript loop. The
Rust engine owns iteration scheduling and settlement.

The feature is complete when an author can run a real `.woml` file, observe
per-item progress, restart the runtime safely, and consume ordered aggregate
results in a later step.

## 2. Proposed contract for FE0 review

### 2.1 Element and attributes

```xml
<for-each
  id="processOrders"
  name="Process orders"
  description="Validate every imported order."
  items="{{context.steps.loadOrders.orders}}"
  concurrency="4"
>
  ...flow items...
  <result value="{{context.steps.buildOrderResult}}" />
</for-each>
```

| Attribute | Required | Contract |
| --- | :---: | --- |
| `id` | Yes | Stable control-flow identity and `context.steps.<id>` output key. |
| `items` | Yes | Exactly one WOML context reference resolving to an array at runtime. |
| `name` | No | Human-facing name used by terminal output and inspection. |
| `description` | No | Human-facing explanation used by terminal output and inspection. |
| `concurrency` | No | Integer from 1 through 64; defaults to `1`. |

`items` is intentionally named for the author, not for the compiler. `source`,
`collection`, `over`, and `in` are not aliases and must produce helpful unknown-
attribute diagnostics.

The first release does not add `on-error`, `continue`, `break`, filtering, or
unordered-result attributes. One failed item fails the `<for-each>` and the
workflow according to the normal workflow failure contract.

### 2.2 Items expression

`items` must be an exact reference such as:

```xml
items="{{context.payload.orders}}"
items="{{context.steps.loadOrders.rows}}"
```

String interpolation and JavaScript expressions are not accepted. The
frontend validates that the reference is visible before the loop and does not
point to a step defined inside that loop or later in the enclosing route.

At runtime the resolved value must be a JSON array. `null`, objects, strings,
numbers, and missing paths fail before the first iteration with an actionable
`WOML_FOR_EACH_ITEMS_NOT_ARRAY` error. An empty array succeeds immediately.

The initial limits are:

- at most 10,000 items after reference resolution;
- the existing context and transport byte limits continue to apply; and
- at most 64 active iterations for one `<for-each>`.

These numbers remain configurable implementation limits, not new syntax.

### 2.3 Iteration context

Scripts running inside an iteration receive deeply read-only bindings:

```js
context.item
context.iteration.index
context.iteration.total
```

| Binding | Meaning |
| --- | --- |
| `context.item` | The current JSON item. |
| `context.iteration.index` | Zero-based stable input index. |
| `context.iteration.total` | Total captured item count. |

Normal outer bindings remain available:

```js
context.payload
context.run
context.steps.loadOrders
```

Nested step outputs are isolated to the current iteration. Inside iteration 3,
`context.steps.classify` means iteration 3's `classify` result. One iteration
cannot read another iteration's local outputs, even when both happen to have
completed.

After the loop, its internal step IDs are no longer visible. Later workflow
items see only `context.steps.<forEachId>` plus the outer outputs that were
already visible before the loop.

The first release keeps executable IDs unique across the workflow, including
loop bodies, to preserve the current lifecycle filters and source diagnostic
model. Runtime instance identity adds the loop ID and item index without
changing the authored step ID.

### 2.4 Body grammar and composition

The initial public body supports:

- `<step>` and expanded reusable custom steps;
- `<choose>`;
- `<switch>`; and
- `<parallel>`.

A body contains one or more flow items followed by an optional `<result>`. The
existing `<result value="{{context...}}" />` reference syntax is reused; no new
result language is introduced.

Nested `<for-each>`, `<fork>`, and `<approval>` are rejected in the first
release. Their dynamic identity, settlement, and long-wait behavior must not be
improvised inside the first implementation. They remain explicit follow-up
composition work.

`<for-each>` itself is accepted anywhere ordinary sequential flow is accepted,
except inside another `<for-each>` in the first release. This includes the root
`<steps>` route and selected conditional routes that are not already in a loop.

### 2.5 Ordered result contract

Without `<result>`, a successful loop publishes:

```json
{
  "total": 3,
  "succeeded": 3
}
```

With a final `<result>`, the selected reference is resolved separately after
each iteration body succeeds. The loop publishes results in original input
order, never completion order:

```json
{
  "total": 3,
  "succeeded": 3,
  "results": [
    { "file": "a.jpg", "folder": "Images" },
    { "file": "b.pdf", "folder": "Docs" },
    { "file": "c.zip", "folder": "Archives" }
  ]
}
```

The result reference may use outer context, `context.item`, iteration metadata,
and values guaranteed by every possible route through the body. Existing
choice/switch merge rules remain the way to normalize route-specific outputs.

An empty loop with `<result>` publishes `results: []`. A failed loop publishes
no normal downstream result. Partial progress remains inspectable through the
event log, `woml get`, logs, and terminal presentation, but the CLI must not
print a misleading successful aggregate for a failed run.

The aggregate must fit the existing durable result/context size limits. WOML
must fail with an actionable size diagnostic rather than truncating data.

## 3. Execution semantics

### 3.1 Stable item capture

When the loop becomes ready, Rust resolves `items` from the folded run context,
validates it, and commits the loop start before scheduling body work. The
resolved array is deterministic run data derived from immutable events. WOML
records its count and canonical digest so recovery verifies that it is resuming
the same iteration set rather than silently accepting a changed projection.

Input order defines stable iteration identity. Array index is part of every
body-node instance, attempt, capability call, retry, lifecycle event, and
idempotency key.

Conceptually:

```text
processOrders[0].validate
processOrders[0].save
processOrders[1].validate
processOrders[1].save
```

These are runtime instance identities, not authored node IDs and not keys added
to the public `context.steps` object.

### 3.2 Scheduling

`concurrency="1"` executes items sequentially. Higher values allow that many
iteration bodies to be active simultaneously. Steps inside one iteration
still follow their compiled dependencies. A nested `<parallel>` applies its
own child concurrency without changing the outer loop's active-iteration
limit.

The scheduler is work-conserving and starts pending indexes in ascending order.
Completion may occur out of order; aggregate results remain input-ordered.

Workflow-level concurrency, rate limits, queue admission, and timeout continue
to apply to the run. `<for-each concurrency>` limits work inside one admitted
run and does not create independent workflow runs.

### 3.3 Failure and cancellation

The default policy is deterministic fail-fast admission:

1. The first terminal item failure marks the loop as failing.
2. No additional pending iterations are started.
3. Already active attempts receive normal cancellation requests.
4. Started external effects are never claimed to be undone.
5. Ambiguous started attempts retain the existing fail-closed behavior.
6. The loop and run settle only after owned active work reaches a durable
   terminal state.
7. Remaining never-started indexes are recorded as skipped for inspection.

Retries remain step-owned. A retry repeats only the failed step instance in the
same item; it does not restart successful earlier steps or the entire loop.
Stable effect identity includes run ID, loop ID, item index, node ID, logical
operation name, and retry contract as appropriate.

Run cancellation stops new item admission, signals active work, records
remaining indexes as skipped, and follows the existing workflow cancellation
and lifecycle ordering.

### 3.4 Recovery

After a process or machine restart, WOML folds events to recover:

- the captured item count and digest;
- succeeded, failed, active, pending, and skipped indexes;
- each iteration's local step projection;
- ordered result slots;
- scheduled retries and backoff deadlines; and
- loop settlement state.

Succeeded iterations and steps are never replayed. A step attempt that began
without a terminal outcome fails closed as interrupted under the existing
side-effect ambiguity rule. Safe pending iterations may continue.

### 3.5 Lifecycle

`<for-each>` is a control-flow container, not an authored step, so it does not
invent new lifecycle hook names. Existing `on-step-start`, `on-step-success`,
`on-step-failure`, and `on-step-complete` hooks run for executable body steps
in each iteration.

For a body step lifecycle invocation, `context.item` and
`context.iteration` identify the affected item. Lifecycle step filters continue
to use the authored body step ID. Workflow `on-success`, `on-error`, and
`on-complete` run only after the loop and all of its owned work settle.

## 4. Versioned architecture contracts

### 4.1 Compiled Model v16

The TypeScript frontend remains the only WOML parser and compiler. Model v16
adds a versioned loop descriptor containing:

- loop identity, metadata, normalized items reference, and concurrency;
- a validated body DAG template;
- its optional result reference;
- outer-input visibility and iteration-local output visibility; and
- deterministic runtime-instance identity rules.

The enclosing workflow remains a DAG. The root graph treats the loop as one
control-flow region with an entry and settlement boundary. Runtime iteration
instances are derived from the immutable body template; Rust does not mutate
the stored definition or parse WOML.

Rust independently validates descriptor identity, references, limits, body
acyclicity, entry/terminal reachability, forbidden nesting, visibility, and
continuation edges before accepting Model v16.

Models v1 through v15 retain their exact existing behavior.

### 4.2 Event v15

Event v15 inherits Event v14 and adds an explicit loop vocabulary. The final
names and payload schemas are frozen in FE0, but the required facts are:

- loop opened with item count and canonical digest;
- iteration admitted/started;
- iteration succeeded with its optional result;
- iteration failed with the shared failure taxonomy;
- iteration skipped with reason;
- loop succeeded with ordered aggregate identity; and
- loop failed or cancelled after all owned work settles.

Step-attempt, retry, capability, lifecycle, and cancellation events inside the
loop carry a bounded iteration scope rather than encoding meaning into a
string-only node ID. One run never mixes event schema versions.

The projection is authoritative only as a fold of these events. New in-memory
indexes remain disposable caches.

### 4.3 Script Host Protocol v9

Protocol v9 adds optional `item` and `iteration` fields to the execution
context envelope. They are required for loop-body step and matching lifecycle
invocations and forbidden elsewhere. The fields remain JSON-only, bounded,
deeply read-only in the Worker, and covered by the existing framing and size
rules.

Rust still supervises execution. Bun receives the current item; it does not
schedule iterations or aggregate results.

### 4.4 Definition packages and storage

The definition-package contract advances only as required to contain Model
v16 and any associated module artifacts. Durable storage schemas advance only
if the implementation adds serialized indexes that cannot be rebuilt from the
definition and Event v15. A store-version bump must not be used merely because
a new event payload exists.

## 5. Diagnostics

At minimum, the frontend and runtime expose stable codes for:

| Code | Situation |
| --- | --- |
| `WOML_FOR_EACH_ID_REQUIRED` | `id` is missing. |
| `WOML_FOR_EACH_ITEMS_REQUIRED` | `items` is missing. |
| `WOML_FOR_EACH_ITEMS_INVALID` | `items` is not exactly one accepted context reference. |
| `WOML_FOR_EACH_ITEMS_NOT_VISIBLE` | The reference points inside/later than the loop. |
| `WOML_FOR_EACH_CONCURRENCY_INVALID` | Concurrency is not an integer from 1 through 64. |
| `WOML_FOR_EACH_EMPTY` | No executable body item exists. |
| `WOML_FOR_EACH_RESULT_ORDER` | `<result>` is not the final child. |
| `WOML_FOR_EACH_RESULT_INVALID` | Result is not one exact guaranteed reference. |
| `WOML_FOR_EACH_NESTING_UNSUPPORTED` | A forbidden nested loop/fork/approval is present. |
| `WOML_FOR_EACH_ITEMS_NOT_ARRAY` | Runtime value is not an array. |
| `WOML_FOR_EACH_ITEMS_LIMIT` | Resolved item count exceeds the configured limit. |
| `WOML_FOR_EACH_RESULT_TOO_LARGE` | Ordered aggregate exceeds the durable result limit. |

Every authoring diagnostic includes code, message, exact source location, and
an actionable hint. Runtime failures identify loop name/ID, zero-based index,
human-facing item number, nested step, and a bounded redacted preview when safe.
Secret values never appear in previews, events, logs, or inspection.

## 6. Terminal experience

At activation:

```text
  02  ◇  Organize files                                      Queued
          For each · concurrency 4
```

While running:

```text
  02  ↻  Organize files                                      Running
          For each · 18/42 completed · 4 active
```

On success:

```text
  02  ✓  Organize files                                      1.84 s
          For each · 42 items · 42 succeeded · concurrency 4
          → { total: 42, succeeded: 42, results: [...] }
```

On failure:

```text
  02  ✕  Organize files                                      Failed
          Item 17 of 42 · index 16 · step "moveFile"
          WOML_SCRIPT_THREW · Permission denied
```

`woml inspect`, `woml get`, foreground logs, background log following, and JSON
output expose the same durable loop progress without dumping every item by
default. Detailed inspection can show bounded per-index state.

## 7. Reviewed acceptance workflow

The primary fixture scans an input array, classifies every item, uses a
`<switch>` inside the loop, returns one normalized per-item result, and builds a
summary afterward:

```xml
<woml>
  <workflow
    id="organize-files"
    name="Organize files"
    description="Classify every file and report its destination."
    version="1.0.0"
  >
    <triggers>
      <manual id="start" />
    </triggers>

    <steps>
      <step id="scan" name="Scan files">
        <script>
          return {
            files: context.payload.files ?? [
              { name: "photo.jpg", category: "image" },
              { name: "notes.txt", category: "document" },
              { name: "backup.zip", category: "archive" }
            ]
          };
        </script>
      </step>

      <for-each
        id="organize"
        name="Organize files"
        items="{{context.steps.scan.files}}"
        concurrency="2"
      >
        <switch id="destination" value="{{context.item.category}}">
          <case value="image">
            <step id="imageDestination">
              <script>
                return { file: context.item.name, folder: "Images" };
              </script>
            </step>
            <result value="{{context.steps.imageDestination}}" />
          </case>

          <case value="document">
            <step id="documentDestination">
              <script>
                return { file: context.item.name, folder: "Docs" };
              </script>
            </step>
            <result value="{{context.steps.documentDestination}}" />
          </case>

          <default>
            <step id="otherDestination">
              <script>
                return { file: context.item.name, folder: "Misc" };
              </script>
            </step>
            <result value="{{context.steps.otherDestination}}" />
          </default>
        </switch>

        <result value="{{context.steps.destination}}" />
      </for-each>

      <step id="summary" name="Build summary">
        <script>
          return {
            message: `Organized ${context.steps.organize.total} files.`,
            files: context.steps.organize.results
          };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The reviewed fixture set must also cover:

- payload and prior-step item references;
- empty, one-item, and maximum-bound arrays;
- primitive, array, object, and null items;
- omitted and explicit concurrency;
- out-of-order completion with input-ordered results;
- choice, switch, parallel, and reusable-step bodies;
- local visibility and forbidden downstream internal references;
- retry of one nested step without replaying completed work;
- failure before scheduling all indexes;
- cancellation with pending and active items;
- restart after partial completion and during retry backoff;
- lifecycle hooks with correct item/iteration bindings;
- native Fetch and managed services using iteration-scoped idempotency;
- aggregate and item size failures;
- secret redaction and hostile diagnostic values; and
- unchanged execution of Models v1 through v15 and Events v1 through v14.

## 8. Implementation phases

Phase labels are planning identifiers only. Production test filenames and
package scripts use descriptive names such as `for-each-runtime.test.ts` and
`for_each_recovery.rs`, never opaque phase prefixes.

### FE0 — Freeze contracts and fixtures

Write and review Model v16, Event v15, and Script Host Protocol v9 schemas;
freeze the grammar, limits, instance identity, context visibility, result
shape, failure settlement, lifecycle timing, and representative source/model/
event fixtures.

**Result:** every layer has one reviewed contract and no implementation phase
must invent observable behavior.

### FE1 — Add parser and authoring validation

Teach the TypeScript frontend to recognize `<for-each>`, validate `id`, `items`,
metadata, concurrency, body grammar, optional final result, references,
namespace rules, forbidden nesting, and exact source diagnostics. Update the
language service/TextMate grammar and authoring types where required.

**Result:** `woml check` accepts valid loops and rejects invalid ones with
precise line/column diagnostics, without executing anything.

### FE2 — Lower to Compiled Model v16

Lower the authored loop into one deterministic body template and enclosing DAG
boundaries. Emit normalized references, visibility, metadata, concurrency, and
result contracts. Add Rust Model v16 deserialization and independent structural
validation.

**Result:** TypeScript and Rust accept the same frozen Model v16 fixture while
older model fixtures remain unchanged.

### FE3 — Add iteration bindings and sequential execution

Implement Script Host Protocol v9, deeply read-only `context.item` and
`context.iteration`, runtime instance identity, loop opening, empty-array
settlement, and sequential `concurrency="1"` execution through the real Rust
engine and Bun Worker host.

**Result:** a simple loop of sequential script steps executes durably and later
workflow steps can consume its summary/result.

Completed in FE3:

- Script Host Protocol v9 carries paired, deeply read-only `context.item` and
  `context.iteration` bindings;
- Rust opens and executes `concurrency="1"` script-only body DAGs in stable
  input order through the real Bun Worker host;
- Event v15 records loop opening, index-scoped iteration progress, and
  successful settlement, and SQLite reopen reproduces the same projection;
- ordered `<result>` values become `context.steps.<forEachId>.results` for
  downstream steps; and
- an empty array settles immediately without creating an iteration.

Bounded concurrency and complete failed/cancelled recovery remain assigned to
FE5 and FE6.

### FE4 — Compose control flow, modules, services, retries, and lifecycle

Execute choose, switch, parallel, and expanded reusable steps inside an
iteration. Carry iteration identity through attempts, services, Fetch
observations, idempotency, retry scheduling, lifecycle hooks, errors, and source
maps.

**Result:** the reviewed organization workflow and realistic service-backed
loops work without losing per-item identity or existing durability guarantees.

Completed in FE4:

- each iteration executes its compiled local DAG with `<choose>`, `<switch>`,
  and `<parallel>` selection/join semantics; bounded concurrent scheduling is
  still owned by FE5;
- expanded reusable WOML steps and imported JavaScript/TypeScript modules run
  with iteration-aware props, module bindings, source artifacts, and Model v16
  Definition Package v11;
- every nested attempt, retry schedule, stable idempotency key, managed service
  call, and observed native Fetch call carries the loop ID and input index;
- generic step lifecycle hooks and reusable-step lifecycle hooks receive
  `context.item` and `context.iteration` and persist their outcomes inside the
  iteration scope; and
- the end-to-end acceptance workflow proves conditionals, a switch, parallel
  boundaries, retry, modules, Fetch, cache service calls, lifecycle hooks, and
  ordered aggregation through the real Rust engine and Bun Worker host.

### FE5 — Add bounded concurrent iteration scheduling

Honor `concurrency`, start indexes fairly in input order, permit out-of-order
completion, preserve ordered aggregation, and compose outer-loop concurrency
with inner parallel limits, workflow deadlines, rate limits, and cancellation.

**Result:** large independent batches execute concurrently while producing
deterministic output.

Completed in FE5:

- Rust admits item indexes in ascending input order and keeps at most the
  authored `concurrency` number of iteration bodies active;
- the multiplexed Bun host executes independent iteration scripts concurrently,
  while the Rust engine remains the single serialized event and projection
  authority;
- iterations may finish out of order, but result slots are aggregated by their
  stable input index before downstream steps receive the loop output;
- direct children of an inner `<parallel>` use that group's own concurrency
  limit without changing the outer loop's active-iteration limit;
- workflow concurrency, queue, and rate-limit policies continue to govern run
  admission, while workflow deadlines and cancellation stop new item admission
  and signal active Bun invocations; and
- focused Rust tests and the real native CLI composition test cover bounded
  scheduling, inner parallel execution, modules, services, retries, lifecycle
  hooks, control flow, and deterministic aggregation.

Complete skipped-index recording, terminal failed/cancelled loop settlement,
and restart recovery remain assigned to FE6.

### FE6 — Complete events, failure settlement, and recovery

Persist and fold the Event v15 vocabulary; implement partial-progress recovery,
active-attempt interruption, retry-backoff recovery, pending-index skipping,
failed/cancelled settlement, SQLite reopen, backup/restore, retention, and
projection rebuild tests.

**Result:** crashes, restarts, failures, cancellation, backup, and retention do
not replay completed effects or leave runs permanently unsettled.

Completed in FE6:

- Event v15 now folds loop-owned attempts, retry schedules, managed operations,
  lifecycle actions, inner parallel groups, per-index outcomes, skipped indexes,
  and exactly one terminal loop settlement as authoritative durable history;
- recovery resumes indexes whose completed effects are already durable without
  replaying them, waits for an existing retry schedule, and starts only its
  recorded next attempt;
- a started loop-owned effect with no terminal event fails closed as
  interrupted, while never-started indexes are recorded as skipped before the
  loop and run settle;
- ordinary failures, operator cancellation, and total workflow timeout close
  active iterations and inner work before deciding the workflow outcome;
- the runtime-policy scheduler detects ambiguous loop-owned work after lease
  loss and routes it through the same run-scoped recovery authority; and
- focused SQLite tests prove reopen, projection rebuild, online backup and
  restore, retention, cancellation, timeout, retry-backoff, partial-progress,
  and fail-closed recovery behavior.

### FE7 — Add terminal presentation and operations

Add loop-aware progress to foreground execution, background logs, inspection,
run retrieval, structured JSON, metrics, and failure diagnostics. Keep default
output concise while permitting bounded per-item detail.

**Result:** users can understand the loop, its progress, failed item, duration,
and result without reading engine events.

Completed in FE7:

- Run Presentation v1 exposes `<for-each>` as one authored workflow step with
  its name, description, status, duration, aggregate result, concurrency, and
  concise durable counts rather than leaking internal loop nodes;
- foreground execution and background presentation receive live count-only
  `for_each_progress` updates while terminal output stays concise;
- failed loop presentation identifies the human item number, zero-based index,
  failed nested step, and safe failure code/message;
- machine presentation includes at most 100 per-index status summaries and
  marks truncation, while item values and secrets never enter the operations
  snapshot or safe run inspection;
- `woml get` uses Run Inspection v6 for Model v16 runs and reports durable loop
  counts in both human and JSON formats;
- `woml inspect` shows the current loop and completed/active/pending counts;
- Prometheus output includes active, pending, and completed iteration metrics;
  and
- focused Rust and CLI integration tests prove successful and failed
  presentation, strict decoding, redaction, real workflow execution, and
  consistent retrieval through the native boundary.

### FE8 — Harden, document, benchmark, and release

Add conformance gates, compatibility tests, fuzz/property tests for dynamic
identity and aggregation, security/redaction tests, small/large/concurrent
benchmarks, the manual example, language/API documentation, VS Code snippets,
packaged-release checks, and cross-platform CI coverage.

**Result:** `<for-each>` is a documented, packaged, cross-platform product
feature ready for ordinary WOML authors.

Completed in FE8:

- generated-workflow property coverage proves deterministic Model v16 lowering,
  collision-free loop identities, bounded concurrency values, and precise
  rejection of malformed or visibility-escaping item references;
- Rust stress and recovery coverage proves unique dynamic iteration identity,
  input-ordered aggregation under concurrent completion, SQLite reopen,
  fail-closed recovery, cancellation, timeout, and retry continuity;
- presentation hardening rejects raw item data, inconsistent counts, invalid
  item identities, and unbounded detail while redacting credentials from human
  and machine diagnostics;
- the language reference, complete API guide, npm README, repository README,
  example catalog, AI-agent skill, and VS Code extension now teach the released
  `<for-each>` contract and its iteration-only context bindings;
- `examples/forEachWorkflow.woml` is the copy-paste manual workflow and the
  editor ships a `woml-for-each` snippet for the same supported shape;
- a clean-consumer test installs the local npm package family, validates the
  documented example, and executes its ordered result through the Rust engine;
- the release-candidate smoke now compiles and executes a concurrent loop on
  every supported Linux, macOS, and Windows native target before artifacts can
  be collected; and
- versioned small, large, and concurrent CLI benchmarks enforce generous
  regression budgets without turning release CI into an endurance test.

## 9. Phase acceptance gates

No phase is complete merely because its happy-path unit test passes.

1. Schema artifacts are versioned and validate reviewed fixtures.
2. TypeScript and Rust reject malformed models independently.
3. Every durable history reopens to the same projection.
4. Concurrent completion never changes result order.
5. No completed nested effect is replayed after restart.
6. Item values and secrets are bounded and redacted in diagnostics.
7. Cancellation and failure always reach a terminal run state.
8. Existing workflow/model/event fixtures retain byte-for-byte behavior where
   their frozen contract requires it.
9. `woml check`, `woml run`, `woml inspect`, `woml get`, and packaged binaries
   agree on the feature.
10. Full TypeScript tests, Rust tests, type checking, formatting, Clippy, docs
    verification, and cross-platform release gates pass.

## 10. Explicitly postponed extensions

The first implementation does not silently resolve these future features:

- nested `<for-each>` loops;
- `<fork>` or `<approval>` inside an iteration;
- object/map iteration with key/value bindings;
- pagination or async-stream iteration;
- `continue`, `break`, filtering, reduction, or unordered output;
- configurable continue-on-error and partial-success contracts;
- per-item priority or delayed scheduling;
- distributed iteration workers; and
- automatic compensation for external side effects.

They can be added through reviewed additive language/model/event versions after
the base loop is proven in real workflows.

## 11. Roadmap after `<for-each>`

1. **v1.0 stabilization and product feedback** — validate the complete
   authoring, runtime, editor, packaging, documentation, and provider experience
   with real workflow authors.
2. **Provider extension architecture** — make third-party communication
   providers installable without expanding the built-in core for every vendor.
3. **Performance profiling and optimization** — optimize measured startup,
   compilation, serialization, worker-host, large-workflow, and concurrency
   bottlenecks; keep Rust `quick-xml` parsing only as a benchmark-gated future
   investigation.
4. **Advanced iteration composition by demand** — evaluate nested loops,
   long-wait approvals, forks, and partial-success policies only after the base
   iteration model has real-world evidence.

This roadmap is directional. It does not make postponed syntax valid before
its contracts and runtime are implemented.
