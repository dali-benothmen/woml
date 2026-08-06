# WOML Parallel Implementation Plan

Status: P0–P7 complete — parallel execution is supported and publishable

## 1. Product Outcome

This milestone adds real concurrent work to WOML.

After it is complete, a workflow author can:

- declare independent work with `<parallel>`;
- limit simultaneous children with `concurrency`;
- choose `fail-fast` or `wait-all` failure handling;
- read every successful child output from `context.steps.<childId>` after the
  join;
- run a parallel group inside an already-selected branch route;
- recover a partially completed group without rerunning successful children;
- inspect durable group-started and group-completed events; and
- receive source-located errors for invalid group structure, sibling
  references, child failures, and cancellation failures.

The acceptance command will be:

```bash
woml run parallel.woml
```

It must execute independent scripts concurrently through the Rust engine and
produce the reviewed JSON result.

## 2. Acceptance Workflow

The first complete fixture will have this shape:

```xml
<workflow version="0.1" id="field-report" name="Build Field Report">
  <triggers>
    <manual id="start" />
  </triggers>

  <steps>
    <step id="loadField">
      <script>
        return {
          fieldId: "field-42"
        };
      </script>
    </step>

    <parallel
      id="fieldData"
      name="Load field data"
      description="Load independent readings"
      concurrency="2"
      on-error="wait-all">
      <step id="loadWeather">
        <script>
          await new Promise(resolve => setTimeout(resolve, 80));
          return {
            fieldId: context.steps.loadField.fieldId,
            temperature: 22
          };
        </script>
      </step>

      <step id="loadSoil">
        <script>
          await new Promise(resolve => setTimeout(resolve, 80));
          return {
            fieldId: context.steps.loadField.fieldId,
            moisture: 41
          };
        </script>
      </step>
    </parallel>

    <step id="buildReport">
      <script>
        return {
          summary: `Weather ${context.steps.loadWeather.temperature}°C, soil ${context.steps.loadSoil.moisture}%`
        };
      </script>
    </step>
  </steps>
</workflow>
```

The reviewed result is:

```json
{
  "summary": "Weather 22°C, soil 41%"
}
```

The event log must prove concurrency structurally: both child
`step_attempt_started` events appear before either child's terminal attempt
event. Wall-clock timing may be measured as a regression datapoint, but timing
alone is not the correctness proof.

### 2.1 Phase summary

| Phase | What changes | Product result |
|---|---|---|
| P0 | Freeze syntax, model v3, event v3, protocol v2, failures, and fixtures. | Every layer targets one reviewed parallel contract. |
| P1 | Teach the WOML frontend to validate `<parallel>`. | Correct groups are understood and invalid groups receive useful source errors. |
| P2 | Lower parallel groups into deterministic model-v3 DAGs. | Markup becomes an engine-ready fork/join graph. |
| P3 | Add durable group events, folding, and recovery. | Group progress survives restarts without mutable authoritative scheduler state. |
| P4 | Add bounded concurrent scheduling and a stable fork-context snapshot. | Independent children genuinely run concurrently and join successfully. |
| P5 | Implement `wait-all`, real `fail-fast`, and Worker cancellation. | Child failures obey the authored policy without pretending cancellation is transactional. |
| P6 | Expose behavior and diagnostics through the packaged CLI. | `woml run parallel.woml` works as a complete user-facing journey. |
| P7 | Harden compatibility, crashes, limits, packaging, and documentation. | Parallel execution becomes a supported WOML feature. |

## 3. Source-Language Contract

### 3.1 `<parallel>`

```xml
<parallel
  id="fieldData"
  name="Load field data"
  description="Load independent readings"
  concurrency="2"
  on-error="wait-all">
  ...
</parallel>
```

| Attribute | Required | Meaning |
|---|---:|---|
| `id` | Yes | Stable structural identity for diagnostics, compiled identities, and events. |
| `name` | No | Human-readable display name. |
| `description` | No | Human-readable description. |
| `concurrency` | No | Positive integer maximum for simultaneously active children; defaults to the child count. |
| `on-error` | No | `fail-fast` or `wait-all`; defaults to `fail-fast`. |

Rules:

- The ID uses the same JavaScript-safe structural-ID grammar as steps and
  branches.
- Step, branch, parallel, and future approval IDs remain in one workflow-wide
  namespace.
- A parallel group contains one or more direct `<step>` children.
- One child is a valid degenerate group. This keeps generated WOML stable when
  a computed list contains one item.
- `concurrency` must not exceed the number of children.
- The current profile accepts only script steps as children because script is
  the only executable step operation.
- A parallel group may appear in root `<steps>` or inside a selected branch
  arm.
- A parallel group cannot directly contain another `<parallel>`, `<branch>`,
  or `<approval>` in this milestone. Multi-step lanes require a future explicit
  `<sequence>` design.
- A branch may follow a parallel group and may reference its child outputs.
- A selected branch arm may place `<parallel>` before its final `<result>`.
- Parallel and approval remain unavailable inside a direct parallel child.

### 3.2 Context and output

- Every child receives the same immutable context view captured immediately
  before the fork.
- A child never observes a sibling output, even when a concurrency cap causes
  that child to start after an earlier sibling finishes.
- Static WOML references from one child to a sibling are compile errors.
- Dynamic JavaScript access to an unavailable sibling remains `undefined` in
  that child's frozen context snapshot; it does not create a dependency.
- Each successful child publishes its own output at
  `context.steps.<childId>`.
- The parallel ID is structural and never becomes a `context.steps` key.
- The group creates no array, object, positional output, implicit `last`, or
  completion-order result.
- After a successful join, all successful child outputs are available to the
  next flow item.

### 3.3 Terminal parallel groups

The current CLI requires one deterministic workflow result. A parallel group
has deliberately no aggregate result, so a root workflow whose final item is
`<parallel>` is rejected in this executable profile.

The author must add a downstream step that constructs the workflow result, or
place the group inside a branch arm whose `<result>` selects a guaranteed child
output.

The compiler must report a stable source-located diagnostic such as:

```text
WOML_PARALLEL_TERMINAL_UNSUPPORTED
```

WOML must never choose the last child to finish because completion order is
nondeterministic. A future explicit workflow-output construct may remove this
restriction without creating an aggregate parallel output.

### 3.4 Error policies

#### `wait-all`

- Every child is scheduled subject to the group concurrency cap.
- A child failure is recorded durably but does not immediately fail the run.
- Remaining and queued children continue.
- After every child reaches a terminal attempt state, the group completes as
  failed when at least one child failed.
- Downstream workflow items do not execute after a failed group.

#### `fail-fast`

- The first observed child failure stops all unstarted children from being
  scheduled.
- Rust sends cancellation requests for every active sibling invocation.
- Already completed children keep their real success or failure.
- A completion that wins the race with cancellation keeps its real outcome.
- A cancellation that wins terminates the isolated Worker and records a
  cancelled attempt.
- Rust waits until every already-started child has a terminal outcome, then
  records the failed group and failed run.
- Unstarted children produce no attempt events and no context output.

Cancellation is not rollback. A script may have performed an external side
effect before its Worker was terminated. A cancelled or interrupted attempt is
never automatically retried in this milestone.

The value `continue` remains reserved. Continuing downstream after a failed
child requires an explicit missing-output/outcome model and is not part of this
plan.

## 4. Compiled Workflow Model v3

Parallel changes the compiled-model shape and must not mutate the frozen v1 or
v2 schemas. Phase P0 creates Compiled Workflow Model v3.

The frozen lowering contract is:

- start node ID: `__woml_parallel__<parallelId>__start`;
- start handler: `engine.parallel-start`;
- join node ID: the public `<parallel id>`;
- join handler: `engine.parallel-join`;
- child edge ID: `<parallelId>:child:<zeroBasedIndex>`;
- child-to-join edge ID: `<parallelId>:join:<zeroBasedIndex>`;
- each group-owned edge carries `parallelId`;
- start-node inputs contain typed `concurrency` and `onError` values;
- every start-to-child edge is unconditional;
- every child has an unconditional edge to the join;
- ordinary sequencing connects the previous item to the start and the public
  join to the following item; and
- the join is control-only and never publishes a user context output.

Illustrative start-node inputs:

```json
{
  "kind": "object",
  "fields": {
    "concurrency": { "kind": "literal", "value": 2 },
    "onError": { "kind": "literal", "value": "wait-all" }
  }
}
```

P0 must freeze the exact model-v3 schema, identities, metadata placement, and
edge tagging before compiler or Rust implementation begins.

Model validation must prove:

- the start and join identities match one public parallel ID;
- group child ordinals are contiguous and preserve document order;
- every child belongs to exactly one fork/join path;
- every child reaches the matching join;
- group routes do not cross or bypass their join;
- `branchId` and `parallelId` edge roles cannot be confused;
- nesting a parallel group inside a selected branch route remains acyclic and
  reachable;
- the concurrency cap and policy are valid; and
- the current CLI terminal node is output-producing rather than a control-only
  parallel join.

Model v1 sequential definitions and model v2 branch definitions remain
immutable and readable.

## 5. Durable Event Contract v3

Run-event schemas v1 and v2 remain immutable. Parallel execution uses
run-event schema v3, selected by compiled-model v3.

The new current event types are:

```text
parallel_group_started
parallel_group_completed
```

The frozen payloads are:

```json
{
  "type": "parallel_group_started",
  "data": {
    "parallelId": "fieldData"
  }
}
```

```json
{
  "type": "parallel_group_completed",
  "data": {
    "parallelId": "fieldData",
    "outcome": "succeeded",
    "failedNodeIds": [],
    "cancelledNodeIds": []
  }
}
```

Failed groups use `outcome: "failed"`. Failed and cancelled node lists are in
compiled document order, never response-arrival order.

`failedNodeIds` contains started children whose terminal outcome was a real
execution failure; it excludes engine-requested cancellations.
`cancelledNodeIds` contains only children for which cancellation won the race.
Children suppressed before attempt start appear in neither list.

Event-v3 rules:

- A group has at most one started event and one completed event.
- The group must be ready before `parallel_group_started` is appended.
- Child attempts may start only after their group-started event.
- No more than `concurrency` child attempts may be active simultaneously.
- One child has at most one attempt because retry remains unavailable.
- A successful group requires every child attempt to succeed.
- A failed `wait-all` group requires every child to have a terminal attempt.
- A failed `fail-fast` group requires every started child to have a terminal
  attempt; unstarted children remain absent.
- The completed event is validated against the immutable compiled definition
  and its earlier child attempt events.
- Folding events derives group state, child outputs, run status, and the set of
  remaining schedulable children.
- Events from different schema versions never mix inside one run.

Event v3 extends the attempt-failure taxonomy with an engine-requested
`cancelled` outcome and extends `run_failed` with a parallel-group failure
scope. The frozen group failure contains:

- `parallelId`;
- `policy`;
- the first observed failing child ID;
- failed child IDs in document order;
- cancelled child IDs in document order; and
- stable code `WOML_PARALLEL_CHILD_FAILED`.

`cancelled`, `interrupted`, `worker_crashed`, and `host_crashed` remain distinct
outcomes. Cancellation requested by Rust is not represented as a script throw.

## 6. Fork Context and Recovery

There is still no authoritative mutable context object.

`parallel_group_started` marks the durable fork point. The context snapshot for
all group children is reconstructed by folding the event prefix before that
event. The runtime may cache the snapshot, but the cache is disposable and must
not become a second persistence authority.

Recovery rules:

- Crash before group start: the pure start operation may run normally.
- Crash after group start but before a child attempt: reconstruct the fork
  context and schedule pending children.
- Crash after some child successes and with no in-flight attempt: preserve
  those outputs and schedule only remaining children, using the original fork
  context.
- Crash with any started external-effect attempt lacking a terminal event:
  apply the existing fail-closed interrupted-attempt policy and fail the run.
- Crash after all successful children but before group completion: append the
  safe, derivable successful group completion.
- Crash after failed children but before group/run failure: derive and append
  the same deterministic group failure without rerunning a child.
- Crash after group completion: never reopen, reorder, or repeat that group.

Successful child outputs are not replayed. A recorded success is final for the
run.

## 7. Script-Host Protocol v2

The existing protocol v1 already supports multiple in-flight execute messages
and out-of-order correlated responses. Those properties are reused.

Real `fail-fast` adds one new protocol shape, so protocol v1 must not be
silently changed. P0 creates script-host protocol v2 with a cancellation
message:

```json
{
  "protocol": "woml.script-host",
  "protocolVersion": 2,
  "messageType": "cancel",
  "invocationId": "inv_load_soil_01",
  "reason": "parallel_fail_fast"
}
```

Protocol-v2 cancellation rules:

- `execute` keeps its v1 correlation and strict JSON contracts.
- Each execute invocation still produces exactly one terminal `completed`
  response.
- If cancellation wins, the response uses failure kind
  `invocation_cancelled` and code `WOML_SCRIPT_CANCELLED`.
- If execution completes first, its real response wins and a later cancel is a
  safe no-op.
- A cancel for an unknown or already-terminal invocation is a safe no-op; it
  must not crash the shared host.
- Worker termination never kills unrelated invocations.
- Host loss remains `host_crashed`, not `cancelled`.
- Content-Length framing, UTF-8 byte accounting, size limits, isolation, and
  secret rules remain unchanged.

The host may run protocol-v2 execute requests for sequential and branch
workflows. Protocol version is independent of compiled-model and run-event
schema versions.

## 8. Rust Scheduling Architecture

Rust remains the only workflow scheduler and persistence authority.

The runtime currently rejects more than one ready node. Parallel replaces that
temporary restriction with a bounded group scheduler:

1. append `parallel_group_started`;
2. reconstruct and cache the fork context;
3. choose pending children in compiled document order;
4. append each attempt-started event serially before dispatch;
5. send up to `concurrency` execute requests without awaiting them one by one;
6. collect correlated completions in their real arrival order;
7. append terminal attempt events through the single event-store authority;
8. schedule another pending child when policy and capacity allow;
9. append the validated group-completed event; and
10. either continue from the join or append the parallel-scoped run failure.

The engine and SQLite store are never held behind a lock across script I/O.
Rust prepares requests and persists starts serially, lets the existing
`ScriptHostClient` futures run concurrently, then serializes terminal event
appends as responses arrive.

Attempt-start order is deterministic within the concurrency window. Terminal
event order is real completion order. Correctness must never depend on which
child finishes first.

The existing sequential and branch paths use the same scheduler, event types,
fold, host client, and handler registry. Parallel does not create a second
execution loop.

## 9. Diagnostics and Public Error Surface

Frontend errors retain the existing stable code, original file, line/column,
message, and optional hint shape.

P0 must freeze codes for at least:

- missing parallel ID;
- empty group;
- duplicate structural ID;
- invalid or excessive `concurrency`;
- invalid `on-error` value;
- unsupported direct child;
- sibling reference;
- terminal root parallel without an explicit workflow result;
- malformed compiled parallel group;
- child failure;
- cancellation failure; and
- inconsistent parallel event history.

The frozen public codes are cataloged in
`docs/protocols/parallel-diagnostics-v0.1.md` and include:

```text
WOML_PARALLEL_EMPTY
WOML_PARALLEL_INVALID_CONCURRENCY
WOML_PARALLEL_INVALID_POLICY
WOML_PARALLEL_CHILD_UNSUPPORTED
WOML_PARALLEL_SIBLING_REFERENCE
WOML_PARALLEL_TERMINAL_UNSUPPORTED
WOML_PARALLEL_LOWERING_NOT_IMPLEMENTED
WOML_PARALLEL_CHILD_FAILED
WOML_SCRIPT_CANCELLED
WOML_PARALLEL_GROUP_INVALID
WOML_PARALLEL_EVENT_HISTORY_INVALID
```

The N-API failure envelope must carry structured `parallelId`, policy,
failed-child IDs, cancelled-child IDs, and primary node ID. The CLI maps a
script failure to the original child `<script>` and a group-contract failure to
the original `<parallel>` opening tag. Identity and reference details must not
be encoded only inside a human message.

Successful CLI stdout contains only final workflow JSON. Diagnostics go only
to stderr with a nonzero exit code.

## 10. Implementation Phases

### P0 — Freeze parallel contracts and reviewed fixtures — complete

Changes:

- Finalize the source rules in `docs/woml-v0.1.md`.
- Freeze the initial direct-step-only parallel profile.
- Freeze the terminal-parallel restriction rather than inventing an aggregate
  result.
- Create `compiled-workflow-model.v3.schema.json` without modifying v1 or v2.
- Freeze the start, child-edge, join, metadata, and `parallelId` shapes.
- Create `run-event.v3.schema.json` and its protocol document.
- Freeze group started/completed events, cancellation attempt failure, and the
  parallel run-failure scope.
- Create `script-host-v2` fixtures and schema rules for cancellation while
  preserving protocol v1.
- Add the source fixture, compiled fixture, success event log, failure-policy
  logs, context snapshots, final result, CLI result, and canonical definition
  hash.
- Review the N-API error details and diagnostic-code catalog.

Result:

Every layer can be reviewed against concrete versioned artifacts before code
changes begin.

Gate:

No parser, compiler, event, scheduler, or host implementation begins until the
model-v3, event-v3, protocol-v2, failure, and fixture contracts are approved
together.

### P1 — Teach the WOML frontend `<parallel>` syntax — complete

Changes:

- Accept `<parallel>` as a recursive flow item in root steps and branch arms.
- Accept only direct `<step>` children in this executable profile.
- Validate ID, metadata, concurrency, policy, child count, namespace, and
  legal placement.
- Reject sibling references and terminal root groups with source-located
  diagnostics.
- Keep nested lanes, approval, retry, services, and designed-only constructs
  explicitly unavailable.
- Add parser/compiler tests for valid one-child and multi-child groups and all
  significant invalid forms.

Result:

WOML understands the parallel authoring language and explains invalid groups,
but does not lower or execute them yet.

Gate:

Sequential and branch fixtures remain byte-for-structure compatible and all
existing frontend tests pass.

### P2 — Lower parallel groups into Compiled Workflow Model v3 — complete

Changes:

- Extend recursive flow lowering with the frozen start/fan-out/join fragment.
- Preserve child document order and deterministic generated identities.
- Add `parallelId` group ownership without changing frozen branch edges.
- Support a parallel fragment inside a selected branch arm.
- Validate fork/join membership, reachability, acyclicity, terminal behavior,
  policy, and concurrency at both TypeScript and Rust boundaries.
- Add deterministic model-v3 fixture and definition-hash tests.
- Keep model-v1 sequential and model-v2 branch definitions readable.

Result:

The reviewed parallel WOML compiles to a deterministic, language-neutral DAG
that Rust accepts structurally while execution remains gated.

Gate:

The generated model deep-equals the reviewed fixture, validates against the
schema in TypeScript and Rust, and changes no frozen v1/v2 artifact.

### P3 — Add durable group events, folding, and recovery — complete

Changes:

- Implement event-v3 group payloads and validation.
- Map compiled model v3 exactly to event schema v3.
- Fold group state exclusively from events.
- Reconstruct the fork context from the event prefix before group start.
- Persist and reopen partial group histories in SQLite.
- Add recovery for safe pre-attempt, partial-success, pre-completion, and
  post-completion crash boundaries.
- Keep in-flight external attempts fail-closed as interrupted.
- Reject duplicate starts/completions, impossible node lists, excessive active
  counts, and mixed event versions.

Result:

Rust can durably describe and recover parallel progress without executing
children concurrently yet.

Gate:

The same event history folds to the same context and group state before and
after SQLite reopen, and successful children are never scheduled again.

### P4 — Execute successful groups concurrently in Rust — complete

Changes:

- Replace the temporary single-ready-node assertion with the bounded parallel
  scheduling loop.
- Launch multiple existing `execute` requests through the already-multiplexed
  Bun host.
- Enforce the group concurrency cap.
- Give every child the reconstructed pre-fork context snapshot.
- Serialize event-store writes while allowing script execution to overlap.
- Complete the join only after every child succeeds.
- Continue into downstream steps with every child output available.
- Prove a one-child group remains valid and trivial.

Result:

Independent scripts genuinely overlap, respect the cap, join once, and feed
their outputs to later workflow steps.

Gate:

The reviewed event log shows multiple attempt starts before the first terminal
attempt, maximum active count never exceeds `concurrency`, and no child can
observe a sibling output.

### P5 — Implement failure policies and cancellation — complete

Changes:

- Separate attempt failure recording from immediate run failure so a parallel
  policy can decide the group outcome.
- Implement `wait-all` scheduling and deterministic multi-failure summaries.
- Implement protocol-v2 cancellation in the Bun host and Rust client.
- Implement `fail-fast` queued-child suppression and active-worker
  cancellation.
- Make cancel/completion races produce exactly one real terminal outcome.
- Append group failure and run failure atomically where required.
- Preserve successful sibling outputs while preventing downstream execution.
- Add crash and host-loss tests during cancellation.

Result:

Both authored policies work as documented, and fail-fast stops further work as
promptly as isolation permits without claiming rollback of external effects.

Gate:

Tests prove wait-all runs every child, fail-fast starts no queued child after
the first observed failure, active siblings receive cancellation, unrelated
Workers survive, and no invocation receives two terminal outcomes.

### P6 — Expose parallel execution through `woml run` — complete

Changes:

- Carry model v3 and event v3 through the existing N-API boundary.
- Extend structured runtime errors with parallel-group details.
- Map group and child failures to original WOML source locations.
- Add black-box CLI tests for success, one-child groups, both error policies,
  invalid concurrency, sibling access, and terminal parallel rejection.
- Verify stdout, stderr, and exit codes.
- Update the clean-package test to run `parallel.woml` with the included Rust
  engine and Bun host.

Result:

Users can run a real concurrent workflow with:

```bash
woml run parallel.woml
```

and receive the reviewed combined result.

Gate:

The packaged command proves real overlap from event ordering, produces only the
reviewed JSON on stdout, and explains failures at the correct source element.

### P7 — Harden, package, and close the milestone — complete

Changes:

- Test concurrency values 1, child count, and invalid values.
- Test one, two, and many children with intentionally out-of-order completion.
- Test parallel at the beginning and middle, inside selected and unselected
  branch routes, and before a downstream branch.
- Test large contexts/results and existing byte limits.
- Test crash recovery before start, after start, after partial success, during
  active work, before completion, and after completion.
- Test fail-fast cancellation races and host/Worker crashes.
- Verify deterministic hashes, model/event/protocol conformance, v1/v2
  compatibility, and branch regressions.
- Run frontend, Rust, N-API, CLI, clean-package, typecheck, Clippy, and timing
  verification.
- Update the parallel plan, Rust integration plan, branch roadmap, and language
  maturity table to mark parallel executable and publishable.

Result:

Parallel execution is a supported WOML product feature rather than an
experimental graph shape.

Gate:

Every verification row below passes in one release build with no skipped
native tests.

Completed proof:

- Frontend-to-Rust tests cover one, two, and four children; concurrency `1`,
  the child count, and a smaller cap; and intentionally out-of-order
  completions without exceeding the authored cap.
- Parallel runs at the beginning and middle of root flow, inside the selected
  branch route, remains absent from an unselected route, and feeds a downstream
  branch.
- Large fork contexts and child results succeed normally and retain the
  existing configured context/result byte-limit failures.
- Durable recovery resumes before/after group start, after partial success,
  before/after group completion, and after run completion without replaying a
  success. An ambiguous active script fails closed; safely derivable missing
  group success/failure events are appended from the immutable history.
- Fail-fast cancellation races, Worker crashes, and host loss retain one real
  terminal outcome and never kill an unrelated invocation.
- The clean installed package executes sequential, branch, and parallel WOML
  using its included native Rust engine and Bun host.
- A local cold-process timing sample on 2026-08-06 produced the reviewed
  parallel JSON after 400.41 ms and exited after 409.56 ms. This is a regression
  datapoint, not a performance guarantee.
- Release verification passed 127 Bun/TypeScript tests and 63 Rust engine
  tests with native integration enabled and no skipped native cases, plus both
  TypeScript typechecks, the optimized package build, the full core build
  check, Clippy with warnings denied for `woml-engine`, and whitespace
  validation.

## 11. Verification Matrix

| Area | Required proof |
|---|---|
| Syntax | Valid groups parse; invalid placement and attributes report original line/column. |
| Lowering | Deterministic model-v3 fork/join identities and canonical hash. |
| Concurrency | Multiple starts precede completion and active count never exceeds the cap. |
| Snapshot | Every child receives the same pre-fork context and never a sibling output. |
| Join | Downstream work starts once and only after a successful group. |
| Context | Child IDs publish outputs; the parallel ID never does. |
| Wait-all | Every child reaches a terminal attempt and any failure fails the group. |
| Fail-fast | Queued work does not start; active Workers are cancelled and drained. |
| Cancellation | Exactly one terminal response; no rollback or automatic retry claim. |
| Events | Started/completed events validate, fold, persist, and reopen deterministically. |
| Recovery | Safe remaining work resumes; ambiguous in-flight effects fail closed. |
| Branch composition | Selected-route groups run; unselected-route groups create no events or effects. |
| Errors | Group and child failures carry stable codes, structured identities, and WOML locations. |
| Compatibility | Model/event v1 and v2 fixtures plus sequential and branch workflows remain unchanged. |
| CLI | `woml run parallel.woml` returns the reviewed JSON and correct exit code. |
| Package | A clean installation contains and executes all required native and Bun components. |

## 12. Explicit Non-Goals

This milestone does not add:

- multi-step lanes or `<sequence>` inside `<parallel>`;
- nested parallel groups;
- branch or approval as a direct parallel child;
- `on-error="continue"` or missing-output wrappers;
- an aggregate value at `context.steps.<parallelId>`;
- implicit last-child or completion-order results;
- parallel iteration or `<for-each>`;
- retries greater than one or idempotency keys;
- workflow-wide or global concurrency controls;
- distributed queues or multi-process workers;
- transactional rollback of script side effects;
- human approval;
- production triggers;
- services, secrets, lifecycle hooks, or engine controls; or
- retirement of the legacy JavaScript chaining SDK.

These remain separate contracts and milestones.

## 13. Open Decisions Carried Forward

Parallel must not silently resolve unrelated architecture questions:

- `context.run` remains unavailable.
- `context.env` and resolved secrets remain unavailable and unpersisted.
- Retry idempotency-key derivation remains unresolved.
- Approval token storage and the HTTP decision endpoint remain separate.
- Engine cancellation of a workflow remains distinct from internal
  fail-fast child cancellation.
- Default production context, result, and frame byte limits remain
  configuration decisions.
- Distributed execution and queue ownership remain out of scope.

If implementation forces any of these decisions, work pauses for an explicit
contract review rather than selecting a default.

## 14. Roadmap After Parallel

After P0–P7, product expansion continues in the existing order:

1. **Human approval — A0/A1 complete** — versioned model/event/HTTP/token
   contracts and frontend validation are complete; A2–A7 add lowering, Rust
   waiting/resolution, recovery, and the HTTP-only product flow defined in
   `WOML Human Approval Implementation Plan.md`.
2. **Retries and idempotency** — idempotency-key derivation, duplicate
   handling, durable retry scheduling, and backoff.
3. **Production triggers** — webhook, schedule, interval, and event triggers
   with complete transport and failure contracts.
4. **Services and capabilities** — HTTP, database, messaging, and registered
   operations without persisting clients or resolved secrets.
5. **Lifecycle and engine controls** — lifecycle hooks, workflow cancellation,
   durable state, and other engine-owned operations.
6. **SDK retirement** — remove the old chaining SDK only after WOML reaches the
   agreed migration parity and users have a supported path.

## 15. Definition of Done

The parallel milestone is complete only when:

- `<parallel>` and all executable attributes are documented and validated;
- the compiler emits the reviewed deterministic model-v3 fork/join DAG;
- Rust runs independent children concurrently with the exact authored cap;
- every child receives the same reconstructed pre-fork context;
- successful child outputs appear only at their step IDs;
- no implicit group result is created;
- `wait-all` and `fail-fast` obey their frozen semantics;
- cancellation has one protocol-v2 terminal outcome and no rollback claim;
- group progress is durable, versioned, foldable, and recoverable;
- ambiguous in-flight attempts still fail closed;
- parallel groups compose correctly with branches;
- errors retain stable codes, structured group identity, and original WOML
  locations;
- model/event/protocol v1 and v2 artifacts remain compatible;
- the clean packaged CLI passes `parallel.woml`; and
- all sequential and branch behavior remains green.
