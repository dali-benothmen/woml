# WOML Durable Workflow Calls Implementation Plan

Status: WC0 and WC1 completed on 2026-08-10; WC2 through WC7 completed on
2026-08-11. Durable Workflow Calls are published for one-machine operation:
they execute through Rust in one runtime or across separate local WOML
processes, reuse one durable child across retries and duplicate delivery,
reject cycles, unsupported Human Approval targets, and corrupted durable
identity, recover fail-closed, expose safe bounded inspection, migrate v9 state
without history loss, and pass the packaged release gate and benchmark.

## 1. Product Outcome

One activated workflow can call exactly one other activated workflow by its
workflow ID, pass it JSON input, wait for its independent durable run, and use
its final JSON result:

```js
const risk = await services.workflows.call('calculate-risk', {
  customerId: context.payload.customerId
});

return {
  score: risk.score
};
```

The called workflow receives the payload directly as `context.payload`:

```xml
<woml>
  <workflow
    id="calculate-risk"
    name="Calculate customer risk"
    version="1.0.0"
  >
    <steps>
      <step id="calculate">
        <script>
          return {
            score: context.payload.customerId === 'customer-42' ? 90 : 20
          };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The missing `<triggers>` section is intentional. It makes this a call-only
workflow: `woml run` activates it, but it does not start until another workflow
calls `calculate-risk`. No `<call>` trigger tag, imported `.woml` file, public
URL, event name, or user-managed control secret is required.

The first complete local journey is:

```bash
woml run examples/workflowCalls
```

WOML registers every workflow in the directory before starting any manual
workflow. The parent calls the child through Rust, the child runs independently,
and the parent prints a result derived from the child's result.

## 2. Why This Is Not an Event

`services.events.emit()` and `services.workflows.call()` solve different product
problems:

| Operation | Target | Waits | Returns | Best for |
| --- | --- | --- | --- | --- |
| `services.events.emit()` | Zero, one, or many subscribers | No | Delivery summary | Announcing that something happened |
| `services.workflows.call()` | Exactly one workflow ID | Yes | Child's final JSON result | Delegating work and using its answer |

Workflow Calls reuse the existing Rust run-admission and durable-event
authority, but they are not implemented by publishing a named event. Event
fan-out, partial delivery, and successful zero-subscriber behavior would give a
call the wrong semantics.

## 3. Frozen Product Direction to Review in WC0

WC0 turns the choices below into versioned contracts. Later phases must not
silently choose different behavior.

### 3.1 JavaScript API

The first API is:

```ts
services.workflows.call(
  workflowId: string,
  payload: Readonly<Record<string, JsonValue>>,
  options?: {
    name?: string;
    timeout?: number | string;
  }
): Promise<JsonValue>
```

Rules:

- `workflowId` identifies exactly one activated workflow.
- `payload` must be a top-level JSON object and becomes the child's complete
  `context.payload`.
- The promise resolves to the child's final JSON result without a wrapper.
- `return null` is an intentional successful result.
- A missing/`undefined` result is invalid and rejects the call.
- `name` distinguishes multiple logical calls to the same target in one step
  and participates in stable operation identity.
- `timeout` limits how long the parent waits. It cannot exceed the calling
  step's remaining timeout.
- A parent timeout does not silently cancel an already admitted independent
  child run. Workflow cancellation policy belongs to the later Lifecycle and
  Engine Controls milestone.
- The facade is deeply read-only and works from inline scripts and local
  JavaScript/TypeScript modules.

One call per target workflow in a step receives an automatic stable name. Two
logical calls to the same target require explicit names:

```js
const [primary, secondary] = await Promise.all([
  services.workflows.call('calculate-risk', firstCustomer, {
    name: 'primary-risk'
  }),
  services.workflows.call('calculate-risk', secondCustomer, {
    name: 'secondary-risk'
  })
]);
```

This reuses Capability Call v1 rather than creating a second Rust/Bun protocol.

### 3.2 Which workflows are callable

Every workflow loaded by the same WOML runtime namespace is callable by ID.
Existing webhook, Slack, schedule, interval, event, and manual triggers keep
their existing behavior.

A workflow may omit `<triggers>` to be call-only. It is activated and registered
by `woml run`, but it never starts automatically. An empty `<triggers>` element
is rejected; omitting the whole section is the one canonical call-only form.

This requires Compiled Workflow Model v10. Models v1-v9 remain immutable and
executable. Model v10 permits zero declared triggers and retains the existing
DAG, script runtime, retry, service, and optional module-runtime contracts.

### 3.3 Target selection

Target resolution uses the exact workflow ID:

- no live target: reject with `WOML_WORKFLOW_TARGET_NOT_FOUND`;
- more than one live owner: reject registration before calls are accepted;
- one live target: bind the child run to that target's exact definition hash;
- target replaced after admission: the admitted child stays bound to the old
  immutable definition;
- target route lost before durable admission: the call may be retried;
- target route lost after admission: lookup the admitted child rather than
  creating a replacement.

Business `version="..."` remains metadata and is not a routing identity.

### 3.4 Context and lineage

The child sees only the supplied payload:

```js
context.payload
```

Parent run ID, parent step ID, call identity, depth, route data, and target
definition hash remain hidden engine metadata. Workflow Calls do not expose
`context.run` and do not place engine metadata in `context.payload`.

### 3.5 Result and failure behavior

The child remains an independent run with its own event history and terminal
status. The parent receives:

- the exact bounded JSON value from `run_succeeded`; or
- one catchable WOML service error if the child fails or cannot be called.

Child error messages are bounded and redacted before crossing into the parent.
The error may safely identify `workflowId`, `childRunId`, and a stable failure
code, but never child context, secret values, arbitrary stack traces, or raw
provider data.

### 3.6 First-profile limits

WC0 freezes exact numbers. The recommended starting limits are:

- payload: 1 MiB;
- returned result: 4 MiB;
- call-chain depth: 32;
- active workflow calls per script invocation: Capability Call v1's existing
  32-call limit;
- workflow ID, operation name, diagnostic, and transport bounds: existing WOML
  identifier and Capability Call v1 limits.

## 4. Honest Durable-Wait Boundary

An arbitrary Bun JavaScript stack cannot be serialized and resumed after a
process crash. Therefore the first publishable Workflow Calls profile does not
pretend that a script can wait inside `await services.workflows.call()` across
an hours-long Human Approval pause.

For v1:

- callable children may use sequential steps, branch, parallel, retry, modules,
  Fetch, and managed services;
- a target that can enter Human Approval waiting is rejected as unsupported for
  a synchronous Workflow Call;
- the child admission, definition binding, lineage, result, and parent-child
  identity are durable;
- if the caller process crashes while awaiting a child, the parent attempt
  follows the existing fail-closed interrupted-attempt policy;
- a later safe retry with the same call identity reattaches to the original
  child instead of creating another run.

Supporting a parent that sleeps for hours without retaining or replaying an
arbitrary JavaScript continuation requires a future engine-level suspension
boundary. That work belongs with Lifecycle and Engine Controls. It must not be
implemented by keeping a Bun Worker alive for 24 hours.

## 5. Architecture

```text
Parent <script>
  -> services.workflows.call(workflowId, payload)
  -> existing Script Host / Capability Call v1
  -> Rust Workflow Call authority
       -> resolve one active target
       -> derive stable call identity
       -> atomically admit or recover one child run
       -> execute locally or wake the owning local process
       -> observe the child's durable terminal event
  <- child JSON result or safe WOML service error
  -> parent script continues
```

Layer ownership remains unchanged:

- TypeScript parses WOML, accepts call-only workflow syntax, validates known
  static script shapes, and lowers Model v10.
- Bun exposes the JavaScript facade and transports nested calls; it does not
  choose targets, own idempotency, or create child runs.
- Rust owns target registration, admission, execution, persistence, lineage,
  idempotency, timeout classification, and result lookup.
- The compiled DAG engine remains unaware of JavaScript syntax and the
  `services.workflows.call()` spelling.

## 6. Persistence and Event Contracts

Parent calls reuse Run Event v8's generic managed-operation vocabulary:

```text
operation_started
operation_succeeded
operation_failed
```

The operation identity is `capability=workflows`, `operation=call`. Safe start
metadata records the target workflow ID, accepted definition hash, child run
ID, payload digest, and lineage depth. Payload and result values never enter
operation metadata.

Run Event v9 adds only a truthful `workflow_call` ingress for a called child.
The child's `run_started` stores its payload as the complete
`context.payload`, while its call key remains hidden engine metadata. It does
not manufacture a source trigger. All later payloads retain the Run Event v8
shapes, and `run_succeeded` or `run_failed` remains the result authority.

The separate Workflow Call v1 schema defines request/result behavior and the
versioned durable call-index contract defines the admission binding.
The call index is a transactional uniqueness/routing index reconstructible from
the authoritative histories, not a second mutable context or competing result
authority.

Child admission and the parent call identity must commit atomically when both
runs share the local state authority. A crash may leave neither admitted or one
recoverable admitted child; it must never create two children.

## 7. Idempotency and Recovery

The stable call key is derived from:

```text
parent step idempotency key
+ target workflow ID
+ automatic target name or explicit options.name
```

The first accepted call stores a digest of the payload and the selected target
definition hash. Repeating the key:

- with the same payload and target returns or waits for the original child;
- with different payload data fails with
  `WOML_WORKFLOW_CALL_IDEMPOTENCY_CONFLICT`;
- after the target definition changes still refers to the admitted old child;
  it never silently retargets an in-flight call.

Recovery follows the established WOML rule:

- pure derivation may repeat;
- an admitted child is looked up rather than re-created;
- a parent operation start without enough durable admission evidence is
  ambiguous and fails closed;
- a terminal child result is read from immutable run history;
- neither host restart nor transport retry manufactures another child run.

## 8. Cycle and Depth Protection

Rust stores hidden call lineage. Admission rejects:

- direct self-call;
- indirect repetition such as `A -> B -> A`;
- chains deeper than 32;
- a route that changes the claimed parent or lineage identity.

Lineage crosses local process boundaries but never enters user context. Calls
started in parallel keep independent branches while sharing the same bounded
ancestor chain.

## 9. Same-Process and Local Cross-Process Routing

### Same process

`woml run <directory>` compiles and registers every definition before firing
startup manual triggers. Calls resolve through an in-memory Rust registry and
do not perform HTTP loopback.

### Separate local processes

Two `woml run` processes can communicate when they use the same project runtime
namespace and durable state authority. Each process:

- registers workflow ID, definition hash, runtime ID, loopback endpoint, and a
  renewable ownership lease;
- rejects duplicate live ownership for the same workflow ID;
- receives only wake-up requests for child runs already durably admitted in the
  shared store;
- verifies a random per-runtime session credential created by WOML, not by the
  user; and
- removes or expires its route on graceful shutdown or crash.

The normal user does not run `woml connect`, configure a control-token secret,
or expose a public workflow-call URL. A local random port and internal session
credential are implementation details. Cross-machine routing, tenant identity,
mTLS, and hosted service discovery remain Production Runtime work.

## 10. Error Contract

WC0 freezes source/runtime errors including:

| Code | Meaning |
| --- | --- |
| `WOML_WORKFLOW_TARGET_INVALID` | Workflow ID or call input is invalid. |
| `WOML_WORKFLOW_TARGET_NOT_FOUND` | No active target owns the ID. |
| `WOML_WORKFLOW_TARGET_AMBIGUOUS` | More than one runtime claims the ID. |
| `WOML_WORKFLOW_TARGET_UNAVAILABLE` | The selected runtime cannot accept work. |
| `WOML_WORKFLOW_DEFINITION_MISMATCH` | The route and admitted definition disagree. |
| `WOML_WORKFLOW_PAYLOAD_TOO_LARGE` | The trigger payload exceeds its bound. |
| `WOML_WORKFLOW_RESULT_MISSING` | The child produced no valid JSON result. |
| `WOML_WORKFLOW_RESULT_TOO_LARGE` | The child result exceeds its bound. |
| `WOML_WORKFLOW_CALL_FAILED` | The child run failed. |
| `WOML_WORKFLOW_CALL_TIMED_OUT` | The parent stopped waiting before completion. |
| `WOML_WORKFLOW_CALL_IDEMPOTENCY_CONFLICT` | One call key was reused with different input. |
| `WOML_WORKFLOW_CALL_CYCLE` | The call repeats a workflow in its lineage. |
| `WOML_WORKFLOW_CALL_DEPTH_EXCEEDED` | The maximum call-chain depth was exceeded. |
| `WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED` | The child may enter an unsupported durable pause. |

Every failure maps into Capability Call v1's single failure taxonomy with
reviewed `retryable` and `ambiguous` flags. Workflow Calls do not introduce a
second overlapping JavaScript error class.

## 11. Scope

### Included

- `services.workflows.call(workflowId, payload, options?)`.
- Call-only workflows by omitting `<triggers>`.
- Model v10 and any required Definition Package update.
- Same-process direct Rust routing.
- Separate-process routing on one local project/state authority.
- Exact target ownership and definition binding.
- Independent durable child runs and hidden lineage.
- Stable call idempotency, duplicate reattachment, cycles, depth, timeouts, and
  result limits.
- Composition with branch, parallel, retries, modules, built-in services, and
  all existing trigger types on the parent.
- CLI progress, run inspection, examples, recovery tests, and release gates.

### Not included

- A `<call>` tag or workflow import syntax.
- Event-based fan-out semantics.
- Calling an arbitrary `.woml` path that has not been activated.
- Public unauthenticated workflow-call endpoints.
- User-managed workflow-call secrets in `.woml`.
- Cross-machine or multi-tenant hosted routing.
- Load balancing across multiple active owners of one workflow ID.
- Long Human Approval suspension inside a called workflow in v1.
- Parent-to-child cancellation policy, compensation, sagas, or transactions
  across workflows.
- `context.run` or parent metadata in `context.payload`.

## 12. Versioned Artifacts to Review Before Execution Code

WC0 produces and pins:

1. Workflow Calls v1 request/result/error schema.
2. Compiled Workflow Model v10 schema and triggerless fixtures.
3. Definition Package v4 only if Model v10 plus local modules requires it.
4. Durable Workflow Call Index v1 schema and SQLite migration contract.
5. Local Runtime Registry and Wake-up Protocol v1.
6. Safe operation metadata fixture using unchanged Run Event v8.
7. Parent/child source, model, definition, and event-history fixtures.
8. Error catalog with retryable/ambiguous classification.
9. Limit manifest for payload, result, depth, concurrency, leases, and frames.

The review must explicitly confirm that Capability Call v1, Run Event v8,
Script Host v6, Models v1-v9, and existing Definition Packages remain immutable.

## 13. Implementation Phases

### WC0 — Freeze Workflow Call contracts and reviewed fixtures

Status: **completed on 2026-08-10.**

Changes:

- Freeze the JavaScript API, call-only grammar, Model v10, errors, identities,
  limits, event usage, durable index, and local routing protocol.
- Produce a reviewed parent/child fixture with one payload and one result.
- Produce failure, null-result, duplicate, cycle, and cross-process fixtures.
- Keep approval waiting and hosted routing explicitly outside v1.

Result:

Every layer has one versioned description of what a workflow call means before
runtime code is written.

Gate:

Schemas validate all accepted fixtures and reject conflicting identities,
ambiguous targets, missing results, invalid triggerless shapes, and unbounded
lineage.

### WC1 — Compile call-only workflows and expose editor contracts

Status: **completed on 2026-08-10.**

Changes:

- Accept an omitted `<triggers>` section as the only call-only source shape.
- Lower call-only definitions to Model v10 without changing the DAG model.
- Reserve `workflows` as an executable built-in service rather than only a
  future name.
- Add `services.workflows.call()` to automatically generated editor types.
- Add source-located diagnostics for known invalid call shapes and unsupported
  call-only combinations.

Result:

Parent and child WOML files validate and compile, and editors recognize the
workflow-call API, but calls do not execute yet.

Gate:

Frontend fixtures prove optional-trigger ordering, old-model compatibility,
payload/result typings, static diagnostics, and `.js`/`.ts` module usage.

### WC2 — Build the Rust target registry and durable child admission

Status: **completed on 2026-08-11.**

Changes:

- Register all loaded definitions before startup manual triggers execute.
- Enforce one live owner per workflow ID and bind routes to definition hashes.
- Add the Rust `workflows.call` capability handler.
- Derive stable call identity and atomically admit one child run.
- Put payload in the child's existing `context.payload` and preserve hidden
  lineage outside user context.
- Add narrow Run Event v9 `workflow_call` ingress so a called child never
  pretends that a source trigger fired.
- Add durable store v9 with an immutable call-identity index.

Result:

Rust can resolve and durably admit one child fixture by workflow ID, and a
duplicate request recovers that exact child. WC2 deliberately stops before
executing or waiting for the child; that is WC3.

Gate:

Rust tests prove atomic admission, definition binding, exact trigger context,
unique ownership, duplicate return, payload conflict, and no child creation on
pre-admission failure. Concurrent duplicate admissions create one child.

### WC3 — Execute the first same-runtime WOML call end to end

Status: **completed on 2026-08-11.**

Changes:

- Add the read-only Bun `services.workflows.call()` facade.
- Transport the call through unchanged full-duplex Capability Call v1.
- Run the selected child through the normal Rust DAG executor.
- Return the child's final JSON value to the parent script.
- Map child failure into one catchable WOML service error.
- Add the two-workflow directory example.

Result:

`woml run examples/workflowCalls` executes a real parent, calls one child, and
prints a parent result that uses the child's answer.

Gate:

The manual journey proves object payload, `context.payload`, object/scalar/null
results, child failure, missing result, timeout, and no event/HTTP workaround.

### WC4 — Make same-runtime calls retry-safe and recoverable

Status: **completed on 2026-08-11.**

Changes:

- Persist and fold safe Workflow Call operation metadata.
- Reattach retries and duplicate transports to the original child run.
- Reject payload/target conflicts for an existing call key.
- Enforce one automatic call per target and explicit names for repeated calls.
- Add cycle/depth enforcement and crash tests at every admission/result edge.
- Preserve the fail-closed parent-attempt policy after ambiguous host failure.

Result:

Retries, crashes, and nested calls cannot manufacture duplicate child runs or
silently change targets.

Gate:

The crash matrix covers before/after operation start, child admission, target
start, child terminal commit, result read, Bun reply, Worker crash, host crash,
and parent terminal commit.

### WC5 — Route calls between local WOML processes

Status: **completed on 2026-08-11.**

Changes:

- Accept multiple explicit file and directory operands in one `woml run`
  command and treat their deduplicated definitions as one runtime unit.
- Implement the versioned local runtime registry and ownership leases.
- Start an internal loopback wake-up endpoint on a random port.
- Generate and rotate internal session credentials automatically.
- Admit the child in shared durable state before notifying the target owner.
- Recover from lost wake-ups by scanning already admitted pending children.
- Expire dead owners and reject ambiguous duplicate ownership.

Result:

One terminal can keep `calculate-risk` active while a parent started in another
terminal calls it by workflow ID and receives its result. No public URL, event,
or user-configured secret is involved. Authors may equivalently activate an
explicit set with `woml run parent.woml child.woml` or a directory containing
those files.

Gate:

Two-process tests cover startup order, concurrent calls, target shutdown,
caller shutdown, lost wake-up, expired lease, stale route, duplicate owner,
shared-state mismatch, and target restart with the same definition.

### WC6 — Complete composition, diagnostics, and inspection

Status: **completed on 2026-08-11.**

Changes:

- Compose calls with branch, parallel, retry, modules, native Fetch, all five
  current services, and every parent trigger type.
- Verify nested call chains across same-process and local cross-process routes.
- Reject Human Approval targets with an actionable v1 limitation message.
- Show parent and child run IDs in safe CLI progress.
- Extend run inspection with bounded parent/child call summaries.
- Add generated types, source diagnostics, troubleshooting, and manual guides.

Result:

Workflow Calls feel like one normal WOML service while remaining inspectable as
independent durable runs.

Gate:

Composition and CLI suites prove no secret/payload leakage, bounded inspection,
helpful unavailable-target errors, and unchanged behavior for workflows that do
not call another workflow.

### WC7 — Harden and publish Durable Workflow Calls

Status: **completed on 2026-08-11.**

Changes:

- Run adversarial identity, lineage, routing, storage, crash, and corruption
  tests.
- Benchmark same-runtime and local cross-process latency and concurrency.
- Test clean installation, schema compatibility, migration, shutdown, and
  artifact recovery.
- Update language, architecture, services, deployment, security, and SDK
  migration documentation.
- Add one Workflow Calls release gate to the complete WOML release suite.
- Correct and exercise named-call identity across the Bun/Rust boundary so
  explicit names support safe repeated and concurrent calls.

Result:

Terminal child workflows can be called safely in-process or across local WOML
processes through one documented API.

Gate:

Frontend, Rust, Bun, protocol/schema, typecheck, Clippy, integration,
cross-process, crash, packaging, compatibility, benchmark, and secret scans pass
from a clean project.

## 14. Expected File Areas

| Area | Expected locations |
| --- | --- |
| Grammar/lowering | `woml/src/compiler.ts`, `model.ts`, schemas and fixtures |
| Script analysis/editor types | `woml/src/script-analysis.ts`, `editor.ts` |
| Bun facade | `woml-cli/src/script-host/worker.ts`, Worker tests |
| Capability routing | `core/woml-engine/src/capability.rs`, new workflow-call handler |
| Definition registry | Rust runtime registration and activation code |
| Durable admission/index | `core/woml-engine/src/durable.rs`, migrations, projections |
| Child execution | Rust runtime/DAG executor using existing definitions |
| Cross-process routing | Rust/CLI local registry, lease, and wake-up host |
| CLI progress/inspection | `woml-cli/src/cli.ts`, Rust executor bindings |
| Contracts | `docs/schemas/*`, `docs/protocols/*`, reviewed fixtures |
| Examples/docs | `examples/workflowCalls`, language and architecture guides |

## 15. Verification Matrix

| Area | Required proof |
| --- | --- |
| API | Direct result, null, errors, timeout, and named duplicate calls match v1. |
| Targeting | Exactly one live workflow ID and exact definition hash are selected. |
| Context | Payload alone becomes child `context.payload`; lineage stays hidden. |
| Durability | One call identity admits at most one child across retries and crashes. |
| Results | Child terminal history is authoritative and bounded. |
| Failures | Child errors are catchable, classified once, bounded, and redacted. |
| Cycles | Self/indirect cycles and depth overflow fail before another child starts. |
| Parallel | Out-of-order child completions correlate by invocation/call identity. |
| Processes | Same-process is direct; local cross-process survives lost wake-ups. |
| Ownership | Duplicate/stale target routes cannot create nondeterministic calls. |
| Recovery | Ambiguous parent effects fail closed; admitted children are reattached. |
| Compatibility | Models v1-v9 and workflows without calls remain unchanged. |
| Security | No payloads, results, secrets, or internal credentials leak to metadata. |
| Packaging | The parent/child example works from a clean installation. |

## 16. Risks and Guardrails

### A service call can look like an ordinary function

It creates an independent durable run and may take much longer than a local
function. Documentation and CLI progress must make the child run visible.

### Dynamic routing can silently change behavior

Admission pins the exact active definition hash. A target deployment after
admission never changes an existing child.

### Waiting inside arbitrary JavaScript is not a durable continuation

The first profile rejects approval-waiting children. We will not retain a Bun
Worker for hours or replay arbitrary script code after a crash while claiming
transparent durability.

### Parent retries can duplicate children

Stable operation identity, payload digest comparison, and atomic child
admission are mandatory before the first executable call.

### Cross-process discovery can become a hidden production control plane

WC5 is deliberately local, project-scoped, loopback-only, and single-owner.
Cross-machine identity and routing remain a separate Production Runtime system.

### Calls can form recursive automation loops

Hidden immutable lineage rejects repeated workflow IDs and caps depth before
admission.

## 17. Global Roadmap After Durable Workflow Calls

1. **Lifecycle and Engine Controls** — cancellation, lifecycle hooks,
   workflow-level concurrency/rate limits/timeouts, durable user state, and a
   reviewed engine suspension boundary for long approval-waiting child calls.
2. **Production Runtime and Operations** — hosting, deployment, multi-node
   ownership, cross-machine workflow-call routing, service identity, OS-level
   isolation, observability, retention, administration, and scaling.
3. **Complete the postponed Module System phases** — MS5 locked third-party
   packages, remaining MS6 package permissions/security, MS7 portable
   distribution, and MS8 final hardening/publication.
4. **WOML Package Registry and Community Ecosystem** — signed publication,
   discovery, provenance, moderation, compatibility, and deprecation.
5. **Additional Infrastructure Adapters** — postponed durable queue/external
   broker support, document databases, external object storage, and distributed
   caches according to demand.
6. **Additional Communication Providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when justified.
7. **Retire the JavaScript Chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

Durable Workflow Calls WC0–WC7 are complete. The next roadmap feature is
Lifecycle and Engine Controls. It must begin with reviewed cancellation,
durable suspension, lifecycle-hook, and engine-control contracts rather than
silently extending Workflow Calls v1.
