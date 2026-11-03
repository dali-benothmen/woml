# Indexed readiness queries

This change follows the [readiness investigation](readiness-investigation.md).
It optimizes repeated graph and attempt searches without changing which work
is eligible, what a historical event must prove, or who may perform an effect.

On the paired optimized Linux chain capture, a 1,024-node/2,048-event history
check used **590 ms rather than 8.99 seconds of CPU**, about **15.2 times faster**.
Small queries do not universally improve: preparation adds microseconds in some
cases, as shown below. These are history-check timings, not complete workflow
execution times.

## What changed

- A private borrowed `ReadinessGraph` prepares node-ID, incoming-edge and
  outgoing-edge lookups from one exact compiled definition. Edge lists preserve
  source order. The index borrows nodes/edges, not copied scripts or context.
- History validation prepares that index at most once, lazily on its first
  readiness query, after the existing full structural fold. Empty histories and
  errors before a readiness query do not prepare a graph. The index is dropped
  with that validation call, not stored in SQLite or a cross-request cache.
- Standalone scheduler queries prepare their own graph. Terminal and unbound
  projections retain the original early status/binding gates before preparation.
- Each indexed readiness query builds a separate latest-attempt map from its
  exact borrowed projection. The last vector occurrence wins, matching the old
  reverse scan; it does not select the largest attempt number. That map cannot
  outlive the query or cursor advancement and never merges run/iteration scopes.
- Reachability and fork-ownership traversal use outgoing adjacency. Predecessor
  completion, choice joins and approval joins use incoming adjacency and indexed
  node lookup. Workflow-settlement failure checks use the latest-attempt map.
- Existing callers of the standalone `node_is_complete` predicate retain their
  scan behavior; they do not pay for a new index on every completion lookup.

Implementation starts in
[`readiness.rs`](../core/woml-engine/src/engine/readiness.rs),
[`readiness/index.rs`](../core/woml-engine/src/engine/readiness/index.rs), and
[`history.rs`](../core/woml-engine/src/engine/history.rs).

## What did not change

Indexes are lookup aids, not validation certificates or cached permission.
Definition/payload validation, structural folding, event ordering, historical
timestamps, prefix replay, context visibility and transactional authority remain
in place. Ready lists still follow the original node vector. Branch/approval
selection, retries, completion, cancellation, leases and ownership are evaluated
from current query inputs, not retained in the graph index.

There are no serialized schema, native-export, dependency or performance-budget
changes. This is not the deferred validated-history fold-reuse implementation,
a targeted historical-membership shortcut, or an incremental scheduling frontier.
The original VSR acceptance record remains unchanged.

## Why these lookups help

For a plain chain with `V` nodes, `E = V - 1` edges and `A` attempts, preparing
the definition costs expected `O(V + E)` time and auxiliary storage. One query
then builds latest-attempt lookup in expected `O(A)` and traverses outgoing
adjacency in `O(V + E)`, instead of inspecting all `E` edges for each reachable
node. Incoming/node/attempt lookups remove the other measured linear searches.
Hash-map operations have expected, not guaranteed worst-case, constant cost.

Across a history with one start per node, the named chain scan terms fall from
cubic to quadratic work. Full validation still checks all events and many
evolving prefixes. Feature-descriptor searches, recursive control-node
completion, branch predicates and other validation work remain. This is not an
`O(V + E)` claim for every WOML workflow or the entire runtime.

The index uses `O(V + E)` auxiliary graph storage plus `O(A)` temporary query
storage. These are cardinality bounds, not measured allocator bytes. There is
no process-global graph retention or speculative incremental cache publication.

## Correctness checks

The pre-index scan implementation is retained independently under `cfg(test)` in
[`reference.rs`](../core/woml-engine/src/engine/readiness/reference.rs).
It does not call the optimized graph or attempt indexes. History differential
checks can select the old scans while sharing the same definition checks.

Regressions compare ordered results and exact errors over every prefix of
recorded sequential, lifecycle, loop and combined histories, plus historical
branch, parallel, approval and retry fixtures. Existing corruption tests compare
structural/payload/graph rejection precedence. Additional predicate tests vary
choice and approval routes, completion subsets, joined/unjoined fork failure,
retry state, latest vector entry, source ordering and missing endpoints. These
synthetic predicate snapshots are not claimed as valid durable histories.

The Combined Developer Experience CI matrix runs the focused tests serially on
Linux, macOS and Windows. Timed captures remain explicit and ignored in normal
test runs; CI does not fail on noisy wall-clock ratios from this benchmark.

```bash
bun run --cwd woml-cli test:readiness-indexes
```

## Paired optimized capture

The [capture runner](../woml-cli/scripts/readiness-index-benchmark.ts) requires an
explicit optimized engine unit-test build receipt. It never builds during a
capture and verifies source, binary, compiler, host and harness identities before
and after each process. Raw outputs and failures are retained in ignored
`woml-cli/.woml/performance/readiness-indexes-*` directories.

```bash
bun run --cwd woml-cli build:validated-state-reuse-baseline
bun run --cwd woml-cli capture:readiness-indexes --receipt /absolute/path/receipt.json
```

Protocol: three independent serial processes; chain sizes 1, 4, 16, 64, 256 and
1,024; one warmup and three measured pairs per size. A successful validation of
both paths precedes timing. Pair order alternates. Full-history comparisons use
the same monotone prefix cursor on both paths, isolating indexing from prefix
replay changes. Standalone query measurements include graph/attempt preparation;
graph construction is also timed separately. Thread CPU is unavailable rather
than fabricated on platforms without the supplied clock implementation.

This synthetic in-memory cohort is not an installed CLI/native-addon journey,
SQLite cache-miss measurement, allocation study or replacement VSR release gate.
It does not predict application-wide latency or close the separate startup and
native-lifecycle acceptance work.

## Local optimized results

Captured 2026-11-03 on Linux x64, Intel Core i7-9750H, rustc/Cargo 1.88.0,
Bun 1.3.14. Both paths ran in the same fresh standard release unit-test binary
(`opt-level = "z"`, LTO). The scan reference uses the original query algorithm
inside the same history validator and monotone cursor. This is not a comparison
between independently installed releases. The host was a developer desktop,
not a dedicated benchmark runner.

Each table value is the median of three process medians, using three measured
pairs per process after warmup. Times are thread CPU milliseconds, not p95s.

| Chain nodes | Events | Scan history validation | Indexed history validation | Ratio of medians |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 2 | 0.00533 | 0.00503 | 1.06x |
| 4 | 8 | 0.02638 | 0.02880 | 0.92x |
| 16 | 32 | 0.23869 | 0.21048 | 1.13x |
| 64 | 128 | 4.19296 | 2.32119 | 1.81x |
| 256 | 512 | 114.81296 | 34.66949 | 3.31x |
| 1,024 | 2,048 | 8,992.51501 | 589.99569 | 15.24x |

The 1,024-node process medians ranged from 8,566.812 to 9,247.862 ms for scans
and 582.612 to 590.510 ms for indexes. The ratio corresponds to about 93.4%
less CPU for that history-check cohort.

Standalone queries below use a halfway-completed chain and include construction
and teardown of graph/attempt lookup views, rather than assuming a free prepared
index. The separate construction timing returns the borrowed graph and drops it
after its timer; it is not an exclusive partition of the query sample.

| Nodes | Scan query | Indexed query including preparation | Separate graph construction |
| ---: | ---: | ---: | ---: |
| 1 | 0.00118 | 0.00118 | 0.00077 |
| 4 | 0.00261 | 0.00323 | 0.00127 |
| 16 | 0.01034 | 0.01614 | 0.00583 |
| 64 | 0.05405 | 0.05540 | 0.02220 |
| 256 | 0.46400 | 0.24160 | 0.08930 |
| 1,024 | 9.53950 | 1.02837 | 0.39544 |

The four-node history check is about 9.2% slower, an absolute difference of
2.4 microseconds. At 16 nodes, a standalone query adds about 5.8 microseconds,
although complete history validation benefits from reusing the definition
index. These results do not establish a universal no-regression claim. No
small-graph threshold, sample filtering or budget relaxation was introduced to
hide preparation costs.

### Capture provenance

- Source checkpoint: `973d753405d0ab29ba63c0bd5110804213855c01` plus the
  receipted implementation patch and new Rust files.
- Build directory: `woml-cli/.woml/performance/validated-state-build-9n72bM`.
- Raw paired capture: `woml-cli/.woml/performance/readiness-indexes-ci3C7r`.
- Executable SHA-256:
  `2ebb7d838f5e919cf11e385bfd60ebdfe303b80742413017c27d228d603072aa`.
- Source-file manifest SHA-256:
  `9fe5c017ffadda0f0460d625081021682adc4cb81c8c515edce421d1f74ee1ba`.
- Tracked source-patch SHA-256:
  `4e8ef5d4ba43b0c71992e33f206be58a67011c1ff3656a100ea8fc55090dbb69`.
- Capture harness SHA-256:
  `538d18b7b029219d692c697135a27af342ec2e5cfa6c2b4f4f7b85f8de46fd73`.

All three processes passed report validation and source/binary checks. Raw
samples include warmups, and none were removed. No builds or unrelated test
suites ran during the captures. Allocation totals and peak-live bytes remain
unmeasured; retained resource RSS uses the existing raw/unverified-unit format.

## Verification results

- All 162 engine unit tests pass in the exact optimized capture executable;
  three explicit capture tests are ignored in the ordinary suite.
- All 68 selected integration tests pass against the final production build:
  `approval_state`, `core_contract_paths`, `durable_wait_runtime`, `fork_state`,
  `locks_and_leases`, `retry_hardening` and `retry_state`.
- All 14 baseline/investigation/paired-report contract tests and all 33 existing
  modularization, source, architecture and CI guard tests pass.
- TypeScript checking, the established engine Clippy policy without new
  exemptions, Rust formatting, documentation/example validation and diff checks
  pass.

These are local Linux results. The added CI matrix coverage still needs actual
macOS/Windows execution; a fresh native-addon build and clean-package journey
are separate checks, not certified by the optimized engine test executable.
