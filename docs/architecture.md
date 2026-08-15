# WOML Architecture

## Authoritative Architecture

WOML is the current product direction. Authors describe workflows in readable
`.woml` markup, while execution remains split across three deliberately narrow
layers:

```text
.woml source
    -> TypeScript frontend: parse, validate, lower to a versioned DAG
    -> Bun CLI / N-API boundary
    -> Rust core: orchestrate, persist events, schedule and recover
    -> long-lived Bun Script Host
    -> isolated Worker: execute one JavaScript attempt
    -> Rust event log and folded context
    -> CLI result
```

The TypeScript frontend is the only layer that understands WOML markup,
attributes, raw `<script>` bodies, and `{{context...}}` references. Its output is
a versioned, language-neutral compiled workflow model whose nodes and edges form
a DAG. It does not execute the workflow.

The CLI does not derive status from console text. Its professional output is a
versioned read-only projection of the frozen definition and Rust event fold;
manual keyboard admission follows the same durable trigger boundary. See
[WOML Terminal Experience](woml-terminal-experience.md).

Module System MS1 makes `<woml>` the canonical document root. The frontend may
resolve `<imports><module name="..." from="..." /></imports>` before lowering:
it validates the direct named-function ESM surface, follows only safe static
local `.js`/`.ts` edges, assigns project-relative identities, and emits an
immutable Definition Package v1 manifest. MS2 deterministically bundles each
entry point as ESM, canonicalizes source maps, generates the imported `services`
declarations, and emits Definition Package v2 plus compiled Model v9. Model v9
contains artifact digests and public exports only; executable bytes stay in the
package. MS3 promotes those unchanged bytes into runtime-ready Definition
Package v3. Script Host v5 lets Rust register bundles by digest, and every
isolated Worker verifies and instantiates fresh module state under the deeply
read-only `services.<alias>` namespace. Native Fetch and built-in services used
inside exported functions retain the SC14 tracking boundary, while effects
during module initialization are rejected. Neither Rust nor Model v9 understands
`<woml>`, module paths, ESM syntax, or export grammar. `woml check` is the
read-only compilation boundary. The frozen contracts are
`docs/protocols/module-system-v1.md` and
`docs/protocols/module-compilation-v1.md`, with activation frozen in
`docs/protocols/module-runtime-v1.md`. Durable Store v8 owns the exact bundle,
source map, exports, and identities for each compiled-definition hash. Script
Host v6 re-registers those artifacts after restart, so run recovery never reads
the current WOML document or project module sources.

The local authoring layer generates a non-runtime `woml-env.d.ts` containing
built-in and imported `services.*` names. This editor artifact never enters the
compiled definition identity. Modules receive `services` only; workflow
context, attempts, and individual secrets cross the boundary as explicit
function arguments. Package permissions remain postponed with package imports.

Reusable Definitions v1 extends the frontend source boundary with two reusable document
profiles: a top-level `<step>` and a `<provider kind="notification">`. Local
`.woml` imports resolve to empty custom tags with declared props, while local
JS/TS imports remain `services.*` modules. The complete canonical dependency
graph is content-hashed, `woml check` validates ordinary workflow structure as
well as custom-tag placement, folder discovery activates workflows only, and
editor custom data is refreshed automatically. Model v14, Definition Package
v9, Reusable Script Binding v3, Custom Notification Provider Protocol v1,
Event v13, and Inspection v5 are the published contracts. The frontend compiles
custom tags away; Rust durably supervises custom-step attempts and notification
delivery, while isolated Bun workers execute the pinned operation and
definition-owned lifecycle scripts. Store v14 remains the approved persistence
shape and recovery never reopens current definition files. See
`docs/woml-reusable-definitions.md`.

Workflow Calls WC0 freezes `services.workflows.call()` as a managed capability,
with an independent durable child run, exact workflow-ID routing, stable call
identity, bounded JSON input/result, hidden lineage, and one versioned local
routing protocol. WC1 adds the frontend only: omitting `<triggers>` lowers a
call-only definition to Model v10, while existing triggered workflows retain
Models v1-v9. Definition Packages v4/v5 carry Model v10 when local modules are
present. WC2 now gives Rust exact target registration and atomic, idempotent
child admission through durable store v9 and truthful Run Event v9
`workflow_call` ingress. WC3 adds the deeply read-only Bun facade and routes an
admitted same-runtime child through the normal Rust DAG executor. The child is
an independent durable run; only its terminal JSON result crosses back into the
parent script. WC4 completes same-runtime crash/retry reattachment. WC5 adds
durable store v10 runtime ownership leases, authenticated private loopback
wake-up, pending-child scans, and explicit multi-input runtime activation. WC6
keeps child execution on that same DAG engine, rejects Human Approval targets
before admission, and adds separate versioned call progress plus bounded safe
parent/child inspection without widening the immutable event vocabulary. WC7
hardens every stored call read against corrupted identity and definition
bindings, proves transactional v9-to-v10 migration, benchmarks both local
routing paths, verifies the packed native CLI, and publishes the one-machine
deployment and security boundary. The complete operational contract is
`docs/woml-workflow-calls-production.md`.

Workflow Start v1 adds `services.workflows.start()` without adding a second
admission engine. It reuses the same target registry, durable child identity,
lineage, routing, and recovery authority, dispatches the admitted child, and
returns its run ID without observing terminal status. `context.payload` is the
public input name in scripts and WOML references. Frozen Rust projections and
Script Host frames retain their internal `trigger` field; Bun exposes
`payload` and keeps `trigger` only as a deprecated compatibility alias for old
compiled scripts.

WC4 makes the same-runtime boundary retry-safe. The first admitting attempt
stores one immutable call key and child run; later attempts and duplicate
delivery observe it instead of executing another child. One atomic index claim
elects the child executor. Hidden lineage rejects direct and indirect cycles,
and startup recovery rebuilds mutable call state from authoritative child
events. If a crash leaves the parent attempt ambiguous, that attempt still
fails closed even when the child already succeeded.

The Rust core is the execution authority. It validates the compiled model,
selects ready DAG nodes, owns branch/parallel/approval/retry decisions, appends
versioned events to SQLite, and rebuilds run context by folding those events. It
must not understand markup, interpolation syntax, or JavaScript source meaning.

The canonical N-API adapter is isolated in the `core/woml-native` crate, which
depends locally only on `woml-engine`. `woml-cli` builds its locked manifest
directly and stages its platform library as
`woml-core.<platform>-<arch>.node`. The retired combined-core compatibility shim
and its entire predecessor Rust package have been removed. `core/Cargo.toml` is a
virtual workspace containing only `woml-engine` and `woml-native`, with one
committed lockfile. There is one adapter implementation and one WOML execution
path.

This separation is enforced rather than documented by convention. The
`test:architecture-separation` gate rejects a restored chaining SDK or retired
Rust package, SDK imports from the WOML frontend or CLI, unexpected local
dependencies in `woml-native`, retired imports in the N-API adapter, drift
between the CLI's native contract and the addon's exact export surface, and any
retired runtime dependency in a clean packed install.
GitHub Actions runs the gate for every push and pull request, and the full
release journey runs it again before feature release tests.

The long-lived Bun host executes JavaScript because JavaScript is part of the
authoring experience. Each invocation runs in an isolated Worker with a real
timeout boundary and receives only the versioned bindings approved for its
model. Model v8 and Script Bindings v1 inject exactly `context`, `attempt`,
`services`, and source-proven `secrets` into capability scripts. Native Fetch,
managed HTTP, the SQLite/PostgreSQL Database v1 facade, durable Storage v1,
workflow-scoped Cache v1, and internal named Events Service v1 are active.
SC14 completes their shared documentation, composition coverage, packaging,
and release gate. Queue remains intentionally postponed. Bun reports
outcomes; it never decides whether to retry or how the graph advances.

### Services and capability boundary

SC0 freezes a generic, full-duplex capability boundary before adding provider
code. Script Host v4 permits multiple invocations and multiple nested calls to
be active; replies correlate by `{invocationId, callId}` and may arrive out of
order. Rust owns the capability registry, operation limits, cancellation,
idempotency identity, durable Run Event v8 append, and recovery. Bun owns the
JavaScript facades and isolated execution.

Native Fetch stays Bun's real Fetch implementation and uses redacted observed
events. `services.http.request()`, `services.db()`, `services.storage`,
`services.cache`, `services.events.emit()`, and future `services.*`
operations use Rust-managed calls. All converge on the generic `operation_started`,
`operation_succeeded`, and `operation_failed` vocabulary. Compiled models keep
only sorted secret names; resolved values exist only in the invocation-memory
boundary and never in context, events, progress, or fixtures. Native Fetch is
executed by Bun; `services.http.request()` is executed by a pooled Rust client;
and Database v1 uses Rust-owned SQLite and PostgreSQL pools. Storage v1 uses a
Rust-owned, checksummed local object directory beside WOML state and managed
HTTP can stream a response directly into it without copying the body through
Bun or context. Cache v1 uses a bounded local SQLite store beside WOML state;
Rust derives its workflow-ID namespace from the durable run binding without
exposing `context.run` or changing Capability Call v1. SQLite user
connections cannot open WOML's internal state database; PostgreSQL connection
strings and credentials never become safe event metadata. Internal Events v1
reuses the durable named-event fan-out authority without an HTTP loopback.
Other `services.*` capabilities remain unavailable until their individual
milestones. The services entry point is `docs/woml-services.md`; the
authoritative Database v1 guide is `docs/woml-database.md`; Storage v1 is documented in
`docs/woml-storage.md`; Cache v1 is documented in `docs/woml-cache.md`.

The local outbound-HTTP profile permits reachable HTTP(S) destinations and is
not an SSRF sandbox. Hosted deployments must apply network-layer egress policy,
including private/link-local denial, DNS-resolution checks, and redirect
revalidation. The authoritative operational guidance is
`docs/woml-http-services.md`.

### Durable retry boundary

Rust commits a failed attempt and its next retry schedule atomically. All
attempts of one logical step share `attempt.idempotencyKey`, allowing a capable
external service to deduplicate effects. A durably scheduled but unstarted
retry is safe to resume. A started attempt without a terminal event is
ambiguous, becomes `interrupted`, and fails closed instead of being replayed.
Only a successful attempt publishes `context.steps.<id>`.

### Production trigger admission boundary

The TypeScript frontend describes triggers in compiled Model v7, but it does
not accept external occurrences or create runs. Every listener, scheduler, and
provider adapter must submit a normalized occurrence to the Rust authority.
Rust validates the compiled trigger and atomically commits three related facts:
the immutable occurrence, its run-to-definition binding, and the run's first
`run_started` Event v7.

SQLite store schema v6 preserves the v4 one-run-per-occurrence guarantees,
adds durable schedule cursors in v5, and adds anchored interval cursors in v6.
Occurrences remain unique by the
workflow, trigger, and hashed source identity. The raw source identity is never
persisted. Payload hashes use RFC 8785 canonical JSON, so reordered object keys
do not create a second run. A same-payload replay returns the original run; a
changed-payload replay conflicts. If the process stops after commit but before
dispatch, recovery resumes that existing run rather than creating another one.
Contradictory occurrence, run, definition, or event history fails closed.

For cron schedules, the mutable cursor is the scheduler's durable coordination
record. Rust commits its advance in the same immediate transaction as the
immutable occurrence, run binding, and first event. Schedule-only and provider-
only runtimes do not bind an HTTP socket; webhook and named-event workflows
bind the configured listener.

For fixed-rate intervals, Rust persists the first registration anchor and the
next positive sequence. Every planned instant is recomputed as `anchor +
sequence × every`, so slow workflow runs cannot introduce timing drift. The
interval cursor advances in the same transaction as occurrence admission.
Restart recovery applies the same bounded `skip` or `run-once` policy as cron,
and only `{ scheduledAt, triggeredAt }` enters workflow context.

Named application events add a fan-out boundary above Trigger Ingress rather
than a second run creator. The frontend lowers each exact event name, symbolic
publisher-secret reference, and optional schema into Model v7. Event
Publication v1 deterministically matches
subscribers and sends one independently validated occurrence per matching
trigger through the existing Rust authority. Its source identity hashes the
publisher event ID together with workflow and trigger identity, allowing a
crash or publisher retry to finish missing deliveries without duplicating
already accepted runs. The Rust host serves the reserved authenticated HTTP
endpoint, and `woml emit` is a secret-store-backed client for the same public
contract. Resolved credential values remain outside compiled models, durable
workflow context, fixtures, and diagnostics.

The webhook listener is part of this WOML Rust path, not a compatibility
module. It validates transport, authentication, body size and JSON
Schema before admission, returns the durable run identity asynchronously, and
dispatches only newly admitted runs. A duplicate returns the original run
without executing it again. Server startup performs crash recovery once;
individual requests never run global recovery while other attempts may be
active. The T5 production gate covers concurrent requests, slow and malformed
clients, SQLite contention, listener and route conflicts, bounded bodies,
secret leakage, host failure, and composition with every already-published
control-flow primitive. Bearer routes retain only a fixed-width credential
digest after registration and compare candidate digests in constant time.

Slack uses a shared Bun transport foundation for symbolic credential
resolution, Web API calls, channel lookup/cache, Socket Mode connection and
reconnect, and secret-safe error classification. The approval adapter and the
Slack-trigger adapter remain separate protocol consumers. Matching app-token
credentials share one Socket connection, while each consumer owns its own
message semantics. The shared layer never acknowledges ordinary event
envelopes automatically: approval interactions acknowledge in the approval
adapter, while trigger events acknowledge only after durable Rust admission.
Slack event decoding, normalization, filtering, redelivery deduplication, and
execution are active in the T13 Production Triggers profile.

`woml run` is the long-lived activation lifecycle. The Bun CLI preflights the
definitions and symbolic secrets, then starts the Rust listener through N-API
and waits for SIGINT or SIGTERM. Rust runs Actix on a dedicated runtime thread,
owns every occurrence and background DAG execution, and emits versioned Trigger
Progress v1. Completing one run does not stop the activated workflow. `woml
test` is the separate one-shot manual journey, and `woml get` reads a safe
folded durable result.

Progress and diagnostics are printed to stderr. A successful asynchronous run
is then folded from durable state and its final JSON is printed with its run ID;
`woml get` provides the durable status later. One-shot manual results remain
JSON on stdout. Secrets and executable capabilities never enter context,
events, progress messages, or durable output.

The contracts between these layers are versioned artifacts under
`docs/schemas/` and `docs/protocols/`. Neither side may infer or silently add a
field that is absent from the negotiated version.

Model v11 keeps workflow-owned lifecycle hooks outside the business DAG.
Event v10 separates the durable business outcome from lifecycle finalization,
and Store v11 maintains the rebuildable summaries used by `woml list`. Rust
admits deterministic hook/action identities, executes lifecycle scripts through
isolated Bun workers, records notification failures as warnings, and owns
cancellation races. Direct `woml list`, `woml get`, and `woml cancel` commands
operate on local durable state without reopening workflow source. See
[Lifecycle and Local Run Control](woml-lifecycle-and-run-control.md).

Runtime Policies RP0–RP7 publish Model v12, Event v11, Store v12 coordination,
Run List v2, Run Inspection v3, and Runtime Policy Progress v1. TypeScript
validates `<config>` and lowers one language-neutral policy outside the DAG;
Rust owns atomic admission, cross-process claims, queue order, rolling-window
history, deadlines, recovery, and every ingress mapping. Definition Package v7
carries Model v12 with unchanged module artifacts. Events and immutable
definitions remain authoritative while scheduler claims, queue indexes, and
summaries are rebuildable coordination state. See
[Runtime Policies](woml-runtime-policies.md) and
[Runtime Policies v1](protocols/runtime-policies-v1.md).

Durable User State DS0–DS5 publishes `services.state` and adds Store v13's
workflow-scoped entry, immutable mutation-result, and quota authority. The
TypeScript frontend discovers state usage and generates editor types; Rust owns
canonical JSON, versions, compare-and-set, quotas, mutation reattachment, and
cross-process SQLite transactions. Bun sends the frozen State v1 request over
Capability Call v1; the engine supplies workflow scope and verified operation
identity, and only digest-based metadata reaches durable operation events. It
is deliberately not cache and is never injected into event-folded context.
Startup audits one consistent SQLite snapshot; a committed mutation record is
the settlement proof used to repair a missing success event without replaying
the mutation. Packaged multi-process, migration, integrity, redaction, and
performance gates publish the boundary. See [Durable User State v1](protocols/durable-state-v1.md),
[Durable User State Operations](woml-durable-state.md), and the
[data choice guide](woml-data-guide.md).

Production Runtime and Operations PRO0 freezes the single-machine production
boundary without adding a public build artifact: users continue to pass direct
`.woml` files and directories. Runtime Configuration v1 is optional. PRO1
extends `woml check` to validate multiple workflows as one deployment and, with
`--config`, preflight writable storage, disk headroom, listener separation, and
all required symbolic secrets without opening ingress or creating a run.
PRO2 adds atomic direct-source activation. PRO3 adds Store v14 leased ownership,
recovery-before-readiness, background `woml run -d`, authenticated exact-instance
`woml stop`, and bounded graceful shutdown. PRO4 adds strict production secret
sources, rotating loopback admin authority, authenticated live
`list`/`get`/`cancel`, and traffic/resource boundaries. PRO5 then adds bounded
store/live snapshots, ordered progress streaming,
structured logs, stable metrics, health probes, and telemetry failure
isolation. PRO6 adds the color terminal inspector. PRO7 adds Rust-owned coherent
SQLite backup, strict Backup Manifest v1 verification, guarded offline restore,
State v1 path portability, and transactional supported-store upgrade recovery.
Retention is implemented by the PRO8 Rust maintenance authority. See
[Production Runtime and Operations](woml-production-runtime.md) and the
[Production Runtime v1 contracts](protocols/production-runtime-operations-v1.md).
PRO9 packages and verifies this architecture as the supported continuous
single-machine Production Runtime v1 profile; deployment recipes do not widen
the one-owner/local-SQLite boundary.
