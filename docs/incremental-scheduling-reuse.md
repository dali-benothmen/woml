# Scheduling-query reuse: immutable topology and prefix attempts

This records ISQ2 and ISQ3 on `perf/runtime-performance`, following the
[ISQ0/ISQ1 checkpoint](incremental-scheduling-queries.md). These are private
execution optimizations, not a new public model or persistence contract.

This page preserves the ISQ2/ISQ3 checkpoint. Later cursor-only frontier work
and its safety boundaries are documented [separately](incremental-scheduling-frontier.md).

## In plain language

A workflow's roads do not change while its engine owns the compiled definition.
Previously a live readiness query rebuilt the map of those roads each time.
The engine can now prepare that map once and reuse it. It still checks which
roads are open and which steps can start using the current run state and clock.

History validation also used to rebuild a list of the latest attempts each time
it advanced to another prefix. The prefix cursor now remembers each node's
latest position in its own attempt vector. A position is not a remembered
status: completion or failure is read from that exact current vector entry.

Neither change remembers permission to execute work. The coordinator's claims,
transactions, cancellation, retry deadlines, locks and fencing remain authority.

## Definition ownership audit

| Consumer | Preparation lifetime |
| --- | --- |
| In-memory engine constructors | One private owner containing the exact immutable definition and a lazy topology |
| Durable engine constructors | Same ownership, after the existing model validation and definition registration |
| Durable resume | Loads the exact stored definition and creates a fresh, unprepared owner |
| Live engine readiness and its append guards | Reuse that owner's topology, but obtain the projection anew |
| Standalone projection readiness | Retains the borrowed call-local topology; caller-owned definitions are not placed in a global cache |
| History validation | Retains its existing one borrowed graph per validation; this is not counted as a new topology gain |
| Synthetic/scoped graphs and recovery queries | Retain borrowed preparation; no retained index inferred from equal IDs or hashes |

`ReadinessDefinition` is crate-private. Its definition is not mutably exposed,
and its `OnceLock` cannot be paired with another definition. Keys and edge/node
ordinals are owned, not references into a movable Rust struct. No unsafe
self-reference, global cache, public export or definition serialization change
is needed. Moving the owner is safe; cloning the public compiled definition
does not clone or retain the preparation.

The preparation is lazy, after the existing non-running/binding gates. Terminal
queries still return before binding errors, and wrongly bound running queries
still reject before preparation. Source-node order, first-node lookup semantics,
duplicate edges and incoming/outgoing edge order are preserved. A full readiness
query still returns source-ordered IDs, not hash-map order.

This trades bounded per-definition retained memory for avoiding repeated
preparation. The map's size scales with the exact owned graph, not the number of
runs. Each engine prepares independently; dropping the owner releases its map.

## Prefix-local latest attempts

`HistoryPrefix::query_at` returns a private `ReadinessPrefix` whose immutable
borrows tie one projection to its own ordinal map. The cursor cannot advance
while that query view is borrowed. The view is not a durable-state witness.

The map is initialized lazily from the current root attempt vector. Thereafter,
each successful structural apply visits only newly appended root attempts.
For an existing attempt, its current status is read by ordinal, so in-place
success, failure or interruption cannot leave a cached status behind. The last
vector occurrence wins, including legacy inputs where its attempt number is
smaller than a previous entry's number.

All events between queries are still applied, even if the definition validator
skips their match arms. Nested iteration attempts stay in their own execution
vectors and cannot overwrite root lookup entries. Fork attempts retain the
projection's original identity model; no new flattened attempt identity or
cross-scope permission is introduced. The fold's exact `AttemptIdentity` index
remains a separate mechanism.

Every readiness consumer of the history cursor uses the paired view: step
starts, choices, forks, sagas, parallel starts, ordinary/rich approvals and wait
registration. Step starts and waits keep the event timestamp; the other callers
keep their existing wall-clock behavior. Active reachability, recursive
completion and retry eligibility are not cached.

Repeated prefixes are allowed; backwards/out-of-bounds requests keep the
existing rejection contract. A failed structural apply can have mutated
tentative fold state, so the cursor discards both that state and its map and
keeps the first error. A poisoned cursor cannot expose an earlier or later
snapshot. Another validation creates a new cursor; no index is retained across
transactions, connections, replacement histories or restarts.

The original full structural pass still precedes definition validation. Final
suffix folding and the validated-history witness path are unchanged.

## Deterministic work and correctness

For the existing 16-node/16-start chain:

| Work | ISQ1 | ISQ2/ISQ3 |
| --- | ---: | ---: |
| Historical candidate predicates | 16 | 16 |
| Historical ready-vector constructions | 0 | 0 |
| Topology builds in one history validation | 1 | 1 |
| Active-node visits | 256 | 256 |
| Latest-attempt vector entries visited | 120 | 15 |

The last start's new attempt is beyond the last queried prefix, hence 15 visits.
These are work counts, not step execution timings. Repeated live queries on one
owner build its topology once; a resumed owner builds its own once.

The independent reference remains the original full scan and full replay.
Checks cover every recorded/historical prefix, repeated queries, retry clocks
before/at/after eligibility, binding errors, duplicate IDs/edges, multiple
definitions with equal IDs/hashes, moving owners, concurrent preparation,
replacement projections, status changes, nested scopes, corruption and rejected
candidates. File-backed coverage checks external commits and reopening rather
than assuming that a prepared graph makes a projection fresh.

## Measurement protocol and limitations

The new `incremental-scheduling-reuse.ts` probe is a separate **post-ISQ1
attribution experiment**, not a rewrite of the frozen ISQ0 acceptance harness.
It does not close ISQ1's previously flagged controls.

Before production edits, an optimized unit-test baseline was built from
`c713e968979dc4b3fde52dc8c33aedb6c4136bfc` production code plus the explicit
test-only probe and disabled allocator observer. Its receipt is
`validated-state-build-O5V3S8/receipt.json`; the initial seven-process capture
is `isq-reuse-UGCCVQ`, under `woml-cli/.woml/performance/` (ignored local evidence).
Receipts include exact source-file and executable hashes, tracked source patch,
host and toolchain. They are local provenance, not signed independent
attestation; the baseline includes uncommitted test instrumentation.

Build explicitly, then compare the two retained executable receipts:

```bash
# Repository root: build and capture are separate; no implicit native build.
bun woml-cli/scripts/validated-state-baseline.ts build
bun woml-cli/scripts/incremental-scheduling-reuse.ts \
  --baseline <post-ISQ1-receipt.json> --candidate <ISQ2-ISQ3-receipt.json>
```

Seven independent processes per artifact alternate order by pair. Each graph
size (1, 4, 16, 64, 256, 1,024 nodes) has one warmup and three measured samples.
Summaries use medians of process medians, with batch timings divided by 20.
Fixture identity and current candidate source/executable hashes are checked;
raw samples and failures are retained, with no hidden retries.

The probe measures engine construction, first/warm readiness queries, compiled
definition cloning and complete history validation separately. Model cloning
for setup and fixture assembly are outside timers. The live fixture has half
its steps already recorded via structural append after independently validating
the exact full history. This is not execution of 1,024 JavaScript steps, a
durable-store benchmark or a warm long-lived native-runtime throughput claim.

Separate resource processes use a test-binary `System` allocator observer:
first-query net live requested Rust bytes measure retained topology, and full
history validation reports requested allocation totals and peak live bytes.
The observer forwards allocation ownership unchanged and is disabled outside
explicit synchronous thread-local scopes. It is absent from production/native
builds. Its disabled branches still affect test-artifact code generation on
both sides. Unit test additions can also change executable layout, so timing
differences are measured observations, not isolation of machine-code causes.

Requested Rust bytes are not physical RSS, SQLite/C allocation or realloc's
transient physical peak. Whole-process resource usage is retained separately,
with unavailable/unverified units explicitly labeled. None of these fields may
silently stand in for missing release resource evidence.

## Local results and tradeoffs

The completed paired capture is `isq-reuse-B9M9MR`; candidate receipt
`validated-state-build-1YGVUt/receipt.json`. Both are beneath the ignored local
performance directory. The exact executable SHA-256 values are:

```text
Post-ISQ1: 86c3ea8a4a43001466a7322add890f5cf15104babafb1b535339de61feb8724a
ISQ2/3:   6376d204c1aaabb64ddb37694d7d15bfef4e48b7b5d65a34a11091d0f98af084
```

The respective source-manifest hashes are
`928cdf8a356d3d6d16ecd6f84c95de51573a397c20dd6aacbcf0bae68ee4962e` and
`b8b4c3dbc3de3aef8ed45008878141c2042a42a9fbd6beb719d2971c9c4ba081`.
This pair measures both reuse changes together against post-ISQ1 production
code, not against the original ISQ0 baseline. Do not add its percentages to the
earlier ISQ1 percentages or treat it as independent per-feature CPU attribution.

Thread CPU milliseconds, medians of seven process medians; arrows are
baseline → candidate. Warm batches are normalized to one query:

| Nodes | First live query | Warm live query | Complete history validation | New retained topology bytes |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 0.002618 → 0.002532 | 0.001183 → 0.000974 | 0.006590 → 0.006469 | 153 |
| 4 | 0.006114 → 0.006753 | 0.004141 → 0.003398 | 0.027705 → 0.029991 | 946 |
| 16 | 0.023970 → 0.026069 | 0.018939 → 0.013142 | 0.168039 → 0.166672 | 5,447 |
| 64 | 0.092421 → 0.096321 | 0.085074 → 0.060329 | 1.384791 → 1.325699 | 21,959 |
| 256 | 0.422872 → 0.440882 | 0.384074 → 0.276429 | 19.973449 → 18.682291 | 88,474 |
| 1,024 | 1.912733 → 1.860271 | 1.637902 → 1.142576 | 336.716300 → 297.091460 | 355,041 |

Warm-query CPU improves 17.73–30.61%; every paired process improves for every
size. The 1,024-node history CPU improves 11.77%, with six of seven pairs
improving; the remaining pair regresses 1.28% and is retained. Wall-clock
history medians are 336.722 → 297.101 ms, a similar 11.77% reduction.

First-use costs must not be hidden by those wins. Four-node first-use CPU
regresses 10.45% and 16-node first-use CPU regresses 8.76%; all seven pairs are
slower in both cases. The new owner must clone index keys and retain ordinal
maps instead of dropping borrowed preparation after the query. Requested
first-query allocations increase 4.40–8.60% across sizes; sizes other than four
cross the 5% allocation investigation trigger. This is a measured retention
tradeoff, not a free cache or an automatically accepted resource increase.
For these two small fixtures the measured extra first-use CPU (0.639/2.099 µs)
is smaller than one subsequent query's median saving (0.743/5.797 µs), suggesting
amortization by the second query. That calculation is not an end-to-end runtime
guarantee or justification for discarding the first-use flags.

Four-node history CPU also regresses **8.25%** (all seven pairs slower), with
wall time regressing 8.40%. Four-node construction wall time crosses the 5%
trigger at 8.206 → 8.628 µs (+5.14%); its CPU delta is +4.63%. Other construction
and compiled-definition-clone median regressions remain below 5%. No topology
allocation occurs during construction, but the engine object grows from
840 to 992 bytes. The small construction/history differences have not been
isolated from code-generation/layout and observer effects; no causal fix or
independent performance acceptance is claimed. In particular the four-node
history slowdown must be investigated, not described as proven host noise.

History allocation/peak bytes in the separate resource capture:

| Nodes | Requested allocation baseline → candidate | Peak live requested bytes baseline → candidate |
| ---: | ---: | ---: |
| 1 | 2,943 → 2,943 | 2,781 → 2,781 |
| 4 | 16,619 → 16,434 | 11,990 → 12,137 |
| 16 | 121,081 → 114,729 | 57,839 → 58,063 |
| 64 | 844,811 → 690,971 | 233,914 → 234,810 |
| 256 | 8,809,283 → 6,139,310 | 940,304 → 944,043 |
| 1,024 | 121,578,629 → 78,099,527 | 3,769,649 → 3,784,931 |

The largest fixture allocates 35.76% fewer requested Rust bytes during history
validation. Its peak increases 15,282 bytes (0.41%), and the live topology retains
355,041 bytes per owner (about 347 KiB). These scoped values are below the 16 MiB
absolute investigation trigger, but do not establish the unmeasured physical
RSS or complex-feature memory acceptance. Validation leaves zero net live bytes
in both artifacts. Drop/clone tests separately verify that the owned topology
is released and is not carried by a cloned compiled definition.

Verification at this checkpoint:

- Final exact optimized engine artifact: **194 passed**, six explicitly ignored
  capture-only probes; no failures. Earlier debug unit checkpoint: 193 passed
  before the additional file-backed test.
- **94 passed** in 12 neighboring integration entrypoints: DAG, retry state and
  hardening, approval state, forks, parallel events, choices, for-each model and
  recovery, fork recovery, policy store and durable waits.
- **10 passed** scheduling/capture contracts plus TypeScript checking;
  **34 passed** modularization/source/architecture/CI contracts.
- Established Clippy policy, Rust formatting, diff checks and the changed
  documents' 22 local link targets passed. This is not the full documentation,
  native/package or cross-platform release gate.

## Release status

No frontier is implemented here; membership still scans source IDs, reachability
still traverses the graph, and complete structural/history checks remain.
ISQ4–ISQ6 are separate implementation, fault-proof and release-evidence phases.

ISQ2/ISQ3 implementation and local safety/work-count evidence are complete;
independent performance acceptance remains open on the small/control and
first-use allocation findings above, as well as the original acceptance scope.

The original ISQ1 control regressions and the
[SRS/VSR verification blockers](validated-state-reuse-verification.md), including
the prepared-runtime pin-file lock lifecycle and failed installed repeated-run
inspection, remain open. New optimized native/clean-package execution and actual
macOS/Windows results are not certified by local Linux unit-artifact tests.
No public format, native export, dependency, frozen fixture or product budget
is changed. `releaseAccepted` remains `false`.
