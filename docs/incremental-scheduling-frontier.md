# Cursor-local readiness frontier and safety verification

This records ISQ4/ISQ5 on `perf/runtime-performance`, after
[immutable topology and latest-attempt reuse](incremental-scheduling-reuse.md).
It is an internal optimization, not a new scheduler, public protocol or durable
queue. Earlier control regressions and release prerequisites remain open.

## Scope: history validation, not live dispatch

The only production entrypoint enabling a frontier is the definition-bound
history validator, `engine/history/validated.rs`. It creates a `HistoryPrefix`
bound by immutable Rust borrows to one exact definition, hash and event slice.
The full structural gate runs first, as before. Prefix state is still tentative
validation state; successful checks do not establish runtime ownership.

`ReadinessPrefix` lends the current projection, latest-attempt ordinals and
optional frontier together. It cannot be held across cursor mutation. The
frontier's definition reference must be the exact definition queried, not another
object with equal IDs or hash. Binding/status checks still precede fast answers.

Live `InMemoryDagEngine`/`DurableDagEngine` scheduling, append admission, standalone
projection queries, parallel selection and scoped iteration scheduling retain
the existing query paths. Their existing immutable topology reuse is unchanged.
No live frontier is enabled: this work does not establish accepted-snapshot
continuity across requests or store connections. Do not infer live dispatch
speedups from the historical measurements.

In particular, live `ready_at` receives a projection, not an event-delta token
proving which accepted transitions led from a previously retained frontier to
that projection. Existing store freshness checks do not automatically certify
another layer's derived-state continuity. Equal sequence/count/tail fields would
not supply that missing proof. No such live cache is inferred here.

## Admitted graph family

The frontier accepts ordinary acyclic `runtime.script` graphs with always edges,
known endpoint/entry IDs and unique node IDs. It retains source-vector order,
including when that differs from topological, event or entry-list order.
Duplicate edges are counted individually. It defensively rejects cycles and
malformed endpoints; these are optimization decisions, not replacement model
validation errors. Definitions still require their existing loader validation.

Any non-script handler, remote placement, lifecycle, runtime policy, reusable
definition, communication requirement, graph choice/fork, scoped visibility,
settlement, loop, wait, lock or saga descriptor selects fallback. So do branch,
parallel or approval edge bindings, non-always edges, reserved choice-edge IDs
and unknown future model versions. Empty descriptor arrays are conservatively
unsupported rather than treated as absence. Module/runtime binding metadata
alone does not widen readiness or managed-service authority.

Unsupported models permanently disable the frontier for that validation cursor.
Future event variants are invalidating by default; no feature is admitted only
because its fields happen to look harmless.

## State and algorithm

For source node ordinal `v`, the frontier stores active/completed flags, outgoing
ordinals, an unfinished-predecessor count, local attempt eligibility and at most
one retry deadline. An ordered set contains structural candidates, not durable
execution permissions.

```text
remaining[v] = number of edges u → v where active[u] and not complete[u]

candidate[v] = active[v] and remaining[v] == 0 and not complete[v]
               and (no attempt, or latest failed with a pending retry)

ready(v, now) = candidate[v] and (no retry deadline, or deadline <= now)
```

Completion for this admitted family means that the exact projection contains
the node's successful output, matching the original ordinary-script predicate.
Starts and successes cannot remain eligible. Latest attempt means vector order,
not the greatest attempt number. The authoritative projection fold still checks
attempt identity, retry adjacency, monotonicity, operation settlement and all
other structural rules.

Initial preparation checks acyclicity with Kahn's algorithm, traverses reachability
from entries, and builds counts from the current prefix. After a new output,
only that node and its outgoing dependents change. Each duplicate outgoing edge
decrements its own counted dependency. Source-ordinal ordering uses `BTreeSet`;
hash iteration never picks the next node.

Topology preparation is `O(V + E)` and latest-attempt bootstrap is `O(A)`.
Including initial output/retry map lookups and ordered-set insertion gives a
conservative `O(E + V log V + A)` initialization bound for the admitted family
(treating bounded IDs as unit keys). An ordinary local update uses ID lookup,
existing output/retry lookups and ordered-set operations, visiting only that
node and, on completion, its outgoing edges: conservatively
`O((outdegree + 1) log V)`. Full-list output still
visits structural candidates, including retries not yet due; membership uses
ID lookup and ordered-set membership. This is not an O(1) full-ready query or
linear end-to-end workflow execution claim. Structural folding, payload/context
copying, definition checks and final suffix verification remain.

## Exhaustive event/time policy

The implementation uses a closed allowlist; every event is either in these
supported cases or invalidating. Scoped envelopes invalidate before payload
classification.

| Accepted structural transition | Frontier treatment |
| --- | --- |
| `RunStarted` | No prepared state yet; initialize lazily at a bound running query |
| `StepAttemptStarted` | Refresh the node using the newly updated latest ordinal; remove in-flight work from candidates |
| `StepAttemptSucceeded` | Observe the new output, update outgoing predecessor counts, then refresh local eligibility |
| `StepAttemptFailed` | Refresh current attempt state; failure alone does not authorize a retry |
| `StepRetryScheduled` | Refresh that node's current pending deadline; no stale per-attempt deadline entries accumulate |
| Any other event, including admission, execution claim, managed operation, policy evidence, controls, cancellation, timeout, correction, outcome or terminal event | Discard frontier and permanently use the existing query path for this cursor |
| Any iteration-scoped event | Same invalidation; never merge root/iteration candidates |
| Failed structural apply | Discard frontier, latest map and tentative projection; preserve the cursor's first error |
| Unchanged prefix at a different clock | Read the stored current deadline against the supplied clock again; no cached due/not-due result |

Retry insertion/consumption follows the actual successfully folded projection.
Illegal schedule replacement still fails the existing structural gate. A future
supported replacement would need an explicit reviewed transition, not a clock
heap entry assumed valid from an old sequence. Forward and backward clock moves,
including exact nanosecond deadline boundaries, use the same comparison as the
full query. Existing callers retain their event-time or wall-clock behavior.

Every delta is processed after successful structural apply **and** after the
cursor's latest-attempt ordinal update. Skipped validator match arms are not
skipped cursor events. No event handler publishes derived state after failure.
Unexpected output removal or an unknown changed node also invalidates.

## Lifetime, authority and failure boundaries

The frontier borrows the immutable definition; its vectors own ordinals/flags,
not references into the movable projection. It is local to one cursor and has
`O(V + E)` additional heap, with one optional deadline per node. No mutex, await,
process-global key, per-terminal-run entry or persistent index is added.

Unsupported/terminal transitions discard the frontier. Invalid bounds reject
without rewinding it; rejected applies poison the cursor. Validation return,
error or drop releases all cursor-derived state. A new validation, replacement
store, external commit or reopened connection gets a new cursor, never a
sequence/count-based eligibility cache. Rust allocation checks additionally
verify zero retained requested heap after repeated complete validations.

SQL sequence allocation, claims, capacity reacquisition, cancellation, lease
expiry/renewal, fencing, precommit revalidation and commit-before-cache publication
are unchanged. A ready candidate still cannot bypass those gates. The full
validated-history witness retains the final fold state, not the last frontier
prefix. A transaction rollback publishes neither tentative projection nor
frontier. The ordinary query remains available rather than swallowing an error
or silently changing execution placement.

## Verification and CI

The independent oracle remains the original full scan plus full replay. New
differential tests cover ordered lists, first selection, membership and binding
errors at every prefix and repeated query, including earlier/later clocks and
exact retry deadlines. Sixteen seeded valid DAGs vary fan-in/out and source order;
their histories are generated using the independent oracle, not the frontier.
They start ready siblings before settling them in reverse order. Mutated input
must keep the reference's exact acceptance/error result.

Additional checks exercise late bootstrap, different runs and equal-length
histories with different in-flight siblings, foreign definition objects,
malformed direct queries, disconnected nodes, failed candidates, bounds errors,
terminal invalidation and real fallback selection for unsupported features.
Existing recorded histories cover choices, approvals, parallel work, forks,
loops, locks, waits, hooks, sagas, compensation and settlement through fallback.

The deterministic 16-start chain retains one borrowed graph preparation and
15 new latest-attempt visits. Historical queries perform **zero** repeated full
reachability visits or candidate-predicate scans, versus 256/16 at ISQ3. The
frontier prepares 16 node states, performs 30 subsequent local refreshes and
updates 15 outgoing edges. Preparation is real work, not omitted from the
measurement; the node-state count is not a hardware instruction count.

```bash
bun run --cwd woml-cli test:incremental-scheduling
bun run --cwd woml-cli test:incremental-scheduling-safety
```

The full safety command composes existing validated-state fault/authority tests
with DAG, retry composition, nested workflow calls, for-each runtime, fork joins,
capability authority and source/modularization guards. It runs Cargo serially on
ordinary test stacks; existing feature-specific stack choices are not raised.
It performs no benchmark, implicit native build, installation or publication.
CI runs its constituent selections serially in the existing Linux/macOS/Windows
combined matrix without duplicating the earlier startup/validated-state steps.
Static CI wiring is not evidence that those external platform jobs have passed.

The local Linux safety checkpoint passed:

- Final optimized artifact: **203 engine unit tests**, with six explicit
  capture-only probes ignored. The earlier debug checkpoint passed 200 before
  the last three added regressions. Filtered unit selections overlap these.
- **217 integration tests across 25 entrypoints**: 157 in the existing 19-file
  validated-state safety selection and 60 in the six composition/authority
  additions. These include real Bun script-host, nested-call and control-flow
  executions, not just source inspection.
- **10 ISQ**, **14 VSR**, and **34 source/architecture/CI** contract tests,
  TypeScript checking and the established Clippy policy. No new lint exemption
  or compatibility-repair bypass was needed; historical snapshots are unchanged.
- Rust formatting, clean diff checks and the changed records' local link
  targets were checked separately, not claimed as the complete release gate.

## Performance and release scope

An optimized ISQ2/ISQ3 baseline was retained before production edits at commit
`0610412`, using the existing unchanged reuse probe:
`validated-state-build-6iFhn4/receipt.json`, under
`woml-cli/.woml/performance/` (ignored local evidence).
The frozen ISQ0 contract and the earlier evidence records are not rewritten.

The existing reuse harness can compare that receipt with an explicitly built
frontier candidate, retaining seven alternating pairs and separate requested
allocation/peak captures. Its live-query/constructor/clone controls remain live
queries, not frontier callers; changes there require investigation, not credit
as frontier speedups. Chains alone do not certify wide-graph or fallback timings.

The completed local capture is `isq-reuse-3rZXev`; its candidate receipt is
`validated-state-build-LkBOhK/receipt.json`. Capture used the existing seven-pair,
one-warmup/three-measured-samples protocol, with resources in separate processes
and all raw samples/outliers retained. No capture builds or retries an artifact.

```text
Executable SHA-256:
ISQ2/3 baseline: 6376d204c1aaabb64ddb37694d7d15bfef4e48b7b5d65a34a11091d0f98af084
ISQ4 candidate:  ff3d17f9671dcdb9cc180ddcd5de3a3e5619ea1443eb600f47b308d7e3fa8265

Source-manifest SHA-256:
Baseline: b8b4c3dbc3de3aef8ed45008878141c2042a42a9fbd6beb719d2971c9c4ba081
Candidate: 8dab1f490b1adca2c8e4d7c3c3ff9da8ec8afc840b608bd4f9e032201347ce94
```

History-validation thread CPU milliseconds, medians of seven process medians;
allocation bytes are separate scoped `System`-allocator requested Rust bytes:

| Nodes | CPU baseline → candidate | CPU improvement | Allocated bytes baseline → candidate |
| ---: | ---: | ---: | ---: |
| 1 | 0.006137 → 0.007622 | −24.20% | 2,943 → 3,150 |
| 4 | 0.028417 → 0.028175 | 0.85% | 16,434 → 16,094 |
| 16 | 0.162762 → 0.123294 | 24.25% | 114,729 → 99,441 |
| 64 | 1.238664 → 0.550199 | 55.58% | 690,971 → 418,515 |
| 256 | 17.343363 → 3.750782 | 78.37% | 6,139,310 → 1,698,598 |
| 1,024 | 286.393258 → 41.217621 | 85.61% | 78,099,527 → 6,826,175 |

The largest fixture improves in **all seven pairs** (81.60–86.67%); wall-clock
medians similarly improve 286.431 → 41.216 ms. Its requested allocation drops
91.26%. This is validation of recorded synthetic attempts, **not execution of
1,024 scripts**, and not a fresh installed-runtime throughput measurement.
Do not add percentages to earlier checkpoints measured in different cohorts.

These wins do not close acceptance. New findings crossing the unchanged 5%
investigation trigger are retained explicitly:

- **One-node history:** CPU +24.20%, wall +26.37%, allocated bytes +7.03%.
  All seven CPU pairs regress. Its single query cannot amortize a frontier's
  preparation; extra requested allocation is observed, not a free optimization.
- **One-node construction:** wall +5.41% (CPU +4.65%).
- **One-node definition clone:** CPU +6.39%, wall +6.53%.
- **64-node warm live query:** CPU +7.26%, wall +7.27%.
- **64-node definition clone:** CPU +8.07%, wall +8.06%.
- **256-node construction:** CPU +5.12%, wall +5.12%.
- **256-node first live query:** CPU +8.82%, wall +8.88%.

Those live controls do not use the frontier and their source algorithm is
unchanged. The protocol measures them after preceding history work, and these
are different optimized unit-test executables with test-only observer/counter
branches and added tests. Code generation/layout, preceding workload and host
effects have not been isolated. These are **unresolved control regressions**,
not evidence of a verified cause or permission to dismiss them as noise. A
separate isolated pre-history control experiment remains relevant. No threshold
was raised and no sample, graph size or slow control was removed to pass.

At 1,024 nodes, scoped peak live requested bytes rise from 3,784,931 to 3,864,912
(+79,981 bytes, below the 16 MiB absolute investigation trigger). Every measured
validation returns zero net live bytes. First-live-query allocation/retention
values are identical at every size, and the engine object remains 992 bytes:
there is no new live/run cache. Physical RSS, native/SQLite allocation and
transient realloc overlap are not established by the scoped Rust observer;
the harness retains Bun process-resource fields with unverified units explicitly
labeled. Added test cases can affect executable layout; these are local exact
source/binary receipts, not signed independent or production-native attestation.

The large-history work reduction justifies the bounded cursor implementation,
not blanket independent performance acceptance. Single-query/tiny-history
fallback policy and the control findings above remain review/investigation work
before release. No post-hoc size threshold was introduced to hide their cost.

ISQ6 still owns final broad performance, fresh optimized native/installed journeys
and actual platform acceptance. Earlier small-control regressions, the
prepared-runtime pin-file lock hazard and failed installed inspection remain
open. There is no claim here of resolved physical-RSS attribution, new installed
performance, complete feature acceleration or readiness of the shared branch
for release. No fixture, public schema/export, dependency or product budget is
changed.
