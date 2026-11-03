# Incremental scheduling: final comparison and release blockers

Recorded locally on Linux on 2026-11-03, branch `perf/runtime-performance`.

**Release acceptance remains blocked.** The final recorded-history CPU result
passes the frozen numeric target, but small-workload/control regressions remain
open. The prepared-runtime locking prerequisite is still unsafe. Fresh final
native/installed and actual macOS/Windows release evidence is not certified.
This is the ISQ6 blocked/deferred handoff allowed by the implementation plan,
not a declaration that the combined performance branch is ready to ship.

## What this checkpoint changes

Adds an explicit historical-baseline continuation command, measurement-contract
tests, and this record. It makes **no production Rust changes**.

The original ISQ runner deliberately pins HEAD to its initial checkpoint. It
has not been relaxed or replaced. The new command accepts only the exact
originally qualified baseline source, binary and capture-adapter identities.
It verifies archived sources against that pinned manifest and requires the
frozen capture function body to match the current one byte for byte. The
candidate needs an explicit optimized build receipt matching current HEAD and
every source-manifest file. Executables, host, toolchain, frozen contract and
harness dependencies are checked before and after captures.

This is local provenance, not signed independent build attestation. A missing
old executable cannot be replaced by a new artifact merely labelled baseline.
If the frozen probe changes, qualify a separate experiment. Nothing builds,
installs or selects a native addon during capture. CI runs the portable
contract tests, not timing acceptance.

## Final original-baseline comparison

Standard optimized engine **unit-test executables**, Rust/Cargo 1.88.0 and Bun
1.3.14, Linux x64, Intel i7-9750H, kernel `6.8.0-138-generic`.

| Identity | Original indexed baseline | Final candidate |
| --- | --- | --- |
| Source commit | `70a5d4cc484b6dca4e03aaf3f8aea1bd7127784b` plus the originally qualified test adapter | `fd5f1e4977460b63e8d7e00fe371856457f70dd4` |
| Receipt directory | `validated-state-build-LSn0lW/isq-receipt.json` | `validated-state-build-IdcaN4/receipt.json` |
| Executable SHA-256 | `dcd04b1db65e2153ae441531273f09aad627d67eaed4d334a43d80dca02b9b15` | `ff3d17f9671dcdb9cc180ddcd5de3a3e5619ea1443eb600f47b308d7e3fa8265` |
| Source-manifest SHA-256 | `3cb433f0f405625da1125664752b46ea075ccdba7bf7cbbd308c300d75a5a0f7` | `8dab1f490b1adca2c8e4d7c3c3ff9da8ec8afc840b608bd4f9e032201347ce94` |

The candidate executable/source digest matches the final ISQ4/ISQ5 artifact;
the new receipt binds that content to its now-committed HEAD. No Rust changes
or additional instrumentation were needed for this comparison.

Raw capture: `woml-cli/.woml/performance/isq-release-QGR2jG` (ignored local
evidence). All fourteen processes completed. Seven adjacent pairs alternate
original/candidate order. Each process records one warmup and three measured
samples at each of six frozen sizes. Summaries are medians of process medians.
Builds/tests did not overlap timing. No failed measured process, outlier or
slow control was removed or retried.

Recorded-history validation, thread CPU milliseconds:

| Nodes | Original | Final | Observed reduction |
| ---: | ---: | ---: | ---: |
| 1 | 0.005477 | 0.006665 | **−21.69% (slower)** |
| 4 | 0.029359 | 0.028349 | 3.44% |
| 16 | 0.202518 | 0.118472 | 41.50% |
| 64 | 2.210550 | 0.562872 | 74.54% |
| 256 | 38.997633 | 3.396079 | 91.29% |
| 1,024 | 640.071577 | 43.724744 | 93.17% |

All seven 1,024-node pairs favor the candidate; pair reductions range
91.44–93.65 percent. Wall medians are 640.092739 and 43.728781 ms.

**These are synthetic histories containing recorded attempts, not execution of
1,024 JavaScript steps.** They do not establish an end-to-end workflow speedup,
startup improvement, native-addon speedup or long-lived request latency.
Disabled test-observer branches remain in these test artifacts.

### Controls that still require investigation

Every CPU row beyond the frozen five-percent regression trigger is retained:

| Nodes | Operation | Original CPU ms | Final CPU ms | Increase |
| ---: | --- | ---: | ---: | ---: |
| 1 | History validation | 0.005477 | 0.006665 | 21.69% |
| 1 | Full query including preparation | 0.00042255 | 0.00046905 | 11.00% |
| 1 | Borrowed topology preparation | 0.00012520 | 0.00013555 | 8.27% |
| 4 | Full query including preparation | 0.00226975 | 0.00241880 | 6.57% |
| 4 | Borrowed topology preparation | 0.00064865 | 0.00069735 | 7.51% |
| 16 | Full query including preparation | 0.01152025 | 0.01216140 | 5.57% |
| 64 | Full query including preparation | 0.04956420 | 0.05354920 | 8.04% |
| 64 | Borrowed topology preparation | 0.02249225 | 0.02366540 | 5.22% |
| 1,024 | Full query including preparation | 1.07809855 | 1.13892385 | 5.64% |

The corresponding wall rows also exceed five percent except the 64-node full
query; all seventeen flagged metric rows remain in `summary.json`. CPU and wall
for one operation are not independent regressions. No other measured row
crosses the trigger in this cohort.

The one-node history pays cursor/frontier preparation for one membership query;
the cost is not amortized over later starts. Its CPU difference is about
1.19 microseconds. Small absolute cost does not waive the frozen investigation
rule. No post-hoc node-count threshold or size exclusion was introduced.

The query/topology controls do not consume the historical frontier. They run
after each history sample, as frozen originally. Code generation, observer
overhead, allocation/cache state and host effects have not been isolated;
these regressions are **not proven harmless noise**. A common-probe comparison
with controls before history, and fresh same-binary component attribution,
remain needed. Earlier controls do not become accepted because a later cohort
has a different median. The prior VSR cache-hit finding also remains open.

### Resource evidence and its limits

GNU time whole-process maximum RSS medians are 19,525,632 bytes original and
19,890,176 bytes final: **+364,544 bytes**, below the frozen 16 MiB investigation
trigger. Every process observation is retained. RSS includes setup and all
sizes; it is not per-validation allocation or scoped peak-live memory.
Unverified Bun RSS units remain raw and do not enter the byte comparison.

Allocation and scoped-peak fields in the unchanged probe remain **null**, not
zero. The earlier [frontier evidence](incremental-scheduling-frontier.md) uses
the identical final executable hash and separately reports requested Rust heap
bytes: at 1,024 nodes, validation allocation is 78,099,527 → 6,826,175 bytes,
peak increases by 79,981 bytes, and completed validation retains zero net bytes.
That comparison is against the post-reuse candidate, not the original ISQ0
baseline. It does not measure native/SQLite allocations, retained large-payload
cohorts or physical allocator overlap. One-node allocation increased 7.03
percent there. Those limits and control findings remain open.

## Attribution across the implementation

These are **different matched cohorts**, not interchangeable medians.
Do not add percentages or compose them into a promised workflow speedup.

| Increment | Retained matched capture | 1,024-node history CPU ms | Reduction |
| --- | --- | --- | ---: |
| Historical single-node membership | `isq-comparison-kbrf87` | 564.277 → 346.068 | 38.67% |
| Immutable topology + prefix latest-attempt reuse | `isq-reuse-B9M9MR` | 336.716 → 297.091 | 11.77% |
| Cursor-local historical frontier | `isq-reuse-3rZXev` | 286.393 → 41.218 | 85.61% |
| Original indexed baseline → final, this checkpoint | `isq-release-QGR2jG` | 640.072 → 43.725 | 93.17% |

Topology and latest-attempt changes share a checkpoint: their separate CPU
contributions have **not** been isolated. Deterministic counters attribute the
work removed: membership avoids unrelated candidate predicates/result vectors;
immutable owners reuse topology; prefix ordinals avoid revisiting old attempts;
frontier updates touch the changed node and outgoing dependents. Counters are
algorithm evidence, not separate measured CPU percentages.
See [membership](incremental-scheduling-queries.md),
[reuse](incremental-scheduling-reuse.md), and
[frontier](incremental-scheduling-frontier.md), including unresolved controls.

## Blocking prerequisite reconfirmed

The existing disposable diagnostic was rerun without touching workflow state:

```bash
bun woml-cli/scripts/investigate-sqlite-file-locks.ts
```

Retained report: `/tmp/woml-sqlite-file-locks-wIHXHp/report.json`.
An independent writer remains blocked during the parent's immediate SQLite
transaction before the probe and after pathname metadata inspection. It
acquires the writer lock after an extra raw database descriptor is opened,
inspected and closed. The current guard still contains that operation sequence;
its source digest is
`789876de99b19b8962adc13fe6c451a2a529b03ceab1679d4d7f78467d0dd6a6`.

This is an OS-level operation-sequence reproduction with Bun SQLite, **not** a
direct production-guard invocation or proof of every earlier disk-I/O failure.
The [validated-state record](validated-state-reuse-verification.md#prepared-runtime-locking-blocker)
explains the required independent-process transaction exclusion test and entire
pin-lifecycle audit. Both revalidation closes and final pin teardown matter.
No identity, ABA, VFS, owner, schema, lease or Windows protection may be removed
to make the failure disappear.

The dedicated startup-locking correction has not been implemented here.
Affected fresh installed long-lived journeys remain blocked, rather than
being retried into apparent release acceptance. The previously failed live
inspection journey and its unexecuted restart/restore tail remain unresolved.

## Evidence matrix and remaining work

| Gate | Evidence/status |
| --- | --- |
| Frozen numeric history CPU target | Passed locally, this matched cohort only |
| All timing/allocation controls accepted | **Not accepted**; findings above and older records remain open |
| Broad performance cohorts | **Incomplete**: this final comparison is chains; wide DAG/fan-in/out, retries/clock crossings, complex fallback, large contexts and multiple retained runs still need optimized performance captures |
| Scope/correctness coverage | Prior ISQ5 Linux integrations cover branches, forks, loops, retries, waits, approvals, locks, cancellation, correction, recovery and rollback; correctness coverage is not a performance capture |
| Scoped memory/physical resources | Partial diagnostics; full original-baseline and realistic retained-payload attribution incomplete |
| Prepared-runtime locking prerequisite | **Blocked**, hazard reproduced again |
| Fresh final optimized native and clean installed journey | **Not run here**; prerequisite first. Older native success is not final-artifact certification |
| Linux/macOS/Windows installed/safety release matrix | Linux evidence only; actual macOS/Windows results **not certified** by local CI-source checks |
| Startup/output/package/isolation product budgets | Unchanged; **not rerun/certified for a fresh final native artifact here**. Earlier artifact passes do not close this gate |
| Developer-book architecture acceptance | No release-accepted architecture claim added; private supported scope remains documented |

The frontier exists only inside one immutable historical validation cursor,
for the conservative unscoped ordinary script DAG subset. Unsupported features
use the fallback; no live dynamic frontier or durable checkpoint is introduced.
Neither readiness indexes nor a validated history authorizes effects. Claims,
policy, ownership, leases, fencing, cancellation, SQL validation and atomic
commit-before-cache-publication remain independent gates.

Resume release acceptance after the locking correction is independently
verified, then close the controls/cohort/resource gaps and run fresh native,
installed and actual platform gates with unchanged budgets. If benefit does
not justify overhead, simplify or defer the relevant optimization instead of
loosening the frozen contract. Do not mark ISQ6 release acceptance complete.

## Verification and reproduction

New portable tests cover pinned historical identities, frozen probe changes,
missing/null measurements, complete process/RSS observations, slow controls and
the prohibition on deriving release acceptance from a numeric win.

Checks run at this checkpoint:

- The exact optimized candidate executable: **203 engine unit tests passed**,
  zero failed, six explicit capture-only tests ignored.
- ISQ portable contracts: **15 passed**, including five new continuation tests;
  TypeScript checking passed.
- Existing source/modularization/architecture/CI contracts: **34 passed**.
- Rust formatting, diff checks, documentation links and every public example
  passed. Documentation initially found a stale link to a deleted private PR
  draft; that link now points to this maintained evidence record.

The earlier ISQ5 feature integrations are retained evidence, not rerun or added
to these counts. The OS diagnostic passed its reproduction assertion, meaning
the hazard exists, not that production locking is safe.

From `woml-cli`, serially:

```bash
# Explicit cached standard release build; note the printed receipt path.
bun run build:validated-state-reuse-baseline

bun run compare:incremental-scheduling-release \
  --baseline .woml/performance/validated-state-build-LSn0lW/isq-receipt.json \
  --candidate <new-current-engine-receipt.json>

bun run test:incremental-scheduling-contracts
bun run test:rust-core-modularization-contracts
```

The old baseline receipt and archived sources must exist locally. Do not
regenerate a new baseline after optimization, edit its receipt, substitute
unavailable CPU with wall time, or rerun only slow samples.
