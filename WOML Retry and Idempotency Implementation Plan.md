# WOML Retry and Idempotency Implementation Plan

Status: RI0 through RI3 completed on 2026-08-07. End-to-end retry execution
remains planned for RI4 onward.

## 1. Product Outcome

A workflow author can add retry behavior directly to a step:

```xml
<step id="loadProfile" retry="3">
  <script>
    const response = await fetch('https://api.example.com/profile', {
      headers: {
        'Idempotency-Key': attempt.idempotencyKey
      }
    });

    if (!response.ok) {
      throw new Error(`Profile request failed with ${response.status}`);
    }

    return await response.json();
  </script>
</step>
```

`retry` is an attribute of `<step>`. There is no `<retry>` tag and this plan
must not introduce one.

When the script throws, WOML records the failed attempt, waits according to the
compiled backoff policy, and starts the next attempt. A successful attempt
publishes exactly one final value at `context.steps.loadProfile`. If all attempts
fail, the workflow fails with the final attempt number and the original safe
failure.

The finished product must support:

```text
woml run workflow.woml
```

with retry behavior owned by the Rust engine, JavaScript execution still owned
by isolated Bun workers, and the event log remaining the authority for recovery.

## 2. What “Done” Means

This milestone is complete when:

1. `retry="N"` is valid and executable for `<step>` where `N` is the total
   number of attempts, including the first attempt.
2. Omitted `retry` and `retry="1"` both mean one attempt and compile to the same
   runtime behavior.
3. Rust—not Bun and not the CLI—decides whether another attempt is legal and
   when it becomes due.
4. Every failed attempt and every scheduled retry is durable and reconstructable
   by folding events.
5. Retry attempts receive a new attempt/invocation identity but share one stable
   step-effect idempotency key.
6. Ambiguous attempts are never replayed automatically.
7. Sequential steps, branch arms, parallel children, and approval arms obey the
   same retry contract.
8. A successful retry publishes one step output and unblocks downstream nodes
   exactly once.
9. The CLI produces useful retry progress and final errors without exposing
   secrets or raw provider payloads.
10. The complete WOML release gate passes from a clean package.

## 3. Scope

### Included

- Executable retry attributes on `<step>`.
- Fixed and exponential backoff expressed through step attributes.
- A stable idempotency key available to script attempts.
- Versioned compiled-model, event, script-host, and progress contracts.
- Durable scheduling and restart recovery.
- Retry composition with branch, parallel, and Human Approval routes.
- Source-aware validation, runtime diagnostics, fixtures, and CLI tests.

### Not included

- A `<retry>` element.
- Retry on `<branch>`, `<parallel>`, `<approval>`, `<notify>`, or trigger tags.
- Selective predicates such as retry only for a specific JavaScript error class.
- Per-error custom backoff expressions.
- Random jitter or distributed scheduler ownership.
- Per-step `timeout` activation; timeout remains a separate milestone decision.
- Exactly-once guarantees for arbitrary external systems.
- Service-specific retries. Future `services.*` handlers will reuse this engine
  contract but define their own retryable failure taxonomy.
- Manual retrying of an already terminal failed run.
- Cancellation of an entire workflow; that remains in Lifecycle and Engine
  Controls.

Slack notification delivery already has a provider-specific retry and
idempotency contract. That implementation is useful evidence, but this milestone
does not change Slack delivery semantics or merge notification attempts with
workflow step attempts.

## 4. Syntax Contract

### 4.1 Simple form

```xml
<step id="generateSummary" retry="3">
  <script>
    if (attempt.number < 3) {
      throw new Error('Temporary failure');
    }

    return { summary: 'Ready' };
  </script>
</step>
```

`retry="3"` means at most three total attempts: attempt 1 plus up to two retry
attempts. It does not mean one initial attempt plus three more attempts.

### 4.2 Advanced attribute form

```xml
<step
  id="generateSummary"
  retry="4"
  retry-backoff="exponential"
  retry-delay="1s"
  retry-max-delay="30s">
  <script>
    return await generateSummary();
  </script>
</step>
```

The accepted attributes are:

| Attribute | Required | Meaning |
|---|---:|---|
| `retry` | No | Maximum total attempts. Allowed range: `1` through `10`. |
| `retry-backoff` | No | `fixed` or `exponential`; defaults to `exponential` when `retry > 1`. |
| `retry-delay` | No | Fixed delay or initial exponential delay; defaults to `1s`. |
| `retry-max-delay` | No | Exponential cap; defaults to the greater of `30s` and `retry-delay`. Invalid with `fixed`. |

Rules:

- Backoff attributes require `retry` greater than `1`.
- `retry="1"` may be authored but lowers as no multi-attempt policy.
- `retry-delay` must be greater than zero and no greater than `24h`.
- `retry-max-delay` must be at least `retry-delay` and no greater than `24h`.
- Exponential backoff uses a frozen multiplier of `2` in the first profile.
- Immediate zero-delay retry is deliberately unavailable in the first profile.
- Attribute order has no semantic effect.

The delay before retry attempt `n`, where `n >= 2`, is:

```text
fixed:       retry-delay
exponential: min(retry-delay × 2^(n - 2), retry-max-delay)
```

No random jitter is added in this local-runtime milestone. Jitter must be
reviewed before distributed scheduling because it affects reproducibility and
fleet behavior.

## 5. Safety and Idempotency Contract

### 5.1 What WOML guarantees

WOML guarantees:

- at most one active invocation for one step attempt;
- monotonically increasing attempt numbers;
- one terminal result per attempt;
- one published output per step;
- a stable idempotency key across all attempts of the same step in the same run;
- no automatic replay after an ambiguous attempt;
- durable retry timing and recovery for a scheduled but not-yet-started retry.

WOML does not claim that arbitrary JavaScript side effects execute exactly once.
If a script calls an external system, that system must honor the supplied
idempotency key for effect deduplication. An author who uses `retry > 1` is
explicitly allowing WOML to execute that script body again after a definitive,
retryable failure.

### 5.2 Attempt binding inside scripts

Every `<script>` executed as part of a Model v6 workflow receives a second
read-only binding named `attempt`:

```ts
interface WomlAttempt {
  readonly number: number;
  readonly maxAttempts: number;
  readonly idempotencyKey: string;
}
```

Example values:

```json
{
  "number": 2,
  "maxAttempts": 3,
  "idempotencyKey": "sha256:..."
}
```

This does not define or expose `context.run`. The existing context contract
remains unchanged:

```text
context.trigger
context.steps
```

`attempt` is execution metadata, not workflow data. WOML does not automatically
copy it into context or a step result.

Stored Model v1–v5 runs continue using their frozen script-host contracts. A
workflow compiled as Model v6 uses Script Host v3 consistently for all of its
script nodes, including nodes whose own maximum is one attempt.

For a sequential retry, the folded upstream `context` is identical across
attempts and the retrying node has no published output until success. The only
attempt-specific script input is the separate `attempt` binding.

Future `services.*` operations should receive the same idempotency key
automatically so most authors do not need to pass it manually. Raw JavaScript
authors can pass it to an external API today through a conventional
`Idempotency-Key` header or equivalent provider field.

### 5.3 Key derivation

The v1 logical step-effect key is:

```text
sha256(RFC8785({
  contract: "woml.step-effect",
  version: 1,
  runId,
  definitionHash,
  nodeId
}))
```

The serialized public form is `sha256:<64 lowercase hexadecimal characters>`.

Consequences:

- Attempt number is intentionally absent, so retries share the key.
- Invocation ID is intentionally absent, so process restart does not change it.
- Different nodes and different runs receive different keys.
- The immutable definition hash binds the key to the exact workflow definition.
- The key is operational metadata, not a credential or authorization token.

### 5.4 Duplicate behavior

The engine rejects duplicate or out-of-order attempt events. It never starts a
second invocation for an attempt already marked active or terminal.

If an external service receives the same idempotency key more than once, that
service is responsible for returning its original effect/result or another
documented duplicate response. WOML cannot invent deduplication for a service
that does not provide it.

## 6. Retryable Failure Matrix

The first executable script profile uses a conservative allow-list:

| Failure kind | Retry automatically? | Reason |
|---|---:|---|
| `script_threw` | Yes, when attempts remain | Bun definitively returned a thrown-script failure; authored retry opts into re-execution. |
| `script_timed_out` | No | The script may have completed an external effect before termination. |
| `invalid_script_result` | No | This is an author/contract error, not a transient failure. |
| `context_too_large` | No | Repeating with identical input cannot fix the limit. |
| `result_too_large` | No | Repeating the same step cannot safely fix the limit. |
| `worker_crashed` | No | External-effect status is ambiguous. |
| `host_crashed` | No | External-effect status is ambiguous. |
| `interrupted` | No | Recovery cannot prove what happened after attempt start. |
| `invocation_cancelled` | No | Cancellation is an engine control result, not a retry signal. |

The retryable classification belongs to the versioned runtime contract. Bun
reports what happened; Rust decides whether that failure may produce a retry.

Future service failures may include safe `retryable` and `retryAfterMs` fields,
but they must be reviewed with the service-capability contract rather than
silently treated as script failures.

## 7. Compiled Workflow Model v6

The TypeScript frontend lowers retry attributes into the existing language-
neutral `retryPolicy` node field:

```json
{
  "id": "generateSummary",
  "handler": "runtime.script",
  "inputs": {
    "kind": "object",
    "fields": {
      "source": {
        "kind": "literal",
        "value": "return { summary: 'Ready' };"
      }
    }
  },
  "retryPolicy": {
    "maxAttempts": 4,
    "backoff": {
      "kind": "exponential",
      "initialDelayMs": 1000,
      "multiplier": 2,
      "maximumDelayMs": 30000
    }
  }
}
```

Model v6 is required even though `retryPolicy` was reserved in earlier model
types. Earlier executable profiles explicitly reject it. Model v6 marks the
first profile where retry policy is an executable contract.

Model validation must enforce:

- retry policy is allowed only on `runtime.script` nodes authored as `<step>`;
- engine-owned selector, join, approval, notification, and parallel-control
  nodes cannot carry retry policy;
- `maxAttempts`, delay, strategy, multiplier, and cap match the frozen bounds;
- definitions without retry remain valid and unchanged;
- v1–v5 definitions and stored runs retain their existing behavior.

## 8. Run Event Schema v6

Run-event v6 adds `step_retry_scheduled` and extends v6
`step_attempt_started` with the stable `idempotencyKey`.

### 8.1 First attempt

```json
{
  "type": "step_attempt_started",
  "data": {
    "nodeId": "generateSummary",
    "attempt": 1,
    "invocationId": "inv_...",
    "handler": "runtime.script",
    "idempotencyKey": "sha256:..."
  }
}
```

### 8.2 Retry schedule

```json
{
  "type": "step_retry_scheduled",
  "data": {
    "nodeId": "generateSummary",
    "failedAttempt": 1,
    "nextAttempt": 2,
    "scheduledAt": "2026-08-07T12:00:01.000Z"
  }
}
```

`step_attempt_failed` remains the attempt outcome. `step_retry_scheduled` is a
separate engine decision derived from the compiled policy and the failure kind.

The durable write after a retryable failure is atomic:

```text
step_attempt_failed + step_retry_scheduled
```

The durable write after a final or non-retryable failure is atomic at the
relevant scope:

```text
step_attempt_failed + run_failed
```

or, inside parallel execution:

```text
step_attempt_failed + parallel/run terminal events when the group policy requires them
```

The folding projection records the latest attempt plus any pending retry and its
due time. A node with a pending future retry is not complete and is not normally
ready. When the due time arrives, Rust may start exactly `nextAttempt`.

## 9. Execution and Recovery Model

```text
                     definitive retryable failure
                              |
                              v
attempt started -> attempt failed + retry scheduled -> durable wait
       |                                               |
       | success                                       | due
       v                                               v
attempt succeeded                              next attempt started
       |                                               |
       +---------------- output published <------------+

ambiguous active attempt after crash -> interrupted -> run fails closed
```

### Safe recovery boundaries

| Durable boundary | Recovery behavior |
|---|---|
| No attempt start recorded | The node may start attempt 1 normally. |
| Attempt failure and retry schedule committed | Wait until `scheduledAt`, then start the recorded next attempt. |
| Retry schedule due but next attempt not started | Start the recorded next attempt once. |
| Attempt started with no terminal result | Append `interrupted` and fail closed; never replay it. |
| Attempt succeeded | Never rerun the node; continue downstream from the folded output. |
| Final attempt failed | Preserve the terminal failure; never schedule attempt `maxAttempts + 1`. |

The current in-memory runtime and durable runtime must use the same retry
decision and delay calculation. Only the durable runtime can recover after
process loss.

## 10. Composition Rules

### Sequential steps

Downstream work remains blocked until the retrying step either succeeds or
reaches a final failure.

### Branch arms

A step inside the selected arm may retry. Branch selection is immutable and is
not reevaluated between attempts. Unselected arms never run.

### Parallel children

Each child owns its own attempt counter, idempotency key, and schedule.

- A retryable child failure is not yet a parallel-child failure.
- A child waiting for backoff does not consume an active concurrency slot.
- Other children may continue while one child waits for retry.
- Every new attempt must still respect the parallel concurrency limit.
- `fail-fast` activates only after a child reaches a final failure.
- A final `fail-fast` failure cancels active siblings and abandons scheduled
  sibling retries durably through the parallel terminal event.
- `wait-all` lets every child reach success or final failure before completing
  the group.
- Every retry attempt receives the original parallel fork context; sibling
  outputs never leak into another child's retry.

### Human Approval arms

A selected approval arm may contain retrying steps. The human decision remains
immutable and is not requested again. Retrying a selected route never re-sends
approval notifications.

## 11. CLI Product Behavior

For a retrying workflow, stderr may show operational progress while stdout
remains reserved for the final JSON result:

```text
Step generateSummary failed (attempt 1/3): WOML_SCRIPT_THROWN
Retry 2/3 scheduled in 1s.
Step generateSummary succeeded on attempt 2/3.
```

The Rust-to-CLI progress surface must be versioned and contain only safe fields:
run ID, node ID, attempt numbers, schedule time, and stable failure code. It must
not forward context, script source, step output, secrets, capabilities, raw
provider errors, or authorization data.

Final exhaustion should be concise and source-aware:

```text
WOML runtime error [WOML_STEP_RETRIES_EXHAUSTED] at workflow.woml:8:5
(step "generateSummary"): attempt 3 of 3 failed [WOML_SCRIPT_THROWN].
```

If the CLI process is interrupted while a retry is durably scheduled, the run
remains resumable. The existing `--state` and `--resume` product path must be
generalized beyond approval-only runs so the same workflow definition and run ID
can continue the scheduled retry.

## 12. Diagnostics

Frontend diagnostics must include a stable code, source location, message, and
hint where useful. The initial catalog should include:

| Code | Meaning |
|---|---|
| `WOML_RETRY_INVALID` | `retry` is not an integer from 1 through 10. |
| `WOML_RETRY_BACKOFF_REQUIRES_RETRY` | A backoff attribute appears without `retry > 1`. |
| `WOML_RETRY_BACKOFF_INVALID` | The strategy is not `fixed` or `exponential`. |
| `WOML_RETRY_DELAY_INVALID` | The delay is zero, malformed, or outside the allowed range. |
| `WOML_RETRY_MAX_DELAY_INVALID` | The exponential cap is invalid or smaller than the initial delay. |
| `WOML_RETRY_MAX_DELAY_NOT_ALLOWED` | A fixed policy declares `retry-max-delay`. |
| `WOML_RETRY_HANDLER_UNSUPPORTED` | Retry policy appears on a handler that cannot execute it. |

Runtime codes should distinguish:

- the original attempt failure code;
- retries exhausted;
- malformed or impossible retry history;
- unsupported model/event/script-host versions; and
- interrupted/ambiguous attempts that were deliberately not replayed.

## 13. Versioned Artifacts to Freeze in RI0

Before runtime implementation, RI0 must produce reviewed artifacts for:

1. `docs/schemas/compiled-workflow-model.v6.schema.json`
2. `docs/schemas/run-event.v6.schema.json`
3. `docs/schemas/script-host-protocol.v3.schema.json`
4. `docs/schemas/execution-progress.v1.schema.json`
5. `docs/protocols/retry-idempotency-v1.md`
6. `docs/protocols/run-events-v6.md`
7. `docs/protocols/script-host-v3.md`
8. One retry WOML source fixture and its exact compiled Model v6 fixture.
9. Success-after-retry, exhausted, scheduled-recovery, and ambiguous-recovery
   event histories.

The compiled model remains the frontend/core interface. The event schema remains
the durable authority. The script-host schema remains the Rust/Bun execution
boundary. The progress schema is a presentation-only Rust/CLI boundary and does
not become workflow context.

## 14. Implementation Phases

### RI0 — Freeze retry and idempotency contracts

Status: completed.

Changes:

- Finalize attribute grammar, defaults, bounds, and `retry` total-attempt
  semantics.
- Freeze key derivation, duplicate behavior, retryable failure matrix, and
  ambiguous-effect policy.
- Freeze Model v6, Run Event v6, Script Host v3, and Execution Progress v1.
- Add reviewed source, compiled-model, event-history, and protocol fixtures.

Result:

Every layer has an exact contract to implement, but retry is not executable yet.

Gate:

Schemas validate every reviewed fixture, reject malformed/out-of-order shapes,
and contain no unresolved policy defaults.

### RI1 — Implement WOML retry attributes and lowering

Status: completed.

Changes:

- Move `retry` from staged to executable `<step>` attributes.
- Add `retry-backoff`, `retry-delay`, and `retry-max-delay` as attributes.
- Add source-aware validation and the frozen diagnostic codes.
- Lower valid attributes into the Model v6 `retryPolicy` field.
- Keep `<retry>` and retry attributes on structural tags invalid.

Result:

The frontend can parse, validate, and compile retry workflows deterministically.
The Rust runtime may still reject execution until later phases.

Gate:

The reviewed `.woml` fixture deep-equals the Model v6 fixture, invalid source
points to the exact attribute, and retry-free workflows compile unchanged.

### RI2 — Implement Rust event folding and durable retry scheduling

Status: completed.

Changes:

- Accept executable Model v6 retry policies on `runtime.script` nodes.
- Add Run Event v6 structures and semantic validation.
- Fold pending retry state and legal next-attempt identity from events.
- Make failed-attempt plus retry-schedule persistence atomic.
- Update ready-node logic so a scheduled retry is neither completed nor started
  early.
- Preserve v1–v5 model/event compatibility.

Result:

Rust can validate and reconstruct retry histories without executing JavaScript.

Gate:

Frozen histories fold identically in memory and SQLite across reopen, including
future schedules, due schedules, exhaustion, and invalid duplicate attempts.

### RI3 — Add the Bun `attempt` binding and stable effect identity

Status: completed.

Changes:

- Implement Script Host Protocol v3 attempt metadata.
- Inject a deeply frozen `attempt` binding into each isolated worker.
- Derive and validate one stable idempotency key in Rust.
- Give each attempt a fresh invocation ID while preserving the effect key.
- Keep environment variables, secrets, run context internals, and sibling
  parallel state outside the binding.

Result:

JavaScript can see its attempt number and can safely pass a stable deduplication
key to an external API.

Gate:

Protocol tests prove fresh worker isolation, stable keys across attempts and
restart, different keys across run/node boundaries, and no secret/context leak.

### RI4 — Execute sequential retries end to end

Changes:

- Replace hardcoded attempt `1` in the Rust script loop with the folded next
  attempt.
- Apply fixed/exponential backoff using the durable schedule.
- Retry only allow-listed failures and fail closed for every ambiguous failure.
- Publish output only after one successful attempt.
- Route Model v6 retry workflows through durable CLI execution.

Result:

A sequential retry workflow runs with `woml run`, fails its early attempts,
waits durably, succeeds, and feeds its single final output to the next step.

Gate:

An end-to-end fixture uses `attempt.number` to fail twice and succeed on attempt
3; the final JSON and durable event order match reviewed fixtures.

### RI5 — Compose retries with branch, parallel, and approval

Changes:

- Keep branch selection fixed across route-local retries.
- Add independent parallel child attempts, backoff queues, and concurrency-slot
  accounting.
- Trigger `fail-fast` only on final child failure and retain `wait-all`
  aggregation.
- Abandon scheduled sibling retries when a final fail-fast outcome closes the
  group.
- Retry selected approval-arm steps without reopening the approval or
  re-delivering notifications.

Result:

Retry behaves as one engine primitive everywhere a normal `<step>` may appear.

Gate:

Composition tests cover retry at workflow root, in every branch case, in both
parallel failure policies, after Human Approval, and in nested combinations.

### RI6 — Complete recovery, CLI progress, and diagnostics

Changes:

- Recover a scheduled retry without replaying completed attempts.
- Convert incomplete active attempts to `interrupted` and fail closed.
- Generalize `--resume` for durable retry runs.
- Add the versioned, secret-safe Rust-to-CLI progress surface.
- Print attempt counts, schedules, exhaustion, and source-aware final errors.
- Verify that context and downstream results contain only the successful output.

Result:

Users can understand retry behavior in the terminal and safely resume a run
that stopped while waiting for its next attempt.

Gate:

Crash-boundary tests cover before schedule, after schedule, before retry start,
during retry execution, after success, and before downstream dispatch.

### RI7 — Harden, package, and publish retry

Changes:

- Add deterministic-clock tests for all backoff boundaries and caps.
- Add a local fake external service proving stable-key duplicate handling.
- Test large context/results, process crashes, Worker crashes, and concurrent
  retry schedules.
- Update WOML language, CLI, architecture, recovery, and migration docs.
- Add the retry fixture to the clean-package smoke suite.
- Run frontend, Rust, Bun host, CLI, typecheck, Clippy, schema, packaging, and
  secret-leak gates.

Result:

`retry` becomes a supported and publishable WOML step attribute.

Gate:

A clean installation executes the reviewed retry workflow, survives every safe
restart boundary, refuses ambiguous replay, and passes the complete release
gate without changing older workflow behavior.

## 15. Expected File Areas

The implementation is expected to touch these load-bearing areas:

| Area | Primary files |
|---|---|
| Language and lowering | `woml/src/compiler.ts`, `woml/src/model.ts`, `woml/tests/compiler.test.ts` |
| Public language documentation | `docs/woml-v0.1.md`, new retry protocol documentation |
| Versioned contracts | `docs/schemas/compiled-workflow-model.v6.schema.json`, `run-event.v6.schema.json`, `script-host-protocol.v3.schema.json`, `execution-progress.v1.schema.json` |
| Rust model/events/folding | `core/woml-engine/src/model.rs`, `event.rs`, `projection.rs`, `engine.rs` |
| Durable scheduler/recovery | `core/woml-engine/src/durable.rs`, `runtime.rs` |
| Rust/Bun protocol | `core/woml-engine/src/protocol.rs`, `host.rs`, `woml-cli/src/script-host/*` |
| CLI execution and diagnostics | `core/src/woml_bridge.rs`, `woml-cli/src/rust-executor.ts`, `woml-cli/src/cli.ts` |
| Product fixture | `examples/retryWorkflow.woml` plus compiled/event fixtures |

Exact edits may be split into smaller modules during implementation, but the
layer ownership must remain unchanged: TypeScript understands WOML syntax, Rust
owns execution and durable decisions, and Bun executes isolated JavaScript.

## 16. Verification Matrix

| Area | Required proof |
|---|---|
| Syntax | Retry remains a step attribute; no retry tag or structural-tag retry compiles. |
| Attempts | Attempt numbers are contiguous and never exceed `maxAttempts`. |
| Backoff | Fixed/exponential schedules match frozen formulas and durable times. |
| Output | Only the successful attempt publishes `context.steps.<id>`. |
| Idempotency | One logical step key survives retries/restart; other run/node keys differ. |
| Failure safety | Only `script_threw` retries; ambiguous and deterministic contract failures do not. |
| Recovery | Scheduled work resumes; started-without-terminal work fails closed. |
| Branch | Selection never changes during a retry. |
| Parallel | Backoff releases slots; retry attempts respect concurrency and failure policy. |
| Approval | Decision/notifications are not repeated by route retries. |
| Compatibility | Model/event v1–v5 workflows behave exactly as before. |
| Security | Progress, errors, events, SQLite, and artifacts contain no secrets or capabilities. |
| Packaging | `woml run examples/retryWorkflow.woml` works from a clean package. |

## 17. Risks and Guardrails

### External effects are not automatically exactly once

The stable key enables deduplication but cannot force an external API to honor
it. Documentation and examples must use an idempotent endpoint or a pure script.

### Retrying timeout/crash failures can duplicate effects

The first profile deliberately fails these attempts closed. Expanding the
retryable matrix requires a separately reviewed effect contract.

### Parallel scheduling can become nondeterministic

Completion order may vary, but authored child order controls stable reporting,
and every attempt/schedule decision is durable before dispatch.

### Long delays can look like a hung CLI

The safe progress surface reports scheduled time and attempt count while stdout
remains machine-readable JSON only.

### Event changes are expensive

Model v6 and Event v6 are frozen before executor work. No implementation phase
may add an undocumented retry event or reinterpret an older history.

## 18. Roadmap After Retry and Idempotency

The global WOML roadmap is:

1. **Retries and idempotency** — current planned milestone. Make the `<step
   retry="...">` attribute genuinely executable, durable, and safe under the
   explicit effect boundary in this document.
2. **Production triggers** — implement webhook first, followed by schedule,
   interval, and event triggers.
3. **Services and capabilities** — add HTTP, database, messaging, and other
   registered operations while automatically carrying the step idempotency key.
4. **Lifecycle and engine controls** — add cancellation, lifecycle hooks,
   durable user state, and other engine-owned operations.
5. **Production runtime** — add hosting, deployment, observability, retention,
   queues, and worker ownership.
6. **Additional notification providers** — implement Discord and WhatsApp when
   product demand justifies their onboarding and operating cost.
7. **Retire the JavaScript chaining SDK** — remove the old SDK only after WOML
   reaches sufficient parity and users have a supported migration path.

After RI7, the next planning milestone is Production Triggers, beginning with
webhook execution.

## 19. Review Gate

Implementation begins only after review of the following hard-to-reverse
decisions:

- `retry` means maximum total attempts, not additional retries;
- fixed attribute grammar and default backoff values;
- `attempt` as the second JavaScript binding without defining `context.run`;
- the stable key derivation and duplicate responsibility boundary;
- only `script_threw` is retryable in the first profile;
- ambiguous attempts always fail closed;
- Model v6, Run Event v6, Script Host v3, and Progress v1 boundaries; and
- parallel scheduled-retry cancellation behavior.

Once these contracts are approved, RI0 is the first implementation phase.
