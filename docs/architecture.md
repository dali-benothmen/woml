# WOML Architecture

WOML separates authoring, durable execution, and JavaScript execution so each
layer has one clear responsibility.

```text
.woml source
    -> Bun/TypeScript parser, validator, and compiler
    -> versioned compiled workflow DAG
    -> Bun CLI through the WOML N-API adapter
    -> durable Rust execution engine and SQLite event store
    -> long-lived Bun host
    -> isolated Worker for one JavaScript attempt
    -> durable outcome event
    -> folded context and terminal presentation
```

## Layer boundaries

| Layer | Owns | Must not own |
| --- | --- | --- |
| TypeScript frontend | WOML markup, raw `<script>` bodies, references, source locations, modules, reusable definitions, validation, DAG lowering | Execution, retries, persistence, scheduling |
| Bun CLI/hosts | Commands, terminal rendering, configuration, secret resolution, trigger/provider transports, worker isolation | Durable graph truth, retry decisions, run settlement |
| Rust engine | Model validation, scheduling, events, folding, recovery, retries, control flow, services, policies, run control | XML/WOML syntax, editor grammar, JavaScript meaning |
| N-API adapter | Narrow typed calls between Bun and Rust | Workflow behavior or a second engine |

The TypeScript frontend is the only WOML compiler. Rust never parses XML and
Bun never advances the graph based on console output. A future compiler move is
valid only as a reviewed complete migration; WOML does not maintain two
competing compilers.

The canonical adapter lives in `core/woml-native` and depends locally only on
`woml-engine`. The old combined core and JavaScript-chaining execution paths are
not part of the product.

## Compilation

The frontend reads one `<woml>` document, validates its workflow or reusable
definition profile, resolves safe local imports, and lowers the result to a
versioned language-neutral model.

The model is a DAG from the first compilation boundary. Sequential source order
creates edges, while choices, switches, parallels, forks, approvals, lifecycle,
and workflow settlement add explicit control nodes and descriptors. The model
also records deterministic identity, metadata, retry policy, runtime policy,
trigger configuration, reference visibility, required secret names, and exact
module artifact digests.

Only the frontend understands:

- `<workflow>`, `<steps>`, and other WOML elements;
- `{{context.payload...}}` and `{{context.steps...}}` references;
- raw JavaScript and inline JSON schema source locations;
- import paths, ESM exports, reusable props, and custom tags; and
- author-facing diagnostic codes, messages, hints, line, and column.

`woml check` exercises this boundary without activating ingress or executing
JavaScript.

## Durable execution and event folding

Rust validates the compiled model independently before accepting a definition.
For each run it selects ready nodes, supervises attempts, and appends immutable
versioned events. The current context is derived by folding those events; an
authoritative mutable context object is never persisted.

```text
run_started(payload)
  -> step_attempt_started
  -> step_attempt_succeeded(result)
  -> fold result into context.steps.<id>
  -> next ready node
  -> run_succeeded(finalResult)
```

In-memory projections and indexes are rebuildable acceleration. Immutable run
events, definition bindings, attempt results, approval decisions, trigger
occurrences, and managed-operation settlement records are durable truth.

Event sourcing does not claim exactly-once arbitrary effects. If recovery finds
an attempt that started without a terminal outcome, the effect is ambiguous and
fails closed as interrupted. WOML replays derivation, not an unknown external
side effect.

## JavaScript host and bindings

One long-lived Bun host multiplexes invocation messages by invocation ID.
Responses may arrive out of order. Each script attempt runs in a fresh isolated
Worker, giving it independent module state and a real termination boundary.

Depending on the compiled profile, a Worker receives deeply read-only bindings:

- `context.payload` and visible `context.steps` values;
- `attempt` retry identity and counters;
- `services` built-ins and imported module aliases;
- source-proven `secrets.NAME` values; and
- `lifecycle` only for lifecycle scripts.

The host returns JSON-compatible outcomes. It does not decide retries,
branch selection, cancellation, or workflow completion.

## Services and capability calls

Native `fetch()` remains Bun's Fetch implementation and is observed with
redacted operation events. Managed capabilities use a full-duplex Rust/Bun
protocol whose nested calls correlate by invocation and call ID.

Rust owns limits, cancellation, stable operation identity, durable settlement,
and recovery for:

- `services.http.request()`;
- `services.db()` for SQLite and PostgreSQL;
- `services.storage`;
- `services.cache`;
- `services.events.emit()`;
- `services.state`;
- `services.workflows.call()` and `.start()`; and
- supervised Telegram, Discord, and WhatsApp messaging.

Resolved credentials exist only in bounded invocation/transport memory. Models,
events, progress, logs, and inspection contain secret names or redacted
metadata—not values.

## Triggers and admission

The frontend compiles trigger definitions; it never creates runs. Manual input,
HTTP listeners, clocks, event publication, and communication-provider adapters
normalize an occurrence and submit it to Rust.

Rust atomically binds:

1. the immutable trigger occurrence;
2. the exact compiled definition; and
3. the first run event.

Stable occurrence identity makes provider redelivery and caller retries return
the original run instead of executing a second one. Payload hashes use
canonical JSON. Conflicting identities or payloads fail closed.

Schedules and intervals use durable cursors owned by Rust. Webhook schemas and
authentication are checked before admission. Named events fan out through the
same authority, whether published over authenticated HTTP, `woml emit`, or
`services.events.emit()`.

Communication transports remain provider-specific—Slack Socket Mode, Telegram
long polling, Discord Gateway, and signed WhatsApp callbacks—but converge on
the same durable trigger, notification, approval, and capability authorities.

## Control flow and settlement

Sequential steps advance by DAG edges. Other structures remain engine-owned:

- `<choose>` and `<switch>` select one durable route and may publish one stable
  merged result;
- `<parallel>` starts independent child steps and applies `fail-fast` or
  `wait-all`;
- `<fork>` runs independent multi-step branches and releases the continuation
  according to its selected join set;
- `<approval>` persists a wait before notifying reviewers and resumes only from
  a committed decision or timeout; and
- lifecycle actions run at versioned workflow/step event boundaries.

The workflow settlement node prevents a run from reporting success while owned
fork work remains active. A joined failure blocks its continuation; an unjoined
failure does not block the selected join but still contributes to final run
failure after all owned work settles.

Lifecycle uses a separate durable vocabulary (including Event v10 history) so
the business outcome and lifecycle finalization remain distinguishable.

## Modules and reusable definitions

`<imports><module ... /></imports>` has two local forms:

- `.js`/`.ts` modules expose named exports at `services.<alias>`; and
- reusable `.woml` definitions compile custom step or notification-provider
  tags away before the model reaches Rust.

The frontend follows only safe static local module edges, creates deterministic
ESM bundles and source maps, and records exact artifact digests. The durable
definition store owns those bytes, so recovery never reads changed source from
the project directory.

`woml-env.d.ts` is editor support and does not enter definition identity.
Imported modules receive `services`; workflow context, attempts, and secrets are
passed as explicit function arguments.

## Workflow calls

Call-only workflows omit `<triggers>` and are registered by exact workflow ID.
Both workflow operations reuse ordinary trigger admission and the normal DAG
engine:

- `services.workflows.call()` waits for an independent durable child result;
- `services.workflows.start()` returns the child run ID after durable dispatch.

Stable call identity reconnects retries to the same child. Hidden lineage
rejects direct and indirect cycles. Synchronous calls reject Human Approval
targets before admission because an arbitrary Bun continuation cannot be stored
across a long wait.

## Runtime policy

Runtime policy is compiled outside the business DAG. Rust applies concurrency,
work-conserving FIFO queues, strict rolling-window rate limits, and total
workflow deadlines consistently to manual, trigger, event, and workflow-call
ingress. Scheduler claims and queue indexes are rebuildable; events and exact
definitions remain authoritative.

## Durable state and data

`services.state` stores bounded workflow-owned JSON across runs. Rust supplies
the workflow namespace, canonical JSON, versions, compare-and-set, quotas,
atomic increments, mutation reattachment, and cross-process SQLite
transactions. State never enters run context and is not removed with run
history.

Cache, storage, database, and state have separate promises:

- cache may expire or evict;
- storage holds checksummed larger objects;
- databases provide application-owned querying and transactions; and
- durable state holds small workflow-owned facts.

## Runtime ownership and operations

`woml run` activates direct `.woml` files and directories—there is no build
artifact required from authors. Activation validates the complete unit, pins
definitions/artifacts, prepares transports with admission closed, performs
recovery, and opens ingress only after every required component is ready.

One live runtime owns one local SQLite state boundary through a durable lease.
The production runtime provides foreground/background operation, exact stop,
graceful drain, authenticated loopback administration, redacted structured
logs, metrics, health, the terminal inspector, coherent online backup, guarded
offline restore, retention, and bounded SQLite maintenance.

This is a continuous single-machine runtime, not a distributed scheduler or a
multi-tenant hostile-code sandbox. Operators must provide TLS, host/container
resource limits, filesystem permissions, network egress policy, secret-provider
security, monitoring, and backup protection.

## Packaging

The public `woml` package is platform-neutral and contains no `.node` file. It
declares exact optional dependencies on the supported `@woml/cli-*` native
packages. The loader selects the matching macOS, Windows, or Linux glibc/musl
x64/ARM64 artifact automatically.

The public package bundles the private TypeScript compiler, CLI, hosts, and
workers. Users install one package and one `woml` executable; native packages
are implementation details.

## Versioned contracts and enforcement

Compiled models, definition packages, script/capability protocols, event
vocabularies, store generations, inspection projections, and operational
responses are explicitly versioned under [`schemas/`](schemas/) and
[`protocols/`](protocols/). Historical versions remain compatibility artifacts.

Automated separation gates reject a restored chaining SDK, a second compiler
or execution path, markup knowledge in Rust, unexpected native dependencies,
adapter export drift, and native binaries embedded in the platform-neutral
package.
