# WOML Fork and Branch Implementation Plan

Status: FJ0-FJ8 completed; fork and branch is a supported, publishable WOML feature

## 1. Product Outcome

This milestone gives WOML a first-class way to split one automation into named,
multi-step routes without forcing each route into a separate `.woml` file.

The motivating product journey is social distribution. A team can keep topic
research, content creation, and distribution as three understandable workflows.
The distribution workflow can then publish the same prepared content through
TikTok, Instagram, Facebook, and Pinterest branches inside one durable run.

After FJ0-FJ8 are complete, a workflow author can:

- create concurrent routes with `<fork>` and `<branch>`;
- put multiple sequential steps inside each branch;
- use `<choose>`, `<parallel>`, `<approval>`, retries, services, modules, and
  Workflow Calls inside a branch under reviewed composition rules;
- make the next main-route step wait for all branches, selected branches, or no
  branches through one `join` attribute;
- let unjoined branches continue while the main route advances;
- trust that all branches remain owned, observable, cancellable, and recoverable
  parts of the same workflow run;
- read joined branch step outputs through the existing `context.steps.<stepId>`
  surface;
- recover after a runtime restart without replaying completed branch work or
  changing which outputs are visible; and
- express conditional choice with `<choose>` instead of overloading the word
  “branch” with two unrelated meanings.

The acceptance command remains simple:

```bash
woml run examples/forkDistributionWorkflow.woml
```

The workflow must execute through the packaged Rust engine and Bun script host.
No second executor or fork-specific CLI command is introduced.

## 2. Acceptance Workflow

The reviewed product fixture will have this shape:

```xml
<workflow version="0.1" id="social-distribution" name="Social Distribution">
  <triggers>
    <manual id="start" />
  </triggers>

  <steps>
    <step id="prepareContent">
      <script>
        return {
          title: "A practical WOML introduction",
          instagramEnabled: true
        };
      </script>
    </step>

    <fork id="distribution" join="instagram facebook">
      <branch id="tiktok">
        <step id="prepareTikTok">
          <script>
            await new Promise(resolve => setTimeout(resolve, 120));
            return {
              caption: context.steps.prepareContent.title
            };
          </script>
        </step>

        <step id="publishTikTok">
          <script>
            return {
              platform: "tiktok",
              published: true
            };
          </script>
        </step>
      </branch>

      <branch id="instagram">
        <choose>
          <when test="{{context.steps.prepareContent.instagramEnabled}}">
            <step id="prepareInstagram">
              <script>
                return {
                  caption: context.steps.prepareContent.title
                };
              </script>
            </step>

            <step id="publishInstagram">
              <script>
                return {
                  platform: "instagram",
                  published: true
                };
              </script>
            </step>
          </when>

          <otherwise>
            <step id="skipInstagram">
              <script>
                return {
                  platform: "instagram",
                  published: false
                };
              </script>
            </step>
          </otherwise>
        </choose>

        <step id="completeInstagram">
          <script>
            return {
              platform: "instagram",
              published: context.steps.prepareContent.instagramEnabled
            };
          </script>
        </step>
      </branch>

      <branch id="facebook">
        <step id="prepareFacebook">
          <script>
            return {
              caption: context.steps.prepareContent.title
            };
          </script>
        </step>

        <step id="publishFacebook">
          <script>
            return {
              platform: "facebook",
              published: true
            };
          </script>
        </step>
      </branch>
    </fork>

    <step id="recordJoinedDistribution">
      <script>
        return {
          instagram: context.steps.completeInstagram,
          facebook: context.steps.publishFacebook
        };
      </script>
    </step>
  </steps>
</workflow>
```

The graph behaves as follows:

```text
                                    ┌─ prepareTikTok ─── publishTikTok ───────┐
prepareContent ── open distribution ├─ choose ─ selected route ─ complete ─┐ │
                                    └─ prepareFacebook ─────── publish ─────┴─┼─ joined main step
                                                                              │
workflow completion waits for the main route and TikTok ──────────────────────┘
```

The important product behavior is:

1. all three branches become eligible after `prepareContent`;
2. steps inside each branch execute sequentially;
3. the Instagram `<choose>` behaves exactly as it would on the main route;
4. `recordJoinedDistribution` waits for Instagram and Facebook;
5. TikTok may still be running when the main route continues;
6. TikTok remains owned by this run and must settle before the run becomes
   terminal; and
7. the main script cannot observe TikTok outputs because TikTok was not joined,
   even if TikTok happened to finish early.

The reviewed final result comes from the main route:

```json
{
  "instagram": {
    "platform": "instagram",
    "published": true
  },
  "facebook": {
    "platform": "facebook",
    "published": true
  }
}
```

Branch completion order must never become the workflow result or change this
JSON.

## 3. Product Decisions

### 3.1 A fork creates branches

`<fork>` is the split point and owns the join rule. Each direct `<branch>` is
one named route containing one or more sequential flow items.

The vocabulary is intentionally literal:

```text
fork
├── branch
├── branch
└── branch
```

The first release does not add a `<path>` or `<sequence>` wrapper.

### 3.2 Conditional choice is `<choose>`

The existing conditional source construct is renamed from `<branch>` to
`<choose>`. A choice selects one mutually exclusive route. A fork activates
several branch routes. These concepts must remain separate in documentation,
diagnostics, and authoring support.

The established conditional execution behavior remains intact:

- `<when>` cases are evaluated in document order;
- only the first true case executes;
- `<otherwise>` is the fallback;
- tests remain exact typed WOML references; and
- conditional selection remains durable and recoverable.

FJ0 freezes the migration of existing conditional-result behavior. The
recommended source profile supports a simple control-only `<choose>` without
requiring an ID or `<result>`, while retaining the existing optional merged
result form when a choice ID and arm results are authored. Existing compiled
models and event histories are never rewritten.

### 3.3 Minimal attributes

The first fork profile deliberately has only these attributes:

```xml
<fork id="distribution" join="all">
  <branch id="instagram">
    ...
  </branch>
</fork>
```

- `<fork>` requires `id` and optionally accepts `join`.
- `<branch>` requires only `id`.
- omitted `join` means `all`.

Priority, ordered execution, concurrency caps, per-fork failure modes,
conditional branch participation, display metadata, and delayed branch starts
remain future feedback-driven additions.

### 3.4 Branches are owned work, not fire-and-forget work

An unjoined branch may outlive the main route, but it never outlives the
workflow run. WOML observes it, recovers it, cancels it with the run, and waits
for it before publishing a terminal workflow outcome.

Independent work that should become a different run continues to use:

```js
await services.workflows.start("another-workflow", payload);
```

### 3.5 Determinism is more important than completion timing

Branch output visibility is decided by the compiled graph and `join`, not by
which external provider responds first.

- A branch sees pre-fork outputs and earlier outputs from its own route.
- It never sees sibling-branch outputs.
- The main route sees outputs from joined branches after the join barrier.
- It never sees outputs from unjoined branches after that fork.

This filtering applies to the complete runtime `context` passed to Bun, not
only to statically parsed references. Dynamic JavaScript access must not bypass
the rule.

## 4. Source-Language Contract

### 4.1 Minimal grammar

The proposed source grammar is:

```text
flow-item      := step | choose | parallel | approval | fork

fork           := <fork id="structural-id" join="join-spec"?>
                    branch+
                  </fork>

branch         := <branch id="branch-id">
                    branch-flow-item+
                  </branch>

branch-flow-item := step | choose | parallel | approval

join-spec      := "all" | "none" | branch-id-list
branch-id-list := branch-id (XML-whitespace+ branch-id)*

choose         := <choose>
                    when+
                    otherwise
                  </choose>

result-choice  := <choose id="structural-id" name="text"? description="text"?>
                    result-when+
                    result-otherwise
                  </choose>

result-when       := <when test="context-reference"> flow-item+ result </when>
result-otherwise  := <otherwise> flow-item+ result </otherwise>
```

The grammar is context-sensitive in one deliberate place: once compilation is
inside a fork-owned branch, `<fork>` is rejected anywhere in that branch's
descendant subtree, including inside a nested choose or approval route. A later
milestone may add nested forks after nested ownership, visibility, and failure
propagation are reviewed.

### 4.2 `<fork>`

```xml
<fork id="distribution" join="instagram facebook">
  ...
</fork>
```

| Attribute | Required | Meaning |
| --- | ---: | --- |
| `id` | Yes | Stable workflow-wide fork identity used by the compiled graph, events, diagnostics, and inspection. |
| `join` | No | `all`, `none`, or a whitespace-separated set of direct child branch IDs. Defaults to `all`. |

Rules:

- A fork contains one or more direct `<branch>` children.
- One branch is valid as a degenerate generated form.
- Unknown elements and text inside a fork are errors.
- Fork IDs use the existing public structural-ID grammar.
- Fork IDs share the workflow-wide namespace with step, choice-result,
  parallel, and approval IDs.
- A fork may appear on the root route or inside an existing selected
  conditional/approval route when it is not already inside a fork-owned branch.
- A fork may be the first route item; the compiler connects it to the virtual
  workflow-entry boundary.
- A fork may be the final route item when a preceding value-producing item
  already defines that route's result. Settlement preserves that value. A
  workflow with no value-producing main-route item remains invalid rather than
  inventing a fork aggregate or implicit `null` result.

### 4.3 `<branch>`

```xml
<branch id="instagram">
  <step id="prepareInstagram">...</step>
  <step id="publishInstagram">...</step>
</branch>
```

Rules:

- A branch is legal only as a direct child of `<fork>`.
- A branch contains one or more supported branch flow items.
- Items inside one branch execute sequentially according to document order.
- Different branches become eligible concurrently.
- A branch may contain multiple steps.
- A branch may contain `<choose>`; the selected choice route remains part of
  that branch.
- A branch may contain existing `<parallel>` and `<approval>` constructs under
  their existing structural rules.
- Step and other executable structural IDs inside branches remain
  workflow-wide so `context.steps.<stepId>` stays unambiguous.
- Branch IDs are scoped to their direct fork. The durable identity is
  `(forkId, branchId)`, allowing a different fork to use `instagram` again.
- Duplicate branch IDs inside one fork are errors.
- A branch ID never becomes a `context.steps` key and creates no implicit
  aggregate output.

### 4.4 `join`

#### Join every branch

```xml
<fork id="distribution" join="all">
```

The next item in the enclosing continuation route waits until every branch
succeeds. Omitting `join` has the same meaning.

#### Join selected branches

```xml
<fork id="distribution" join="instagram facebook">
```

The next item in the enclosing continuation route waits for Instagram and
Facebook. Other branches keep running under workflow ownership.

#### Join no branches

```xml
<fork id="analytics" join="none">
```

The enclosing continuation route proceeds after the fork opens. All branches
still have to settle before the workflow itself completes.

Join-list rules:

- Values are case-sensitive direct child branch IDs.
- `all` and `none` are reserved values and cannot be branch IDs.
- A list cannot mix `all` or `none` with an ID.
- Duplicate IDs are rejected rather than silently deduplicated.
- Attribute order does not become execution order.
- The compiler canonicalizes a valid set into branch document order.
- The join is a barrier immediately after the fork; the first profile does not
  target an arbitrary later step.

### 4.5 `<choose>` composition

`<choose>` may be used inside a branch exactly as it is used on the main route:

```xml
<branch id="instagram">
  <choose>
    <when test="{{context.steps.settings.instagramEnabled}}">
      <step id="publishInstagram">...</step>
    </when>

    <otherwise>
      <step id="recordInstagramSkip">...</step>
    </otherwise>
  </choose>
</branch>
```

The selected route must settle before the containing branch settles. An
unselected route creates no attempts, outputs, or side effects.

The canonical choice profiles are explicit:

- A control-only `<choose>` omits `id`; each arm contains one or more flow
  items and must not contain `<result>`. The compiler assigns a deterministic
  definition-local internal identity for durable selection, but no
  `context.steps` key is created.
- A result-producing `<choose id="decision">` retains the established
  conditional-result contract: every arm ends in exactly one `<result>`, and
  the selected value is published at `context.steps.decision`.
- `name` and `description` remain optional only on the result-producing form
  for source compatibility.
- `<otherwise>` remains required in both first-release profiles so every choice
  has one selected route.

Providing `id` without arm results or using `<result>` without `id` is rejected.
This keeps the common control-only syntax small while preserving predictable
merged results for existing workflows.

## 5. Execution Semantics

### 5.1 Opening a fork

After the preceding item in the enclosing route succeeds—or after the virtual
workflow-entry boundary when the fork is first—Rust:

1. confirms that the fork is ready from folded event history;
2. reconstructs the deterministic pre-fork context view;
3. durably opens the fork once;
4. makes every branch entry eligible; and
5. makes the enclosing continuation's join barrier eligible only according to the compiled join
   set.

All branches are eligible concurrently. The runtime may schedule their first
steps in any order, but document order remains the stable tie-breaker for tests,
diagnostics, and inspection. The first profile makes no priority guarantee.
The existing runtime Worker/capability capacity remains the safety ceiling:
eligible branches beyond available execution capacity wait durably rather than
creating unbounded Workers. This operational cap does not become a fork source
attribute or change which branches logically participate.

### 5.2 Executing one branch

Within a branch:

- ordinary sequencing applies;
- each step sees the pre-fork visible context plus successful earlier outputs
  from that same branch;
- retries pause only the current branch step;
- an approval waits only in that branch;
- a `<parallel>` group joins before the next item in that branch;
- a `<choose>` runs only its selected route; and
- a failed item prevents later items in that branch from starting.

Concurrent branches that update the same `services.state` key retain State v1
atomic-operation semantics but gain no transaction or ordering guarantee.
Authors use atomic operations such as `increment`/`set_if_absent`, separate
keys, or an explicit joined coordinator step when updates must not race.

### 5.3 Releasing the main route

The fork's join barrier settles successfully only when every joined branch
succeeds. When it succeeds, the next item in the enclosing continuation route
becomes eligible exactly once.

If a joined branch fails or is terminally cancelled, the join barrier settles
failed exactly once. Rust marks its downstream continuation inactive, so those
nodes cannot deadlock workflow settlement. The other owned branches continue
under the fixed attempt-all rule, after which the workflow can fail or cancel.

For `join="none"`, the barrier releases immediately after the durable fork-open
event. For a selected list, only those branches participate in the barrier. For
`join="all"`, every branch participates.

Successfully settling the join barrier does not claim that every fork branch is complete.
It means only that the branches required by the main route are complete.

### 5.4 Completing the workflow

The run can succeed only after:

- the main route reaches its terminal output; and
- every branch owned by every opened fork reaches a terminal state.

Failure and cancellation do not require an impossible successful main terminal.
If a joined branch fails, downstream main work remains blocked; after every
owned branch and already-active main operation settles safely, Rust records one
terminal failed or cancelled run outcome.

The main route remains the sole source of the workflow's public result. An
unjoined branch finishing later cannot replace that value. The compiler adds an
engine-owned settlement boundary so the DAG retains one deterministic terminal
outcome without inventing an aggregate branch result.

### 5.5 Failure behavior

The first profile has one fixed, conservative behavior and no `on-error`
attribute:

- a failed branch does not stop sibling branches that are already eligible;
- a failed joined branch prevents the main join barrier from releasing;
- an unjoined branch failure never participates in the selected join barrier,
  whether it happens before or after that barrier releases;
- therefore an unjoined failure does not prevent later main work from starting
  and does not cancel main work already running;
- WOML lets all owned fork work settle safely; and
- any unhandled branch failure makes the overall workflow fail.

This is “attempt all, report failure” behavior. It is suitable for distribution
automation because one provider failure does not prevent attempts to the other
providers, while the final run remains truthful about incomplete delivery.

Future user feedback may justify explicit `fail-fast`, `continue`, or
`fail-after-all` policies. They are not source syntax in this milestone.

The durable output of a successful main terminal step remains part of event
history if an unjoined branch later fails. It is not promoted to a successful
workflow result: `woml run`, Workflow Call, Workflow Start inspection, and
runtime APIs report failure and never emit that partial value as success JSON.
Authorized diagnostics may show that a main result was recorded, but production
inspection remains payload-free. The failed run's `on-failure` and
`on-complete` hooks receive the final lifecycle context under the rules below.

### 5.6 Lifecycle ordering

Forks do not change the established lifecycle order. Rust decides the business
outcome only after all opened fork branches and already-active main work settle
safely:

- a joined branch failure makes downstream main continuation inactive;
- an unjoined branch failure allows main continuation but makes the eventual
  business outcome failure;
- a main-route failure prevents unopened later forks from opening, while
  branches from already-open forks continue to settle under attempt-all; and
- after step hooks drain, Rust runs `on-failure` or `on-cancel`, then
  `on-complete`, then finalizes the run.

No workflow outcome hook runs merely because the main route completed while an
owned branch was still active.

### 5.7 Cancellation and shutdown

Cancelling the workflow covers:

- the main route;
- every active branch;
- every waiting branch approval or retry;
- queued branch steps; and
- active branch script Workers through the existing cancellation protocol.

Graceful runtime shutdown preserves the current fail-closed rule. Safe pending
branch work resumes after restart; an ambiguous in-flight external effect is
not replayed merely because it belongs to a branch.

## 6. Context and Reference Contract

### 6.1 Existing context surface

Forks do not add `context.forks`, `context.branches`, positional arrays, or an
implicit merged result. Successful executable items continue to publish through:

```js
context.steps.<stepId>
```

### 6.2 Visibility by route

The compiler calculates a visibility set for every executable node.

| Consumer | Visible fork outputs |
| --- | --- |
| A branch item | Earlier outputs in the same branch only |
| A sibling branch | None |
| Main item after `join="all"` | Outputs from every successful branch |
| Main item after a selected join | Outputs from selected joined branches only |
| Main item after `join="none"` | No branch outputs |
| Terminal workflow lifecycle hook | Every successful output from the fully settled run, while the public workflow result remains the main result |
| Workflow settlement | Main terminal result plus branch terminal status, not a merged script context |

Joined outputs remain visible transitively to every later descendant of that
continuation route, including later choices and approvals. Unjoined outputs
remain invisible on that continuation forever, even after their branch finishes.

The runtime filters `context.steps` to this compiled visibility set before
sending context to Bun. `context.payload` and other existing context fields keep
their established contracts. This prevents timing-dependent dynamic reads
without accidentally removing unrelated run input.

### 6.3 Static reference validation

The frontend rejects:

- a branch referencing a sibling branch step;
- a branch referencing a later item in its own route;
- the main route referencing an unjoined branch output;
- a join list naming an unknown branch; and
- any reference that is not guaranteed to be available on every selected path.

Existing conditional merged results remain the way to normalize mutually
exclusive choice output when required.

Terminal workflow lifecycle hooks execute only after all fork work settles, so
they may inspect all successful outputs. Step lifecycle hooks retain the
visibility of their subject's route and must not become a sibling-output side
channel.

## 7. Compiled Workflow Model Contract

FJ0 proposes Compiled Workflow Model v13, following the current Model v12.
Model v1-v12 artifacts remain immutable and readable.

Model v13 must describe the complete graph without teaching Rust about XML:

- a deterministic internal identity plus pure `engine.choice-select` and
  `engine.choice-join` nodes for an ID-less control-only `<choose>`;
- one pure `engine.fork-open` control node;
- one entry and one engine-owned terminal boundary for each branch;
- ordinary compiled nodes for the branch's flow items;
- one pure `engine.fork-join` barrier carrying the canonical joined branch set;
- the continuing main-route edge;
- one engine-owned workflow-settlement boundary that depends on the main
  terminal and every owned branch terminal;
- explicit fork/branch ownership metadata for validation and inspection; and
- explicit per-node context visibility metadata or an equivalently
  deterministic derivation that Rust validates independently.

The model must prove:

- every branch belongs to exactly one fork;
- every branch entry is reachable only from its fork-open node;
- branch routes cannot overlap;
- branch terminal boundaries dominate the correct join/settlement nodes;
- the join set is a subset of direct branch IDs;
- the graph is acyclic;
- the public main result is deterministic; and
- the graph has one engine-recognized terminal settlement outcome.

A fork compiled inside an unselected conditional or approval route never opens,
creates no fork events, and contributes only inactive edges to workflow
settlement. Only branches belonging to durably opened forks can block a run.

The TypeScript compiler emits Definition Package v8 when a packaged definition
contains Model v13. Script Host Protocol v7 and Capability Call contracts do not
change because scripts still receive ordinary JSON context and return ordinary
JSON results.

The core continues to know only the compiled model, durable events, and handler
registry. It never parses `<fork>`, `<branch>`, `<choose>`, `join`, WOML source,
or `{{...}}` syntax.

## 8. Durable Event and Projection Contract

FJ0 proposes Run Event v12, following the current Event v11. The reviewed
vocabulary should include the minimum durable facts required to recover and
inspect a fork:

- `fork_opened` — one immutable activation of a compiled fork;
- `fork_branch_settled` — one terminal outcome for a direct branch; and
- `fork_join_settled` — one terminal `succeeded`, `failed`, or `cancelled`
  outcome for the enclosing continuation barrier.

The exact payloads are frozen in FJ0. They carry stable identities and outcomes,
not copied script context, branch outputs, secrets, or error messages.

Required rules:

- `(runId, forkId)` opens at most once.
- `(runId, forkId, branchId)` settles at most once.
- a branch cannot settle before its fork opens.
- a successful join cannot settle before every joined branch succeeds.
- a failed/cancelled join names the joined branch outcome that made its
  continuation permanently inactive.
- unjoined branches are still present in the projection and workflow
  settlement requirements.
- restart folds the same opened, active, joined, succeeded, failed, cancelled,
  and waiting states without mutable authoritative scheduler memory.
- safe pure control derivation may be repeated before its event is committed;
  user-authored effects retain fail-closed interruption behavior.

The event-sourced projection gains fork and branch status for scheduling and
inspection. The in-memory projection remains a disposable cache.

Event v12 also adds `choice_selected` for the new ID-less control-only
`<choose>`. Its compiler-generated choice identity is derived deterministically
from the immutable definition path and is validated against Model v13. Existing
result-producing choices continue to reuse historical conditional lowering and
`branch_selected`; stored events are not renamed.

No new SQLite authority is assumed. Store v14 remains the proposed storage
profile unless FJ0 proves that an indexed durable record is required. If the
store shape must change, Store v15 is reviewed before implementation rather
than introduced incidentally.

## 9. Conditional `<choose>` Migration

The source-language rename must not break durable history or surprise existing
users.

The recommended transition is:

1. `<choose>` becomes the canonical documented conditional tag.
2. `<branch>` inside `<fork>` always means a fork route.
3. Historical conditional `<branch>` directly inside `<steps>` or a conditional
   arm remains accepted for the complete FJ release line because its parent
   makes the grammar unambiguous.
4. `woml check` and `woml run` emit warning
   `WOML_DEPRECATED_CONDITIONAL_BRANCH` to stderr with a direct `<branch>` to
   `<choose>` replacement hint. A warning alone leaves `woml check` at exit code
   `0` and never contaminates successful JSON stdout.
5. Existing compiled Model v2-v12 conditional groups, `branch_selected` events,
   definition hashes, and stored runs remain unchanged.
6. New `<choose>` definitions may initially reuse the established internal
   conditional lowering and handlers; internal historical names are not exposed
   as new source terminology.
7. The workflow `version` metadata attribute does not control this migration.
   Removal of the legacy source alias requires a separately reviewed future
   language release, migration tooling, and major compatibility decision; it is
   not date-based and is not part of FJ8.

This gives new users one clear vocabulary without making an upgrade destroy
working `.woml` files.

## 10. Validation and Error Contract

Every error retains the existing stable code, source file, line, column,
message, and optional corrective hint shape.

The milestone adds diagnostics for:

- missing or invalid fork IDs;
- missing branches and empty branches;
- `<branch>` outside `<fork>` when it is not the recognized legacy conditional
  form;
- non-branch children inside `<fork>`;
- duplicate branch IDs in one fork;
- reserved `all` or `none` branch IDs;
- malformed, empty, duplicated, mixed, or unknown join values;
- unsupported nested forks;
- sibling, forward, unjoined, or path-conditional references;
- a terminal fork with no earlier value-producing continuation result;
- malformed compiled ownership, join, visibility, or settlement graphs;
- contradictory fork event history;
- branch settlement or join-settlement ordering failures; and
- attempted workflow success while owned fork work remains unresolved.

Runtime errors include structured `forkId`, optional `branchId`, node ID,
failure kind, and WOML source mapping where available. They never depend on
parsing an error-message string.

## 11. Production and Observability Behavior

Forks must compose with the Production Runtime v1 profile from their first
publishable release.

- `woml get <runId>` exposes bounded fork and branch status through a versioned
  Run Inspection v4 profile.
- `woml inspect` shows active/waiting/failed fork counts and selected run detail
  without displaying payloads or script outputs.
- structured logs may include run, workflow, fork, branch, and node IDs.
- Prometheus metrics use bounded status labels and never branch IDs as
  unbounded metric labels.
- backup/restore preserves fork histories without a special side channel.
- retention treats a run with unresolved branches as active and ineligible.
- runtime ownership and graceful shutdown cover branch work automatically.
- secret and payload scans include fork diagnostics, operations output, and
  crash paths.

## 12. Implementation Phases

### 12.1 Phase summary

| Phase | What changes | Result after the phase |
| --- | --- | --- |
| FJ0 — completed | Freeze source, model, event, migration, context, error, and fixture contracts. | Every layer targets one reviewed fork/branch design before code changes. |
| FJ1 — completed | Introduce canonical `<choose>` and the legacy conditional-source migration. | Conditions have the correct product name without breaking stored history or existing files abruptly. |
| FJ2 — completed | Parse and validate minimal `<fork>`, `<branch>`, and `join` syntax. | WOML understands valid forks and explains invalid markup at the source. |
| FJ3 — completed | Lower forks into deterministic Model v13 DAGs with visibility and settlement boundaries. | TypeScript produces a complete engine-ready graph while Rust still gates execution. |
| FJ4 — completed | Add Event v12, folding, persistence, inspection, and recovery projections. | Fork ownership and branch outcomes survive restart as durable truth. |
| FJ5 — completed | Execute concurrent branches and `join="all"` in Rust. | Multi-step branches run concurrently and the main route safely waits for all. |
| FJ6 — completed | Execute selected joins and `join="none"` with deterministic context filtering. | The main route can continue early without seeing timing-dependent outputs. |
| FJ7 — completed | Complete failure, cancellation, approval/retry/control-flow composition, CLI, and observability behavior. | Forks work inside real automations and remain operable in production. |
| FJ8 — completed | Harden recovery, compatibility, packaging, benchmarks, documentation, and release gates. | Forks and branches are a supported, publishable WOML language feature. |

### FJ0 — Freeze fork, branch, join, and choice contracts

Changes:

- Update the proposed language profile in reviewed fixtures without enabling
  runtime execution.
- Freeze the minimal source grammar and exact `join` token grammar.
- Freeze fork-global and branch-local identity rules.
- Freeze the control-only and merged-result `<choose>` compatibility shape.
- Add and review Model v13, Event v12, Definition Package v8, and Run
  Inspection v4 schemas.
- Decide explicitly whether Store v14 is sufficient; add Store v15 only if a
  real durable index is required.
- Freeze graph lowering, context visibility, failure, cancellation, workflow
  result, and settlement rules.
- Freeze source diagnostics and structured runtime failures.
- Add reviewed source, compiled, event-history, context-view, result,
  inspection, and recovery fixtures for `all`, selected, and `none` joins.
- Cover joined failure, early and late unjoined failure, main failure with
  active branches, lifecycle ordering, partial-main-result non-exposure, a
  first-item fork, a terminal fork preserving a prior result, and recursive
  nested-fork rejection.
- Add both true and false Instagram-choice executions so the flagship output is
  path-stable.
- Add a historical conditional `<branch>` source fixture and canonical
  `<choose>` equivalent.

Reuse:

- Current Model v12, Event v11, Definition Package v7, Store v14, and Run
  Inspection v3 remain immutable baselines.
- Existing conditional selection, parallel, approval, retry, lifecycle,
  cancellation, and event-folding contracts remain authoritative.

Result:

The syntax and every expensive cross-layer boundary can be reviewed before an
implementation begins.

Gate:

No FJ1 compiler or runtime behavior starts until the source fixture, compiled
graph, event history, context snapshots, final result, and schema set are
approved together.

### FJ1 — Rename conditional source flow to `<choose>` safely

Changes:

- Teach the frontend to accept canonical `<choose>` in every legal current
  conditional location.
- Preserve ordered `<when>` and `<otherwise>` behavior.
- Preserve the existing merged-result profile for workflows that use it.
- Accept legacy conditional `<branch>` only where its parent and children make
  it unambiguously conditional.
- Emit one stable deprecation diagnostic through `woml check` without changing
  runtime stdout.
- Update editor declarations, examples, and language terminology.
- Prove canonical `<choose>` and its legacy equivalent lower to semantically
  equivalent conditional graphs.

Result:

Users can rename existing result-producing conditional flow to readable
`<choose>` syntax while existing source and all historical definitions/runs
remain usable. The new ID-less control-only profile is enabled with Model v13
in FJ3.

Gate:

All existing conditional branch tests pass under compatibility fixtures, and
new documentation uses `<choose>` exclusively for conditions.

### FJ2 — Add minimal fork and branch authoring

Changes:

- Extend recursive frontend validation with `<fork>` and fork-owned
  `<branch>`.
- Validate the exact ID-less control-only `<choose>` profile without lowering
  it to an older model.
- Parse omitted, `all`, `none`, and branch-list joins.
- Validate branch placement, non-empty multi-item bodies, local branch IDs,
  workflow-wide executable IDs, and join membership.
- Allow reviewed branch items: step, choose, parallel, and approval.
- Reject a fork recursively anywhere inside a fork-owned branch subtree and
  reject unsupported attributes with source-located errors.
- Extend static reference analysis with branch ownership and join visibility.
- Generate automatic editor types/diagnostics with no manual `woml types`
  step.

Result:

WOML understands the complete minimal authoring surface and rejects unsafe or
ambiguous source before compilation. `validateWoml` accepts the reviewed
fixtures; `compileWoml` returns an explicit FJ3 feature gate rather than
silently emitting an older model.

Gate:

The three acceptance fixtures validate, every invalid join/reference fixture
fails at the responsible token, and Rust execution remains explicitly gated.

### FJ3 — Lower forks into the compiled DAG

Changes:

- Add Model v13 TypeScript and Rust types.
- Lower ID-less control-only choices with deterministic internal identities and
  no `context.steps` output.
- Lower fork-open, branch entry/body/end, main join, and workflow-settlement
  boundaries with deterministic internal IDs.
- Carry canonical fork ownership, branch ownership, and join membership.
- Compute per-node visible step-output sets.
- Preserve one deterministic public main result and one settlement terminal.
- Extend TypeScript and Rust graph inspection for reachability, cycles,
  overlapping routes, ownership, visibility, join dominance, and settlement.
- Emit Definition Package v8 for module-backed Model v13 definitions.
- Preserve exact Model v1-v12 validation and package compatibility.

Result:

The reviewed source fixtures compile byte-for-structure to the reviewed
language-neutral DAGs. Rust validates them but still refuses fork execution
until FJ5.

Gate:

TypeScript and Rust independently accept the reviewed graphs, reject every
malformed graph fixture, and reproduce stable definition hashes.

### FJ4 — Make fork progress durable and recoverable — completed

Changes:

- Add Event v12 fork and control-only choice payloads with closed validation
  rules.
- Extend pure folding with fork/branch/join projections.
- Append, reload, and validate fork histories through in-memory and SQLite
  stores.
- Extend recovery to distinguish safe pending work from ambiguous active
  attempts.
- Add Run Inspection v4 fork status without outputs or payloads.
- Reject duplicate open/settle/join events, unknown identities, impossible
  ordering, mixed run-event versions, and contradictory terminal histories.
- Preserve old Event v1-v11 histories exactly.

Result:

Rust can persist and recover fork ownership and progress without an
authoritative mutable branch object.

Gate:

Reviewed Event v12 histories round-trip through SQLite, fold identically after
reopen, and recover to the same eligible work.

### FJ5 — Execute concurrent branches and join all — completed

Changes:

- Open a ready fork once and make all direct branches eligible.
- Execute each branch as an independent sequential route through the existing
  multiplexed Bun host.
- Reuse runtime policy admission and active-attempt limits rather than creating
  an unbounded executor.
- Capture and enforce each branch's deterministic context view.
- Settle branch success once after its terminal item succeeds.
- Settle `join="all"` successfully only after every branch succeeds.
- Keep the main route's public result independent from branch completion order.
- Add event-order tests proving actual overlap rather than relying only on wall
  time.

Result:

A real workflow executes several multi-step branches concurrently, waits for
all of them, and continues the main route once.

Gate:

`woml run` executes the `join="all"` acceptance fixture through Rust and returns
the reviewed JSON with no sibling output leakage.

### FJ6 — Execute selected and non-blocking joins — completed

Changes:

- Settle a selected join successfully only after every named branch succeeds.
- Settle `join="none"` successfully immediately after durable fork opening.
- Keep unjoined branches active under the same run after the main route
  continues.
- Filter main and branch Bun contexts from compiled visibility rather than
  event timing.
- Prevent static and dynamic sibling/unjoined reads.
- Add the workflow-settlement boundary so main completion waits for every owned
  branch while preserving the main result.
- Test fast and slow completion permutations to prove identical context and
  output.

Result:

Users can overlap long branch work with useful main-route work without races,
hidden background tasks, or timing-dependent context.

Gate:

The selected-join acceptance fixture records main-route completion before an
unjoined slow branch, returns the same JSON under reversed timing, emits no CLI
success result before whole-run settlement, and keeps the run active until every
branch settles.

### FJ7 — Complete failures, composition, CLI, and operations — completed

Changes:

- Implement fixed attempt-all branch failure semantics.
- Settle a joined barrier failed and make its downstream continuation inactive
  after a joined branch failure.
- Prove an unjoined failure never blocks a selected barrier, regardless of event
  order.
- Allow already-running main work to settle when a later unjoined branch fails,
  then report one truthful workflow failure.
- Propagate workflow cancellation and shutdown to active/waiting branch work.
- Compose branches with choose, parallel, approval, retry, services, modules,
  Workflow Calls, Workflow Start, lifecycle hooks, runtime policies, and State
  v1.
- Preserve the established outcome-hook order after every opened fork settles
  and ensure failed runs never publish a partial main value as success JSON.
- Map runtime fork failures to original source locations through structured
  native errors.
- Expose fork status through `woml get`, structured logs, metrics, and
  `woml inspect` without payload leakage or unbounded labels.
- Protect unresolved fork runs from retention and preserve them through
  backup/restore.

Result:

Forks behave correctly in real long-running automations rather than only in an
isolated script demo.

Gate:

Production integration tests cover failures, cancellation, approval waiting,
retry waiting, lifecycle outcomes, runtime restart, inspection, backup/restore,
and retention protection.

### FJ8 — Harden, package, document, and publish — completed

Changes:

- Test one, two, and many branches with multiple steps and out-of-order
  completion.
- Test forks at root, inside selected choices/approval routes, and adjacent to
  existing parallel groups.
- Test every join form, whitespace form, invalid form, and completion order.
- Test crashes before/after fork open, during each branch, before/after branch
  settlement, before/after join settlement, after main continuation, and before
  workflow settlement.
- Verify historical Model v1-v12, Event v1-v11, Store v14, Definition Package
  v1-v7, and conditional source compatibility.
- Run frontend, Rust, N-API, CLI, clean-install, production-runtime,
  compatibility, security, typecheck, Clippy, and performance gates.
- Update the language specification, architecture, examples, migration guide,
  editor declarations, and package documentation.
- Add `test:fork-branch` as the independent release command and include
  it in the repository release gate.

Result:

Fork and branch execution becomes a supported, recoverable, observable, and
publishable WOML feature.

Gate:

A clean installed package runs the social-distribution workflow and the full
compatibility/recovery matrix with no skipped native tests.

## 13. Verification Matrix

| Area | Required proof |
| --- | --- |
| Choice migration | Canonical `<choose>` works; legacy conditional source remains readable with a useful deprecation diagnostic; historical models/events are unchanged. |
| Syntax | Forks and multi-item branches parse; malformed placement and join tokens report exact source locations. |
| IDs | Fork IDs are workflow-wide; branch IDs are fork-local; generated identities are collision-safe. |
| Lowering | Model v13 graphs are deterministic, acyclic, reachable, owned, visibility-safe, and have one settlement outcome. |
| Concurrency | All branches become eligible; event ordering proves overlap; steps within one branch remain sequential. |
| Join all | The continuation waits for every branch and settles once. |
| Selected join | The continuation waits for exactly the named branches; unjoined failures never block it. |
| Join none | Main work continues immediately while the run retains all branch ownership. |
| Visibility | Siblings and unjoined branches remain invisible regardless of timing or dynamic JavaScript access. |
| Result | The main route remains the sole public result; later branch completion never replaces it. |
| Failure | Siblings continue, joined failure inactivates downstream continuation without deadlock, and any unhandled branch failure makes the run fail truthfully. |
| Failed result | A recorded main value remains durable but is never emitted as successful CLI/API/Workflow Call output for a failed run. |
| Lifecycle | Outcome hooks wait for all opened fork work, then preserve `on-failure`/`on-cancel` followed by `on-complete`. |
| Choice composition | `<choose>` behaves identically on the main route and inside a branch. |
| Existing composition | Parallel, approval, retry, services, modules, calls, lifecycle, policies, State, and triggers retain their contracts. |
| Events | Event v12 opens, settles, joins, validates, folds, persists, and reopens deterministically. |
| Recovery | Completed branch work never replays; safe pending work resumes; ambiguous effects fail closed. |
| Cancellation | Main, active branch, waiting approval/retry, and Worker cancellation converge on one terminal outcome. |
| Operations | Inspection, logs, metrics, backup/restore, and retention represent forks without leaking data. |
| Compatibility | Historical models, events, stores, packages, examples, and non-fork workflows remain green. |
| CLI/package | A clean package runs the acceptance workflow with the Rust engine and Bun host. |

## 14. Explicit Non-Goals

This milestone does not add:

- priority between branches;
- ordered branch execution;
- a fork concurrency attribute;
- configurable branch failure policies;
- `when` on `<branch>`;
- arbitrary delayed branch starts;
- a later named join point;
- nested forks inside branches;
- a `<path>` or `<sequence>` wrapper;
- `context.forks` or `context.branches`;
- an implicit fork result or automatic output merge;
- detached branches that outlive the workflow run;
- cross-machine branch scheduling or distributed ownership;
- new communication providers;
- package-registry or postponed Module System work;
- queue/document-store/distributed-cache adapters;
- advanced pause/saga/state-transaction controls;
- a browser operations dashboard; or
- retirement of the JavaScript chaining SDK during fork implementation.

These are not silently designed by implementation convenience. User feedback
may justify separate reviewed milestones later.

## 15. Expected File Areas

| Area | Expected locations |
| --- | --- |
| Language specification | `docs/woml-v0.1.md`, this plan, migration documentation |
| Parser/compiler | `woml/src/compiler.ts`, `woml/src/model.ts`, frontend tests and fixtures |
| Package artifacts | `woml/src/modules.ts`, Definition Package v8 schemas/fixtures |
| Rust model validation | `core/woml-engine/src/model.rs`, graph-validation tests |
| Events/folding | `core/woml-engine/src/event.rs`, `projection.rs`, event schemas/fixtures |
| Scheduling/recovery | `core/woml-engine/src/engine.rs`, `runtime.rs`, `durable.rs` |
| Native/CLI errors | `core/src/woml_bridge.rs`, `woml-cli/src/rust-executor.ts`, `cli.ts` |
| Operations | run inspection, production observability, terminal inspector, retention tests |
| Examples | `examples/forkDistributionWorkflow.woml` and reviewed fixture directories |
| Release gates | `woml/tests`, `core/woml-engine/tests`, `woml-cli/tests`, verifier scripts |

Exact files remain subject to the contracts frozen in FJ0. Implementation must
reuse existing graph, event, scheduler, context, cancellation, and production
runtime authorities rather than introduce a second fork executor.

## 16. Risks and Guardrails

### Conditional and route terminology can remain confused internally

The source language uses `<choose>` for mutually exclusive conditions and
`<branch>` only for a fork route. Historical internal names remain compatibility
details and are not copied into new author-facing terminology.

### Early main continuation can leak unjoined outputs

The runtime sends a compiled visibility-filtered context to every node. Static
validation alone is insufficient because JavaScript can access keys
dynamically.

### Unjoined branches can become invisible background jobs

Workflow settlement, cancellation, inspection, backup, and retention all treat
them as owned work. Only `services.workflows.start()` creates a separate run.

### Branch completion can accidentally become the workflow result

Model v13 separates main-route result ownership from final run settlement. The
settlement node passes through the main result and never selects a result by
completion time.

### A branch failure can stop useful distribution work

The first profile attempts every branch and reports an overall failure after
owned work settles. No implicit fail-fast behavior is introduced.

### Recovery can replay successful branch side effects

Branch control state is folded from durable events. Successful attempts remain
settled, and ambiguous active effects retain the existing fail-closed policy.

### Fork syntax can grow into a policy language too early

Only `id` and `join` are accepted on `<fork>` and only `id` on `<branch>`.
Priority, concurrency, and policy attributes require user evidence and new
contracts.

## 17. Global Roadmap After Fork and Branch

1. **Additional Communication Providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when product demand
   justifies them.
2. **Retire the JavaScript Chaining SDK** — remove the old SDK only after fork
   and branch execution gives WOML the required workflow-composition parity and
   users have a documented migration path.

Completed milestones—including branching/choice, parallel execution, Human
Approval, retries and idempotency, production triggers, services and
capabilities, the essential Module System, Durable Workflow Calls, Workflow
Start, lifecycle and engine controls, runtime policies, Durable State, and
Production Runtime and Operations—remain the baseline and are not repeated as
future work.

## 18. Definition of Done

The milestone is complete only when:

- `<choose>` is the canonical conditional syntax and legacy source migration is
  safe and documented;
- `<fork>` contains one or more direct `<branch>` routes;
- every branch can contain multiple sequential flow items;
- all branches become eligible concurrently;
- `join` supports omitted/`all`, selected branch IDs, and `none`;
- the next main item waits for exactly the joined branches;
- every unjoined branch remains owned until workflow settlement;
- branch and main contexts are deterministic and timing-independent;
- the main route remains the sole public workflow result;
- failures, retries, approvals, cancellation, lifecycle, and shutdown obey
  their durable contracts inside branches;
- Model v13 and Event v12 are versioned, validated, foldable, and recoverable;
- historical models, events, stores, packages, and conditional workflows remain
  compatible;
- production inspection, logs, metrics, backup/restore, and retention understand
  unresolved fork work;
- errors retain stable codes, structured fork/branch identity, and original
  WOML locations;
- the packaged CLI passes all three join acceptance workflows; and
- the independent fork release gate and repository release gate pass.

## 19. FJ0 Review Gate

Before FJ1 begins, review these artifacts together:

- canonical and legacy conditional-choice source fixtures;
- `join="all"`, selected-join, and `join="none"` WOML fixtures;
- Model v13 compiled fixtures and stable hashes;
- Event v12 successful, failed, cancelled, and recovered histories;
- pre-fork, branch-local, joined-main, and unjoined-main context snapshots;
- transitive later-main and nested-choice context snapshots;
- expected successful public results and failed-run non-results under reversed
  completion timing;
- joined-failure downstream-inactivation and unjoined-failure continuation
  fixtures in both event orders;
- workflow lifecycle ordering for joined failure, unjoined failure after main
  completion, and main failure while branches remain active;
- Definition Package v8 and Run Inspection v4 fixtures;
- the Store v14/no-migration decision or an explicitly reviewed Store v15;
- frontend and runtime diagnostic catalog;
- failure/cancellation/recovery tables; and
- historical Model v1-v12 and Event v1-v11 compatibility proof.

FJ0 is approved only when the same fixtures answer all of these questions:

1. Which branches start?
2. Which branches block the next main item?
3. Which outputs can each branch and main item see?
4. What becomes the public workflow result?
5. When may the run succeed or fail?
6. What survives and resumes after a crash?
7. How does an existing conditional `<branch>` source file migrate safely?

No implementation phase may answer one of these differently from the frozen
artifacts.
