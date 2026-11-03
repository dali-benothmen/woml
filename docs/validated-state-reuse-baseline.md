# Validated execution state reuse: baseline and verification record

Reference source: `d9338480f1888808ddcc9d349d97ff0f29e77b40` on
`perf/runtime-performance`, after startup separation. Local Linux checkpoint,
2026-11-03. This proposal has **not changed production execution behavior**.

## Phase status

- **VSR1:** private protocol and test-only reference implemented; local history
  tests pass. This is not a shipped validated-result API.
- **VSR0:** caller inventory/acceptance frozen; seven-process optimized baseline
  and independent attribution trace captured. Full completion requires allocator/peak-live evidence and
  the post-startup native one-shot/activated controls. Do not mark those passed
  from the engine microbenchmark.
- **VSR2–VSR5:** not implemented. No production fold has been removed.

See the [inventory and capture protocol](validated-state-reuse.md) and
[private result specification](protocols/validated-history-internal-v1.md).

## Structural baseline

The current miss path discards a full validator fold, performs definition/
prefix checks, then builds `ProjectionFoldState` again for the same events.

| Synthetic active history | Full passes / visits | Monotone prefix visits | Requests | Public projection clones |
| --- | --- | --- | --- | --- |
| 128 events | 2 / 256 | 127 | 64 | 1 |
| 512 events | 2 / 1,024 | 511 | 256 | 1 |
| 2,048 events | 2 / 4,096 | 2,047 | 1,024 | 1 |
| Ordinary hit, any length | 0 / 0 | 0 | 0 | 1 |

Full-pass/clone counts above are source-derived, not allocator instrumentation.
Prefix/hit/miss attribution is independently inspectable in the isolated trace.
They do not count loader envelope validation, graph queries, model validation,
SQL, all transition-owned copies, or hydration. A missing span for the direct
second `from_events` must not be mistaken for its absence.

## Local checks

- All **14** history tests passed: six existing tests plus eight new
  checked-result reference tests. Recorded scoped/lifecycle histories and the
  completed retry variant are included; historical rejected fixtures remain
  rejected. Every relevant prefix compares exact outcomes/errors.
- The initial development fixture check exercised all three generated sizes
  through a real durable miss and normal hits: passed. It took 117 seconds in
  debug, so the ordinary test now checks 128 events; the explicit ignored
  optimized capture still validates/exercises all three sizes.
- All **7** portable baseline-contract tests and TypeScript checking passed.
- All **11** durable boundary tests passed in the optimized binary, covering
  rollback, cache publication, competing openers and schema/opening boundaries;
  the explicit capture test was intentionally ignored in this correctness run.
- All **33** established modularization/source/CI contract tests passed.
- Rust formatting, documentation links/public examples and diff checks passed.

These are selected checks, not the complete engine, package or release suite.
No macOS/Windows execution has been performed locally.

## Optimized baseline results

Seven independent processes completed successfully. Each size has 35 measured
misses and 175 measured hits, plus retained warmups. Statistic below is the
median of seven process medians, not a pooled p95. Units are milliseconds.

| Events / nodes | Miss CPU | Miss wall | Hit CPU | Separate fold CPU |
| --- | ---: | ---: | ---: | ---: |
| 128 / 64 | 5.057 | 5.055 | 0.063 | 0.165 |
| 512 / 256 | 138.029 | 138.654 | 0.191 | 0.924 |
| 2,048 / 1,024 | 9,304.320 | 9,304.748 | 0.668 | 8.535 |

Process-median miss CPU ranges were 4.694–5.578, 115.773–145.095 and
8,224.285–9,717.509 ms respectively. These are large sequential **graph stress
cases**, not ordinary small-workflow latency promises or fixed-size graphs
with many managed operations. The separate fold measures construction; its
destruction is outside that sample. It is attribution, not an exact prediction
of the time an eventual optimization would save.

A separate traced process verified **21 natural misses and 105 hits**, including
warmups. Each miss had one profiled `fold_events` span plus the source-verified
unprofiled second `from_events`. The 2,048-event definition/prefix-validation
region had median 9,472.877 ms inside a median 9,504.289 ms miss. Prefix counts
matched the structural table exactly. Trace timings are not mixed with the
untraced cohort and do not isolate individual graph functions inside that region.

**Decision:** this cohort does not justify the frozen 10% primary CPU-improvement
target for removing a single fold. Do not proceed to VSR2 on an assumption of a
large speedup. Review the definition/readiness work and whether a separately
approved, representative fixed-graph/long-history cohort warrants reuse.
Do not silently replace the frozen cohort after seeing this result. This finding
does not prove reuse is useless for every history shape.

### Provenance

- Optimized engine unit-test executable SHA-256:
  `27e22c10fa3a8a17f22adbf59e8aa03f52879a80969737b286a261366e527531`.
- Captured source-file manifest SHA-256 (includes test harness/fixtures):
  `2e703e2aae828c3ed0e74e9a36ea8d3268daabb994b0256d2b8153cbd65625b6`.
- Tracked core/compiler/host patch SHA-256:
  `ec3be4c4d8fa8ea06330bb509370511cae2ec9994e199fc8cf5fe2c13cb57a8b`.
  Untracked test files are additionally bound by the source-file manifest;
  that tracked-only patch is not their complete source archive.
- Compiler: rustc 1.88.0, Cargo 1.88.0; Bun 1.3.14; Linux x64;
  Intel Core i7-9750H. At least 9.49 GiB available RAM before each process.
- Raw build/capture/trace directories, relative to `woml-cli/.woml/performance/`:
  `validated-state-build-zAo0Cy`, `validated-state-capture-UUnDPL`,
  `validated-state-trace-LLXyCF`. All are intentionally ignored generated data.
  The original harness is archived in the build directory and its hash matches
  the captured manifest; it is a review copy, not a relocated runnable entrypoint.
- Generated fixture SHA-256 identities for 128/512/2,048 events:
  `4eb448e0f267f3c8a2834985ae6a631518d6d53a0a4c02988682e87b71227923`,
  `58827c5a62603fabd2d67f8b81c37cab6c763dfe21e3e40b192405171d3f949e`,
  `9a53b75baf11239880d11c6c0a30d5a6d5ba0f24cb00a5667be2440a94484c06`.

## Measurement limitations and remaining gate

The capture retains exact build/source/fixture identities and raw process
samples under ignored `woml-cli/.woml/performance/` paths. It records thread CPU
and wall time for the actual scoped call. The original subprocess resource
records contained `{}` because Bun's getters are non-enumerable; **that cohort
has no valid process-memory result**. Raw data was not rewritten.

The reporter now materializes getters and preserves actual BigInt CPU fields.
Actual Bun 1.3.14 RSS values also differ from the installed typings' implied byte
unit, so the reporter conservatively labels RSS as raw/unverified, not bytes.
Regression tests cover both issues. A reporting-only refreshed receipt at
`validated-state-build-af4r6C` builds the **same executable SHA-256**; it does
not backfill the missing earlier measurements or certify a byte-budget pass.
Future subprocess lifetime figures include setup and are not scoped allocations
or script-host-tree peaks.

Allocation totals and peak live bytes are explicitly **unmeasured (`null`)**.
No allocation profiler is currently installed in this environment. Reviewing
Rust ownership and counting explicit clones does not replace that evidence.
The result may retain a complete state and a prefix state simultaneously; that
peak must be measured for a later implementation.

The optimized unit-test binary does not certify a fresh N-API addon, installed
CLI behavior, actual one-shot/activated feature controls or performance on
other platforms. Those remain prerequisites for completing VSR0 and proceeding
to production reuse. Startup separation's earlier open acceptance is unchanged.
