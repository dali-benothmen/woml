# Invocation setup: baseline and feasibility decision

ISR0/ISR1 investigation on `perf/runtime-performance`, captured on
2026-11-03 UTC and documented across 2026-11-03/04. Production checkpoint:
`3064a2b5092b1b24da328f9843965139d9079407`.

**Outcome: retain the existing fresh-worker design. No production optimization
was made.** The baseline and a scoped go/no-go investigation are complete;
current evidence does not justify compilation caching, used-worker pooling or
another source-map preparation cache. ISR2/ISR3 are deferred rather than
implementing an unproven optimization. Bootstrap sub-attribution and actual
managed/native settlement performance remain open, not inferred from totals.

## What was frozen

The [ISR acceptance contract](invocation-setup-acceptance.v1.json) was written
before capture and before any production optimization. Seven independent
measurement processes each run eight cohorts on separately started built hosts.
Each cohort records host readiness, registration, one first invocation, one
declared warmup batch and three measured batches, then drains/exits the host.

The primary is 32 sequential invocations, alternating inline scripts and one
repeated module. Controls have eight invocations per batch. Every invocation
checks for leaked worker-global state and must return exactly `1`; module calls
increment a module-local counter whose fresh-worker value must also be `1`.
The module is imported/evaluated in the worker, not replayed from recorded facts.

There were **2,520 real invocations** in the untraced baseline, counting first
requests and declared warmups, with 56 separately started hosts. All completed.
No measured failure, outlier or slow sample was retried/discarded. The earlier
five-invocation smoke check is separate and not a timing sample.

The existing host cache and outer process pooling are not new improvements.
This harness explicitly starts hosts to separate cold and warm costs; it does
not benchmark Rust pool acquisition or claim a durable workflow speedup.

### Provenance

Bun 1.3.14, Linux x64, Intel i7-9750H, kernel `6.8.0-138-generic`.
The explicit build ran the existing JavaScript build script, preserved built
host/worker files and maps, and recorded source, harness, fixture, Bun executable
and artifact hashes. There was **no native build, addon load or SQLite store**.
The preserved host is 239,319 bytes and worker entry is 57,513 bytes. File size
alone does not attribute parse/bootstrap cost or justify splitting that entry.

| Artifact | SHA-256 |
| --- | --- |
| Built host | `d81ca54095ff5d040c6ff2dd96ba0f18b43f45b56f57934d1271efac13e15cbf` |
| Built worker | `af632902e850fdba61b7e40dd3c66e3cb21f32d797698e9abd788e83f6e83f08` |
| Production source manifest | `8dab1f490b1adca2c8e4d7c3c3ff9da8ec8afc840b608bd4f9e032201347ce94` |

Retained under ignored `woml-cli/.woml/performance/`:

- Build/archive: `invocation-setup-build-PysNNm/receipt.json`.
- Initial harness smoke: `invocation-setup-smoke-qnT4Rm`.
- Uninstrumented baseline: `invocation-setup-timing-R2Ia0E`.
- Separate profiler capture: `invocation-setup-trace-Xicocy`.
- Separate feasibility/component probe: `invocation-setup-probe-0OgzqY`.
- Offline diagnosis: `invocation-setup-timing-R2Ia0E/diagnosis.json`.

Receipts are local provenance, not independent signed attestation. Captures
check their source/harness/runtime/artifact identities and available headroom.
Raw results, per-process logs and checkpoints survive a later failure. The
offline analysis preserves the original raw files and cannot overwrite an
earlier diagnosis. It also checks archived harness and artifact hashes.

## Untraced baseline results

Batch values are medians of seven process medians, each of three measured
batches. The per-invocation column divides batch time by count; it is **not** an
individual-request latency median. Concurrent rows express throughput with
overlapped work, not a reduction in each worker's individual latency.

| Cohort | Invocations/batch | Batch ms | Batch mean ms/invocation |
| --- | ---: | ---: | ---: |
| Mixed primary, sequential | 32 | 307.92 | 9.62 |
| Inline only | 8 | 78.26 | 9.78 |
| Repeated module | 8 | 80.63 | 10.08 |
| Eight distinct modules | 8 | 75.58 | 9.45 |
| 256 KiB context padding | 8 | 91.70 | 11.46 |
| 256 KiB bundle + source-map padding | 8 | 108.39 | 13.55 |
| Module calls, concurrency 2 | 8 | 45.66 | 5.71 |
| Module calls, concurrency 4 | 8 | 30.19 | 3.77 |

The primary process medians were 301.81, 334.21, 334.19, 305.06, 308.34, 307.24
and 307.92 ms. These are a baseline distribution, not seven speedup pairs.
The frozen future comparison requires alternating baseline/candidate pairs,
at least 10% primary improvement and investigation of controls beyond 5%.
`primaryAccepted` remains null; `releaseAccepted` is false.

For the mixed cohort, median host readiness was 423.54 ms, registration 18.77 ms,
first invocation 13.08 ms and final drain/exit 15.62 ms. The complete host
lifetime and every request duration are retained. Do not add independent
medians to construct a fictitious exact lifetime. First-host cost is distinct
from the roughly 9.6 ms warm invocation cost, and outer pooling already exists.

### Limits of these fixtures and resources

“Unique” means eight different registered bundles within a batch, reused in
later batches. It is **not** sustained unique-per-invocation cache churn or
cache saturation. The large module/map are synthetic padded inputs, not a
representative complex AST/source-map corpus. Context padding is one large
string, not every possible deep object shape. Scripts have tiny fixed results.
Large-result, near-limit, cache-pressure and long-running memory-plateau cohorts
remain later acceptance work if an optimization is pursued.

Raw host-process resource observations are retained, including Bun worker
threads as reported by Bun. RSS units remain explicitly unverified. Per-batch
CPU, allocated bytes and scoped peak-live heap are **null**, not zero. These
resource observations cannot certify native, per-invocation allocation, total
process-tree or retained-memory budgets. Nothing relaxed the existing limits.

## Separate trace attribution

One diagnostic process runs nine actual invocations per cohort (first request
plus eight diagnostic requests), not the primary's full sampling protocol.
Instrumentation/flush overhead is present; none of these times enters baseline
or future acceptance. The following serial module-cohort medians are local
span durations, not a sum of disjoint phases:

| Span | Median ms |
| --- | ---: |
| Host-observed worker journey | 11.545 |
| Synchronous Worker constructor call | 0.076 |
| Host request post/clone call | 0.046 |
| Worker context preparation | 0.171 |
| Worker module-loading scope | 1.154 |
| Worker inline compilation | 0.074 |
| User JavaScript | 0.213 |
| Complete worker invocation scope | 2.389 |
| Worker result-transfer preparation | 0.027 |
| Host synchronous termination request | 0.010 |

A cheap constructor call does not mean a fully initialized worker was ready
in 0.076 ms; most asynchronous startup happens after it returns. Similarly,
`terminate()` timing measures the request, not proof all thread reclamation
finished synchronously. Host process exit supplies a separate lifecycle bound.

Worker profiler-load offsets are retained but are not exact cross-realm startup
latencies. Do not subtract host and worker timestamps, subtract independent
medians to name a residual “bootstrap,” or sum nested spans. Full worker
bootstrap, scheduling and delivery costs are not completely separated here.

Large-module loading measured 4.000 ms in its worker scope, while compilation
remained 0.087 ms. Context padding increased host post/clone and worker context
preparation observations. These identify useful investigation areas, not an
accepted component optimization.

### Profiler limitations found and retained

The concurrency-2 trace has 117 host records but only 86 unique host span IDs;
concurrency-4 has 124 records but only 88 unique host IDs. Repeated host IDs
contain identical records. Quantitative host attribution for those captures is
flagged unusable. The raw records and initial summary remain untouched; the
offline diagnosis adds quality flags instead of silently deduplicating data.

Workers also share the Bun process PID while their profiler counters restart,
so span IDs collide across fresh workers, even in sequential traces. Worker
invocation-scoped `(invocationId, name)` keys are unique in these captures;
unscoped bootstrap records cannot be joined into a globally unique span tree.

The host flush path can run concurrently, and its append/snapshot accounting
has no serialization in `flushPerformanceProfile`; that is a source-based
candidate explanation for host duplicates, not a correction made here.
Profiler serialization/realm identity deserve a separate diagnostic fix before
stronger concurrent attribution. Untraced baseline samples have profiling
disabled and do not consume these span summaries.

## Feasibility and go/no-go decision

A fresh diagnostic worker reproduced the existing deterministic source-map
embedding transformation, measured bundle/map hashing, imported fresh Blob URLs
and constructed inline functions. Each component has one warmup and three
measured samples with repeated operations to expose small costs. This is
**not** the complete production loader or a fresh-worker-per-operation timing.
Repeated operations can use runtime/JIT caches; do not substitute these numbers
for the production worker spans above.

| Diagnostic component | Small module/map median ms | Padded module/map median ms |
| --- | ---: | ---: |
| Bundle + source-map digest calculation | 0.00257 | 1.40375 |
| Source-map base64/embedding transformation | 0.00077 | 0.18696 |
| Fresh Blob-URL module import | 0.04044 | 0.30753 |

The small fixture has a 72-byte bundle and 102-byte map. The padded fixture has
a 262,216-byte bundle and 262,229-byte map; its base64 representation alone is
349,640 bytes. Precomputing that string retains expanded bytes and still cannot
skip worker-side integrity verification or module evaluation. No such cache
was added. Miss/registration costs and memory would need explicit accounting.

Sending an actual constructed `AsyncFunction` through Worker `postMessage`
raised **`DataCloneError`** in both probes. That rules out the tested direct
function-transfer approach, not every conceivable Bun/JSC preparation API.
No supported cross-worker compiled-artifact mechanism has been established.

| Candidate | Decision and reason |
| --- | --- |
| Outer host pool / immutable module cache | Already present; not new ISR work |
| Host-created function reused in fresh workers | No-go for the tested route: function transfer fails |
| Retain executed workers or module namespaces | Out of scope; violates fresh-state contract without a separate isolation design |
| Another cached source-map embedding | No-go on current evidence: tiny primary-fixture cost, extra retained bytes, hashing/import still required; no evidence for the 10% mixed target |
| Skip digest checks, module imports or context freezing | Rejected: changes integrity, initialization or isolation semantics |
| Simplify worker bootstrap/build entry | Possible future investigation, not an approved optimization; startup sub-costs and compatibility effects are not yet isolated |

The roughly 0.074 ms compile span does not support a large claimed saving
against roughly ten-millisecond invocations. This is not a formal speedup bound
computed from mixed instruments; no baseline/candidate implementation was run.
Keep fresh workers and defer production preparation changes. A narrower future
design should begin with reliable bootstrap attribution and its own evidence,
not a promise to proceed with ISR2 regardless of findings.

## Isolation reference and verification

The archived unmodified built host/worker are the independent future reference.
Existing tests verify globals, module state and namespaces, secret/environment
isolation, protocol versions, source-map integrity, module-initialization effect
denial, result/context bounds, cancellation races and out-of-order messages.
Rust tests independently cover settlement acknowledgements and completion
ordering. Direct-host timing makes **no durable-authorization claim**.

Recorded local checks:

- Seven ISR contract/analysis tests passed, plus TypeScript checking of all new
  runner/probe modules.
- All 64 existing script-host tests passed. Subprocess cases explicitly select
  the archived built host; in-process host tests use current unmodified source
  and real workers. This is not a clean npm-installed execution claim.
- Four Rust host settlement unit tests passed, including bounded missing and
  lost acknowledgements. These are unit regressions, not actual native-addon
  end-to-end settlement performance.
- All 34 existing source/modularization/architecture/CI contracts passed, as
  did documentation links, public-example validation and diff checks.

The new portable contracts are wired into the existing combined platform
matrix; timing and diagnostic captures are not automatic CI benchmarks.
Actual macOS/Windows execution is not certified by this Linux run.

## Remaining boundaries and reproduction

No public protocol/model/event/store/native export, dependency, cache limit,
worker lifetime or product budget changes. No Rust/Bun production source was
modified. The new files are measurement, fixtures, analysis, tests and records.

Prepared-store locking, the failed installed inspection journey, earlier
SRS/VSR/ISQ controls and their resource/platform gaps remain open. Real managed
service waits, Rust dispatch, durable settlement and installed workflows are
not measured by this no-effect direct-host baseline. Do not mark ISR4–ISR6 or
the whole performance branch accepted from these results.

Commands from `woml-cli`, run serially with at least 2 GiB available RAM/disk:

```bash
bun run build:invocation-setup-baseline
bun run capture:invocation-setup-baseline --receipt <receipt.json>
bun run profile:invocation-setup --receipt <same-receipt.json>
bun run probe:invocation-setup --receipt <same-receipt.json>
bun run analyze:invocation-setup <timing-directory> <trace-directory> <probe-directory>
bun run test:invocation-setup-contracts
```

Build/capture currently qualify only this original baseline checkpoint, with
unchanged production sources and matching harness hashes. Retain the archived
receipt/artifacts after committing these tools. Later candidate builds and
matched comparisons require an explicit provenance extension; do not rewrite
the baseline commit/receipt or disable the guard to accommodate a new HEAD.
The offline analyzer can read the retained verified evidence after HEAD changes.
