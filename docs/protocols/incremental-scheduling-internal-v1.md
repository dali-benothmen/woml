# Incremental scheduling queries: private contract v1

This is an internal implementation/testing contract, not a new public model,
event, store or native protocol. Work continues on `perf/runtime-performance`.

## Query inventory and authority

| Consumer | Question and clock | Initial treatment |
| --- | --- | --- |
| `engine/history.rs`, historical `StepAttemptStarted` | Is this node ready in the prefix before the event, at `event.occurred_at`? | ISQ1 membership query only |
| Other history checks: choices, forks, parallel-group starts | Membership obtained from existing full queries, with existing wall-clock behavior | Unchanged |
| `InMemoryDagEngine` / `DurableDagEngine` append guards | Full readiness and existing error/transaction sequence | Unchanged |
| Runtime main loop and forks | Full ordered list; source-first selection | Unchanged |
| Parallel group child selection | Ordered group children, current projection and retry clock | Separate algorithm; unchanged |
| For-each body readiness | Scoped body graph, completed set and selections | Separate algorithm; unchanged |
| Policy admission, capacity, claims, lease renewal | Authoritative store transactions, not DAG readiness | Out of scope |

Topology and latest-attempt indexes already exist. ISQ0/ISQ1 do not introduce
an event frontier, persist an index, change prefix replay or lengthen cache
lifetimes. Later phases have their own proof obligations.

## Membership equivalence

For the exact same definition `D`, hash `H`, projection `P`, clock `t` and ID `x`:

```text
membership(D,H,P,t,x) = full_ready(D,H,P,t).map(ids => ids.contains(x))
```

This includes errors, non-running status before binding validation, legacy
workflow-ID enrichment rules, unknown IDs and malformed direct graph inputs.
Duplicate IDs require any matching source node to pass the predicate; using
only the first node-ID lookup would not preserve full-list membership.

Both APIs share the candidate predicate, not their iteration strategy. Full
queries retain source-vector ordering. Membership must not evaluate unrelated
candidates or construct a ready-result vector. Active reachability and latest
attempt lookup still come from the exact projection on each query. They are
not cached eligibility or proof of authority.

ISQ2/ISQ3 extend preparation ownership, not eligibility: live engines own a lazy
immutable ordinal topology, and monotone history cursors own latest-attempt
ordinals. Each prefix view borrows the exact projection/map pair until the next
advance. Status reads come from the current vector entry; failed applies discard
the view/state. Standalone and scoped definitions retain borrowed topology.
See the [reuse record](../incremental-scheduling-reuse.md) for the ownership audit,
resource tradeoffs and separate post-ISQ1 attribution protocol. The initial
inventory above describes ISQ1; query questions and clocks remain unchanged.

ISQ4 adds only a definition-bound **history cursor** frontier for ordinary
always-edge script DAGs. Structural apply and latest-ordinal refresh precede
each derived update. Source-ordered structural candidates retain current retry
deadlines, not cached clock eligibility. A closed event allowlist invalidates
on every scoped/control/authority/terminal/future event. Live adapters retain
full readiness; the frontier is neither an accepted-state witness nor permission
to dispatch. The [frontier safety record](../incremental-scheduling-frontier.md)
defines the complete scope, event/time mapping and ISQ5 verification selections.

Preserve latest **vector** occurrence, in-flight/succeeded blocking, due pending
retry requirements, active-edge predecessors, recursive completion, settlement
failure ownership, context scope and exact existing error precedence. Historical
validation still runs the complete structural gate first. The new query cannot
validate a later append, establish ownership or bypass a transaction.

## Measurement protocol frozen before production changes

The [acceptance contract](../incremental-scheduling-acceptance.v1.json) freezes
workloads, sampling, the final workstream target and resource investigation
triggers. A trigger requires investigation; it is not permission to increase
product budgets. ISQ1's narrower correctness/work-count exit is distinct from
the complete workstream's performance/release acceptance.

Build the optimized engine test executable explicitly before capture. Preserve
the build receipt, exact source manifest/patch and binary hash. The initial
baseline has current production code plus test-only capture/disabled counters.
Keep an immutable copy before implementing membership. Capture never builds,
installs, selects native addons or retries failed samples.

Use seven independent processes per artifact, alternating baseline/candidate
order by pair; one warmup and three measured samples for every frozen graph
size. A sample records real complete history validation, membership queries,
unchanged full readiness including graph preparation, and topology preparation.
Query batches have 20 repetitions and report per-query time. Fixture creation,
correctness assertions, hashing and JSON output stay outside timers. All raw
samples, failures and outliers are retained. Summaries are medians of process
medians, not pooled best cases. CPU unavailable on a platform remains null.

Thread-local work counts are enabled only in explicit untimed tests. Counters
are absent from non-test builds; disabled test-counter branches remain in the
optimized test executable on both sides. These are unit-test artifact timings,
not production native-addon or workflow execution measurements. Keep counters
and traces disabled during capture.

Initial captures cover chains; ISQ0 inventories the broader cohorts in the
implementation plan, and ISQ6 must measure wide/fan-in graphs, retries,
retained contexts and complex-feature fallback before release. The initial
frontier scope in the acceptance contract is for future ISQ4, not an execution
restriction added by ISQ1.

Allocation totals and scoped peak-live bytes need a separate resource capture;
unmeasured fields are null and block full release acceptance. Linux GNU time
maximum RSS, when available, is whole-process KiB converted explicitly to bytes,
not scoped Rust heap. Existing SRS/VSR blockers remain open, including the
prepared-runtime file-lock lifecycle and cache-hit control investigation.
