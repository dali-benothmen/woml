# WOML Services and Capabilities Implementation Plan

Status: active. SC0 through SC9 completed on 2026-08-10. The cross-layer
contracts are frozen, the TypeScript frontend emits Model v8 with source-proven
Script Bindings v1 and symbolic secret dependencies, and a real WOML script can
now call from an isolated Bun Worker through the durable Rust capability
authority. Native `fetch()` preserves Bun's API while Rust records safe,
durable observations. Managed `services.http.request()` now executes through
Rust with pooling, bounded results, cancellation, safe errors, and durable
operation events. The HTTP capability foundation is hardened and publishable.
Database v1 now provides one read-only `services.db()` facade backed by
Rust-owned SQLite and PostgreSQL pools, prepared SQL and CRUD, atomic
transaction batches, bounded/redacted results, cancellation, recovery, and
strict separation from WOML runtime state. Storage v1 now adds Rust-owned,
checksummed durable objects and managed HTTP direct-to-storage streaming without
copying large bodies through Bun or context. SC10 is next: local expiring cache.

## 1. Product Outcome

This milestone gives JavaScript inside a WOML `<script>` a small, dependable
set of built-in operations without requiring an npm package:

```js
fetch(...)

services.http
services.db
services.storage
services.cache
services.events
services.queue
```

The author writes normal JavaScript. WOML carries the operational complexity:
connection reuse, cancellation, timeouts, retries where safe, idempotency,
durable audit events, size limits, secret redaction, and helpful failures.

The first publishable journey is a real HTTP workflow:

```xml
<workflow id="customer-sync" name="Customer sync" version="1.0">
  <triggers>
    <manual id="runNow" />
  </triggers>

  <steps>
    <step id="loadCustomer">
      <script>
        const response = await services.http.request({
          method: "GET",
          url: "https://api.example.com/customers/42",
          headers: {
            authorization: `Bearer ${secrets.CUSTOMER_API_TOKEN}`
          },
          responseType: "json"
        });

        return response.data;
      </script>
    </step>
  </steps>
</workflow>
```

The same script may use Bun's native Fetch API instead:

```js
const response = await fetch('https://api.example.com/customers/42');
return await response.json();
```

Native `fetch()` remains familiar and standards-compatible. The managed
`services.http.request()` API is the documentation-first WOML experience: it is
more declarative, returns JSON-serializable values, and gives Rust stronger
control over the operation.

## 2. Product Principles

### 2.1 One simple namespace

Every WOML-owned capability lives under the reserved `services` binding. The
six built-in names are reserved and cannot be replaced.

The user never declares whether code is pure, effectful, Rust-backed, or
Bun-backed. WOML observes controlled operations when they occur.

### 2.2 Use the standard platform API when one exists

HTTP already has a universal JavaScript API, so WOML keeps native `fetch()`.
WOML instruments Bun's implementation rather than replacing it with a partial
Fetch clone.

There is no equivalent standard API for databases, storage, cache, events, or
queues, so those use `services.*`.

### 2.3 Rust supervises managed effects

Bun executes the user's JavaScript. Managed `services.*` calls cross a
versioned protocol and execute through a Rust capability handler. The Rust DAG
engine remains generic; provider logic lives in a registry of handlers adjacent
to the engine, not in graph traversal or event folding.

### 2.4 Track operations, not modules

The future Module System may expose user code under `services.<moduleName>`.
No module will need `kind="capability"`. Pure functions stay in Bun; calls to
native Fetch or a managed capability reach the same operation boundary defined
by this milestone.

This plan deliberately defines no `<modules>`, `<require>`, or `<import>`
syntax. Module loading, dependency resolution, packaging, permissions, and
reusable `.woml` composition belong to the separate WOML Module System
milestone.

### 2.5 No false exactly-once promise

An event log proves what WOML recorded; it cannot prove what an external server
did during a process crash. Started operations without a trustworthy terminal
record are ambiguous and fail closed. Unsafe effects are never automatically
replayed merely because a host restarted.

### 2.6 Useful before universal

Each service becomes advertised only when one real end-to-end backend works.
The public service family may later gain additional drivers without changing
the generic Rust/Bun capability protocol.

## 3. Baseline Entering This Milestone

The current production path is:

```text
.woml
  -> TypeScript parse / validate / lower
  -> compiled Model v7 DAG
  -> Rust durable engine and Run Event v7
  -> Script Host Protocol v3
  -> one isolated Bun Worker per script attempt
  -> context + attempt bindings
```

The existing Script Host v3 is asynchronous across invocations, but one
invocation is currently a one-way request followed by one terminal result. A
Worker cannot ask Rust to perform work while its script is running. The current
Worker injects only `context` and `attempt`.

This milestone adds a nested, full-duplex boundary:

```text
Rust sends execute(invocationId)
  -> Bun Worker starts the script
  -> script calls a service
  -> Bun sends capability_call(invocationId, callId)
  -> Rust durably starts and executes the call
  -> Rust sends capability_result(invocationId, callId)
  -> script continues
  -> Bun sends completed(invocationId)
```

Several invocations and several calls inside each invocation may be active at
the same time. Correlation, not message order, is authoritative.

Frozen Model/Event/Script Host v1-v7/v1-v3 artifacts remain immutable and
executable for stored older runs. Services use new negotiated versions.

## 4. What “Done” Means

Services and Capabilities are complete when:

1. Every executable Model in the new profile receives read-only `services` and
   `secrets` bindings in addition to the existing `context` and `attempt`.
2. Bun and Rust can multiplex nested capability calls while multiple script
   invocations are active, including `Promise.all` and out-of-order results.
3. Native `fetch()` retains Bun's real Request/Response/Headers/streaming and
   cancellation behavior while emitting safe operation observations.
4. `services.http.request()` executes through Rust and supports useful JSON,
   text, and bounded binary HTTP journeys.
5. `services.db()` executes a real user-owned SQLite and PostgreSQL database
   journey with safe parameterization and Rust-owned pools.
6. `services.storage` stores and retrieves durable objects through a first
   WOML-local backend and returns portable JSON references.
7. `services.cache` provides workflow-scoped, expiring JSON key/value
   operations without pretending cache data is authoritative workflow state.
8. `services.events.emit()` directly reuses the named-event fan-out authority
   without an HTTP loopback or publisher token.
9. `services.queue` has a complete producer-and-consumer journey; it is not
   advertised as a producer API with no usable consumer.
10. Rust durably records managed operation state and never persists secret
    values, authorization headers, raw database values, or unbounded bodies.
11. A capability failure reaches JavaScript as one catchable, documented error
    shape and maps safely into the existing step retry/failure model.
12. Interrupted or ambiguous calls fail closed, cancellation reaches active
    handlers, and completed service calls never manufacture a step output on
    their own.
13. Branch, parallel, approval, retry, and every production trigger compose
    with service calls through the same DAG execution loop.
14. Older workflows that do not use services behave exactly as before.
15. Each published service passes a clean-package manual journey and the full
    WOML release gate.

## 5. Scope

### Included

- The reserved `services` and `secrets` JavaScript bindings.
- Literal `secrets.NAME` discovery for scripts in the first profile; secret
  values remain absent from compiled models and durable history.
- A generic Rust capability registry and handler contract.
- A full-duplex, asynchronous Rust/Bun protocol with nested call IDs.
- A versioned generic operation-event vocabulary.
- Instrumentation around Bun's native global `fetch()`.
- Managed HTTP, SQL database, local durable object storage, local expiring
  cache, internal named-event publication, and a usable durable queue profile.
- Rust-owned limits, cancellation, error classification, safe metadata,
  idempotency information, and connection/resource reuse.
- Source-aware diagnostics, terminal progress, fixtures, recovery tests,
  benchmarks, packaging, and documentation.

### Not included

- Public JavaScript/TypeScript module imports or reusable `.woml` imports.
- `<db>`, `<http>`, `<storage>`, or other capability tags. Service tags remain
  a possible future shorthand over the same registry.
- Slack, Discord, WhatsApp, OpenAI, Google, email, CRM, or other product/provider
  capabilities.
- Arbitrary npm package installation inside a workflow.
- NoSQL behavior disguised as SQL behavior. MongoDB/document adapters may join
  `services.db` later with their own reviewed method contract.
- Unrestricted filesystem, environment, child-process, or raw-socket access.
- A claim that the local trusted-code Worker is already a multi-tenant security
  sandbox. OS-level tenant isolation belongs to Production Runtime.
- Automatic replay of ambiguous HTTP, database, or queue effects.
- Distributed connection pools, distributed cache coherence, broker clusters,
  multi-node queue leases, or active-active operation ownership.
- Lifecycle hooks, workflow cancellation UI/API, workflow-level rate limits,
  and durable user state. Service-call cancellation needed by the existing
  engine is included; the larger product controls remain later work.

## 6. Proposed JavaScript Contract

The author-facing shapes in this section are now backed by the SC0 frozen
contracts. Later phases must not grow fields opportunistically; a shape change
requires the appropriate versioned contract update.

### 6.1 Runtime bindings

New-profile scripts receive:

```ts
declare const context: Readonly<WomlContext>;
declare const attempt: Readonly<WomlAttempt>;
declare const secrets: Readonly<Record<string, string>>;
declare const services: Readonly<WomlServices>;
```

`context` remains limited to `context.trigger` and `context.steps`.
`context.run` remains unavailable. Service handles and secrets are executable
capabilities and must never be copied into context or returned as step output.

Only statically named secret access is executable in the first profile:

```js
secrets.CUSTOMER_API_TOKEN; // valid
secrets['CUSTOMER_API_TOKEN']; // rejected in v1
secrets[name]; // rejected in v1
Object.keys(secrets); // does not enumerate the project store
```

The frontend records required secret names, never values, in the compiled
definition. The CLI/Rust runtime resolves only those names before execution.
Missing values fail preflight before a trigger becomes active or a manual run
starts.

### 6.2 Native Fetch

Native Fetch remains the standard Bun call:

```js
const controller = new AbortController();
const response = await fetch(
  new Request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
    signal: controller.signal,
  })
);

if (!response.ok) throw new Error(`HTTP ${response.status}`);
return await response.json();
```

WOML installs its wrapper before user code executes and delegates the original
arguments directly to Bun's captured native function. It returns the native
`Response` object unchanged. WOML does not auto-parse, auto-throw on non-2xx,
or auto-retry.

The durable observation stores only safe metadata such as method, sanitized
origin/path, status, duration, and bounded byte counts. It does not persist
authorization/cookie headers, query secrets, request bodies, or response
bodies. Fetch resolves when response headers arrive; WOML does not wrap the
response stream merely to manufacture a later body-completed event.

### 6.3 `services.http`

The managed HTTP API is optimized for workflow automation rather than browser
compatibility:

```js
const response = await services.http.request({
  method: 'POST',
  url: 'https://api.example.com/orders',
  headers: {
    authorization: `Bearer ${secrets.ORDER_API_TOKEN}`,
  },
  json: {
    orderId: context.trigger.orderId,
  },
  responseType: 'json',
  timeout: '10s',
});

return response.data;
```

HTTP v1 must freeze:

- request fields and mutual exclusion of `json`, `text`, and bounded bytes;
- URL/query/header normalization;
- the exact `{ status, ok, headers, data }` result;
- `json`, `text`, `bytes`, and later `storage` response modes;
- default accepted status range and an explicit override;
- redirect, compression, timeout, cancellation, and TLS behavior;
- safe retry classification by method and failure kind;
- how a caller opts into an external `Idempotency-Key` header; and
- request/response/frame byte limits.

Rust performs the request. Results crossing back into Bun are JSON-compatible.
Large bodies use `services.storage` rather than entering context.

### 6.4 `services.db`

`services.db()` returns a lightweight JavaScript proxy; the real pool and
connections remain in Rust:

```js
const db = services.db({
  driver: 'postgres',
  connection: secrets.DATABASE_URL,
});

const rows = await db.query({
  text: 'SELECT id, name FROM customers WHERE active = $1',
  values: [true],
});

await db.insert({
  table: 'audit_log',
  values: {
    customer_id: rows[0].id,
    action: 'loaded',
  },
});
```

SQL v1 includes parameterized `query`, ergonomic `read`/`insert`/`update`/
`delete`, `execute`, and an explicit transaction batch. It does not hold a
database transaction open across an arbitrary JavaScript callback in v1.

The first zero-setup conformance backend is a user-owned SQLite database; it
must never expose WOML's internal state database. PostgreSQL is the first
production database driver. SQL text and parameters are not persisted in run
events; diagnostics use a statement fingerprint and safe timing/count
metadata.

### 6.5 `services.storage`

Storage keeps large or reusable objects outside workflow context:

```js
const object = await services.storage.put({
  key: 'reports/2026-08-09.json',
  value: report,
  contentType: 'application/json',
});

const saved = await services.storage.get({
  key: object.key,
  responseType: 'json',
});
```

The first backend is a WOML-owned local object directory beside the selected
state location. Handles are JSON objects containing a backend-neutral key,
version/checksum, size, and content type. Paths are normalized and cannot
escape the store. Events retain references and digests, never object bodies.

V1 includes `put`, `get`, `head`, `list`, and `delete`, with explicit overwrite
and conditional-write behavior. External object-store adapters come later.

### 6.6 `services.cache`

Cache provides best-effort, workflow-scoped JSON values:

```js
await services.cache.set('customer:42', customer, { ttl: '15m' });
const cached = await services.cache.get('customer:42');
```

V1 includes `get`, `set`, `delete`, `has`, `increment`, and `setIfAbsent` with
bounded keys, JSON values, and TTLs. Atomic mutations happen in Rust. Cache
entries may expire or be evicted and therefore must not control workflow
correctness. Cache is not the future durable user-state feature.

The first backend is local to one WOML runtime/state location. Distributed
cache adapters and coherence belong to Production Runtime or the Module System.

### 6.7 `services.events`

Internal event publication reuses the Event Publication v1 authority directly:

```js
const publication = await services.events.emit('customer.updated', {
  customerId: 'customer-42',
});
```

No HTTP loopback or control token is used for a workflow-originated event.
Rust validates subscribers, creates occurrence/run records atomically, and
returns safe publication/run identities. Event publication receives a stable
operation identity so a safe retry does not duplicate subscriber runs.

The engine adds internal causation/lineage and bounded depth/fan-out protection
so workflows cannot create an unbounded event cycle. That metadata remains
engine control data and does not silently expand `context.run`.

### 6.8 `services.queue`

Events fan out to every matching subscriber. A queue delivers one message to
one consumer. WOML must preserve that distinction.

Producer:

```js
await services.queue.send('image-processing', {
  imageId: 'image-42',
});
```

The first usable queue profile includes a durable WOML-owned queue and a
reviewed queue-trigger consumer contract. `services.queue` is not advertised
until a message can be enqueued, claimed once, bound to a durable run,
acknowledged or failed, recovered after restart, inspected, and dead-lettered
under a frozen policy.

Advanced queue concurrency, multiple competing deployments, external brokers,
priority, and distributed leases remain later work. The first profile may
reject more than one active consumer for the same local queue rather than
inventing premature distributed selection semantics.

## 7. Capability Registry and Operation Model

Rust owns a generic registry:

```text
Capability Registry
  http.request
  db.*
  storage.*
  cache.*
  events.emit
  queue.*
```

Each handler declares:

- a stable capability and operation name;
- input and output contract versions;
- execution and cancellation behavior;
- byte/time/concurrency limits;
- read, idempotent-write, or unsafe-write effect classification;
- retryable, final, and ambiguous failure classifications;
- safe diagnostic metadata extraction; and
- whether it supports an engine idempotency key.

The DAG engine dispatches a generic operation. It does not contain a switch for
PostgreSQL, HTTP status codes, storage keys, or queue payloads.

### 7.1 Call identity

Every invocation-local call has a unique `callId`. Every managed operation also
receives the current step's stable `attempt.idempotencyKey` plus an operation
identity. Rust-owned handlers such as storage, event publication, and the local
queue use that identity for deduplication where their semantics permit it.

The operation contract must distinguish:

- correlation identity: unique for one call attempt;
- logical effect identity: stable where a safe retry must deduplicate; and
- provider idempotency identity: passed externally only under the reviewed
  service-specific contract.

Call ordering alone is not a universal exactly-once mechanism for arbitrary
dynamic JavaScript. SC0 must freeze the automatic single-call behavior and the
explicit naming/key escape hatch for a step that performs several writes.

SC5 freezes that escape hatch as an optional second argument:
`services.http.request(request, { name: "stable-name" })`. A step may use one
automatic effectful `http.request`; additional effectful calls require stable
names. Read-only HTTP calls remain freely multiplexable.

### 7.2 Persistence boundary

Rust appends operation-started before dispatching a managed effect. A terminal
success or failure is appended before the result is released back to Bun.

Operation result bodies are not replay caches in v1. The surrounding script is
still one step attempt. If it crashes after a service call succeeded, WOML does
not reconstruct and continue the JavaScript instruction pointer. Existing step
retry rules, provider idempotency, and fail-closed ambiguity still apply.

### 7.3 Concurrency and backpressure

This must work:

```js
const [profile, orders] = await Promise.all([
  services.http.request({ url: profileUrl, responseType: 'json' }),
  services.http.request({ url: ordersUrl, responseType: 'json' }),
]);
```

Calls correlate by `{ invocationId, callId }`; replies may arrive out of order.
Per-invocation and global in-flight limits prevent one script from exhausting
the runtime. Exceeding a limit produces a catchable error rather than an
unbounded queue in memory.

## 8. Versioned Contracts

SC0 creates and reviews these artifacts before handler implementation:

1. Compiled Workflow Model v8, adding only the runtime-binding/required-secret
   metadata needed by the new script profile.
2. Attempt Failure v3, preserving uncaught service failures without flattening.
3. Run Event v8, adding a generic operation vocabulary while retaining all v7
   trigger and v1-v6 control-flow events.
4. Script Host Protocol v4, adding full-duplex nested calls and replies without
   modifying v3.
5. Capability Call v1, defining generic call, result, safe failure, size-limit,
   cancellation, and idempotency envelopes.
6. Service Progress v1, for safe terminal diagnostics that never enter context.
7. Native Fetch Observation v1.
8. Managed HTTP v1.
9. Reviewed `.woml`, compiled-model, event-history, protocol, concurrency,
   UTF-8/CRLF, size-limit, cancellation, and crash fixtures.

Database, storage, cache, events, and queue receive separately versioned public
operation contracts before their implementation phase. They reuse Capability
Call v1 and generic Run Event v8 rather than forcing another hot-path protocol
version for every handler.

Protocol v4 must reserve generic failure slots for request/result too large,
unsupported capability/version, invalid input/result, timeout, cancellation,
handler crashed, host crashed, and ambiguous/interrupted execution.

## 9. Run Event v8 and Recovery

The proposed generic durable vocabulary is:

```text
operation_started
operation_succeeded
operation_failed
```

Each event identifies the run, node, step attempt, invocation, call, logical
operation, capability, method, execution mode (`observed` or `managed`), and
safe timestamps/metadata.

Native Fetch uses observed mode. A managed `services.*` handler uses managed
mode. Adding a future custom module does not require a new event type.

Folding checks:

- one start before one terminal event;
- no duplicate terminal events;
- the operation belongs to the active step attempt;
- call IDs cannot cross invocations/runs;
- result/failure contract versions match the registered capability;
- a step cannot succeed while a managed call is still active; and
- a recovered started-without-terminal operation makes the attempt ambiguous
  unless that handler's separately frozen recovery contract proves otherwise.

Operation events do not add values to `context.steps`. Only
`step_attempt_succeeded` publishes the script's final JSON result.

## 10. Errors and JavaScript Behavior

Managed services reject with a shared safe error shape:

```ts
interface WomlServiceError extends Error {
  readonly code: string;
  readonly service: string;
  readonly operation: string;
  readonly retryable: boolean;
  readonly ambiguous: boolean;
  readonly details?: Readonly<Record<string, JsonValue>>;
}
```

Service-specific details may include safe status/category information, never
credentials or an unbounded provider body. Authors can catch the error in
ordinary JavaScript. If it leaves the script, the host preserves its canonical
classification instead of flattening it into a generic string.

Native Fetch keeps Bun's native `TypeError`, `DOMException`, and response
behavior. WOML does not change non-2xx into an exception. A failure to durably
record an observed request before dispatch prevents the request and produces a
specific WOML tracking error; WOML never silently sends an untracked request
after claiming it is tracked.

## 11. Secrets and Security

The first service profile enforces:

1. Compiled models contain secret names only.
2. Resolved values live only in runtime memory and are never serialized to
   models, events, SQLite metadata, progress, fixtures, or terminal logs.
3. Only statically referenced secrets are injected into a script attempt.
4. `services` and `secrets` are deeply read-only runtime bindings.
5. Returning a known secret value, directly or nested, is rejected before the
   step result is persisted.
6. HTTP authorization/cookie headers and sensitive query values are redacted.
7. Database connection strings, SQL parameters, cache values, queue payloads,
   event payloads, and storage bodies do not enter operation audit events.
8. Capability input/result/frame sizes are checked on both Rust and Bun sides.
9. Dynamic import and known raw networking bypasses remain unsupported in this
   no-module profile and receive explicit diagnostics where enforceable.
10. Production multi-tenant isolation is not claimed until an OS-level sandbox
    exists; this milestone still treats authored local JavaScript as trusted
    code.

Because native Fetch needs real credential strings, the Bun Worker necessarily
receives the explicitly referenced values. Rust tracking reduces accidental
leakage; it cannot prevent deliberately authored code from sending a secret to
an external server.

## 12. Performance Contract

Native Fetch is the latency baseline because Bun performs the request directly.
Managed HTTP adds a protocol and durable-event boundary and must not be marketed
as faster without evidence.

The benchmark suite measures:

- warm single-request p50/p95 latency;
- cold and warm connection behavior;
- 1, 10, and 100 concurrent requests;
- small JSON, medium text, and bounded binary responses;
- Rust and Bun connection reuse;
- serialization/copy cost;
- direct-to-storage behavior; and
- event-store overhead.

Managed HTTP may win on throughput or large-body workflows through Rust pooling,
bounded concurrency, and avoiding a Bun round trip for direct-to-storage
responses. Durability is never skipped solely to win a benchmark.

## 13. Implementation Phases

### SC0 — Freeze the capability contracts and reviewed fixtures

Status: completed on 2026-08-09.

Changes:

- Resolve every blocking item in Sections 6-11.
- Freeze Model v8, Attempt Failure v3, Run Event v8, Script Host v4,
  Capability Call v1, Service Progress v1, Native Fetch Observation v1, and
  Managed HTTP v1.
- Freeze native-Fetch compatibility, managed-HTTP result/error/status behavior,
  logical operation identity, retry/ambiguity, limits, and cancellation.
- Add exact source, compiled-model, operation-history, protocol, multiplexing,
  UTF-8/CRLF, secret, and crash fixtures.
- Confirm `context.run`, module syntax, service tags, and engine-control APIs
  remain unresolved and unavailable rather than receiving defaults.

Result:

Every layer has a reviewed contract to build, but no service is executable yet.

Gate:

All schemas validate positive fixtures, reject malformed/cross-run/duplicate
histories, contain explicit size-limit failures, and leave no protocol-shape
decision as an undocumented TODO.

### SC1 — Compile the new runtime bindings and secret dependencies

Status: completed on 2026-08-09.

Changes:

- Add the Model v8 script profile without changing the WOML `<script>` syntax.
- Discover literal `secrets.NAME` references with a real JavaScript parser,
  ignoring comments and strings and rejecting computed access.
- Lower required symbolic names and binding versions into the model.
- Add source-located errors for unsupported binding use.
- Keep Models v1-v7 byte-for-byte compatible.

Result:

The frontend can prove which secrets and runtime bindings a script requires
before the workflow starts.

Gate:

Reviewed WOML deep-equals Model v8; missing/dynamic/false-positive secret cases
produce exact source-aware diagnostics and no value enters the model.

### SC2 — Build the Rust capability authority and event folding

Status: completed on 2026-08-09.

Changes:

- Implement the generic capability registry and handler trait.
- Implement Capability Call v1 validation, limits, cancellation, safe errors,
  and a test-only echo/delay/failure handler.
- Add Run Event v8 structures, folding, SQLite persistence, and recovery rules.
- Derive and validate correlation/logical operation identities.
- Keep capability-specific decisions out of DAG traversal.

Result:

Rust can durably supervise a generic operation without Bun or a real provider.

Gate:

In-memory and SQLite folds match every frozen history; concurrent, duplicate,
oversized, cancelled, and interrupted fake operations obey the contract.

### SC3 — Implement Script Host v4 and inject `services`/`secrets`

Status: completed on 2026-08-09.

Changes:

- Add full-duplex Content-Length-framed v4 messages.
- Route Worker call requests through the long-lived Bun host to Rust and route
  correlated replies back to the correct Worker.
- Support several calls per invocation, several invocations per host, and
  out-of-order replies.
- Inject deeply read-only `services` and resolved `secrets` bindings into new
  isolated Workers.
- Preserve timeout, cancellation, host/Worker crash, context/result limits, and
  v1-v3 compatibility.

Result:

A real WOML script can call the test capability from Bun, await Rust, and use
the JSON result. Model v8 CLI runs now resolve only each script's declared
secret names, use the durable Rust execution path automatically, and reject a
known secret before it can become a persisted step result. Native `fetch()`
remains explicitly unavailable until SC4 rather than running untracked.

Gate:

Protocol tests cover `Promise.all`, two simultaneous runs, duplicate IDs,
unknown replies, host/Worker crashes, cancellation races, multibyte content,
literal CRLF, backpressure, and secret-result rejection.

### SC4 — Instrument Bun's native Fetch without changing it

Status: completed on 2026-08-09.

Changes:

- Capture and wrap Bun's native global `fetch` before script execution.
- Pass native inputs through and return Bun's native `Response` unchanged.
- Durably acknowledge safe request-start metadata before dispatch.
- Report response status when headers arrive, or native failure, without
  consuming bodies or persisting response headers.
- Preserve Request, Response, Headers, FormData, Blob, streams, redirects,
  AbortSignal, binary data, and native error behavior.
- Add unsupported raw-network/dynamic-import diagnostics for the current
  no-module profile.

Result:

Ordinary `fetch()` works as it does in Bun and its external operation is
visible to WOML automatically. Rust acknowledges the redacted start before Bun
dispatches, records success or failure without consuming the response body,
and closes interrupted observations as ambiguous without replay. The public
CLI now executes Model v8 Fetch workflows, while the frontend rejects known
raw-network and runtime-module bypasses in the current no-module profile.
Stored Model v1-v7 workflows retain their pre-observation Fetch behavior for
backward compatibility; newly compiled Fetch scripts always target Model v8.

Gate:

A Fetch compatibility suite compares wrapped and unwrapped Bun behavior across
the frozen cases; local HTTP tests prove safe events, cancellation, concurrent
fetches, redaction, and fail-closed crash handling.

### SC5 — Execute `services.http.request()` through Rust

Status: completed on 2026-08-09.

Changes:

- Implement the frozen JavaScript facade and Rust HTTP handler.
- Add connection pooling, JSON/text/bounded-byte bodies, status handling,
  redirects, compression, timeouts, cancellation, limits, and safe errors.
- Integrate the stable operation identity and reviewed idempotency behavior.
- Produce JSON-compatible results and source-aware terminal diagnostics.
- Add a real example comparing native Fetch and managed HTTP.

Result:

The first managed service works end to end from `.woml` through Rust and back
to the script. Authors receive only `{ status, ok, headers, data, url,
redirected }`; request credentials, query values, and bodies do not enter the
durable operation metadata.

Gate:

A local deterministic server proves methods, bodies, headers, statuses,
redirects, timeout, cancellation, duplicate protection, concurrency, malformed
responses, size limits, and no credential/body persistence.

### SC6 — Harden and publish the HTTP capability foundation

Status: completed on 2026-08-10.

Changes:

- Test process/Worker/server crashes at every start/send/receive/persist/reply
  boundary.
- Compose native Fetch and managed HTTP with retry, branch, parallel, approval,
  manual, webhook, Slack, schedule, interval, and event triggers.
- Add SSRF/network-policy documentation for local and hosted profiles.
- Add latency/throughput benchmarks against native Bun Fetch.
- Update language, architecture, CLI, recovery, security, and deployment docs.
- Add the HTTP example to clean-package smoke tests.

Result:

Capability Call v1, Script Host v4, native Fetch tracking, and
`services.http.request()` are independently publishable. Their crash/recovery
matrix, composition coverage, local-versus-hosted network boundary, deployment
guidance, benchmark method, and clean-package journey are explicit release
artifacts rather than implied behavior.

Gate:

The HTTP-specific and full WOML release gates pass, older workflows remain
unchanged, and performance claims match recorded benchmark evidence.

### SC7 — Freeze and execute SQL Database v1 with user-owned SQLite

Status: completed on 2026-08-10.

Changes:

- Freeze Database v1 configuration, result, transaction, error, limit, and
  redaction contracts.
- Implement the `services.db()` proxy and Rust pool registry.
- Implement parameterized query/execute and reviewed CRUD helpers.
- Implement explicit transaction batches with rollback on failure.
- Add a user-owned SQLite backend separated from WOML state storage.

Result:

A zero-setup workflow can create, insert, read, update, and transactionally
modify its own SQLite database through Rust.

Gate:

Tests cover injection attempts, parameter types, constraints, rollback,
contention, cancellation, large results, ambiguous writes, path isolation, and
proof that the internal state database cannot be opened as a user database.

### SC8 — Add PostgreSQL and harden `services.db`

Status: completed on 2026-08-10.

Changes:

- Add PostgreSQL connection parsing, Rust-owned pooling, prepared parameters,
  transaction batches, cancellation, and server error classification.
- Reuse Database v1 without changing the JavaScript surface.
- Add pool reuse and bounded concurrency diagnostics without logging connection
  strings, SQL parameters, or row data.
- Document SQL portability and explicitly defer document/NoSQL method sets.

Result:

`services.db` has a production database backend as well as a zero-setup local
backend.

Gate:

SQLite/PostgreSQL conformance tests return the same portable results where the
contract promises portability; restart, connection-loss, transaction, pool,
retry-safety, packaging, and secret-scan tests pass.

### SC9 — Build durable object storage

Status: completed on 2026-08-10.

Changes:

- Freeze Storage v1 operations and portable object-reference schema.
- Implement the local Rust object store with atomic put, checksum/version,
  bounded reads, conditional overwrite, listing, and deletion.
- Keep bodies outside events/context unless the script explicitly returns a
  bounded value.
- Add managed HTTP direct-to-storage support for large responses.

Result:

Workflows can safely retain files and large objects without placing their
contents in the event log.

Gate:

Tests cover traversal, symlinks, partial writes, checksum mismatch, concurrent
writes, conditional conflict, restart, deletion, limits, and HTTP-to-storage
without a large Bun/context copy.

### SC10 — Build the local expiring cache

Changes:

- Freeze Cache v1 keys, JSON values, TTLs, atomic operations, namespacing, and
  eviction semantics.
- Implement workflow-scoped `get`, `set`, `delete`, `has`, `increment`, and
  `setIfAbsent` in Rust.
- Add deterministic-clock expiry tests and bounded local storage.
- Make diagnostics explicitly describe cache misses as normal behavior.

Result:

Workflows can avoid repeated expensive reads without treating cache as durable
business state.

Gate:

Atomicity, expiry boundaries, eviction, concurrent runs, definition updates,
restart behavior, limits, and isolation tests pass.

### SC11 — Publish internal named events from workflows

Changes:

- Freeze Events Service v1 input/result/error contracts.
- Route `services.events.emit()` directly into the existing Rust Event
  Publication authority.
- Derive stable publication identity from the operation identity.
- Add internal lineage, cycle-depth, and fan-out limits.
- Report accepted, duplicate, partial, and failed subscriber outcomes safely.

Result:

One workflow can start every matching event workflow without an HTTP call,
control token, or duplicate run.

Gate:

Tests cover zero/one/many subscribers, duplicate retry, partial fan-out
recovery, schema mismatch, cycles, maximum depth/fan-out, concurrent emission,
and coexistence with the public event HTTP endpoint.

### SC12 — Freeze the usable queue contract

Changes:

- Freeze Queue v1 producer, message, consumer trigger, lease, acknowledgement,
  failure, redelivery, dead-letter, ordering, and recovery semantics.
- Define the first queue-trigger syntax and Model/Event additions without
  changing existing event fan-out semantics.
- Decide single-consumer registration, message/run identity, visibility time,
  maximum deliveries, and CLI inspection.
- Add reviewed source/model/history fixtures before queue execution code.

Result:

Queue has one complete product meaning and is no longer an attractive but
underspecified method name.

Gate:

Fixtures prove one-consumer delivery, event-versus-queue differences, crash
boundaries, redelivery/dead-letter rules, and contain no unresolved defaults.

### SC13 — Execute and publish the durable local queue

Changes:

- Implement `services.queue.send()` and the Rust durable local queue.
- Activate the reviewed queue trigger in `woml run`.
- Atomically claim a message and bind it to one occurrence/run.
- Implement success acknowledgement, reviewed failure/redelivery behavior,
  dead-letter storage, restart recovery, progress, and inspection.
- Preserve stable send identity across safe retries.

Result:

One workflow can enqueue work and one consumer workflow can process it exactly
according to the frozen local queue contract.

Gate:

End-to-end tests cover producer/consumer processes, FIFO boundaries, duplicate
send, consumer crash, run failure, redelivery, dead-letter, restart, graceful
shutdown, and no message loss at transactional boundaries.

### SC14 — Complete Services and Capabilities

Changes:

- Run cross-service compositions, including parallel HTTP/DB calls, event-to-
  queue flows, storage references, cache misses, approvals, and retries.
- Test resource exhaustion, shutdown, recovery, corrupt histories, version
  compatibility, and secret leakage across every capability.
- Add examples and manual instructions for all six built-ins and native Fetch.
- Update architecture, language, CLI, security, deployment, recovery, and SDK
  migration documentation.
- Add clean-package smoke journeys and a single Services release gate.
- Preserve the generic registry extension point for the future Module System
  without publishing module syntax.

Result:

All six built-in services and tracked native Fetch are supported, documented,
packaged, and ready for the next WOML milestone.

Gate:

Frontend, Rust, Bun host, protocol/schema, typecheck, Clippy, integration,
benchmark-regression, packaging, compatibility, and secret/artifact scans pass
from a clean installation.

## 14. Expected File Areas

| Area                         | Expected locations                                                       |
| ---------------------------- | ------------------------------------------------------------------------ |
| WOML model/lowering          | `woml/src/compiler.ts`, `woml/src/model.ts`, frontend tests and fixtures |
| Script analysis and bindings | new WOML script-analysis helpers, Model v8 fixtures                      |
| Bun host and Worker          | `woml-cli/src/script-host/*`, `script-host.ts`, Worker tests             |
| Rust protocol/host           | `core/woml-engine/src/protocol.rs`, `host.rs`                            |
| Capability registry          | new `core/woml-engine/src/capabilities/*` modules                        |
| Events/folding/store         | `event.rs`, `projection.rs`, `durable.rs`, SQLite migrations             |
| Native Fetch facade          | new Bun Worker runtime/fetch modules and conformance tests               |
| Service JavaScript facades   | new `woml-cli/src/services/*` modules and public typings                 |
| Rust service handlers        | HTTP, database, storage, cache, events, and queue capability modules     |
| Trigger/runtime bridge       | Rust runtime and frontend changes for the queue consumer profile         |
| CLI/secrets/progress         | `woml-cli/src/cli.ts`, secret store/resolution, `rust-executor.ts`       |
| Versioned artifacts          | `docs/schemas/*`, `docs/protocols/*`, compiled/event fixtures            |
| Examples                     | new native Fetch and one example per built-in service                    |

Exact filenames may change as the capability subsystem takes shape. Layer
ownership may not: TypeScript understands source and symbolic dependencies,
Rust owns durable managed operations, and Bun executes user JavaScript and
preserves native Fetch behavior.

## 15. Verification Matrix

| Area          | Required proof                                                                            |
| ------------- | ----------------------------------------------------------------------------------------- |
| Compatibility | Models v1-v7 and Script Host v1-v3 execute unchanged.                                     |
| Multiplexing  | Nested calls and invocations correlate only by IDs and may complete out of order.         |
| Fetch         | Wrapped behavior matches native Bun for every frozen compatibility case.                  |
| HTTP          | Rust handles status, body modes, timeout, cancellation, limits, pooling, and safe errors. |
| Database      | Parameterization, transactions, pool isolation, and ambiguous-write rules hold.           |
| Storage       | Objects are atomic, bounded, addressable, and absent from run-event bodies.               |
| Cache         | TTL/atomicity work; eviction never masquerades as durable state loss.                     |
| Events        | Internal emit reuses durable fan-out and cannot form an unbounded cycle.                  |
| Queue         | One message follows the frozen claim/ack/redelivery/dead-letter lifecycle.                |
| Retry         | Safe failures follow reviewed policy; ambiguous effects fail closed.                      |
| Parallel      | Calls are independent, bounded, cancellable, and do not cross contexts.                   |
| Secrets       | Only symbolic names are durable; known values cannot enter results/logs/events.           |
| Recovery      | Every started-without-terminal boundary has an explicit outcome.                          |
| Performance   | Benchmarks report evidence; no unsupported faster-than-Fetch claim is published.          |
| Packaging     | Every example works from the produced clean package.                                      |

## 16. Risks and Guardrails

### A script is becoming a nested effect orchestrator

Multiple service calls inside one script are useful, but the engine cannot
resume a JavaScript instruction pointer after a crash. V1 records each call and
fails ambiguous attempts closed; it does not pretend to offer Temporal-style
deterministic replay inside arbitrary JavaScript.

### Native Fetch cannot be both untouched and Rust-executed

Native Fetch is executed by Bun and observed by Rust. Managed HTTP is executed
by Rust. Documentation must keep that difference honest while presenting both
as supported choices.

### Tracking Fetch is not a complete sandbox

Instrumentation covers global Fetch. The no-module profile can reject known
bypass APIs, but production protection against malicious authored code requires
OS-level sandboxing. Do not market source scanning as a security boundary.

### Service result serialization can become the hot path

Large responses crossing Rust -> host -> Worker and later returning as step
output multiply copies. Size limits, storage references, direct-to-storage, and
benchmarks are required from the first HTTP release.

### Database abstraction can become misleading

`services.db` is one discoverable entry point, not a promise that SQL and
document stores share semantics. Each driver family gets a reviewed method
contract. SQL v1 must not hardcode assumptions that make future document
drivers pretend to be relational.

### Cache is not user state

Cache may disappear. Durable workflow-owned state remains a separate engine
feature with stronger concurrency and history semantics.

### Queue can silently become a second workflow engine

Queue consumption must enter through the same occurrence authority and DAG
runtime. It may not introduce a provider-owned executor, mutable context, or a
second persistence authority.

### Event loops can create runaway automation

Workflow-originated events carry internal lineage and enforce bounded depth and
fan-out without exposing an undefined `context.run` contract.

## 17. Global Roadmap After Services and Capabilities

1. **Retries and idempotency** — completed in RI7.
2. **Production triggers** — completed in T13: manual, webhook, Slack,
   schedule, interval, and named event.
3. **Services and capabilities** — this SC0-SC14 milestone: native Fetch plus
   HTTP, database, storage, cache, events, and queue built-ins.
4. **WOML Module System** — import local JavaScript/TypeScript modules, expose
   user-built operations under `services.*`, import reusable `.woml` files,
   bundle dependencies, pin hashes/versions, and add a React-like composition
   experience. Exact `<import>` syntax remains deliberately unfrozen.
5. **Lifecycle and engine controls** — workflow cancellation, lifecycle hooks,
   workflow-level concurrency/rate limits/timeouts, advanced queue controls,
   and durable user state.
6. **Production runtime and operations** — hosting, deployment, multi-node
   ownership, OS-level isolation, observability, retention, administration,
   distributed storage/cache/queue adapters, and scaling.
7. **Additional infrastructure adapters** — document databases, external
   object storage, distributed caches, and external brokers according to demand.
8. **Additional communication providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when justified.
9. **Retire the JavaScript chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

After SC14, the next design milestone is the WOML Module System. It reuses the
capability registry and operation protocol built here rather than creating a
second service architecture.

## 18. SC0 Review Gate

Status: passed on 2026-08-09. SC1 began only after the following items were
explicitly resolved in the frozen schemas, protocol documents, and reviewed
fixtures:

- the exact managed HTTP request/result/non-2xx behavior;
- native Fetch observation timing and failure behavior;
- operation correlation, logical effect, and provider-idempotency identities;
- multiple mutating calls inside one retried step;
- Capability Call v1 nested multiplexing, cancellation, and crash taxonomy;
- Run Event v8 safe metadata and recovery rules;
- literal script-secret discovery and secret-result rejection;
- global/per-invocation call, frame, input, result, and time limits;
- managed-service retry versus step retry interaction;
- SQLite/PostgreSQL database result and transaction direction;
- local storage/cache namespace and retention direction;
- event causation depth/fan-out policy; and
- the queue producer/consumer lifecycle before `services.queue` is advertised.

The following remain explicitly deferred and were not silently resolved by
SC0 or SC1: `context.run`, arbitrary module imports, reusable `.woml` module
syntax, service tags, unrestricted npm execution, general engine-control
operations, and multi-node runtime ownership.
