# WOML Runtime Policies and Durable User State Implementation Plan

Status: RP0 through RP7 and DS0 through DS5 completed on 2026-08-12. Runtime
Policy v1, Model v12, Event v11, Store v12 coordination, public
inspection/progress, and Definition Package v7 contracts are frozen. Durable
User State v1, Mutation Identity v1, State Operation Metadata v1, and the Store
v13 contract are also frozen. `<config>` validates and lowers outside the DAG.
Rust now durably executes concurrency, work-conserving FIFO queueing, strict
rolling-window rate limits, and immutable total workflow deadlines across every
run ingress. `woml run` and `woml test` execute all four Runtime Policy v1
fields, including workflows that use local modules.
Runtime Policies are publication-hardened through the clean-package,
compatibility, adversarial durability, performance-budget, package-audit, and
secret-scan `bun run test:runtime-policies` gate.
`services.state` is now a published, protected, typed authoring and runtime
surface in scripts and local modules. Calls execute through the Store v13 Rust
authority and never fall back to cache or process memory. Runtime Policies and
Durable User State are one roadmap milestone, delivered as two independently
reviewable releases: RP0-RP7 first, then DS0-DS5.

## 1. Product Outcome

WOML authors can place operational limits on a workflow without writing control
code, and can safely keep small correctness-critical values between runs without
misusing the disposable cache.

The runtime-policy authoring experience is:

```xml
<woml>
  <workflow id="process-order" name="Process an order" version="1.0.0">
    <config
      concurrency="4"
      timeout="10m"
      rate-limit="100/1m"
      queue="orders"
    />

    <triggers>
      <event id="newOrder" name="order.created" secret="EVENT_CONTROL_TOKEN" />
    </triggers>

    <steps>
      <step id="processOrder">
        <script>
          return {
            orderId: context.payload.orderId,
            processed: true
          };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The durable-state experience is an ordinary built-in service:

```xml
<step id="rememberConversation">
  <script>
    const key = `conversation:${context.payload.channelId}`;
    const previous = await services.state.get(key);

    await services.state.set(
      key,
      {
        lastMessage: context.payload.text,
        messageCount: previous.found
          ? previous.value.messageCount + 1
          : 1
      },
      {
        name: 'remember-conversation',
        ifVersion: previous.found ? previous.version : 0
      }
    );

    return { remembered: true };
  </script>
</step>
```

After the complete milestone:

- bursts of triggers are admitted durably and started according to policy;
- process restarts do not lose or duplicate queued runs;
- concurrent WOML processes sharing one state location obey the same limits;
- a workflow timeout becomes a truthful failed outcome;
- `woml list` and `woml get` explain why a run is queued or delayed;
- `services.state` provides durable, workflow-scoped JSON state with atomic
  compare-and-set behavior; and
- `services.cache` remains a best-effort performance optimization and never
  becomes an authority for workflow correctness.

## 2. Product Principles

### 2.1 Policy is declarative

`<config>` describes workflow runtime policy. It does not contain scripts,
lifecycle hooks, or implementation-specific Rust fields. The TypeScript
frontend lowers it into a language-neutral compiled policy that Rust owns.

### 2.2 Admission is not execution

A valid trigger occurrence can create a durable run before that run is allowed
to execute. The run may be queued because its workflow has no concurrency slot
or has exhausted its rate limit. Queueing is a first-class durable state, not an
in-memory delay hidden inside the CLI.

### 2.3 Limits must work across processes

The first supported runtime remains local to one machine and one WOML state
location. Within that boundary, two `woml run` processes sharing the same state
database must observe the same concurrency, rate-limit, queue, and timeout
authority. A process-local semaphore is not an acceptable implementation.

### 2.4 Waiting work does not consume active execution capacity

A concurrency slot is held while Rust is actively running business or lifecycle
work. It is released during a durable approval wait, retry backoff, synchronous
child-workflow wait, or policy queue wait, then reacquired before execution
continues. This prevents a 24-hour approval from blocking every later run.

Releasing a slot does not end the run and does not reset its timeout.

### 2.5 Timeout is failure, not cancellation

An author-declared workflow timeout means the workflow failed to finish within
its budget. It produces the stable `WOML_WORKFLOW_TIMED_OUT` failure, executes
`on-error`, then `on-complete`, and remains inspectable as failed. It is not
misreported as an operator cancellation.

### 2.6 Durable state is not context

Run context is still derived by folding immutable run events. `services.state`
is an explicit external capability, comparable to a small workflow-owned
database. Its mutable values never become a hidden authoritative context object
and are never injected automatically into `context`.

### 2.7 Durable state is not cache

`services.cache` may expire or evict data. `services.state` never evicts a value
to make room: it either commits a mutation durably or rejects it with a stable
quota/storage error. Correctness may depend on state; it may not depend on
cache.

### 2.8 Version every expensive boundary

The compiled policy, run events, store changes, scheduler claims, public
inspection, progress, and durable-state capability are schemas before they are
implementations. Models v1-v11 and Events v1-v10 remain immutable.

## 3. Scope

### Included in Runtime Policies

- One optional `<config>` direct child of `<workflow>`.
- `concurrency`, `timeout`, `rate-limit`, and `queue` attributes.
- A language-neutral top-level runtime policy in Compiled Workflow Model v12.
- Durable run admission before execution.
- A public `queued` run status.
- Workflow-ID-scoped concurrency across definitions and processes sharing one
  state location.
- Durable, work-conserving FIFO queueing.
- A strict rolling-window rate limit.
- A workflow deadline beginning at first execution, not trigger admission.
- Slot release and reacquisition around durable waits.
- Composition with manual, webhook, Slack, schedule, interval, named-event,
  Workflow Call, and Workflow Start admission.
- Durable queued-run cancellation.
- Model v12, Run Event v11, Store v12, Run List v2, Run Inspection v3, Runtime
  Policy Progress v1, and Definition Package v7 artifacts.
- Compatibility execution and inspection for Models v1-v11 and Events v1-v10.

### Included in Durable User State

- A built-in `services.state` capability.
- Workflow-scoped `get`, `set`, `delete`, `has`, `increment`, and
  `setIfAbsent` operations.
- JSON values, bounded keys, durable versions, and atomic conditional writes.
- Stable author operation names for mutation idempotency.
- One Rust/SQLite authority shared by processes using the same state location.
- Store v13 state tables and mutation-result records.
- Safe operation-event metadata without raw keys or values.
- Explicit quotas that reject writes rather than evicting durable values.
- Generated editor types and normal module access through the injected
  `services` object.
- Crash recovery, retry reattachment, compare-and-set contention, and
  migration coverage.

### Not included

- Distributed scheduling across multiple machines.
- Remote worker pools, autoscaling, or queue workers.
- Priority, weighted, delayed, or dead-letter workflow queues.
- Shared concurrency pools across unrelated workflow IDs.
- Per-customer, per-channel, or expression-keyed concurrency.
- Runtime changes to a policy already bound to an admitted run.
- A public `context.run` object.
- General workflow pause/resume.
- Timeout compensation, rollback, or cancellation propagation to independent
  child workflows.
- Cross-workflow state reads or a globally shared state namespace.
- State transactions spanning multiple keys.
- Querying, indexing, watching, or listing arbitrary state values.
- Automatic state expiry or eviction.
- A remote state administration API.
- Transparent encryption-at-rest in the local first profile.
- Postponed `services.queue` producer/consumer infrastructure.
- External Redis, DynamoDB, or other state backends.

## 4. Source-Language Contract

### 4.1 Placement

The canonical workflow order remains:

1. Optional `<config>`.
2. Optional `<lifecycle>`.
3. Optional `<triggers>`; omission means call-only.
4. Required `<steps>`.

`<config>` is a singleton, self-closing, data-only element.

```text
config := <config
            concurrency=positive-integer?
            timeout=duration?
            rate-limit=rate?
            queue=queue-name?
          />
```

At least one attribute must be present. Unknown attributes, child elements,
text, interpolation, and secret references are compile errors.

### 4.2 `concurrency`

```xml
<config concurrency="4" />
```

`concurrency` is the maximum number of actively executing runs for one workflow
ID in one state location. It is a positive integer. It is not:

- the concurrency of child nodes inside `<parallel>`;
- a per-process limit;
- a limit per workflow definition version; or
- a shared capacity pool for different workflow IDs.

Different versions of the same workflow ID share the limit. Every admitted run
keeps the exact policy snapshot of the definition that admitted it, while the
runtime rejects simultaneously activated definitions for the same workflow ID
whose active policy contracts conflict.

Omitting `concurrency` preserves current unbounded run admission, subject to
the runtime's global safety limits and other declared policies.

### 4.3 `rate-limit`

```xml
<config rate-limit="100/1m" />
```

The source grammar remains:

```text
rate := positive-integer "/" duration
```

V1 uses a strict rolling window: at any instant, no more than the declared
count of first execution starts may exist in the preceding duration. The next
queued run becomes eligible when the oldest relevant start leaves the window.

Only the first execution start consumes rate capacity. Resuming an approval,
retry, or child-workflow wait does not consume a second token. The durable start
event is the authority, so restart cannot reset the limit.

### 4.4 `timeout`

```xml
<config timeout="10m" />
```

The existing duration grammar applies:

```text
duration := positive-number ("ms" | "s" | "m" | "h" | "d")
```

The deadline starts when the run first begins execution. Time spent waiting in
the policy queue before that point does not count. After execution starts, the
deadline includes scripts, managed capabilities, retry backoff, approvals, and
synchronous Workflow Call waiting.

If the business outcome was already durably decided before the deadline won,
the timeout cannot rewrite that outcome. Lifecycle finalization uses its own
bounded action timeouts and completes with warnings when necessary.

### 4.5 `queue`

```xml
<config queue="orders" />
```

`queue` names the durable admission lane. A run that cannot start immediately
is ordered by:

```text
admitted_at + occurrence_sequence + run_id
```

V1 is work-conserving FIFO: the oldest eligible run whose workflow has capacity
starts first; a rate-limited or concurrency-blocked workflow does not prevent
an eligible workflow in the same lane from starting. If `queue` is omitted,
Rust derives a private lane from the workflow ID.

The queue name is durable routing and inspection metadata. It does not make two
workflow IDs share one concurrency limit. Shared-capacity pools, priorities,
and remote worker routing belong to Production Runtime.

The existing staged example `queue="moderation"` remains source-compatible.
Because `<config>` has never been executable, RP0 may tighten the queue-name
identifier grammar without breaking a published runtime contract.

### 4.6 Queue safety and overflow

Queue entries are durable runs, not transient messages. The local runtime has a
bounded safety ceiling per state location. Reaching it never silently drops a
trigger occurrence: admission returns `WOML_POLICY_QUEUE_FULL` through the
trigger's reviewed transport behavior and records enough occurrence metadata to
deduplicate a publisher retry without creating duplicate work.

An author-configurable `queue-limit` or `on-overflow` attribute is deliberately
not added in v1. Its semantics differ across webhook, schedule, provider, and
workflow-call ingress and require a separate reviewed product contract.

### 4.7 Queue policy is not `services.queue`

`<config queue="orders">` schedules workflow runs that already exist.
`services.queue`, which remains postponed, would be a producer/consumer message
system with claim, acknowledgement, redelivery, and dead-letter semantics. They
must not share an implementation or public contract merely because both use the
word “queue.”

## 5. Runtime Policy Semantics

### 5.1 Run lifecycle

For Model v12, the high-level lifecycle is:

```text
trigger accepted
  -> run admitted
  -> queued
  -> first execution started
  -> active execution / durable waits / resumptions
  -> business outcome decided
  -> lifecycle finalization
  -> terminal result
```

A run with no blocking policy may pass from admitted to started immediately,
but both facts remain distinct in Event v11.

### 5.2 Slot ownership

A durable scheduler claim, not an in-memory semaphore, grants one execution
slot. Claims have owner identity, lease expiry, and heartbeat. Claim and start
event changes use immediate SQLite transactions.

A slot is held while:

- a business step is active;
- branch or parallel business work is actively dispatching/executing;
- `on-start` or an eligible lifecycle action is active; or
- a resumed run is actively progressing toward its next durable wait/outcome.

A slot is released while:

- the run waits for approval;
- a retry delay is pending;
- a synchronous child workflow is pending;
- no policy currently permits the run to start; or
- the business outcome is decided and no lifecycle action is executing.

Before resumed work or lifecycle work executes, the run reacquires a slot. A
lease loss stops new dispatch and invokes the existing fail-closed recovery
rules for ambiguous active effects.

### 5.3 Policy scope and definition updates

Concurrency and rate history are scoped by:

```text
state location + workflow ID
```

They are not reset by a workflow version or source-file change. The exact
compiled policy is pinned to each admitted run. Activating a replacement
definition does not rewrite queued or active runs.

The runtime refuses conflicting simultaneously active policies for one workflow
ID until the older target is deactivated or drained. This avoids undefined
behavior such as one process enforcing concurrency 1 while another enforces 10.

### 5.4 Composition with triggers and workflow calls

All run creators use one admission authority:

- webhook returns an accepted run identity even when it is queued;
- Slack acknowledges only after durable admission;
- schedules and intervals persist their occurrence/cursor with admission;
- named events preserve subscriber deduplication;
- `services.workflows.call()` waits while its child is queued and resumes when
  the child reaches a terminal outcome;
- `services.workflows.start()` returns the durably admitted child run ID without
  waiting for execution; and
- `woml test` uses the same policy engine and waits visibly if its manual run is
  queued.

Parent runs release their active slot while waiting for a synchronous child.
This prevents a concurrency-1 workflow call from manufacturing a local slot
deadlock. Existing cycle detection remains authoritative.

### 5.5 Queue cancellation

`woml cancel run_...` accepts queued Event v11 runs. No business step and no
`on-start` hook executes. If the definition contains lifecycle cancellation
hooks, Rust acquires bounded lifecycle capacity, executes `on-cancel`, then
`on-complete`, and finalizes the run as cancelled.

### 5.6 Crash recovery

After restart, Rust folds Event v11 and reconstructs:

- admitted but never started runs;
- rate history from durable first-start events;
- active, waiting, finalizing, and terminal projections;
- expired scheduler claims; and
- each run's pinned policy and queue identity.

Mutable queue indexes, summary rows, and leases are rebuildable coordination
data. They are never the only proof that a run was admitted or started.

## 6. Compiled Workflow Model v12

Model v12 adds one optional top-level language-neutral object outside the DAG:

```ts
interface CompiledRuntimePolicyV1 {
  readonly profileVersion: 1;
  readonly concurrency?: number;
  readonly timeoutMs?: number;
  readonly rateLimit?: Readonly<{
    count: number;
    windowMs: number;
    algorithm: 'rolling_window';
  }>;
  readonly queue?: Readonly<{
    name: string;
    discipline: 'work_conserving_fifo';
  }>;
}

interface CompiledWorkflowDefinitionV12 {
  readonly schemaVersion: 12;
  readonly workflowId: string;
  readonly metadata?: CompiledWorkflowMetadata;
  readonly triggers: readonly CompiledTrigger[];
  readonly graph: CompiledWorkflowGraph;
  readonly moduleRuntime?: CompiledModuleRuntimeV1;
  readonly lifecycle?: CompiledLifecycleDefinitionV1;
  readonly runtimePolicy?: CompiledRuntimePolicyV1;
}
```

The exact schema is an RP0 review artifact. Important boundaries:

- the workflow graph remains a DAG;
- policy is outside graph nodes and edges;
- milliseconds are bounded safe integers;
- no WOML attribute spelling leaks into Rust;
- no secret, source location, resolved queue owner, or mutable runtime value is
  compiled;
- lifecycle and module definitions retain their frozen profiles; and
- Definition Package v7 carries Model v12 without changing existing module
  bundle or source-map identity rules.

During RP1, only workflows containing `<config>` emit Model v12. RP2 promotes
new admissions to the Model v12/Event v11 authority after Rust can persist it.
Models v1-v11 remain immutable and executable.

## 7. Run Event v11 and Folded State

Event v11 preserves the applicable v10 lifecycle/control vocabulary and adds
the minimum facts required to separate durable admission from execution:

```text
run_admitted
run_execution_started
run_timeout_reached
```

`run_admitted` contains the reviewed trigger/payload binding, definition
identity, immutable policy snapshot identity, queue identity, and admission
time. It is the first run event and makes public status `queued`.

`run_execution_started` occurs once. It establishes the timeout deadline and
is the durable rolling-window rate-limit fact. It does not recur after approval,
retry, or child-workflow waits.

`run_timeout_reached` records the deadline race before the existing Event v10
outcome/finalization sequence decides the run as failed with
`WOML_WORKFLOW_TIMED_OUT`.

Slot claim/release and heartbeats are bounded Store v12 coordination records,
not immutable run events. Their truth can be reconstructed from run projections
and live owners, and persisting every heartbeat would pollute the event log.

The public run status becomes:

```text
not_started | queued | running | waiting | cancelling | finalizing |
succeeded | failed | cancelled
```

`not_started` remains available for historical/in-memory pre-admission states.
New durable Model v12 runs begin at `queued`.

## 8. Store v12 Scheduling Authority

Store v12 adds rebuildable indexes and transactional authorities for:

- queued admissions;
- queue ordering and eligibility;
- active execution-slot claims and leases;
- rolling-window start history;
- pinned compiled-policy identity;
- timeout deadlines; and
- Run List v2 summaries with `admittedAt`, optional `startedAt`, queue name,
  and bounded waiting reason.

Events remain authoritative for run history. Queue indexes and public summaries
can be deleted and rebuilt exactly from definitions and Event v11 streams.

Leases are coordination state and may depend on a live clock. Recovery must
distinguish an expired owner from an ambiguous external effect: reclaiming a
slot does not authorize replay of an attempt that had started without a
terminal event.

## 9. Public Policy Inspection and Progress

`woml list` gains the `queued` status and Run List v2 fields. Human output may
show:

```text
run_...  process-order  queued  concurrency  queue=orders  admitted 8s ago
```

`woml get run_...` uses Run Inspection v3 and may expose:

```json
{
  "status": "queued",
  "policy": {
    "queue": "orders",
    "waitingFor": "rate_limit",
    "eligibleAt": "2026-08-11T12:00:30Z"
  }
}
```

The public surface never includes competing run payloads, exact internal lease
tokens, raw event IDs, operation keys, secrets, or another workflow's state.

Runtime Policy Progress v1 prints important transitions to stderr:

```text
Run run_... queued: workflow concurrency 4 is full.
Run run_... eligible at 2026-08-11T12:00:30Z due to rate limit 100/1m.
Run run_... started from queue "orders".
Run run_... failed: workflow timeout 10m was reached.
```

Long-lived `woml run` remains active after individual runs settle.

## 10. Durable User State v1

### 10.1 Public API

The proposed frozen service is:

```ts
interface StateEntry<T = JsonValue> {
  readonly found: true;
  readonly value: T;
  readonly version: number;
  readonly updatedAt: string;
}

interface MissingStateEntry {
  readonly found: false;
}

interface StateMutationOptions {
  readonly name: string;
  readonly ifVersion?: number;
}

interface StateService {
  get(key: string): Promise<StateEntry | MissingStateEntry>;
  has(key: string): Promise<{ present: boolean; version?: number }>;
  set(
    key: string,
    value: JsonValue,
    options: StateMutationOptions
  ): Promise<{ stored: true; version: number; updatedAt: string }>;
  delete(
    key: string,
    options: StateMutationOptions
  ): Promise<{ deleted: boolean }>;
  increment(
    key: string,
    amount: number,
    options: StateMutationOptions
  ): Promise<{ value: number; version: number; updatedAt: string }>;
  setIfAbsent(
    key: string,
    value: JsonValue,
    options: { readonly name: string }
  ): Promise<{
    stored: boolean;
    value: JsonValue;
    version: number;
    updatedAt: string;
  }>;
}
```

DS0 reviews the exact result shapes before freezing them. `name` is required on
mutations because durable state controls correctness. Rust derives one stable
logical mutation identity from run, logical step/lifecycle action, and author
operation name, allowing a retry to observe a committed mutation result instead
of applying it again.

Mutation names must be unique within one logical script subject. A duplicate
name with different operation, key digest, or input digest fails closed.

### 10.2 Compare-and-set

`ifVersion` is an optimistic concurrency check:

- `0` means the key must not exist;
- a positive value means the current version must match exactly;
- omission means unconditional replacement/deletion; and
- a mismatch returns `WOML_STATE_CONFLICT` without changing the value.

Every committed creation or mutation receives a monotonically increasing
version for that key. A same-operation retry returns its original result even
if the current key version has since changed.

### 10.3 Scope and lifetime

Rust derives state scope from:

```text
state location + workflow ID
```

Runs and definition versions of the same workflow ID share state. Different
workflow IDs cannot read it. State survives workflow runs, runtime restarts,
definition updates, and run-history retention. It remains until explicitly
deleted or an operator performs a future reviewed state-retention action.

Cross-workflow state sharing is deliberately unavailable. Workflows communicate
through `services.workflows.call()`, `services.workflows.start()`, named events,
or an explicit database rather than hidden shared keys.

### 10.4 Storage and authority

Store v13 adds a separate logical user-state area in the selected SQLite state
database. Every mutation transaction atomically commits:

1. the new state value/version or deletion;
2. the stable mutation identity and canonical result needed for reattachment;
3. bounded audit metadata; and
4. the immutable settlement proof required by the run authority.

The normal managed-operation success event follows immediately in the run
authority. If the host stops between those commits, recovery verifies the
immutable proof and appends the missing event without replaying the state
mutation. The interrupted script attempt still fails closed because other
script effects may be ambiguous.

State values are authoritative application data. They are not reconstructed by
folding the run event log. This does not weaken event-sourced run context:
`services.state` is an explicit managed side effect, just as a database write is
an external side effect.

Run events contain service/operation name, key digest, value/input digest,
version, byte counts, duration, and safe outcome only. Raw keys and values never
enter run events, summaries, progress, or diagnostics.

### 10.5 Bounds and quotas

The proposed local v1 defaults, finalized in DS0, are:

- non-empty UTF-8 keys up to 256 bytes;
- canonical JSON values up to 256 KiB each;
- at most 10,000 live keys per workflow ID;
- at most 64 MiB of canonical values per workflow ID; and
- safe-integer versions and numeric increments.

Unlike cache, durable state never evicts least-recently-used data. A write that
would exceed a bound fails atomically with `WOML_STATE_QUOTA_EXCEEDED` and
preserves the previous value.

### 10.6 Secrets and sensitive application data

Secret-store credentials are still resolved only for declared runtime secret
bindings and are never copied automatically into state. Because JavaScript can
derive arbitrary strings, WOML cannot truthfully prove that an author did not
write sensitive data into `services.state`.

The contract therefore guarantees:

- WOML never persists the `secrets` object or undeclared secret values by
  itself;
- state values are absent from events, CLI output, logs, diagnostics, and
  crash reports;
- exported state administration is not added in this milestone; and
- documentation warns that the local v1 state database is not transparently
  encrypted and must be protected like an application database.

Encryption-at-rest, tenant-managed keys, and remote authorization belong to
Production Runtime.

### 10.7 Relationship to cache

| Need                                  | Use                        | Behavior                                |
| ------------------------------------- | -------------------------- | --------------------------------------- |
| Avoid repeating an API call           | `services.cache`           | May expire or be evicted                |
| Remember an agent conversation cursor | `services.state`           | Persists until changed/deleted          |
| Maintain an atomic workflow counter   | `services.state.increment` | Durable and retry-safe by mutation name |
| Store large reports/files             | `services.storage`         | Durable object handle, not inline state |
| Query business records                | `services.db`              | Database schema/query capability        |

No cache row is migrated into state, and state never falls back to cache.

## 11. Error Surface

Every error retains WOML's code, source location when applicable, message, and
actionable hint contract.

### Compile-time policy errors

```text
WOML_CONFIG_DUPLICATE
WOML_CONFIG_EMPTY
WOML_CONFIG_CHILD_NOT_ALLOWED
WOML_CONFIG_ATTRIBUTE_UNKNOWN
WOML_CONFIG_CONCURRENCY_INVALID
WOML_CONFIG_TIMEOUT_INVALID
WOML_CONFIG_RATE_LIMIT_INVALID
WOML_CONFIG_QUEUE_INVALID
```

### Runtime-policy errors

```text
WOML_POLICY_VERSION_UNSUPPORTED
WOML_POLICY_CONFLICT
WOML_POLICY_QUEUE_FULL
WOML_POLICY_STORE_UNAVAILABLE
WOML_POLICY_STORE_CORRUPT
WOML_POLICY_LEASE_LOST
WOML_WORKFLOW_TIMED_OUT
```

Queueing because capacity is temporarily unavailable is normal progress, not an
error.

### Durable-state errors

```text
WOML_STATE_KEY_INVALID
WOML_STATE_VALUE_INVALID
WOML_STATE_VALUE_TOO_LARGE
WOML_STATE_CONFLICT
WOML_STATE_OPERATION_NAME_INVALID
WOML_STATE_OPERATION_IDENTITY_CONFLICT
WOML_STATE_QUOTA_EXCEEDED
WOML_STATE_INTEGER_REQUIRED
WOML_STATE_INTEGER_OVERFLOW
WOML_STATE_STORE_UNAVAILABLE
WOML_STATE_STORE_CORRUPT
WOML_STATE_CANCELLED
WOML_STATE_INTERRUPTED
WOML_STATE_RUNTIME_UNAVAILABLE
```

A missing key, deleting a missing key, or losing `setIfAbsent` is a successful
result, not an exception.

## 12. Versioned Artifacts Required Before Execution Code

### RP0 artifacts

1. Compiled Workflow Model v12 schema.
2. Definition Package v7 schema.
3. Run Event v11 schema and reviewed event sequences.
4. Runtime Policy v1 schema.
5. Scheduler Claim v1 schema.
6. Store v12 schema/migration contract.
7. Run List v2 schema.
8. Run Inspection v3 schema.
9. Runtime Policy Progress v1 schema.
10. Trigger admission/overflow response compatibility fixtures.

### DS0 artifacts

1. Durable User State v1 operation/result schema.
2. Store v13 schema/migration contract.
3. Durable State Mutation Identity v1.
4. State Operation Metadata v1.
5. Generated `services.state` TypeScript declaration fixture.
6. Crash, retry, contention, quota, and redaction event fixtures.

Capability Call v1 and Script Host v7 remain unchanged if DS0 proves that their
existing generic operation boundary can carry State v1 exactly. If that proof
fails, DS0 must version the boundary before implementation rather than adding an
undeclared field.

## 13. Implementation Phases

Runtime Policies ship first. Durable User State starts only after RP7 so a
state mutation does not land while the run-admission/store authority is still
changing underneath it.

### Phase summary

| Phase          | What changes                                                                | Result after the phase                                                                                 |
| -------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| RP0 (complete) | Freeze policy/model/event/store/inspection contracts and reviewed fixtures. | Every frontend, Rust, CLI, and persistence boundary agrees before code changes behavior.               |
| RP1 (complete) | Validate `<config>` and lower it into Model v12.                            | Authors can check policy syntax and inspect deterministic compiled policy, but it is not executed yet. |
| RP2 (complete) | Add Event v11 admission and Store v12 scheduling authority.                 | Rust can durably represent queued versus started runs without dispatching by policy yet.               |
| RP3 (complete) | Execute concurrency limits and durable FIFO queueing.                       | Bursts wait safely and start as capacity becomes available across processes/restarts.                  |
| RP4 (complete) | Execute strict rolling-window rate limits.                                  | Starts are durably paced without resetting after restart.                                              |
| RP5 (complete) | Execute workflow timeouts.                                                  | Overdue runs fail truthfully and run their failure lifecycle.                                          |
| RP6 (complete) | Integrate all trigger/call paths, cancellation, inspection, and progress.   | Policies behave consistently everywhere users can create or control a run.                             |
| RP7 (complete) | Harden, migrate, benchmark, document, and publish Runtime Policies.         | `<config>` becomes a supported executable WOML feature.                                                |
| DS0 (complete) | Freeze State v1, mutation identity, Store v13, and reviewed fixtures.       | Durable state semantics are reviewable before a mutable authority is added.                            |
| DS1 (complete) | Add frontend discovery, editor types, and the Bun `services.state` facade.  | Scripts and modules receive a typed service surface; runtime calls remain deliberately gated.          |
| DS2 (complete) | Build Store v13's transactional state authority.                            | Rust can perform atomic, versioned, idempotent state operations without Bun execution.                 |
| DS3 (complete) | Connect State v1 through the managed capability path.                       | Real `.woml` scripts can read and mutate durable state end to end.                                     |
| DS4 (complete) | Add contention, retry, recovery, quota, security, and inspection hardening. | State remains correct under concurrent runs, crashes, migrations, and adversarial input.               |
| DS5 (complete) | Package, benchmark, document, and publish Durable User State.               | `services.state` becomes a supported correctness-capable service distinct from cache.                  |

### RP0 — Freeze runtime-policy contracts and reviewed fixtures (completed)

Changes:

- Freeze source grammar and the exact meaning of all four `<config>`
  attributes.
- Freeze workflow-ID/state-location scope, policy conflicts, slot ownership,
  wait release/reacquisition, queue ordering, and rolling-window rate behavior.
- Freeze timeout start/end boundaries and the timeout-versus-outcome race.
- Freeze Model v12, Definition Package v7, Event v11, Store v12, scheduler
  claims, public statuses, inspection, list, and progress schemas.
- Freeze admission and overflow behavior for every trigger and workflow-call
  source.
- Add reviewed source/model/event/store fixtures for immediate start, queued
  start, wait/resume, rate delay, timeout, cancellation, crash, and policy
  conflict.
- Preserve Model v1-v11 and Event v1-v10 fixtures byte-for-byte.

Result:

The TypeScript frontend, Rust scheduler, SQLite store, N-API bridge, CLI, and
operator surfaces target the same reviewed contracts.

Gate:

Every RP0 schema validates its fixtures, deep-equality tests pass, compatibility
fixtures are unchanged, and the RP0 Review Gate in Section 18 is answered.

Completed implementation:

- Frozen schemas now cover Runtime Policy v1, Model v12, Definition Package v7,
  Event v11, Scheduler Claim v1, Store v12 logical records, Run List v2, Run
  Inspection v3, and Runtime Policy Progress v1.
- Reviewed fixtures pin admission/start/timeout events, durable queue/claim
  records, safe public inspection, wait/resume, queued cancellation, owner
  crash, policy conflict, and queue-overflow behavior for every ingress.
- `docs/protocols/runtime-policies-v1.md` is the normative RP0 contract.

### RP1 — Compile `<config>` into Model v12 (completed)

Changes:

- Move `<config>` from staged to executable frontend syntax.
- Validate singleton placement, canonical order, data-only content, integer,
  duration, rate, and queue-name bounds with line/column diagnostics.
- Normalize durations and rate windows into bounded milliseconds.
- Lower `CompiledRuntimePolicyV1` outside the DAG.
- Add Model v12 TypeScript types, language-neutral JSON Schema validation, and
  Definition Package v7 compilation for workflows with local modules. Rust
  deserialization/execution begins with the Event v11 authority in RP2.
- Preserve Model v11 output for source without `<config>` until RP2's Event v11
  authority is active.
- Update `woml check`, formatter/editor metadata, examples, and negative
  fixtures.

Result:

Users can author and validate `<config>`, and its compiled JSON exactly matches
the reviewed Model v12 fixture. Running policy-bearing source remains explicitly
gated until RP3 rather than silently ignoring limits.

Gate:

Frontend tests cover every attribute alone/together, boundaries, bad units,
overflow, duplicates, ordering, lifecycle/modules/triggers composition, and
Models v1-v11 compatibility.

Completed implementation:

- `<config>` is now a supported frontend element in its canonical workflow
  position and rejects empty, duplicate, misplaced, unknown, or executable
  content with source-aware diagnostics.
- The compiler normalizes source values into `CompiledRuntimePolicyV1` and
  emits Model v12 outside the DAG, including lifecycle and call-only
  composition.
- Local-module workflows emit deterministic Definition Package v7; runtime
  package promotion remains gated.
- `woml check` accepts policy workflows and explains the RP1 boundary, while
  `woml run` rejects Model v12 with
  `WOML_RUNTIME_POLICY_RUNTIME_UNAVAILABLE` instead of stripping or ignoring
  policy.
- `examples/runtimePolicyWorkflow.woml` is the manual RP1 authoring fixture and
  `bun run test:runtime-policy-authoring` is the focused contract/frontend/CLI gate.

### RP2 — Build durable Event v11 admission and Store v12 (completed)

Changes:

- Implement Event v11 validation/folding for `run_admitted`,
  `run_execution_started`, and `run_timeout_reached`.
- Add `queued` to Rust/public projections without reinterpreting older event
  streams.
- Migrate Store v11 to Store v12 transactionally.
- Atomically bind trigger occurrence, definition, policy identity, queue
  identity, run ID, and `run_admitted`.
- Add rebuildable queue/rate/summary indexes and leased scheduler claims.
- Add Run List v2 and Run Inspection v3 Rust authorities.
- Implement startup recovery and unknown/corrupt-version rejection without
  policy dispatch yet.

Result:

Rust can durably admit, fold, list, inspect, migrate, and recover Model v12 runs
while keeping execution behind a deliberate RP3 gate.

Gate:

Event/store tests cover atomic admission, duplicate occurrence, queue rebuild,
lease expiry, definition-policy conflict, v11-to-v12 migration, corruption,
unknown future versions, and compatibility inspection.

Completed implementation:

- Rust now validates Model v12 Runtime Policy v1 and Event v11's distinct
  `run_admitted`, `run_execution_started`, and `run_timeout_reached` facts.
- Event v11 folds admission to `queued`; execution begins only after the one
  durable execution-start fact, and timeout deadlines must match the compiled
  policy.
- Store v12 transactionally migrates the run-summary authority, binds policy
  and queue identities during trigger admission, and rebuilds queue, start,
  and summary indexes from immutable events on startup.
- Expiring Scheduler Claim v1 records coordinate ownership without becoming
  run truth. Claims can be reclaimed after expiry, but RP2 never dispatches a
  policy run.
- Run List v2 and Run Inspection v3 Rust authorities expose queued/admitted,
  optional started, queue, wait, eligibility, and timeout fields without
  leaking payloads or scheduler credentials. Legacy Run List v1 and Run
  Inspection v2 remain available for Event v1-v10 runs.
- Focused RP2 tests cover atomic/duplicate admission, queued folding, index
  rebuild, lease expiry, definition-policy conflict, start/timeout folding,
  v11-to-v12 migration, corrupt shapes, unknown future versions, and legacy
  inspection/list compatibility.

### RP3 — Execute concurrency and durable FIFO queueing (completed)

Changes:

- Add one Rust-owned policy scheduler that claims eligible runs and invokes the
  existing DAG runtime.
- Enforce workflow-ID concurrency across processes sharing Store v12.
- Implement work-conserving FIFO ordering and deterministic tie-breaks.
- Release/reacquire slots for approval, retry, Workflow Call, and lifecycle
  waits.
- Wake the next eligible run after slot release without polling every workflow.
- Recover expired owners without replaying ambiguous active attempts.
- Support queued cancellation and lifecycle finalization.
- Emit bounded policy progress.

Result:

A burst of ten runs against `concurrency="2"` executes at most two active runs
at once, the remaining eight survive restart, and eligible runs continue in
deterministic order.

Gate:

Tests cover sequential/parallel work, approval, retry, call/start, cancellation,
two processes, owner crash, lease expiry, fairness, no lost wake-up, and no
slot leak or oversubscription.

Completed implementation:

- Model v12 admissions now flow through a Rust-owned durable scheduler before
  the existing DAG executor. Claim acquisition and the one
  `run_execution_started` fact commit atomically.
- Store v12 enforces the workflow-ID concurrency ceiling across independent
  connections and preserves deterministic, work-conserving FIFO selection in
  shared queue lanes.
- Scheduler ownership is heartbeated and fail-closed. An expired owner may
  resume unambiguous work, while an active attempt with an unknown outcome is
  durably failed instead of replayed.
- Approval pauses release their claim. Retry delays and synchronous Workflow
  Calls retain a lightweight live-owner lease while releasing concurrency
  capacity, then reacquire capacity before business execution resumes.
- Queued cancellation claims only lifecycle/finalization capacity: it never
  writes a false execution-start fact and never executes a business step.
- Trigger-host startup rediscovers queued and ownerless Model v12 work, so runs
  survive process restarts. Bounded Runtime Policy Progress v1 reports queued,
  eligible, and started transitions.
- At the RP3 checkpoint, the CLI executed only concurrency/queue and rejected
  rate-limit/timeout rather than silently ignoring them; RP4 and RP5 have now
  removed that temporary gate.
- Definition Package v7 workflows that combine local modules with Model v12
  remain explicitly gated for the all-ingress/package integration in RP6.
- `examples/runtimePolicyConcurrencyWorkflow.woml` is the manual RP3 example;
  `bun run test:runtime-policy-scheduler` is the focused Rust/frontend/CLI gate.

### RP4 — Execute rolling-window rate limits (completed)

Changes:

- Calculate eligibility from durable first-start events.
- Claim a rate start atomically with queue eligibility and concurrency capacity.
- Schedule exact wake-ups for the oldest start leaving the window.
- Keep resumes free from additional rate consumption.
- Preserve rate history across restart, clock boundaries, and definition
  updates.
- Expose safe `rate_limit` waiting reason and `eligibleAt` inspection.

Result:

`rate-limit="3/10s"` starts no more than three new runs in any rolling ten-second
window, even when multiple WOML processes race or restart.

Gate:

Deterministic-clock tests cover boundary instants, bursts, concurrency plus
rate, clock rollback/forward policy, process races, restart, and no busy loop.

Completion notes (2026-08-12):

- First starts claim rolling-window capacity in the same transaction as queue
  eligibility and concurrency ownership.
- Resumes never consume rate capacity again; durable start events remain the
  rebuildable authority after restart.
- Rate waits expose `waitingFor: rate_limit` and an exact `eligibleAt` boundary.
- `examples/runtimePolicyRateLimitWorkflow.woml` is the manual RP4 example;
  `bun run test:rate-limits` is the focused gate.

### RP5 — Execute workflow timeouts (completed)

Changes:

- Establish one immutable deadline at `run_execution_started`.
- Race deadline settlement transactionally against business outcome and
  operator cancellation.
- Signal active Bun scripts and cancellable managed operations when timeout
  wins.
- Stop new steps/retries, settle active work truthfully, and append
  `run_timeout_reached`.
- Decide failure as `WOML_WORKFLOW_TIMED_OUT`, then run `on-error` and
  `on-complete` without rewriting the failure.
- Include approval, retry, and synchronous child waits in elapsed time while
  excluding pre-start queue delay.
- Recover overdue runs immediately after restart.

Result:

A workflow that exceeds its declared total run budget fails predictably,
remains inspectable, and cannot later commit a contradictory success.

Gate:

Race/crash tests cover before/after start, step success, managed effect,
approval, retry, child wait, cancellation, outcome decision, lifecycle
finalization, and restart past deadline.

Completion notes (2026-08-12):

- The first durable execution start freezes one deadline; queued time remains
  excluded and later waits remain inside the budget.
- Timeout settlement is transactional against normal outcomes and operator
  cancellation and records `run_timeout_reached` before the failed outcome.
- Active Bun work receives cancellation, late success cannot commit, and the
  public failure code is `WOML_WORKFLOW_TIMED_OUT`.
- `examples/runtimePolicyTimeoutWorkflow.woml` is the manual RP5 example;
  `bun run test:workflow-timeouts` is the focused release gate.

### RP6 — Integrate all ingress, controls, and operator surfaces (completed)

Changes:

- Route manual, webhook, Slack, schedule, interval, named event, Workflow Call,
  and Workflow Start through the same Event v11 policy authority.
- Implement transport-specific queue-full and accepted-queued responses without
  changing each source's deduplication identity.
- Update `woml list`, `woml get`, `woml cancel`, and `--json` schemas.
- Print curl/trigger readiness as before while explaining queued executions.
- Ensure long-lived `woml run` keeps scheduling after individual outcomes.
- Add multi-file and folder activation policy-conflict diagnostics.
- Add manual examples for concurrency, rate limit, timeout, and mixed policies.

Result:

Users receive the same policy behavior and understandable status regardless of
how a workflow was triggered or which local process owns execution.

Gate:

Packaged CLI tests and real/manual trigger fixtures prove every ingress path,
shared state across terminals, cancellation, JSON output, redaction, shutdown,
and restart.

Completed implementation:

- Manual, webhook, Slack, schedule, interval, named-event, Workflow Call, and
  Workflow Start admission now converge on Model v12's Event v11 scheduler;
  call/start children use a reserved durable `workflow-call` admission identity
  instead of bypassing policy, without changing frozen Event v11's shape.
- The local state queue has a 10,000-entry fail-closed ceiling. Overflow creates
  no run, preserves the publisher's retry identity, returns HTTP 503 plus
  `Retry-After` for webhooks, remains retryable for event/provider/call ingress,
  and surfaces `WOML_POLICY_QUEUE_FULL`.
- Trigger-host and direct executions publish frozen Runtime Policy Progress v1,
  and the CLI explains queued, eligible, started, and timed-out policy states.
- `woml list` now uses Run List v2 (including queued runs), while `woml get`
  selects Run Inspection v3 for policy runs and retains v2 for legacy runs;
  human and `--json` output expose queue, wait, eligibility, and timeout data.
- Runtime startup checks active definition/policy conflicts before activation,
  producing `WOML_POLICY_CONFLICT` rather than failing after a trigger arrives.
- Definition Package v7 local modules execute with Model v12 by promoting the
  exact frozen compiled artifacts at activation time; the immutable compilation
  package shape is unchanged.
- The concurrency, rate-limit, timeout, and mixed-policy examples are the RP6
  manual suite. `bun run test:runtime-policy-ingress` is the integrated release gate.

### RP7 — Harden and publish Runtime Policies (completed)

Changes:

- Run adversarial scheduler, SQLite contention, time, queue, crash, corruption,
  migration, and cancellation tests.
- Benchmark policy-disabled overhead, admission, queue wake-up, slot claim,
  rate eligibility, list/get, and timeout detection.
- Prove that source without `<config>` retains negligible additional overhead.
- Test large but bounded queues and long-lived runtime memory behavior.
- Validate clean install, native packaging, schemas, definition packages,
  recovery, and secret scans.
- Update language, architecture, triggers, workflow calls, lifecycle, recovery,
  deployment, CLI, and migration documentation.
- Add one `test:runtime-policies` publication gate.

Result:

`<config>` is executable and publishable rather than staged syntax. Runtime
Policies can release independently before Durable User State.

Gate:

The clean-package release matrix passes on supported platforms, all old model
and event fixtures remain compatible, performance budgets pass, and no mutable
queue index is required to reconstruct durable run truth.

Completed implementation:

- A 1,000-run reconstruction test proves queue/summaries are rebuildable from
  Event v11 truth after deliberate index damage; repeated recovery is
  idempotent and a 24-process contention test cannot oversubscribe capacity.
- The packaged CLI is installed into a clean consumer and proves `<config>`
  check, execution, Run List v2, and redacted Run Inspection v3 behavior.
- Runtime Policy Progress v1 decoding rejects unknown/private fields and
  accepts only the frozen queue, eligibility, start, and timeout shapes.
- `benchmark:runtime-policies` publishes a versioned local report for the
  no-`<config>` baseline, policy overhead, shared durable bursts, list/get,
  rate eligibility, timeout detection, and bounded long-lived memory growth.
- The operator guide now connects language syntax, architecture, every trigger,
  Workflow Calls, lifecycle, recovery, webhook deployment, SDK migration, and
  CLI usage to one production contract.
- `verify-runtime-policy-release.ts` compiles all schemas, validates historical fixtures, enforces
  benchmark budgets, audits the package allowlist, and scans built artifacts
  for active WOML secrets.
- `bun run test:runtime-policies` builds and installs the release package, runs frontend/CLI
  and Rust adversarial suites, checks TypeScript and strict Rust Clippy, then
  runs the publication verifier. The gate passes on 2026-08-12.

### DS0 — Freeze Durable User State contracts (completed)

Changes:

- Freeze State v1 methods, result shapes, key/value bounds, versions,
  compare-and-set, missing-key behavior, and quotas.
- Freeze workflow scope, definition-update behavior, lifetime, and separation
  from cache/context/storage/database.
- Freeze required mutation names and retry/reattachment identity.
- Freeze Store v13 tables, atomicity, audit metadata, event redaction, and
  corruption behavior.
- Prove whether Capability Call v1 and Script Host v7 remain sufficient.
- Add reviewed success, conflict, duplicate, crash, quota, and redaction
  fixtures.

Result:

Frontend, Bun, Rust, SQLite, retry, and recovery work target one reviewed State
v1 contract before any user-controlled mutable data is stored.

Gate:

Every DS0 schema/fixture passes, the DS0 Review Gate in Section 19 is answered,
and no cache guarantee is silently promoted into durable state.

Completed implementation:

- Durable User State v1 freezes the six methods, exact result unions, JSON and
  key bounds, monotonic versions, conditional writes, missing-key behavior,
  quotas, workflow scope, and stable failures.
- Mutation Identity v1 freezes how named writes reattach safely, while State
  Operation Metadata v1 proves that operation records contain no raw keys or
  values.
- Store v13 freezes the logical entry, mutation-result, and quota records plus
  its transactional Store v12 migration. It does not claim encryption that the
  local profile does not yet provide.
- Reviewed success, conflict, duplicate, interruption, quota, redaction, editor
  declaration, and generic Capability Call v1 fixtures are validated together.
- Capability Call v1 and Script Host v7 require no shape change: State v1 fits
  their existing versioned generic operation boundary.
- `docs/protocols/durable-state-v1.md` answers all 14 DS0 review questions, and
  `bun run test:durable-state-contracts` is the contract gate. No user value is stored by DS0.

### DS1 — Add frontend discovery, types, and Bun facade (completed)

Changes:

- Reserve `state` as a built-in service/module alias.
- Discover `services.state` usage in steps, lifecycle scripts, and imported
  local modules.
- Generate the reviewed `StateService` editor declaration automatically during
  normal `woml run`/`woml check` preparation.
- Add the deeply read-only Bun facade and strict argument/result validation.
- Reject calls with a staged `WOML_STATE_RUNTIME_UNAVAILABLE` response until
  DS2/DS3 are enabled; never fall back to cache or an in-memory map.

Result:

Authors receive a normal typed `services.state` experience without installing a
package or generating types manually, while execution remains safely gated.

Gate:

TypeScript/JavaScript/module fixtures cover method types, immutable facade,
aliases, dynamic access, invalid arguments, and no accidental `context` or
`secrets` widening.

Completed implementation:

- `state` is a protected built-in name and cannot be shadowed by a local module.
- Static discovery covers step and lifecycle scripts plus imported JavaScript
  and TypeScript modules; computed `services[...]` access in modules is rejected
  with an actionable diagnostic.
- Normal `woml check` and `woml run` preparation generates the reviewed
  `StateService` types even when the workflow imports no modules.
- Bun exposes one deeply frozen facade with strict method, key, value, option,
  and future result validation.
- Every call is deliberately rejected with
  `WOML_STATE_RUNTIME_UNAVAILABLE` before a capability request is emitted, so
  DS1 cannot accidentally mutate cache, memory, or an undeclared backend.
- `bun run test:durable-state-authoring` is the authoring-surface gate.

### DS2 — Build Store v13 transactional state authority (completed)

Changes:

- Add transactional Store v12-to-v13 migration.
- Implement workflow-scoped get/has/set/delete/increment/set-if-absent.
- Canonicalize JSON and enforce key/value/quota/integer bounds.
- Implement monotonic key versions and atomic `ifVersion` checks.
- Atomically store mutation identity, input digest, and canonical result.
- Return the original result for a duplicate logical mutation and reject an
  identity reused with different inputs.
- Emit bounded generic managed-operation metadata without raw key/value data.
- Add direct Rust authority tests without the Bun host.

Result:

Rust provides a durable, atomic, retry-aware user-state database even though
workflow scripts cannot invoke it until DS3.

Gate:

Tests cover all operations, multiple database connections, conflicts, duplicate
identity, rollback, crash boundaries, quota, corruption, migration, and exact
redaction.

Completed implementation:

- Store v12 now migrates transactionally to Store v13 and creates separate
  state-entry, immutable mutation-result, and quota tables without rewriting
  workflow definitions or run histories.
- Rust directly executes all six State v1 operations with canonical JSON,
  bounded UTF-8 keys and values, exact public result shapes, and monotonically
  increasing per-key versions that survive deletion and recreation.
- `ifVersion` compare-and-set, `setIfAbsent`, quota accounting, value changes,
  mutation identity, and the original canonical result commit in one immediate
  SQLite transaction.
- Duplicate named mutations return their first result without executing twice;
  reuse with another canonical input fails closed.
- Independent SQLite connections cannot both win the same version condition,
  and quota or validation failures leave existing state and counters unchanged.
- The direct authority produces only digest-based safe operation metadata;
  keys, values, workflow/run IDs, and operation identities remain absent.
- Startup rejects missing or malformed v13 objects, and a failed migration
  rolls back without claiming schema version 13.
- `bun run test:durable-state-store` is the direct Rust authority and compatibility gate. WOML
  scripts remain deliberately gated until DS3.

### DS3 — Execute `services.state` end to end (completed)

Changes:

- Register State v1 in the existing managed capability registry.
- Route Bun calls through Capability Call v1 to the Rust Store v13 authority.
- Derive workflow scope and logical mutation identity internally; scripts
  cannot override them.
- Compose with step retry, lifecycle scripts, branches, parallel, approvals,
  cancellation, workflow timeout, Workflow Call, and Workflow Start.
- Reattach a repeated named mutation to its durable result.
- Add safe service progress and one real `.woml` example.

Result:

A user can run a workflow twice and observe a durable value/version from the
first run, while concurrent updates are protected by compare-and-set.

Gate:

End-to-end tests cover read/write/delete, atomic increment, set-if-absent,
conditional conflict, retry after later script failure, host/worker crash,
cancel/timeout, modules, lifecycle, and restart.

Completion notes:

- The production registry now exposes all six State v1 operations and binds
  them to the same Store v13 database selected by `woml run --state`.
- Bun performs author-facing argument validation and sends the frozen State v1
  request through Capability Call v1; Rust supplies workflow ID, active step
  identity, cancellation, deadlines, and durable operation events.
- Named mutations reattach to their original committed State v1 result when a
  later script failure causes the step to retry.
- Operation events contain the State metadata profile, operation, key/input/
  result digests, outcome, duration, and optional version. Raw keys and values
  are excluded.
- `examples/durableStateWorkflow.woml` is the manual product journey: running
  it repeatedly with the same state path returns a growing durable counter.
- `bun run test:durable-state-runtime` covers the frozen authoring/authority gates plus real
  Bun-to-Rust execution, restart persistence, redaction, retry reattachment,
  branches, parallel children, lifecycle scripts, and imported modules.

### DS4 — Harden concurrency, recovery, limits, and security (completed)

Changes:

- Stress concurrent runs and processes contending for the same keys.
- Prove same-operation retry is applied once while distinct operation names are
  independent.
- Add database busy handling, bounded backoff, integrity checks, backup/recovery
  guidance, and fail-closed corruption behavior.
- Audit every event, progress, error, inspection, log, fixture, and crash path
  for raw key/value/secret leakage.
- Prove run deletion/retention does not accidentally delete workflow state.
- Add state-size and operation-latency benchmarks.
- Document local database permissions and lack of transparent encryption.

Result:

Durable state remains correct and diagnosable under real concurrency and crash
conditions without becoming visible through general run inspection.

Gate:

Contention, retry, crash, migration, quota, redaction, and performance budgets
pass with multiple Rust connections and packaged CLI processes.

Completion notes:

- Independent Rust processes prove that SQLite serializes atomic increments
  without lost updates. Thread contention proves that a duplicate logical
  mutation applies once while distinct names remain independent.
- State connections use a five-second bounded SQLite busy handler. Short locks
  recover automatically; exhausting the bound remains the stable retryable
  `WOML_STATE_STORE_UNAVAILABLE` failure.
- Startup audits SQLite integrity and foreign keys, required Store v13 objects,
  entry and mutation digests, canonical JSON, timestamps, result envelopes,
  live versions, and quota totals. Contradictions fail closed with
  `WOML_STATE_STORE_CORRUPT`.
- A committed immutable mutation record is a recovery settlement proof. If the
  host stops before the operation-success event, recovery reconstructs that
  event without replaying the mutation and still fails the interrupted script
  attempt closed.
- Unix state files are hardened to owner-only `0600`. Run inspection and event
  surfaces remain redacted, and state tables are independent of run ownership.
- The performance gate writes 256 one-KiB values, enforces a 100 ms local p95
  operation budget and 16 MiB database-size budget, and is available as
  `bun run benchmark:state`.
- `docs/woml-durable-state.md` documents contention, cancellation, coherent
  backup/restore, fail-closed corruption, local permissions, and the lack of
  transparent encryption.
- `bun run test:durable-state-hardening` composes DS0 through DS3 compatibility with the DS4
  concurrency, recovery, corruption, redaction, permissions, and performance
  suite.

### DS5 — Publish Durable User State (completed)

Changes:

- Add clean-consumer installation and packaged native execution tests.
- Add one agent/conversation-state example and one atomic-counter example.
- Update services, cache, storage, database, architecture, recovery, security,
  deployment, CLI, and SDK migration documentation.
- Add a clear cache-versus-state decision guide.
- Validate every repository schema together and preserve historical fixtures.
- Add one `test:durable-state` publication gate and a combined milestone release check.

Result:

`services.state` is a supported built-in service for small durable workflow
state, and users have a clear path for cache, state, storage, or database data.

Gate:

The feature passes frontend, Rust, Bun, N-API, SQLite, retry, lifecycle, policy,
workflow-call, migration, packaging, benchmark, and secret/redaction tests from
a clean project.

Completion notes:

- `examples/atomicCounterWorkflow.woml` demonstrates an atomic counter under
  concurrent policy runs; `examples/conversationStateWorkflow.woml`
  demonstrates version-checked, workflow-owned conversation memory.
- A clean consumer installs the packed CLI archive, generates StateService
  declarations, runs the native engine in fresh processes, observes counter and
  conversation values across restarts, verifies redacted inspection, and checks
  Unix owner-only state-file permissions.
- `docs/woml-data-guide.md` gives one cache/state/storage/database decision
  model. Services, cache, storage, database, architecture, recovery, local data
  security, deployment, CLI, language, and SDK migration documents now carry
  the published State v1 boundary.
- `verify-durable-state-release.ts` compiles every repository schema together, validates Models
  v1-v12 and Events v1-v11 against historical fixtures, validates all State v1
  fixtures, runs both examples twice, enforces the state benchmark, audits the
  package allowlist, and scans public artifacts for active WOML secrets.
- `bun run test:durable-state` is the State publication gate.
  `bun run test:runtime-state-release` composes the RP7 Runtime Policies gate
  with DS5, and the repository release command now includes that combined
  milestone check.

## 14. Expected File Areas

| Area                  | Expected locations                                                            |
| --------------------- | ----------------------------------------------------------------------------- |
| Grammar and lowering  | `woml/src/compiler.ts`, `model.ts`, editor/type helpers                       |
| Model/package schemas | `docs/schemas/compiled-workflow-model.v12.schema.json`, Definition Package v7 |
| Events and projection | `core/woml-engine/src/event.rs`, `projection.rs`, `engine.rs`                 |
| Policy scheduler      | new focused Rust policy/scheduler module plus `runtime.rs` integration        |
| Durable store         | `core/woml-engine/src/durable.rs`, Store v12/v13 migrations and tests         |
| Trigger admission     | webhook/Slack/schedule/interval/event/workflow-call Rust authorities          |
| Runtime waits         | retry, approval, workflow-call, lifecycle, cancellation integration           |
| State capability      | `core/woml-engine/src/capability.rs` plus focused State v1 Rust module        |
| Bun services          | `woml-cli/src/script-host/*`, service facade/types/tests                      |
| N-API boundary        | `core/woml-native/src/bridge.rs`                                                     |
| CLI                   | `woml-cli/src/cli.ts`, `rust-executor.ts`, packaged tests                     |
| Contracts             | `docs/schemas/*`, `docs/protocols/*`, reviewed fixtures                       |
| Examples/docs         | runtime-policy and durable-state examples/operator guides                     |

The final implementation may split large existing Rust files into focused
modules. The contracts, not the current file layout, decide ownership.

## 15. Verification Matrix

| Area            | Required proof                                                            |
| --------------- | ------------------------------------------------------------------------- |
| Grammar         | `<config>` is singleton, data-only, bounded, and precisely diagnosed.     |
| Model           | Model v12 keeps policy outside the DAG and preserves v1-v11.              |
| Admission       | Every trigger creates at most one durable Event v11 run before dispatch.  |
| Queue           | FIFO is deterministic, work-conserving, durable, and rebuildable.         |
| Concurrency     | Active slots never exceed the workflow limit across processes/restart.    |
| Waits           | Approval, retry, and call waits release then safely reacquire slots.      |
| Rate            | Strict rolling windows survive boundaries, races, and restart.            |
| Timeout         | Deadline races cannot produce contradictory success/failure.              |
| Cancellation    | Queued and active runs cancel durably with correct lifecycle hooks.       |
| Calls           | `.call()` waits without slot deadlock; `.start()` returns queued run ID.  |
| Inspection      | List/get/progress explain queue state without leaking data.               |
| Migration       | Store v11-to-v12-to-v13 is transactional and old runs remain valid.       |
| State scope     | Same workflow/state location shares; other workflows cannot read.         |
| State atomicity | Versions, CAS, increment, and set-if-absent are transaction-safe.         |
| State retries   | One named logical mutation reattaches rather than double-applies.         |
| Cache boundary  | State never evicts/falls back; cache never becomes authoritative.         |
| Recovery        | Index loss, owner crash, ambiguous effect, and expired lease fail safely. |
| Security        | No secrets, state values, raw keys, leases, or operation IDs leak.        |
| Performance     | Policy-disabled overhead is negligible; scheduler/state budgets pass.     |
| Packaging       | A clean consumer can compile, activate, queue, inspect, and use state.    |

## 16. Risks and Guardrails

### Concurrency can be confused with `<parallel>`

Documentation and diagnostics call one “workflow-run concurrency” and the other
“parallel child concurrency.” Their compiled fields and schedulers remain
separate.

### Waiting approvals can exhaust capacity

Durable waits release active slots. Timeout continues, so authors still control
how long a run may remain unresolved.

### Queueing can hide overload

Queued status, reason, age, and queue safety ceilings are visible. WOML never
silently drops work or lets an in-memory array grow without bound.

### Different processes can oversubscribe

All eligibility and claim decisions happen in immediate Store v12 transactions.
Process-local counters are caches only.

### Definition updates can bypass limits

Limits key on workflow ID rather than definition hash. Conflicting active
policies fail activation instead of creating two competing authorities.

### A timeout can be mistaken for user cancellation

Timeout has its own event/failure code and triggers failure lifecycle. Operator
cancellation retains the existing cancelled outcome.

### State can become a hidden database

State v1 is intentionally small, JSON-only, workflow-scoped, quota-bounded, and
unqueryable. Large objects use storage; relational/queryable business data uses
database.

### State retries can double-apply mutations

Mutations require a stable author name. Rust stores identity and result in the
same transaction as the value change and rejects identity/input conflicts.

### State can leak sensitive data

Values stay out of events and CLI surfaces, but the local SQLite file is still
application data. Documentation requires filesystem protection and does not
promise encryption that is not implemented.

### Cache and state can blur together

They have separate names, contracts, tables, documentation, and failure
behavior. No fallback or migration connects them.

### Shared named queues can imply a worker pool

V1 named lanes provide durable ordering/routing only. They do not promise
remote workers or shared capacity across workflow IDs. Production Runtime owns
that expansion.

## 17. Global Roadmap After Runtime Policies and Durable User State

1. **Fork and Branch Execution** — rename conditional source flow to
   `<choose>` and add durable `<fork>`/`<branch>` routes with multi-step branch
   bodies and selective main-route joins.
2. **Additional Communication Providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when justified.
3. **Retire the JavaScript Chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

Completed roadmap milestones—Retries and Idempotency, Production Triggers,
Services and Capabilities, the essential Module System, Durable Workflow Calls,
Workflow Start, Lifecycle and Engine Controls, Runtime Policies, Durable State,
and Production Runtime and Operations—remain the baseline and are not repeated
as future work.

## 18. RP0 Review Gate

No runtime-policy execution code begins until RP0 answers and tests:

1. Is admission durably separate from execution without changing old events?
2. Is `queued` a public Event v11 state with exact list/get behavior?
3. Are concurrency and rate limits scoped by state location plus workflow ID?
4. Do definition versions share limits without rewriting pinned queued runs?
5. Does one durable transaction prevent cross-process slot oversubscription?
6. Do approval, retry, Workflow Call, and lifecycle waits release slots safely?
7. Is FIFO deterministic and work-conserving without claiming shared capacity?
8. Is rolling-window rate behavior exact at every clock boundary?
9. Does the timeout exclude pre-start queue delay and include later durable
   waits?
10. Can timeout, success, failure, and cancellation races produce only one
    business outcome?
11. Does every trigger/call ingress retain its deduplication and response
    contract while queued?
12. Can all queue/rate/summary indexes be rebuilt from definitions and events?
13. Does owner recovery preserve the fail-closed rule for ambiguous effects?
14. Are Models v1-v11, Events v1-v10, Store v11, and their fixtures unchanged?
15. Is `<config queue>` explicitly separate from postponed `services.queue`?

Once those artifacts are reviewed, RP1 may begin with frontend compilation.

## 19. DS0 Review Gate

No durable user value is stored until DS0 answers and tests:

1. Are State v1 methods and return shapes small, predictable, and JSON-only?
2. Is state scoped internally by state location plus workflow ID?
3. Are state and cache impossible to confuse in guarantees or storage?
4. Is every mutation named and durably reattachable across step retries?
5. Does identity reuse with different inputs fail closed?
6. Are compare-and-set and version changes atomic across processes?
7. Can a duplicate increment return its first result without incrementing
   twice?
8. Do quota failures preserve the previous value and never evict another key?
9. Are raw keys and values absent from events, summaries, progress, and errors?
10. Is Store v13 mutation plus operation-result settlement one transaction?
11. Does run-history retention leave state untouched?
12. Do Capability Call v1 and Script Host v7 carry State v1 without an
    undeclared protocol change?
13. Are encryption and remote authorization described honestly as deferred?
14. Does `services.state` avoid expanding `context.run` or injecting state into
    context automatically?

Those questions are answered by the frozen DS0 schemas, fixtures, and
`docs/protocols/durable-state-v1.md`. DS1 through DS5 have completed without
changing the frozen public contract.

## 20. Definition of Done

Runtime Policies are complete when a user can:

1. author one `<config>` with concurrency, rate, timeout, and queue policy;
2. send a burst through any supported trigger and observe durable queueing;
3. share one state database across processes without exceeding limits;
4. restart WOML without losing queue order, rate history, or run identity;
5. inspect and cancel queued runs through direct CLI commands;
6. observe a workflow timeout as failed with correct lifecycle behavior; and
7. pass the RP7 clean-package release gate with historical compatibility.

Durable User State is complete when a user can:

1. read, create, conditionally update, increment, and delete workflow state;
2. run the workflow again and observe the committed value/version;
3. safely retry a named mutation without applying it twice;
4. resolve concurrent changes through explicit version conflicts;
5. restart or move execution to another local process sharing the same state
   location without losing data;
6. distinguish state from cache, storage, and database using clear docs; and
7. pass the DS5 clean-package, concurrency, crash, migration, benchmark, and
   redaction gates.

The milestone is not complete merely because `<config>` parses, a semaphore
limits one process, or a SQLite key/value table exists. Durable admission,
events, recovery, cross-process scheduling, time semantics, idempotent state
mutation, inspection, migration, security, and compatibility must agree on the
same reviewed contracts.
