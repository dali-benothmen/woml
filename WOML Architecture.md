# WOML Architecture

Status: current executable architecture through Fork and Branch FJ8

WOML turns readable `.woml` markup into a language-neutral workflow DAG and
executes that DAG durably. JavaScript remains the escape hatch inside
`<script>`, while workflow structure is validated data rather than JavaScript
chaining.

## 1. The four runtime boundaries

| Boundary | Responsibility | Must not understand |
| --- | --- | --- |
| TypeScript frontend (`woml/`) | Parse WOML, validate authoring rules, resolve local modules, lower a deterministic Definition Package and compiled DAG | Durable scheduling or event folding |
| Rust core (`core/woml-engine/`) | Validate compiled contracts, admit and schedule runs, append/fold events, supervise effects, recover, and expose redacted operations data | WOML tags, markup, `{{...}}`, or editor behavior |
| Bun script host (`woml-cli/src/script-host*`) | Execute JavaScript and local modules in isolated workers with `context`, `services`, `secrets`, and `attempt` bindings | Workflow scheduling authority or durable truth |
| CLI/runtime surface (`woml-cli/`) | Load deployments, manage secrets/configuration, call the frontend and Rust through N-API, host triggers, and render author-facing output | A second execution engine |

The compiled model is the interface. The frontend and core share versioned JSON
schemas and conformance fixtures; neither layer reaches through that boundary
to reinterpret the other's source.

## 2. Source to durable run

```text
.woml text
  -> TypeScript XML-like parser
  -> source-aware validation
  -> deterministic lowering
  -> Definition Package v8 / Model v13 DAG
  -> N-API
  -> Rust admission + Store v14
  -> Rust ready-node scheduler
  -> multiplexed Bun host for script invocations
  -> Event v12 append
  -> pure event fold
  -> next ready nodes or terminal outcome
```

WOML is XML-like rather than ordinary XML because raw `<script>` bodies may
contain JavaScript operators such as `<`, `>`, and `&&` without CDATA. The
frontend therefore uses its source-aware parser rather than an off-the-shelf
strict XML parser. Errors carry a stable code, file, line, column, and message.

The frontend validates IDs, placement, references, graph reachability,
acyclicity, route visibility, module boundaries, and feature-specific rules.
It then lowers structure into explicit nodes and edges. Rust independently
validates that compiled graph before it can be stored or executed.

## 3. Rust is the execution authority

Rust owns one execution model, one event vocabulary, one persistence authority,
one folding function, and one ready-node loop. It does not walk WOML source and
does not contain tag-specific parsing.

A run is authoritative only as an append-only event history. `context` is a
projection rebuilt by folding those events; an in-memory projection is a
discardable cache. Store v14 persists definitions, events, trigger state,
runtime-policy coordination, approvals, operations, workflow calls, and user
state without making any cache authoritative.

Event sourcing does not claim exactly-once external effects. If recovery finds
an attempt that started but has no durable success or failure event, the effect
is ambiguous. WOML records an `interrupted` failure and does not replay it.
Safe pending work may resume, and completed work is never executed again.

## 4. JavaScript execution

Rust communicates with one long-lived Bun host using an asynchronous,
multiplexed protocol. Multiple invocations may be in flight, responses are
correlated by invocation ID, and responses may arrive out of order. The host
creates an isolated worker context per invocation, giving scripts fresh global
state and real timeout/cancellation control without paying for a new Bun
process for every step.

Each script receives a JSON snapshot:

- `context.payload` — data supplied by the trigger or parent workflow;
- `context.steps.<id>` — successful outputs visible on this compiled route;
- `services` — supervised built-in and local-module capabilities;
- `secrets` — only resolved secret names declared by the workflow; and
- `attempt` — retry and idempotency identity.

`context.run` is intentionally unavailable in the current language profile;
WOML will not expose run fields until that author-facing schema is frozen.

Native `fetch()` uses Bun's Fetch behavior with redacted observation.
`services.http`, database, storage, cache, events, state, and workflow-call
operations cross the supervised capability boundary so Rust can durably track
attempts, timeouts, cancellation, idempotency, and bounded results.

## 5. DAG scheduling and fork semantics

The model is a DAG even when an early executor selected ready nodes
sequentially. The current scheduler supports sequential steps, mutually
exclusive `<choose>` routes, bounded `<parallel>` groups, approval arms,
retries, lifecycle hooks, and Model v13 forks.

A `<fork>` opens all direct `<branch>` routes. Every branch may contain multiple
ordered workflow items, while different branches become eligible concurrently.
Its `join` barrier waits for all branches, none, or an explicit set. The main
continuation receives outputs only from joined branches; sibling and unjoined
outputs are filtered by compiled visibility rather than wall-clock timing.

Opening, each branch settlement, and join settlement are Event v12 facts. A run
does not publish success until all work owned by every opened fork settles—even
when `join="none"` lets the main continuation proceed immediately. A joined
failure closes the continuation without deadlock. An unjoined failure never
blocks the selected barrier, but it still makes the final run fail truthfully.

## 6. Long-lived automation and operations

`woml run` activates one or more workflow files/directories as an atomic local
deployment and stays alive. Webhook, Slack, schedule, interval, and named-event
triggers admit durable occurrences into the same Rust engine. Manual one-shot
execution is available as `woml test` for local checks and CI.

Runtime policies apply concurrency, durable FIFO queueing, rate limits, and
workflow deadlines before execution. `woml list`, `get`, `cancel`, `inspect`,
`backup`, `restore`, and `prune` operate on the same state boundary. Inspection,
logs, and metrics are bounded and redacted; they never become execution truth.

## 7. Compatibility and versioning

Contracts evolve additively through explicit versions. The current fork
profile is Model v13, Event v12, Definition Package v8, Run Inspection v4, and
Store v14. Historical Model v1–v12, Event v1–v11, Package v1–v7, and existing
Store v14 histories remain readable. Rust never guesses a new meaning for an
old event.

Legacy conditional `<branch>` source remains a compatibility alias for one
migration window and lowers to the historical choice model. New source uses
`<choose>` for mutually exclusive conditions and reserves `<branch>` for a
route directly owned by `<fork>`.

## 8. Security invariants

- Secrets are resolved at runtime and are never stored in compiled models,
  events, logs, inspection data, package artifacts, or error messages.
- Script and module state is isolated per invocation.
- External effects are supervised and bounded; ambiguous effects fail closed.
- Compiled context visibility prevents dynamic JavaScript from reading a
  sibling or unjoined branch merely because it happened to finish early.
- The Rust core validates every definition and event history independently of
  frontend validation.

These boundaries keep WOML easy to author without making markup, JavaScript, or
the CLI an execution authority.
