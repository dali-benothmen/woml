# WOML Rust Core Integration Plan

Status: R0–R6 complete; Rust is the only WOML workflow executor; conditional
branches and bounded parallel execution are supported post-cutover expansions

This document defines how WOML moves from the validated TypeScript execution
slice to the intended production architecture: Bun and TypeScript own the WOML
language frontend, Rust owns workflow orchestration and state, and Bun executes
JavaScript in isolated workers.

The earlier `WOML CLI Vertical Slice Plan.md` proved that a real `.woml` file
can be parsed, compiled, and executed. Its Phase 5 packaging exercise was
completed during R5 against the production Rust execution path.

## 1. Product Outcome

The target user experience does not change:

```bash
woml run hello.woml
```

The implementation behind that command changes from a temporary TypeScript DAG
executor to the authoritative Rust workflow engine.

The first Rust milestone must still print exactly:

```json
{ "message": "Hello World" }
```

The migration is successful only when the same reviewed `hello.woml`, compiled
model fixture, and expected output pass through both the TypeScript reference
executor and the Rust executor.

## 2. Final Architecture

```text
.woml file
    │
    ▼
WOML TypeScript frontend
parse → validate → compile
    │
    ▼
Compiled Workflow Model JSON
schemaVersion: 1, 2, or 3
    │
    ▼ N-API
Rust core
validate model → append events → fold state → select ready nodes
    │
    ▼ Content-Length-framed JSON over child stdin/stdout
Long-lived Bun script host
    │
    ▼ fresh Worker for every invocation
JavaScript `<script>` execution
    │
    ▼ JSON result or typed failure
Rust appends terminal attempt event and continues the DAG
```

### 2.1 Ownership

| Concern                                             | Owner                    |
| --------------------------------------------------- | ------------------------ |
| Read `.woml` source                                 | Bun CLI                  |
| Parse XML and preserve raw script bodies            | WOML TypeScript frontend |
| Validate WOML tags and attributes                   | WOML TypeScript frontend |
| Lower WOML into the compiled DAG                    | WOML TypeScript compiler |
| Validate the compiled-model boundary                | Rust core                |
| Select ready nodes and control workflow progression | Rust core                |
| Own the run event log and fold derived context      | Rust core                |
| Supervise the script-host process                   | Rust core                |
| Execute JavaScript                                  | Isolated Bun Worker      |
| Format terminal output and diagnostics              | Bun CLI                  |

The Rust core never understands XML, WOML tags, `{{ }}` syntax, editor
concerns, or raw WOML source. It consumes only the compiled workflow model.

The WOML frontend never becomes the authoritative owner of run state.

## 3. CLI to Rust Boundary

The Bun CLI loads the Rust core through N-API.

This is a conscious packaging decision, not an accidental dependency. Cronflow
already builds and distributes its Rust core with `napi-rs`, so N-API reuses the
existing native-addon toolchain and avoids adding a second CLI-to-engine
subprocess protocol.

Consequences:

- WOML continues to ship native binaries for each supported operating system
  and architecture.
- The Rust core lives inside the Bun CLI process.
- Rust owns a separate Bun script-host child process.
- A subprocess-based Rust engine is not part of v1.

The initial N-API surface must remain small and compiled-model-oriented. It must
not accept the old JavaScript SDK `WorkflowDefinition` shape.

## 4. Rust to Bun Script-Host Protocol v1

### 4.1 Transport

Rust starts one long-lived Bun child process and communicates through its
stdin/stdout using UTF-8 JSON frames.

Every frame uses Content-Length framing:

```text
Content-Length: <UTF-8 byte count>\r\n
\r\n
<JSON payload>
```

The byte count is the number of encoded UTF-8 bytes, never a JavaScript string
length, Unicode character count, or Rust scalar count.

The host reserves stdout exclusively for framed protocol messages. Diagnostic
logs, if enabled, go to stderr.

This transport is selected because it is private, cross-platform, requires no
port allocation, and lets Rust supervise the exact process handling script
invocations.

### 4.2 Versioned Envelope

Every message contains:

```json
{
  "protocol": "woml.script-host",
  "protocolVersion": 1,
  "messageType": "execute",
  "invocationId": "inv_01"
}
```

Unknown protocol names, versions, message types, or malformed envelopes are
protocol errors. Neither side silently guesses compatibility.

### 4.3 Asynchronous Multiplexing Is Required in v1

Protocol v1 is asynchronous from its first implementation:

- Rust may send multiple `execute` messages without waiting for a response.
- Each active request has a unique `invocationId`.
- Responses correlate only by `invocationId`.
- Responses may arrive in any order.
- Message order is not workflow execution order.
- Every accepted invocation produces exactly one terminal `completed` response
  unless the host connection is lost.
- The host may enforce a configurable concurrency limit without changing the
  protocol.

Example:

```text
Rust → execute inv_01
Rust → execute inv_02
Rust → execute inv_03

Bun  → completed inv_02
Bun  → completed inv_01
Bun  → completed inv_03
```

Supporting multiplexing does not make `<parallel>` executable. It only prevents
the transport from blocking that later capability.

### 4.4 Execute Request

```json
{
  "protocol": "woml.script-host",
  "protocolVersion": 1,
  "messageType": "execute",
  "invocationId": "inv_01",
  "runId": "run_01",
  "nodeId": "a",
  "attempt": 1,
  "handler": "runtime.script",
  "timeoutMs": 5000,
  "source": "return { x: context.trigger.name ?? \"World\" };",
  "context": {
    "trigger": {},
    "steps": {}
  }
}
```

The first profile supports only `runtime.script`. `services`, `context.run`,
`context.env`, engine controls, and typed reference inputs are absent.

### 4.5 Successful Response

```json
{
  "protocol": "woml.script-host",
  "protocolVersion": 1,
  "messageType": "completed",
  "invocationId": "inv_01",
  "outcome": {
    "kind": "success",
    "value": {
      "x": "World"
    }
  },
  "durationMs": 3
}
```

Only strict JSON values may appear in `value`.

### 4.6 Failed Response

```json
{
  "protocol": "woml.script-host",
  "protocolVersion": 1,
  "messageType": "completed",
  "invocationId": "inv_01",
  "outcome": {
    "kind": "failure",
    "error": {
      "kind": "script_threw",
      "code": "WOML_SCRIPT_THROWN",
      "message": "boom"
    }
  },
  "durationMs": 2
}
```

Throwing, timing out, returning invalid data, exceeding size limits, and
crashing are never collapsed into one generic script error.

### 4.7 Size-Limit Mechanism

Protocol v1 reserves two distinct failures:

- `context_too_large`
- `result_too_large`

An optional failure `details` object may carry `actualBytes` and `limitBytes`.

The mechanism is frozen in v1. The following numbers remain deliberately open:

- Default maximum context bytes.
- Default maximum result bytes.
- Absolute frame-size limit.
- Whether a workflow may request a lower or higher limit.

Rust checks context size before dispatch. Bun checks it defensively after
decoding. Bun validates and measures a result before sending it to Rust.

## 5. Unified Attempt-Failure Taxonomy

The script-host protocol and run-event schema share one canonical taxonomy.
They do not define overlapping error systems.

| Failure kind            | Stable code                     | Producer         |
| ----------------------- | ------------------------------- | ---------------- |
| `script_threw`          | `WOML_SCRIPT_THROWN`            | Bun Worker/host  |
| `script_timed_out`      | `WOML_SCRIPT_TIMEOUT`           | Bun host         |
| `invalid_script_result` | `WOML_SCRIPT_NON_JSON_RESULT`   | Bun host         |
| `context_too_large`     | `WOML_SCRIPT_CONTEXT_TOO_LARGE` | Rust or Bun host |
| `result_too_large`      | `WOML_SCRIPT_RESULT_TOO_LARGE`  | Bun host         |
| `worker_crashed`        | `WOML_SCRIPT_WORKER_CRASHED`    | Bun host         |
| `host_crashed`          | `WOML_SCRIPT_HOST_CRASHED`      | Rust supervisor  |
| `interrupted`           | `WOML_STEP_INTERRUPTED`         | Rust recovery    |

The protocol response schema permits the failures a living host can report.
The event schema permits the complete taxonomy.

If Rust observes the script host fail while running, affected attempts receive
`host_crashed`. If recovery sees `step_attempt_started` without a terminal event
and cannot prove the cause, the attempt receives `interrupted`.

## 6. Bun Host Lifecycle and Isolation

### 6.1 Ownership

Rust starts, monitors, and stops the Bun script host. The CLI supplies the
packaged host entry-point location and Bun executable during engine startup;
ownership then belongs to Rust.

### 6.2 Per-Invocation Isolation

The host creates a fresh Bun Worker for each invocation. Each Worker receives a
structured clone of a strict-JSON context, and that clone is deeply frozen
before user code runs.

The Worker is terminated after success or failure. JavaScript globals and
module state are not reused between invocations.

Worker isolation prevents accidental shared JavaScript state. It is not a
security sandbox for hostile code. Supporting untrusted third-party scripts
would require an OS process, container, or another stronger sandbox.

### 6.3 Timeout

Every execute request requires `timeoutMs`.

When the deadline expires, the host terminates the individual Worker and sends
`script_timed_out`. Other Workers and the host remain alive.

If a Worker cannot be terminated or the host stops responding, Rust terminates
the host, fails all in-flight invocations, and starts a fresh host for future
work.

The public default timeout remains an open configuration decision.

### 6.4 Host Crash

Rust detects child exit, EOF, or a broken protocol pipe.

Rust then:

1. Fails every in-flight attempt with `host_crashed`.
2. Does not replay any affected attempt automatically.
3. Restarts the host with bounded backoff for future work.
4. Allows the run policy to decide whether the run terminates.

The initial runtime has no automatic retry, so the affected run fails closed.
This protects against replaying a script whose side effects may have occurred
before the crash.

## 7. Context Across the Boundary

The first Rust slice sends the complete available context for every invocation:

```json
{
  "trigger": {},
  "steps": {
    "a": {
      "x": "World"
    }
  }
}
```

This is semantically simple but may approach quadratic serialization work as a
long workflow accumulates large step outputs.

Possible later optimizations include:

- Send only ancestor outputs.
- Add explicit script dependency declarations.
- Cache script source using a workflow-definition hash.
- Introduce incremental immutable context snapshots.

The compiler cannot safely infer every JavaScript dependency because scripts
may access `context.steps` dynamically. Selective context therefore requires an
explicit language/runtime contract.

Rust stores context and event data as JSON values. Bun validates every result
before returning it, Rust validates every response before appending it, and Bun
validates context again before starting a Worker. Invalid data never becomes a
successful step output.

Secrets never enter this serialized context or the event log.

## 8. Run Events and Folding

### 8.1 Authority

The event log is authoritative. Context and run status are derived projections.

Conceptually, Rust provides a pure fold:

```rust
fn fold(events: &[RunEvent]) -> Result<RunProjection>
```

The projection contains:

- Trigger data.
- Run status.
- Attempt states.
- Successful outputs exposed as `context.steps`.
- Nodes currently executing.
- Nodes ready to execute.

An in-memory projection may be cached, but deleting it and folding the event log
again must produce the same state.

### 8.2 Versioned Envelope

```json
{
  "eventSchemaVersion": 1,
  "eventId": "evt_01",
  "runId": "run_01",
  "sequence": 1,
  "occurredAt": "2026-08-06T12:00:00Z",
  "type": "step_attempt_succeeded",
  "data": {}
}
```

Sequences are monotonic within one run. Immutable runs remain bound to the
compiled workflow definition that created them.

### 8.3 Events Executable in the First Rust Slice

- `run_started`
- `step_attempt_started`
- `step_attempt_succeeded`
- `step_attempt_failed`
- `run_succeeded`
- `run_failed`

Only folding `step_attempt_succeeded` publishes its output into
`context.steps.<nodeId>`.

### 8.4 Reserved Future Vocabulary

- `step_retry_scheduled`
- `run_paused`
- `run_resumed`
- `branch_selected`
- `parallel_group_started`
- `parallel_group_completed`

These names document the expected direction. Their payloads are not silently
invented during the hello slice. Adding executable future events requires a
reviewed event-schema revision if their complete payloads are not frozen in v1.

### 8.5 Interrupted Attempt Rule

After durable persistence exists, recovery folds stored events in sequence.

If it finds `step_attempt_started` without a corresponding succeeded or failed
event, it appends `step_attempt_failed` with failure kind `interrupted`, then
fails the initial runtime's run. It never interprets a recorded start as proof
that a side effect did or did not happen, and it does not automatically replay
the step.

## 9. Contract Artifacts Required Before Rust Implementation

The Rust integration starts only after review of these versioned artifacts:

```text
docs/protocols/script-host-v1.md
docs/protocols/run-events-v1.md
docs/schemas/script-host-protocol.v1.schema.json
docs/schemas/run-event.v1.schema.json
docs/schemas/attempt-failure.v1.schema.json

woml-cli/tests/fixtures/script-host/execute.v1.json
woml-cli/tests/fixtures/script-host/ready.v1.json
woml-cli/tests/fixtures/script-host/success.v1.json
woml-cli/tests/fixtures/script-host/thrown.v1.json
woml-cli/tests/fixtures/script-host/timeout.v1.json
woml-cli/tests/fixtures/script-host/non-json.v1.json
woml-cli/tests/fixtures/script-host/context-too-large.v1.json
woml-cli/tests/fixtures/script-host/result-too-large.v1.json
woml-cli/tests/fixtures/script-host/worker-crashed.v1.json
woml-cli/tests/fixtures/script-host/unicode-crlf.execute.v1.json

woml/tests/fixtures/run-events/hello.events.v1.json
woml/tests/fixtures/run-events/host-crashed.event.v1.json
woml/tests/fixtures/run-events/interrupted.event.v1.json
```

The framing suite must prove:

- Multibyte UTF-8 source and context.
- Emoji and non-Latin text.
- CRLF characters inside JSON values.
- CRLF source line endings.
- One frame split across multiple reads.
- Multiple frames combined into one read.
- Multiple active invocations completing out of order.
- Byte counts use encoded UTF-8 length.

Rust and Bun must both pass the same fixtures.

## 10. Implementation Phases

These are Rust-integration phases and are named `R0`, `R1`, and so on to avoid
confusion with the deferred phases in `WOML CLI Vertical Slice Plan.md`.

### R0 — Freeze the two execution-boundary contracts — complete

One phrase: define exactly what Rust, the Bun host, and the event log exchange.

Changes:

- Write the normative script-host v1 protocol document.
- Write the protocol JSON Schema.
- Write the shared attempt-failure schema.
- Write the run-event v1 schema and minimal fold rules.
- Add reviewed request, response, failure, Unicode, CRLF, and framing fixtures.
- Confirm async multiplexing and out-of-order completion in the normative text.
- Leave size numbers configurable while freezing their failure mechanisms.

Result:

Rust and Bun can be developed independently against the same frozen contracts.

Gate:

- Every required artifact exists and is versioned.
- Fixtures validate against their schemas.
- No Rust execution or host behavior relies on an undocumented field.
- The blocking protocol decisions receive review before implementation starts.

### R1 — Build the long-lived Bun script host — complete

One phrase: turn the existing Worker runner into a multiplexed protocol service.

Changes:

- Add Content-Length frame encoding and incremental decoding.
- Accept multiple active execute requests.
- Create one fresh Worker per invocation.
- Correlate terminal responses by `invocationId`.
- Enforce timeout and strict-JSON result validation.
- Emit distinct typed failures, including both size-limit failures.
- Keep stdout protocol-only and use stderr for diagnostics.

Result:

A supervised Bun process can execute concurrent script requests without sharing
JavaScript state or depending on the TypeScript workflow executor.

Gate:

- All protocol and framing conformance fixtures pass.
- Responses may complete out of order and remain correctly correlated.
- Infinite-loop timeout kills only the responsible Worker.
- A new invocation cannot observe a prior Worker's global state.
- Throw, timeout, bad result, Worker crash, and size failures remain distinct.

### R2 — Build the minimal Rust event-driven DAG engine — complete

One phrase: make Rust understand the compiled model and derive run state from an
in-memory event log.

Changes:

- Add Rust types for Compiled Workflow Model schema v1.
- Deserialize and independently validate compiled-model JSON.
- Reject invalid schema versions, cycles, missing nodes, unknown handlers, and
  unsupported staged constructs.
- Add typed run events and the pure fold function.
- Add an in-memory append-only event store.
- Select ready nodes deterministically from the folded projection.
- Support only unconditional sequential execution and `runtime.script`.

Result:

Rust can start a run, determine which node is ready, and derive context without
an authoritative mutable context object.

Gate:

- Rust accepts the reviewed `hello.compiled.v1.json` fixture unchanged.
- Folding the same events always produces the same projection.
- Only successful attempt events publish step outputs.
- The engine cannot execute retry, branch, parallel, approval, or services.

Implementation note: R2 lives in the isolated `core/woml-engine` Rust crate so
the new event-driven engine does not inherit the legacy SDK execution paths.
R3 will connect this crate to the existing native core boundary.

### R3 — Connect Rust, the Bun host, and `hello.woml` — complete

One phrase: run the proven two-step workflow through the real Rust-to-Bun path.

Changes:

- Add the minimal asynchronous N-API bridge.
- Let Rust start and supervise the script host.
- Dispatch ready `runtime.script` nodes through protocol v1.
- Append started and terminal attempt events around every invocation.
- Return the terminal result through N-API to the CLI.
- Add a cross-engine equivalence test against the TypeScript reference executor.

Result:

The same `.woml` file produces the same output through Rust orchestration and
Bun script execution.

Gate:

- TypeScript compiles the existing root `hello.woml`.
- Rust consumes the existing compiled fixture shape.
- Nodes execute in order `a`, then `b`.
- Step `b` reads `context.steps.a.x`.
- The Rust and TypeScript executors return identical JSON and execution order.
- Script throw, timeout, invalid result, and host crash reach Rust as distinct
  failures.

Implementation note: the asynchronous native entry point and TypeScript adapter
are now available for conformance testing. The public `woml run` command remains
on the TypeScript reference executor until the deliberate production cutover in
R5.

### R4 — Add durable events and fail-closed recovery — complete

One phrase: make Rust runs reconstructable after the process stops.

Changes:

- Add the append-only SQLite event store.
- Bind each run immutably to its compiled workflow definition.
- Rebuild projections by folding stored events.
- Detect nonterminal attempts during recovery.
- Append `interrupted` failure events instead of replaying ambiguous work.
- Make event append and sequence allocation atomic.

Result:

A stopped process can reconstruct a run without relying on an in-memory context
object or automatically repeating an uncertain side effect.

Gate:

- Deleting the projection cache does not change reconstructed state.
- Restarting a completed run preserves its exact result.
- Restarting after `step_attempt_started` fails the attempt as `interrupted`.
- No interrupted attempt is replayed automatically.
- Stored event payloads validate against the versioned schema.

Implementation note: durability is exposed through an explicit SQLite file
path; R4 does not silently choose a database for users. SQLite itself enforces
immutable definitions, immutable run bindings, and append-only run events. On
restart, Rust derives state exclusively by folding those events. It safely adds
a missing final run event when the terminal step outcome is already known, but
an attempt left in progress is recorded as `interrupted` and is never replayed.
The public `woml run` command remains unchanged until the R5 cutover.

### R5 — Move the public CLI to Rust execution — complete

One phrase: make the working `woml run` command use Rust as its only production
workflow executor.

Changes:

- Replace the CLI's import of the TypeScript `executeWorkflow` function with the
  N-API Rust execution entry point.
- Preserve existing stdout, stderr, source diagnostics, and exit-code behavior.
- Map Rust node failures back to WOML source locations where available.
- Keep the TypeScript executor only as a temporary equivalence oracle.
- Resume the deferred packaged-installation smoke test from the earlier CLI
  plan.

Result:

Users run the same command and syntax, but Rust is now authoritative for every
run.

Gate:

- `woml run hello.woml` uses Rust and prints the pinned output.
- No production CLI path imports or calls the TypeScript DAG executor.
- The packaged CLI works from a clean temporary installation.
- Normal success writes nothing to stderr or beside the workflow source.
- Runtime error behavior remains stable and source-aware.

Implementation note: the CLI still uses Bun and TypeScript to parse, validate,
and compile WOML, then sends only the compiled model to Rust through N-API.
Release builds package the platform-specific native addon, long-lived Bun host,
and isolated Worker entry point. The ordinary `woml run` command uses Rust's
in-memory event engine and does not create a database implicitly; the explicit
durable API from R4 remains available for a later product surface. The
temporary TypeScript executor is removed in R6.

### R6 — Retire the TypeScript execution implementation — complete

One phrase: remove the temporary engine after Rust has replaced every behavior
it proved.

Changes:

- Remove the TypeScript DAG executor, handler scheduler, and direct workflow-to-
  Worker execution path.
- Retain the TypeScript WOML parser, validator, compiler, and CLI frontend.
- Retain protocol fixtures, compiled-model fixtures, and cross-language
  conformance tests.
- Convert any still-useful executor tests into Rust or black-box CLI tests.

Result:

There is one production executor and one authoritative state model: Rust.

Gate:

- Removing the TypeScript executor does not reduce behavioral coverage.
- Every existing executable WOML fixture passes through Rust.
- No package exports the temporary execution API.

Implementation note: the TypeScript DAG loop, handler registry, direct Worker
runner, mutable context projection, and execution-error API were deleted from
the `woml` package. Its public surface now contains only the parser, validator,
compiler, compiled-model types, and source diagnostics. The isolated JavaScript
Worker and strict JSON boundary remain under `woml-cli/src/script-host/` because
they are execution components supervised by Rust, not a second workflow
engine. Former executor behavior is covered by Rust engine tests, script-host
tests, reviewed result/context fixtures, and black-box CLI tests.

## 11. Feature Expansion After the Rust Cutover

The first Rust cutover proves only sequential scripts. Later features follow
separate design-and-implementation phases in this order:

1. **Complete:** `<branch>`, stable merged results, and durable
   `branch_selected` events. The executable milestone and its proof are in
   `WOML Branch Implementation Plan.md`.
2. **Complete:** `<parallel>` with model v3, event v3, bounded scheduling,
   `wait-all`, `fail-fast`, protocol-v2 Worker cancellation, durable recovery,
   and packaged CLI diagnostics. The milestone proof is in
   `WOML Parallel Implementation Plan.md`.
3. **In progress — A0–A3 complete:** approval model v4, event v4, store v2,
   HTTP v1, native-outcome v1, token, timeout, diagnostic, and fixture contracts
   are frozen; frontend lowering, Rust structural validation, event folding,
   durable waiting projections, store migration, and hashed credentials are
   complete. A4 begins automatic pause behavior; later phases add Rust
   resolution and the HTTP-only decision flow defined in
   `WOML Human Approval Implementation Plan.md`.
4. Resolve idempotency keys, then enable retry values greater than one.
5. Add the remaining triggers, lifecycle behavior, services, and engine-control
   operations required for product parity.
6. Remove the old JavaScript chaining SDK only after WOML reaches the agreed
   feature and migration parity.

Existing Cronflow code may inform behavior, but it is not proof that these
features work with the new compiled model, protocol, or event vocabulary.

## 12. Deferred Decisions That Must Stay Deferred

The Rust hello slice does not authorize defaults for:

- `context.run` public schema.
- `context.env`.
- Secret resolution or persistence.
- Typed `contextReference` inputs.
- Retry idempotency-key derivation and duplicate handling.
- Workflow-level cancellation and durable user state. Internal fail-fast
  Worker cancellation is implemented and remains a separate engine concern.
- Service calls from scripts.
- Approval token generation, storage, and hashing are frozen by A0 but remain
  unimplemented until A3; the Rust hello slice must not invent a second shape.
- Default production timeout.
- Default context, result, and frame byte limits.
- Per-workflow overrides for runtime resource limits.

For the first slice:

- `context` contains only `trigger` and successful `steps` outputs.
- Retry has one attempt.
- Approval, lifecycle, and services are rejected before Rust execution. Branch
  and parallel are executable; approval syntax lowers to model v4 and its
  state survives durably, but automatic runtime pausing remains gated until A4.
- Secrets never appear in compiled inputs, context, protocol messages, or
  events.

If implementation forces one of these decisions, work stops at that boundary
and the decision returns for architecture review. It must not be resolved by an
implicit default.

## 13. What Is Kept and What Is Replaced

Kept:

- WOML raw-script handling.
- TypeScript XML parsing.
- Source locations and diagnostics.
- WOML validation and DAG compilation.
- Executable Compiled Workflow Models v1–v3 and structurally accepted model-v4
  approval DAGs with event/store foundations, which gain automatic runtime
  behavior through A4–A7.
- The CLI command surface.
- The isolated Bun Worker implementation where compatible with protocol v1.
- Existing fixtures and expected outputs.

Reimplemented or replaced:

- TypeScript DAG execution.
- TypeScript-owned run context.
- TypeScript handler scheduling.
- Direct executor-to-Worker invocation.
- Mutable run state as an authority.

The old JavaScript chaining SDK is not part of the WOML runtime path. It remains
temporarily only while WOML is built to full product parity, then it is removed.

## 14. Definition of Rust Integration Done

The Rust integration is complete when:

- WOML still parses and compiles in Bun/TypeScript.
- The compiled DAG crosses N-API into Rust without an SDK-shaped adapter.
- Rust validates the model, owns run events, folds context, and selects nodes.
- Rust owns and supervises the long-lived Bun script host.
- Every script runs in a fresh Worker with a required timeout.
- Protocol v1 remains conformance-tested and explicitly compatible; protocol
  v2 preserves multiplexing and adds targeted fail-fast cancellation.
- Failures keep their canonical identity across protocol, event log, and CLI.
- SQLite recovery reconstructs state and fails ambiguous attempts closed.
- `woml run hello.woml` uses no TypeScript production executor.
- The packaged CLI passes its clean-install smoke test.
- Removing the temporary TypeScript executor does not reduce coverage.

This definition completes the Rust execution migration. Removing the entire old
JavaScript chaining SDK requires the later, broader WOML feature-parity gate.
