
# WOML Architecture

**Workflow Orchestration Markup Language — Core Architecture (Layers 1–4)**

This document describes the architecture of the WOML runtime: how a `.woml`
file is read, parsed, validated, and executed. It covers the four foundational
layers — the Rust core, the WOML frontend, the JS bridge, and RAK (dependency
and service management). Surfaces built on top (the CLI, ClickWork) are out of
scope here.

---

## 1. Overview

### 1.1 Design principles

- **Layered, one-directional dependencies.** Each layer depends only on the
  layer below it. The core never knows WOML exists.
- **WOML is a frontend, not the engine.** The execution core is language-
  agnostic. WOML is one way to produce a workflow it can run; other frontends
  (e.g. a TypeScript SDK) can feed the same core. This keeps WOML neutral and
  independently useful.
- **`context` is pure data.** The running state of a workflow is serializable
  data with no functions or capabilities mixed in. This single property is what
  makes persistence, replay, resume, and observability nearly free.
- **Declarative-first, escape hatch beneath.** Common operations are declarative
  tags executed natively; genuine custom logic drops to a `<script>`. The same
  hybrid principle governs dependencies (`services` for the common case, raw npm
  for the long tail).
- **One capability, two faces.** Every capability is registered once and both
  its tag form and its `services.*` form are derived from that single
  declaration. They can never drift.
- **Analyzable before execution.** Because the source of truth is static text,
  a workflow can be validated, linted, and reasoned about without running it.

### 1.2 The five layers

| Layer | Name | Responsibility |
|-------|------|----------------|
| 1 | Rust core | Execution, state, concurrency, scheduling, persistence |
| 2 | WOML frontend | Read → parse → resolve → validate → lower into a workflow model |
| 3 | JS bridge (Bun) | Execute `<script>` bodies and adapter code; inject `context`, plus `services` only in capability-enabled profiles |
| 4 | RAK | Resolve dependencies and configure services before execution |
| 5 | Surfaces | CLI, ClickWork (out of scope for this document) |

The rule: **each layer talks only downward.** The core is a library first and a
binary second, so the same core serves the CLI, a long-running server, and
ClickWork's backend without forked behavior.

---

## 2. Layer 1 — The Rust core

The core's sole job: given a workflow model and a trigger, execute it correctly.
It has no knowledge of XML or WOML syntax.

### 2.1 Event-sourced execution

The core is **event-sourced** rather than state-mutating. A run is an
append-only log of events:
run_started  
step_started(id)  
step_completed(id, output)  
step_failed(id, error)  
run_completed
The `context` object is **derived** by folding this event log — it is not a
mutable blob the executor edits in place.

This buys, as architectural properties rather than bolted-on features:

- **Crash recovery / resume** — reload the log, fold it, continue from the last
  event. A workflow that dies after charging a card resumes *after* that step,
  never re-charging.
- **Replay & time-travel** — fold the first *N* events to reconstruct the state
  at any point; play the log to animate a past run.
- **Observability** — the log *is* the trace. Per-step inputs, outputs, timing,
  and errors are inherent, not added.

This is the same approach durable-execution engines use, for the same reasons.

### 2.2 Core components

- **Scheduler** — decides what runs when: cron timers, concurrency limits,
  parallel fan-out, queuing. Backed by Rust's async runtime for high concurrency
  on a small footprint.
- **Executor** — walks the ordered step list. For each step: resolve inputs →
  run → capture output → append event → advance. A loop over a list with a
  derived state object; small enough to make both fast and correct.
- **State store** — persists the event log per run. Because derived `context` is
  pure serializable data, it can be written after each step, enabling all of
  §2.1's properties.
- **Handler registry** — a table mapping capability operations to their
  implementations (see §5). When the executor hits a semantic tag, it looks up
  the operation here and runs it — natively in Rust where possible, never
  touching JavaScript.

### 2.3 Interface to upper layers

The core exposes a narrow interface: *accept a workflow model and a trigger
event; execute; emit events about progress.* It is consumed as a library, so
it makes no assumptions about being driven by a CLI.

---

## 3. Layer 2 — The WOML frontend

The translator from `.woml` text to an executable workflow model. Four stages.

### 3.1 Stage A — Read

A `.woml` file is plain UTF-8 text, read into a string. The `.woml` extension is
a convention for tooling only; it carries no execution semantics.

### 3.2 Stage B — Parse

An **off-the-shelf XML parser** turns the text into a generic tree of nodes and
attributes. No custom parser is written — this is the primary dividend of
choosing a markup syntax. The output is a "dumb" tree: it knows a tag named
`step` exists with an `id` attribute, but nothing about workflows.

### 3.3 Stage C — Resolve

Legal tags depend on what has been required, so `<requires>` must be resolved
before validation completes. The frontend scans top-level `<requires>` and:

- `<requires module="./path.woml"/>` — recursively run Stages A–C on that file,
  then merge its definitions into the current file's vocabulary under the bound
  name. **A module is just another `.woml` file; imports are recursion.**
- `<requires package="name"/>` — hand to RAK; ensure the package is present in
  the Bun runtime so scripts may `import` it.
- Services configured in the RAK manifest — their adapters are loaded, and both
  their tag and `services.*` forms become available.

Resolution is depth-first and **must detect import cycles** (A requires B
requires A). Output: the complete vocabulary of legal tags for this file.

### 3.4 Stage D — Validate and lower

Walk the tree against the resolved vocabulary and enforce WOML's rules:

- Root is a `<workflow>` with exactly one `<trigger>` and one `<steps>`.
- Every `<step>` has a unique `id`.
- Every `{{context.steps.x.y}}` reference targets a step that **exists** and runs
  **earlier** than the reference.
- Required attributes are present on every capability operation.
- Retry / error policies are well-formed.

This stage produces good errors and catches a whole class of bugs **before
anything runs** — a dangling reference is a compile error, not a 3 a.m.
production failure.

Output: the **workflow model** — a validated, engine-ready DAG (nodes, edges,
resolved references, declared triggers, and policies). A v1 executor may choose
ready nodes sequentially, but sequentiality is not encoded into the model. This
single artifact feeds three consumers:

1. The core — executes it.
2. The graph surface — reads nodes and edges from it.
3. The markdown surface — describes it in prose.

**One parse, three projections.**

### 3.5 Caching

The workflow model is **cached after first parse** and reused for every trigger
firing. Re-parsing per webhook invocation is the obvious performance mistake to
avoid; parsing is otherwise not on the hot path.

---

## 4. Layer 3 — The JS bridge

A **long-lived Bun process** (never spawn-per-step). The core communicates with
it over a fast local channel.

### 4.1 Executing a `<script>`

When the executor reaches a `<script>`, it sends the script body plus a snapshot
of `context` to Bun. Bun executes the body with two ambient globals injected
into scope:

- **`context`** — the v0.1 run data surface contains only `context.trigger` and
  `context.steps.<id>`. Reads are free; **mutations do not persist**. Neither
  `context.run` nor `context.env` exists in v0.1. Internal run fields and
  resolved environment/secrets MUST NOT leak through those names. A future
  version may add a frozen `context.run` schema only through an explicit
  language-version decision. The *only* way a step contributes data downstream
  is its `return` value. This rule keeps the graph and logs honest — there are no
  invisible side channels.
- **`services`** — available only in capability-enabled profiles, containing
  configured integrations such as `services.slack.send(...)` and
  `services.db.query(...)`. It is absent from the first CLI vertical slice and
  is populated later by RAK-installed adapters (§6).

The script's `return` value becomes `context.steps.<thisStep>`, which the next
step can read. Then execution advances.

### 4.2 Isolation and safety

- **Per-script timeout** — a runaway script must not hang a run indefinitely.
- **Isolation** — one workflow's script cannot read another run's `context`.

### 4.3 Why Bun (and not an embedded JS engine)

Bun is retained as the JS execution layer. Embedding a JS *engine* in Rust
(e.g. V8/QuickJS) is straightforward, but embedding a JS *ecosystem* is not:
raw engines provide the language but not npm resolution, Node APIs, or the
standard library. Since npm compatibility is central to WOML's design (§6),
Bun does substantial work beyond mere execution. A future single-binary
distribution should **bundle** Bun, not replace it.

---

## 5. Capabilities — one implementation, two faces

This is the unifying model for all operations (`http`, `db`, `slack`, etc.).

### 5.1 Single registration

Every capability registers **once** with:

- a name (`slack`),
- a set of operations (`send`, `upload`, `react`),
- each operation's argument signature.

### 5.2 Two derived faces

From that single registration, the system **derives** both surfaces
automatically — neither is hand-written twice:

- **Tag face** — `<slack.send channel="#x" text="y"/>`; attributes map to the
  operation's named arguments.
- **Script face** — `services.slack.send({ channel: '#x', text: 'y' })`; a
  generated proxy calling the same operation.

The tag is a *syntactic projection* of the capability — the same relationship
the graph surface has to the WOML source.

### 5.3 Consequences

- **No drift** — the two faces cannot disagree; there is one source of truth.
- **Free validation** — a missing required argument is caught in Stage D because
  the requirement is declared once.
- **Free docs & AI knowledge** — documentation and ClickWork's model of "what's
  available" both read from the capability registry; no separate knowledge of
  tags vs services is needed.

### 5.4 Native handlers callable from JS

Capabilities implemented natively in Rust are still callable from scripts. The
Bun bridge exposes a callback channel: `services.http.get(...)` is a thin JS
stub that messages the Rust core to run the native `http.get` handler and
returns the result. **`<http>` and `services.http` execute the same Rust
function.**

Note: the script face costs a bridge round-trip, so the tag form is faster than
the script form. This is a desirable gradient — it nudges workflows toward the
declarative path, which is both the faster and the more analyzable one.

### 5.5 Registry components vs raw npm

| Source | Is a capability? | Tag form | `services.*` | Script `import` |
|--------|------------------|----------|--------------|-----------------|
| Registry component (`@woml/slack`) | Yes — declares operations | ✓ | ✓ | — |
| Raw npm package (`pdf-lib`) | No — just a library | — | — | ✓ |

This asymmetry is intentional. Capabilities are curated, declared things with
two faces. Raw npm is the unopinionated, one-faced escape hatch that guarantees
"zero integration limits." An **adapter** is a thin capability that wraps an npm
package — the mechanism by which long-tail packages get promoted into
first-class tags.

---

## 6. Layer 4 — RAK

RAK owns dependency resolution and service configuration. It is a **thin
coordinator**, not a package manager rebuilt from scratch: it resolves names,
fetches from two sources, writes a lockfile, and hands npm packages to Bun's
existing resolver.

### 6.1 One manifest

A single manifest (`rak.toml`) is the only dependency file the user manages.
There is **no user-facing `package.json`**; if one exists on disk it is a
generated, machine-owned artifact.

The manifest does two jobs.

**Declare dependencies.** Name encodes source, so resolution never guesses:

- Namespaced names → the WOML registry (`@woml/slack`, `@org/thing`).
- Bare names → npm (`pdf-lib`, `lodash`).

**Configure services** (Expo-plugin style). Each service entry carries its
config — tokens, connection strings, defaults. This is what lets scripts call
`services.slack.send(...)` with **no imports and no credentials in the workflow
file**, which keeps `.woml` files safely shareable.

### 6.2 Startup sequence

At engine startup, RAK:

1. Resolves all declared dependencies (registry + npm).
2. Ensures packages are present in the Bun runtime.
3. Instantiates each configured adapter with its config.
4. Registers each capability into the handler registry — providing **both** the
   tag form (`<slack.send>`) and the `services.slack.send()` form (§5).

### 6.3 Lockfile

A lockfile pins resolved versions so a workflow that runs today runs identically
later.

### 6.4 `<requires>` vs dynamic import

`<requires package="...">` is the **declaration** of a dependency; dynamic
`import()` inside a script remains available for genuinely dynamic cases. The
declaration is preferred because it makes dependencies **knowable before
execution** — RAK installs ahead of the run, the file self-describes its needs,
and a shared workflow can be validated without running it. Relying solely on
dynamic import risks discovering a missing package *mid-run*.

---

## 7. Execution lifecycle — end to end

A file with: a webhook trigger, a `validate` step (`<script>`), a `create-user`
step (`<db.insert>`), and a `<slack.send>` notification.

1. **Startup.** `woml serve` starts. RAK reads the manifest, installs
   `@woml/slack` and `@woml/postgres`, instantiates both with their configured
   tokens, and registers them into the handler registry and `services`. Bun
   starts and stays warm.
2. **Frontend.** The file is read, parsed to a tree, its `<requires>` resolved,
   every tag and `{{context...}}` reference validated, and the workflow model
   produced and cached. The core registers the webhook route and waits.
3. **Trigger.** A request hits `/signup`. The core opens a run: it appends
   `run_started`, sets `context.trigger` from the request body, assigns a run
   id, and persists the event log.
4. **Step `validate` (`<script>`).** The core sends the body + context snapshot
   to Bun. The script reads `context.trigger.email`, runs its checks, and
   returns an object. The core appends `step_completed(validate, output)` →
   folds into `context.steps.validate` → persists.
5. **Step `create-user` (`<db.insert>`).** The core looks up `db.insert` in the
   handler registry (native Rust). It resolves the attribute template
   `{{context.steps.validate.email}}` against live context, runs the insert
   **without touching JavaScript**, appends the completion event, persists.
6. **Step `slack.send` (adapter capability).** Resolve template, call the
   operation, append event, persist.
7. **Completion.** The core appends `run_completed`. The persisted event log is
   now a full record of the run — the same data the graph surface reads for
   status and the logs expose.

**Failure path.** If step 5 throws, the core consults that step's retry policy:
retry with backoff, or append `step_failed` and fire the failure hook. Because
the log is persisted at each step, a resume continues from the last successful
step — never re-charging a card or repeating a completed side effect.

---

## 8. Performance notes

- **Parse once, cache the model** (§3.5). Never re-parse per trigger.
- **The JS boundary is the real cost, not the markup.** XML parsing is
  microseconds; the per-script bridge hop to Bun is where time goes. Mitigations:
  a warm Bun process (never spawn-per-step) and native tag handlers.
- **Declarative tags run in Rust** and never pay the JS cost. A workflow with no
  `<script>` tags never crosses the bridge — hence *"the more declarative your
  workflow, the faster it runs."*
- **Parallelism is where the Rust core wins most.** `<parallel>` and iteration
  fanning out across the async runtime is a larger real-world advantage than
  parse speed.

---

## 9. Tooling notes

### 9.1 Extension guarding

The CLI is **helpful but strict**: running `woml run file.oml` (misspelled
extension) or a nonexistent file produces a warning with a did-you-mean
suggestion. The CLI should **content-sniff** — if a file parses as valid WOML,
run it and warn about the extension; if not, error clearly. A wrong-extension
file that is never invoked cannot be caught by the engine; that is the editor's
responsibility.

### 9.2 Linting

- **Syntax linting is reused** — malformed XML (unclosed tags, bad nesting) is
  handled by the parser and existing XML tooling. Do not rewrite it.
- **Semantic linting is Stage D exposed as a command.** Dead steps, dangling or
  forward `{{context...}}` references, duplicate ids, unused `<requires>`,
  missing required attributes, and import cycles are all detected by the
  existing validation stage. The linter is not a separate product — it is Stage
  D wearing three hats: `woml validate` (CLI), CI, and (later) a language server
  for live in-editor errors.

This static analyzability is the structural reason WOML can tell a user their
workflow is broken **without running it** — something a runtime-bound node graph
cannot do as well.

---

## 10. Open questions (deferred, not yet designed)

These were identified as important but are out of scope for layers 1–4 as
specified here:

- **Error handling & retries** — declarative `retry` / `backoff` / `on-error`
  attributes and a `<catch>` / `<on-error>` construct. *(Highest-priority gap.)*
- **Loops / iteration** — a `<for-each>` construct; sequential vs parallel
  semantics and concurrency caps.
- **Secrets management** — where secrets live, how runtime-only service
  adapters receive them without exposing `context.env`, and how they are kept
  out of logs, events, and graph/markdown surfaces.
- **Idempotency keys** — explicit surfacing of the resume/replay guarantees the
  event-sourced core already makes possible.
- **Dry-run** — `woml run --dry` walking the model and reporting what *would*
  happen without side effects.
- **Versioning & migration** — `<workflow version="...">` and behavior when a
  definition changes mid-flight.
