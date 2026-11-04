# Validated execution state reuse verification

Recorded 2026-11-03 on `perf/runtime-performance`.

**Release acceptance is blocked.** Engine safety checks and actual native
execution pass locally. A complete matched attribution cohort reaches the
10 percent CPU threshold, but a cache-hit control regresses and the clean
installed repeated-request journey fails during live inspection. A separate
diagnosis reproduces a POSIX locking hazard in the operation sequence used by
the existing prepared-runtime identity guard. That needs correction before
shipping; no assertions, authority checks or budgets were weakened.

This work continues on `perf/runtime-performance`. The implementation is
described in [the implementation record](validated-state-reuse-implementation.md).
The [frozen acceptance contract](validated-state-reuse-acceptance.v1.json)
remains unchanged.

## Safety checks

The new tests exercise the production checker and converted store methods:

- At every recorded prefix, five deterministic mutations are compared with
  the independent old folding and readiness reference, including exact errors.
  Mutations affect sequence, version, run identity, event identity and omissions.
- Deferred foreign-key faults fail at COMMIT after policy claims or wait
  registration have completed their statements. Every durable table, history,
  summary and policy index must match its previous contents after rollback.
  The old cached projection remains authoritative and a valid retry succeeds.
- Revoking prepared runtime ownership immediately before commit rejects both
  converted transaction paths. A checked history cannot authorize that write.
- Another connection's committed tail invalidates a reader's cached state;
  reopening the store starts with an empty connection-local cache.
- More than 256 active runs cannot enlarge the existing cache. Evicted runs
  reload normally, and terminal runs remain uncached.

Earlier tests continue to cover exact input borrows, earlier-content changes
with the same tail, structural error precedence, indexed JSON mismatches,
complete suffix state, hydration failure and raw versus public review outputs.
The count/tail freshness rule is unchanged: these checks do not claim it detects
arbitrary offline edits to an earlier event while retaining the same tail.

Recorded local Linux results:

| Selection | Result |
| --- | --- |
| Complete engine unit run | 175 passed, 3 explicit capture tests ignored |
| Focused history, store-boundary and prepared checks | 18, 21 and 30 passed; overlap with the complete unit run |
| Feature integrations across 19 test binaries | 157 passed |
| Baseline, comparison and allocation-parser contracts | 14 passed; TypeScript checking passed |
| Rust source, modularization, architecture, CI and native-cleanup contracts | 34 passed |

The feature selection includes policy-managed execution, lifecycle, retries,
parallel work, choices, fork and loop recovery, waits and signals, ordinary and
rich approvals, compensation and cleanup, leases and fencing, cancellation,
deadlines, runtime ownership, backup/restore and retention. Tests using real Bun
hosts retain their ordinary invocation and settlement paths.

The complete unit run preceded the additional all-table rollback assertions;
the subsequent boundary selection passed those assertions. Counts overlap and
must not be summed into a larger unique total.

The established Clippy policy, Rust formatting, source/architecture guards,
documentation links and validation of every public example also pass without
new allowances or baseline changes.

## Reproducible comparison

The explicit comparison command does not build, install or select a library:

```bash
bun run --cwd woml-cli build:validated-state-reuse-baseline
bun run --cwd woml-cli compare:validated-state-reuse \
  --baseline <baseline-receipt.json> \
  --candidate <candidate-receipt.json> \
  --baseline-commit <full-baseline-commit>
```

The runner verifies the candidate against the current source manifest and each
historical baseline source file against the explicitly named Git commit. Both
binaries, compiler versions, Bun version and host identifiers must match their
receipts. Fixture fingerprints must match across every process. The comparison
harness and receipts are retained with the raw results under ignored
`woml-cli/.woml/performance/` directories. These are local build records, not
independently signed attestations.

Seven adjacent process pairs alternate baseline/candidate then candidate/baseline.
Each artifact uses the frozen 128, 512 and 2,048 event fixtures, two connection
warmups, five measured connections and five hits per connection. Only the
declared warmups are excluded. All measured processes, outliers and failures are
retained. Summaries use the median of process medians, not pooled samples or the
best run. Unavailable CPU remains null and cannot pass the primary target.

For reuse attribution, the pre-reuse baseline is the post-index commit
`471e926`. Its retained optimized executable has SHA-256
`2ebb7d838f5e919cf11e385bfd60ebdfe303b80742413017c27d228d603072aa`.
All 935 files in its source manifest match that commit. Its older receipt names
the earlier working-tree checkpoint; the file-by-file comparison establishes
the post-index source identity without rewriting that receipt.

This attribution baseline is deliberately different from the original frozen
baseline commit. The runner labels that distinction and cannot certify release
acceptance from the attribution comparison. In particular, the earlier roughly
15-fold history-validation gain belongs to readiness indexing, not reuse.

On Linux with GNU time available, whole-process maximum RSS is retained in
explicit bytes from its KiB measurement. It includes fixture setup and is not
scoped peak-live Rust allocation. Bun's raw RSS retains its unverified unit and
is not substituted into the byte budget. The uninstrumented timing rows keep
allocator fields null; the separate resource capture below reports intercepted
whole-process heap observations, not per-miss allocation or scoped peak-live bytes.

## Optimized artifacts

Both builds used the repository's standard optimized release profile, serially
and separately from capture, with Rust 1.88.0, Cargo 1.88.0 and Bun 1.3.14 on
the same Linux x64 i7-9750H host. The source checkpoint is
`24d718c3f3b406f93f8883461bb074922050f583` plus the verification patch.

| Artifact | Retained receipt directory | SHA-256 |
| --- | --- | --- |
| Post-index baseline engine executable | `validated-state-build-9n72bM` | `2ebb7d838f5e919cf11e385bfd60ebdfe303b80742413017c27d228d603072aa` |
| Reuse candidate engine executable | `validated-state-build-7XnZLT` | `2cdf7aa4e023595e2bbee4a2d1cfd82680e18e6418e3690a736799131c693df0` |
| Fresh optimized native addon | `startup-build-nhP3Zx` | `4842a765c7f8f15555cc598e0106c988095b7ab297208a2f2c838d672b633008` |

The candidate engine's source-manifest digest is
`dbbf075eb7df7b7c9014db8489f1b99dd411caf12b7312b391db6713062fe531`.
Its retained core/source patch digest is
`9a2642189146ab17d336d5fd31bb79346c9b7725e6edd33c0d0422c8f0b0d536`;
the native build records the same patch. Later changes in this checkpoint are
verification scripts, CI selection and documentation, not production Rust.

## Matched CPU and memory observations

Raw timing capture: `validated-state-comparison-uQRL2X`. All 14 independent
processes completed, with 210 measured cache misses and 1,050 measured cache
hits across the three history lengths. Builds, installations and unrelated
tests did not overlap capture. No measured sample was rerun, trimmed or dropped.

| Events | Baseline miss thread CPU ms | Candidate miss thread CPU ms | Observed reduction |
| --- | ---: | ---: | ---: |
| 128 | 3.782 | 3.487 | 7.79% |
| 512 | 48.656 | 44.248 | 9.06% |
| 2,048 | 760.167 | 680.285 | 10.51% |

The long miss's wall median changes from 760.216 to 681.019 ms. These are
store-projection queries on synthetic histories, not the time to execute
1,024 real steps or an end-to-end workflow-latency claim.

### Why the numeric threshold is not release acceptance

The primary CPU process medians range 623.612–883.085 ms for the baseline and
615.398–816.742 ms for the candidate. Five pairs favor the candidate; two do
not. The isolated full-fold control changes 9.889 to 9.291 ms. Removing one such
fold does not by itself explain the entire roughly 80 ms miss difference.
The observed percentage must not be presented as an established universal gain
from that one removed fold.

The 2,048-event hit control exceeds the frozen 5 percent investigation trigger:
CPU changes **0.775592 to 0.831191 ms**, about 7.17 percent slower, or 55.6
microseconds. Wall time is about 7.08 percent slower. These are two measurements
of the same control, not two independent regressions. Other measured timing
controls remain below the regression trigger.

The hit process-median ranges overlap: 0.648327–1.285794 ms versus
0.650033–0.903093 ms. The production method prefix through the hit return is
byte-identical to the post-index baseline, with SHA-256
`3153257bf83373bcd550e46e3a89cb2ecb069543b5326712075c17591e44234f`.
However, cached state is now allocated during the earlier fold. Allocation
layout and host variability remain possible influences; unchanged hit code
does not prove the measured regression harmless. Focused clone/allocation
attribution remains required. The command's `primaryAccepted: true` describes
the numeric target only; its `releaseAccepted` remains false.

### Separate allocation observations

Raw resource capture: `validated-state-allocations-Ff9x4z`. Another seven
alternating pairs use the same verified executables and fixtures with the
installed glibc allocation interceptor. All instrumented timings are excluded
from acceptance. Each variant's allocation totals are identical across its
seven processes.

| Whole capture process observation | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Intercepted cumulative heap bytes | 1,354,427,428 | 1,332,088,097 | −1.65% |
| Profiler-reported heap peak bytes | 14,359,538 | 15,024,079 | +664,541 bytes |
| Uninstrumented maximum RSS median bytes | 26,812,416 | 27,406,336 | +593,920 bytes |

Cumulative heap bytes are allocation requests and realloc growth across setup,
warmups and measured calls, not memory simultaneously resident. The profiler
intercepts malloc/calloc/realloc/free; this record makes no claim to cover all
aligned allocations or mmap activity. Its peak label retains the profiler's
semantics and is not renamed a scoped cache-miss peak-live measurement.

The observed allocation total does not increase beyond the 5 percent trigger;
the RSS median increase remains below 16 MiB. Peak memory nevertheless increases,
so this is not a zero-memory-cost optimization. Per-miss allocation attribution
and scoped peak-live bytes remain open, especially for larger output/context
payloads than this synthetic fixture.

## Native and installed results

The fresh optimized addon passes all 68 native export checks, including
synchronous/asynchronous errors, callbacks, activation, telemetry, shutdown and
listener rebind. The prepared-runtime verifier passes all eight isolated cases:
sequential work, hooks, managed state, storage, parallel work, loops,
retry/wait/output, and owner transfer. These compare real engine facts and
results, not matched end-to-end performance.

The clean-package command installs the fresh optimized addon through
`WOML_TEST_NATIVE_ARTIFACT`; `WOML_RUST_CORE_PATH` is absent, so installed CLI
execution cannot silently load a repository runtime instead. Four journeys
pass: remote attempts, durable artifacts, combined project operations, and
OpenTelemetry. The repeated-request/restart/restore journey fails on the second
request's live `get --json` call:

```text
WOML_WAIT_INSPECTION_FAILED: disk I/O error
```

The failing consumer and state are retained at
`/tmp/woml-startup-package-i2powf`. The failure was not retried into a passing
release result. Restart and restore portions of that journey were not reached.

Existing combined product budgets pass unchanged against this exact addon:
cold activation 643.55 ms, warm p95 579.75 ms, streamed-output overhead 93.56
percent, main JavaScript 1,150,183 bytes and installed JavaScript 2,497,413 bytes.
The static combined release verifier also passes. Its success does not override
the actual failed installed journey or certify remote platform execution.

## Prepared runtime locking blocker

The existing identity check opens a temporary raw file handle for the database,
reads its identity, then closes it. Linux POSIX record locks belong to the
process: closing another descriptor for the same file can release SQLite's
locks while SQLite still believes its transaction owns them.

The disposable diagnostic script reproduces that operation sequence with
independent writer processes:

| Parent still inside its SQLite immediate transaction | Independent writer |
| --- | --- |
| Before the identity probe | Blocked |
| After a pathname metadata check | Blocked |
| After raw open, fstat and close | Acquires the writer lock |

```bash
bun woml-cli/scripts/investigate-sqlite-file-locks.ts
```

The result is retained at `/tmp/woml-sqlite-file-locks-O5fAp9/report.json`.
This is an OS-level reproduction using Bun SQLite, not a direct invocation of
the private Rust guard or attribution of every disk-I/O failure. The unsafe
sequence is present in the current production guard, whose source is unchanged
from the post-index baseline; it belongs to startup separation, not the three
reuse conversions.

Repair must account for the entire identity-pin lifecycle: acquisition,
revalidation, revocation and final close, including other active SQLite
connections. In particular, merely replacing the temporary probe with metadata
does not audit the original pin's close during revocation. Preserve native VFS
identity, ABA protection, schema/owner/lease checks, precommit rejection and
Windows replacement restrictions. Verify an independent process remains
excluded for the full transaction and rerun the failed installed journey.

This requires a dedicated startup-locking correction. It is intentionally not
hidden inside the history-reuse verification patch.

The later [incremental scheduling release record](incremental-scheduling-release-evidence.md)
keeps these blockers visible after the final history comparison. Neither
checkpoint is a release-ready handoff.

## Compatibility and release gates

No production Rust method, native export, public format, dependency, authority
rule or budget changes in this verification checkpoint. The three earlier
conversions remain the entire reuse scope. The public hydrated projection is
still cloned separately from raw cached fold state; retaining a complete state
during prefix checking can increase peak-live memory and needs measurement.

`test:validated-state-reuse-safety` runs the feature and ownership selection
serially in the existing Linux, macOS and Windows combined matrix. That matrix
also retains actual native lifecycle, prepared runtime, installed package and
frozen product-budget checks. Local Linux results do not establish either other
platform's success.

The shipping decision must satisfy the unchanged primary improvement target,
investigate every control regression above 5 percent and account for allocation
and memory costs, fresh native execution, installed journeys and platform CI.
No timing-only report can set `releaseAccepted` to true. If useful benefit is
absent, defer or simplify the optimization; do not relax the acceptance contract.
