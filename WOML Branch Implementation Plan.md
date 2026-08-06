# WOML Branch Implementation Plan

Status: Proposed — ready for review before implementation

## 1. Product Outcome

This milestone turns WOML from a sequential workflow format into a workflow
language that can make decisions.

After this plan is complete, a workflow author can:

- declare a conditional flow with `<branch>`;
- evaluate ordered `<when>` cases using typed WOML context references;
- provide an `<otherwise>` fallback;
- run only the selected route;
- expose one stable branch result at `context.steps.<branchId>`;
- use that result in every step after the branch;
- inspect a durable `branch_selected` event to know which route ran; and
- receive source-located validation and runtime errors when a condition or
  reference is invalid.

The acceptance command remains:

```bash
woml run branch.woml
```

The command succeeds only when the selected route and the downstream step
produce the reviewed JSON result.

## 2. Acceptance Workflow

The first complete branch fixture will have this shape:

```xml
<workflow version="0.1" id="review-content" name="Review Content">
  <triggers>
    <manual id="start" />
  </triggers>

  <steps>
    <step id="checkContent">
      <script>
        return {
          needsReview: true
        };
      </script>
    </step>

    <branch
      id="decision"
      name="Choose review route"
      description="Send uncertain content to review">
      <when test="{{context.steps.checkContent.needsReview}}">
        <step id="reviewContent">
          <script>
            return {
              status: "reviewed",
              accepted: true
            };
          </script>
        </step>

        <result value="{{context.steps.reviewContent}}" />
      </when>

      <otherwise>
        <step id="acceptContent">
          <script>
            return {
              status: "accepted-automatically",
              accepted: true
            };
          </script>
        </step>

        <result value="{{context.steps.acceptContent}}" />
      </otherwise>
    </branch>

    <step id="publishDecision">
      <script>
        return {
          message: `Final status: ${context.steps.decision.status}`
        };
      </script>
    </step>
  </steps>
</workflow>
```

For this fixture, WOML executes `checkContent`, `reviewContent`, and
`publishDecision`. It does not execute `acceptContent`.

The selected `<result>` publishes the complete `reviewContent` value at:

```js
context.steps.decision
```

This makes downstream code independent of which route produced the value.

### 2.1 Phase summary

| Phase | What changes | Product result |
|---|---|---|
| B0 | Freeze syntax, compiled-model v2, event v2, errors, and fixtures. | Everyone builds against one reviewed branch contract. |
| B1 | Add the four tags, typed reference parsing, and validation to the WOML frontend. | WOML understands correct branches and clearly rejects incorrect ones. |
| B2 | Compile branches into a deterministic DAG that Rust can validate. | Markup becomes a complete engine-ready branch model. |
| B3 | Add `branch_selected`, folding, persistence, and recovery. | The selected route survives restarts and cannot silently change. |
| B4 | Add conditional scheduling and merged-result publication in Rust. | Exactly one route executes and later steps receive `context.steps.<branchId>`. |
| B5 | Connect errors and behavior through the packaged CLI. | `woml run branch.woml` works as a user-facing command. |
| B6 | Test nesting, crashes, packaging, compatibility, and regressions. | Branching becomes a supported WOML feature ready for the next milestone. |

## 3. Language Contract

### 3.1 `<branch>`

```xml
<branch
  id="decision"
  name="Choose route"
  description="Select one mutually exclusive route">
  ...
</branch>
```

| Attribute | Required | Meaning |
|---|---:|---|
| `id` | Yes | Stable structural ID and the key used for the merged result in `context.steps`. |
| `name` | No | Human-readable display name. |
| `description` | No | Human-readable description. |

Rules:

- The ID uses the same JavaScript-safe grammar as step IDs.
- Step, branch, parallel, and approval IDs share one workflow-wide namespace.
- A branch contains one or more `<when>` cases followed by exactly one
  `<otherwise>` in the first executable branch profile.
- Cases are checked in document order.
- The first `<when>` whose test is `true` is selected.
- At most one route executes.
- The branch completes only after its selected route and result complete.
- The branch ID becomes a `context.steps` key only after its result is
  published successfully.

Requiring `<otherwise>` in the first executable profile guarantees that every
successful branch selects a route and can produce a downstream result. A later
language profile may add control-only branches, but it must define their output
and no-match behavior explicitly.

### 3.2 `<when>`

```xml
<when test="{{context.steps.someDecision}}">
  ...
</when>
```

Rules:

- `test` is required.
- `test` contains exactly one WOML context reference.
- Arbitrary JavaScript, mixed text, operators, function calls, and optional
  chaining are invalid inside `test`.
- The reference must point to a step or completed branch that structurally
  dominates this branch.
- The runtime value must be the JSON boolean `true` or `false`.
- WOML does not coerce strings, numbers, objects, arrays, or null to booleans.

The frozen reference:

```xml
{{context.steps.checkContent.needsReview}}
```

lowers to the language-neutral compiled expression:

```json
{
  "kind": "contextReference",
  "path": ["steps", "checkContent", "needsReview"]
}
```

### 3.3 `<otherwise>`

`<otherwise>` has no attributes. It must be the final case and is selected only
when every preceding `<when>` evaluates to `false`.

### 3.4 `<result>`

Each `<when>` and `<otherwise>` arm contains exactly one `<result>` as its final
child:

```xml
<result value="{{context.steps.reviewContent}}" />
```

Rules:

- `value` is required.
- The first executable profile accepts exactly one typed context reference.
- The referenced value must be available on the selected route before the
  result is evaluated.
- The referenced value must be JSON-compatible.
- The successful value is published at `context.steps.<branchId>`.
- Outputs from steps in the unselected route do not exist in context.
- A downstream step references the branch ID, never a route-specific step, when
  it needs a value guaranteed across every route.

The result operation is pure engine-owned derivation. It does not run through
Bun and it cannot perform external side effects.

## 4. Execution Semantics

Branching uses the existing compiled DAG rather than creating a WOML-specific
execution path inside Rust.

The TypeScript compiler lowers a branch into:

1. an engine-owned branch-selection node;
2. one conditional outgoing edge for every `<when>`;
3. one fallback edge for `<otherwise>`;
4. the ordinary script nodes inside each route;
5. an engine-owned result/join node whose public node ID is the branch ID; and
6. an unconditional edge from that result node to the next workflow item.

Compiler-generated selector and edge IDs use a reserved internal namespace
that cannot collide with user-authored JavaScript-safe IDs. Arm identities are
derived only from the branch ID and case ordinal. Names, source locations, and
timestamps never become durable identities.

The branch milestone introduces Compiled Workflow Model v2. Model v1 remains
immutable and readable for existing sequential definitions. Model v2 is a
superset that reuses the existing language-neutral primitives:

- `ContextReferenceExpression` for tests and result values;
- typed `CompiledWorkflowEdge.condition` values;
- `CompiledWorkflowEdge.branchId` for exclusive edge groups;
- ordinary DAG nodes for route steps; and
- a registered engine handler for the pure branch result.

The exact proposed lowering is:

- selector node ID: `__woml_branch__<branchId>__select`;
- selector handler: `engine.branch-select`;
- arm edge ID and durable arm ID: `<branchId>:when:<zeroBasedIndex>` or
  `<branchId>:otherwise`;
- every arm edge carries the public `branchId`;
- `<when>` edges use a v2 `boolean` condition that rejects non-boolean values;
- the `<otherwise>` edge uses `always` and is ordered last;
- result/join node ID: the public branch ID;
- result/join handler: `engine.branch-result`; and
- result/join inputs: an object keyed by durable arm ID, where each value is
  that arm's compiled `<result>` expression.

User-authored IDs cannot start with the reserved selector prefix or contain the
colon used by generated arm IDs, so generated identities cannot collide with
the shared structural namespace.

The compiler emits model v2 for WOML definitions after this milestone. Rust
continues to accept historical model-v1 definitions. The workflow's public
`version="..."` attribute remains unrelated to this internal `schemaVersion`.

The core continues to receive only the compiled model. It never parses WOML,
`{{...}}`, XML, or JavaScript source-level references.

### 4.1 Route selection

When the selector node becomes ready, Rust:

1. resolves `<when>` references against the event-derived context;
2. checks cases in their compiled order;
3. requires each evaluated value to be a JSON boolean;
4. selects the first `true` case or the fallback;
5. appends `branch_selected`; and
6. makes only the selected edge active.

Unselected edges are durably resolved as inactive. Their nodes are not failed,
attempted, or added to context.

### 4.2 Joining and publishing the result

The result/join node becomes ready when the selected route finishes. Incoming
edges from unselected routes do not block the join.

Rust resolves the selected arm's result expression and publishes it under the
branch ID. Publication uses the existing successful-output contract so folding
the run produces:

```js
context.steps.decision
```

The `engine.branch-result` node uses the normal `step_attempt_started` and
`step_attempt_succeeded` event shapes, but Rust executes it in-process rather
than sending it to Bun. In durable mode, the pure evaluation and its two event
appends are one atomic transaction. There must not be a crash window that turns
safe result derivation into an ambiguous external side effect.

### 4.3 Nested branches

The compiler must lower flow items recursively so a `<when>` or `<otherwise>`
can contain another `<branch>`. Parallel and approval items remain rejected
until their own executable milestones.

Each nested branch owns its selection and result independently. Its result may
be used later in the same selected parent route.

## 5. Durable Event Contract

`branch_selected` becomes a durable event rather than temporary scheduler
state. Its proposed payload is:

```json
{
  "eventSchemaVersion": 2,
  "eventId": "event-branch-decision",
  "runId": "run-example",
  "sequence": 4,
  "occurredAt": "2026-01-01T00:00:01Z",
  "type": "branch_selected",
  "data": {
    "branchId": "decision",
    "armId": "decision:when:0"
  }
}
```

The event records identity, not duplicated condition data. The selected value
already exists in earlier successful-output events and can be reconstructed.

Event rules:

- A branch may have only one `branch_selected` event in one run.
- `branchId` must exist in the immutable compiled definition.
- `armId` must belong to that branch.
- The branch selector must be ready when the event is appended.
- Folding the event records the selected arm in the disposable projection.
- Recovery uses the recorded arm and never chooses a different route.
- A crash before the event may safely re-evaluate the pure condition.
- A crash after the event resumes from the recorded selection.
- A crash during a selected script attempt keeps the existing fail-closed
  interrupted-attempt policy.

Run-event schema v1 remains immutable. Phase B0 creates run-event schema v2,
which contains the existing event shapes plus `branch_selected`. Rust must keep
reading v1 sequential histories while new runs use v2. Events from different
schema versions must not be mixed inside one run.

Schema v2 also separates workflow-level engine failures from attempt failures.
`step_attempt_failed` continues to accept only the existing attempt-failure
taxonomy. `run_failed` accepts a versioned workflow-failure union containing:

- an attempt failure that terminated the run; or
- a branch-evaluation failure such as a non-boolean test or unavailable
  reference, including `branchId`, optional `armId`, and the compiled reference
  path.

This keeps one coherent failure contract without pretending that Rust condition
evaluation was a failed Bun script attempt.

## 6. Validation and Error Contract

All frontend errors keep the existing shape: stable code, original file,
line/column, message, and an optional corrective hint.

The branch milestone adds errors for:

- missing or duplicate branch IDs;
- branch IDs colliding with any structural ID;
- missing `<when>`;
- missing, duplicated, or misplaced `<otherwise>`;
- missing, duplicated, or misplaced `<result>`;
- unknown elements or attributes inside a branch arm;
- malformed `{{context...}}` syntax;
- references to unknown IDs;
- forward references and references that do not dominate the consumer;
- result references unavailable on every path to that result;
- a non-boolean runtime condition;
- a missing nested property at runtime; and
- malformed compiled branch groups or event histories at the Rust boundary.

Recommended stable runtime codes:

- `WOML_BRANCH_TEST_NOT_BOOLEAN`
- `WOML_REFERENCE_NOT_AVAILABLE`
- `WOML_BRANCH_SELECTION_INVALID`

The exact frontend code catalog and event payload are frozen in Phase B0 before
implementation code uses them.

## 7. Implementation Phases

### B0 — Freeze branch contracts and reviewed fixtures

Changes:

- Update `docs/woml-v0.1.md` with the executable `<branch>`, `<when>`,
  `<otherwise>`, and `<result>` rules.
- Freeze `context.steps.<branchId>` as the branch result location.
- Freeze generated selector, arm, and edge identity rules.
- Add and review `docs/schemas/compiled-workflow-model.v2.schema.json` without
  mutating model v1.
- Freeze the compiled DAG representation and strict v2 `boolean` edge
  condition.
- Add and review `docs/schemas/run-event.v2.schema.json`.
- Freeze the `branch_selected` payload and v1/v2 compatibility rules.
- Freeze the schema-v2 workflow-failure union for branch evaluation errors.
- Add a source fixture, compiled fixture, successful event log, expected
  context snapshots, and expected final result.
- Freeze frontend and runtime diagnostic codes used by the fixture.

Result:

The syntax, compiled graph, durable event, context result, and expected CLI
output can be reviewed without executable code. Every later phase targets the
same artifacts.

Gate:

No compiler or runtime implementation begins until the B0 fixtures and schemas
are approved together.

### B1 — Teach the WOML frontend the branch syntax

Changes:

- Extend `woml/src/compiler.ts` from flat step validation to recursive flow-item
  validation.
- Accept `<branch>`, `<when>`, `<otherwise>`, and `<result>` in their legal
  locations.
- Add a dedicated exact-reference parser for `test` and `value` attributes.
- Validate structural IDs across steps and branches in one namespace.
- Validate document order, arm structure, result placement, references, and
  source locations.
- Add parser/compiler tests for valid branches and every important invalid form.

Reuse:

- `woml/src/parser.ts` already preserves ordered elements, raw scripts, and
  source spans.
- `WomlValidationError` and `WomlCompileError` already provide the public error
  surface.
- `ContextReferenceExpression` already exists in the compiled-model types.

Result:

WOML can understand and validate the branch language. Invalid files fail with
useful source-located errors. Rust execution is not enabled yet.

### B2 — Lower branches into the compiled DAG

Changes:

- Replace the current linear `lowerNodes` and edge construction with recursive
  flow lowering.
- Generate the selector node, conditional route edges, fallback edge, route
  subgraphs, and result/join node.
- Preserve first-match document order in deterministic compiled identities.
- Prove the lowered graph is acyclic, reachable, and has one terminal outcome.
- Validate reference dominance across the outer flow, selected arms, nested
  branches, and downstream consumers.
- Extend the compiled-model conformance fixtures and TypeScript graph tests.
- Add model-v2 TypeScript/Rust types and update Rust validation to accept only
  the newly frozen branch shapes.
- Keep historical model-v1 sequential definitions readable while continuing to
  reject parallel, approval, retry, and other staged constructs.

Reuse:

- `woml/src/model.ts` already models typed value expressions, conditional
  edges, branch IDs, and a DAG.
- `inspectCompiledWorkflowGraph` already checks IDs, reachability, cycles, and
  terminal-node count.
- `core/woml-engine/src/model.rs` already deserializes the same expressions and
  edge conditions.

Result:

The reviewed branch WOML fixture compiles deterministically into the reviewed
language-neutral DAG, and Rust accepts that DAG as structurally executable.

### B3 — Add durable branch selection and folding

Changes:

- Add run-event schema v2 without mutating schema v1.
- Add `BranchSelectedData` and `RunEventPayload::BranchSelected` in
  `core/woml-engine/src/event.rs`.
- Add the workflow-failure union used by schema-v2 `run_failed`, while keeping
  `step_attempt_failed` restricted to attempt failures.
- Extend event validation against the immutable workflow definition.
- Extend `RunProjection` with selected branch arms derived only by folding
  events.
- Teach in-memory and SQLite event stores to preserve and reload v2 histories.
- Keep v1 sequential histories readable and reject mixed-version histories.
- Add conformance tests for duplicate selection, unknown branch/arm identities,
  invalid ordering, SQLite reopen, and recovery.

Reuse:

- The append-only event authority and pure fold in
  `core/woml-engine/src/projection.rs`.
- Durable run/definition binding and recovery in
  `core/woml-engine/src/durable.rs`.
- The existing JSON Schema conformance tests in
  `woml-cli/tests/protocol-contract.test.ts`.

Result:

Rust can append, fold, persist, and recover one immutable branch selection. A
restart cannot silently select a different route.

### B4 — Execute only the selected route in Rust

Changes:

- Add strict `ContextReferenceExpression` resolution against folded context.
- Evaluate `<when>` cases in order and reject non-boolean values.
- Replace the current unconditional predecessor check in
  `ready_node_ids_for_projection` with active/inactive edge resolution.
- Mark only the selected branch edge active.
- Ensure unselected nodes are skipped without attempts or context outputs.
- Make the selected route's result/join node ready without waiting for inactive
  routes.
- Resolve and atomically publish the selected `<result>` at the branch ID.
- Allow a branch workflow to finish even though unselected nodes never execute.
- Preserve fail-closed behavior for interrupted script attempts.
- Add in-memory and durable engine tests for first-match ordering, fallback,
  strict booleans, route skipping, joins, downstream context, and nesting.

Reuse:

- Rust remains the only workflow scheduler and context authority.
- Bun remains responsible only for user-authored `<script>` execution.
- Existing `step_attempt_started`, `step_attempt_succeeded`, and
  `step_attempt_failed` events continue to describe script work.

Result:

The Rust engine executes exactly one branch route, publishes one predictable
branch result, and continues into downstream steps.

### B5 — Expose branch behavior through `woml run`

Changes:

- Run the accepted branch model through the existing N-API boundary.
- Map runtime condition/reference failures back to their original WOML
  attributes and source locations.
- Extend the structured N-API error envelope with branch identity and reference
  details; do not encode them into an unstructured message.
- Add black-box CLI tests for selected `<when>`, `<otherwise>`, invalid boolean,
  and unavailable references.
- Verify that CLI output contains only the final workflow JSON while errors go
  to stderr with a nonzero exit code.
- Update the CLI package fixture so a clean installed package runs the branch
  workflow with its native Rust engine and Bun script host.

Reuse:

- `woml-cli/src/cli.ts` for parsing, compilation, diagnostic formatting, and the
  public command.
- `woml-cli/src/rust-executor.ts` for the N-API execution boundary.
- The long-lived Bun host and isolated Worker protocol remain unchanged.

Result:

Users can run a real conditional workflow with:

```bash
woml run branch.woml
```

and receive the correct result from the selected route.

### B6 — Harden, package, and close the milestone

Changes:

- Test nested branches and branches at the beginning, middle, and end of a
  workflow.
- Test multiple true conditions to prove first-match behavior.
- Test large JSON branch results and existing size-limit behavior.
- Test crashes before selection, after selection, during a selected script, and
  before/after result publication.
- Verify deterministic compiled-definition hashing with branches.
- Run frontend tests, Rust tests, protocol/event conformance tests, clippy,
  typechecks, native integration tests, and the clean-package CLI journey.
- Run the timing script as a regression measurement, without treating cold
  startup optimization as part of branch correctness.
- Update both implementation plans and the language document to mark the branch
  executable profile complete.

Result:

Branching is a supported WOML product feature rather than an experimental parser
shape. The package can execute it, recover it, explain its failures, and prove
its behavior from reviewed artifacts.

## 8. Verification Matrix

| Area | Required proof |
|---|---|
| Syntax | Valid branch parses; illegal placement and attributes report original line/column. |
| References | Exact typed references compile; malformed, unknown, forward, and non-dominating references fail. |
| Selection | First true `<when>` wins; `<otherwise>` runs only when all tests are false. |
| Isolation | Only selected scripts reach the Bun host. |
| Context | Selected route outputs exist; unselected outputs do not; merged output exists at the branch ID. |
| DAG | Nested and downstream nodes remain reachable and the graph remains acyclic. |
| Events | `branch_selected` validates, folds, persists, and reopens deterministically. |
| Recovery | A recorded selection is never recomputed into a different route. |
| Errors | Non-boolean and missing-reference failures carry stable codes and useful source locations. |
| CLI | `woml run branch.woml` returns the reviewed JSON and correct exit code. |
| Package | A clean installation includes the Rust addon, Bun host, compiler, and branch support. |

## 9. Explicit Non-Goals

This milestone does not add:

- arbitrary JavaScript expressions inside `test`;
- comparisons, operators, fallbacks, optional references, or bracket notation;
- parallel execution;
- approval or resume tokens;
- retries greater than one;
- webhook, schedule, interval, or event triggers;
- HTTP, database, Slack, or other services;
- lifecycle hooks, cancellation, or durable user state;
- `context.run`, `context.env`, or secrets; or
- removal of the legacy JavaScript chaining SDK.

Those capabilities remain separate milestones so branch selection does not
silently decide their contracts.

## 10. Roadmap After Branching

Once B0–B6 are complete, product expansion continues in this order:

1. **Parallel execution** — implement `<parallel>`, schedule ready children
   concurrently through the already multiplexed Bun protocol, join them, and
   add durable parallel-group events.
2. **Human approval** — implement `<approval>`, notifications, pause/resume,
   durable approval tokens, and `woml.resume(token, decision)`.
3. **Retries and idempotency** — freeze idempotency-key derivation and duplicate
   behavior, then enable retry values greater than one with durable attempt
   events and backoff.
4. **Production triggers** — add webhook, schedule, interval, and event triggers
   with their validation, payload, failure, and lifecycle contracts.
5. **Services and capabilities** — add HTTP, database, messaging, and other
   registered operations without exposing clients or secrets through persisted
   context.
6. **Lifecycle and engine controls** — add lifecycle hooks, cancellation,
   durable state, and other engine-owned control operations with explicit event
   semantics.
7. **SDK retirement** — remove the old JavaScript chaining SDK only after WOML
   reaches the agreed feature and migration parity and existing workflows have
   a supported migration path.

## 11. Definition of Done

The branch milestone is complete only when:

- the four source tags and their placement rules are documented;
- every successful branch produces one stable `context.steps.<branchId>` value;
- the compiler emits the reviewed deterministic DAG;
- Rust evaluates conditions and schedules only the selected route;
- `branch_selected` is durable, versioned, validated, and foldable;
- old v1 sequential histories remain readable;
- unselected routes produce no attempts, outputs, or side effects;
- nested branches work while parallel and approval remain explicitly rejected;
- errors carry stable codes and original WOML source locations;
- in-memory and durable execution produce the same result;
- the packaged public CLI passes the branch acceptance workflow; and
- all existing sequential `hello.woml` behavior remains compatible.
