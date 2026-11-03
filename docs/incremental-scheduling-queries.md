# Incremental scheduling queries: ISQ0 and ISQ1

Work continues on `perf/runtime-performance` from source checkpoint `70a5d4c`.
The local `WOML Incremental Scheduling Queries Implementation Plan.md` and
[private contract](protocols/incremental-scheduling-internal-v1.md) describe the
staged scope. This record is not a release-acceptance certificate.

This page preserves the ISQ0/ISQ1 checkpoint and its unresolved controls.
The later [topology/prefix reuse record](incremental-scheduling-reuse.md)
documents ISQ2/ISQ3 separately; statements below describe the earlier checkpoint.

## What changed

Historical step-start validation asks whether **one particular node** could
start in the preceding history prefix. Previously it computed every ready node
and searched that result. ISQ1 adds private `ReadinessGraph::is_ready_at` and
uses it only for that historical `StepAttemptStarted` check.

The full-list and membership APIs share `candidate_is_ready`, preserving the
same completion, active-predecessor, settlement and attempt/retry predicates.
Membership tests source IDs and evaluates only matching candidates; it avoids
the ready-result vector and unrelated candidate predicates. Duplicate IDs in
malformed direct inputs still use any-matching-source semantics, rather than
the first-node lookup's semantics.

This does **not** make readiness incremental yet. Active reachability and
latest-attempt views are still rebuilt from the exact projection per query.
The membership query also still scans source IDs. It is not an O(1) lookup or
a new cross-query cache. Later phases address those different costs.

For a 16-node chain with 16 historical starts, deterministic production-path
tests assert:

| Work | Previous full-list algorithm | ISQ1 |
| --- | ---: | ---: |
| Candidate predicate checks | 256 | 16 |
| Full ready-result vector construction paths | 16 | 0 |
| Topology preparations per validation | 1 | 1 |
| Active-node visits | 256 | 256 |
| Latest-attempt vector visits | 120 | 120 |

These are algorithm work counts, not allocated-byte or hardware measurements.
The counters are test-only, thread-local and enabled only for untimed checks.

## Boundaries deliberately unchanged

- Complete structural validation still precedes definition/readiness checks;
  all historical acceptance/rejection and error ordering must match the
  independent original scan/full-replay oracle.
- The prefix before a step-start event and that event's timestamp are unchanged.
  Retry eligibility can change with time alone, including clock reversal.
- Non-running status still returns not-ready before binding validation.
  Running wrong-definition inputs still fail with the original error.
- Stable idempotency-key checks, scoped-event rules, context visibility and
  later graph checks remain in their previous order.
- Live readiness APIs, source-ordered runtime scheduling, append guards,
  parallel/fork/for-each selection, cancellation, claims, leases, fencing,
  policy capacity, transaction boundaries and cache publication are unchanged.
- No public schema/export/version, dependency, historical fixture or existing
  performance budget changes. No new retained dynamic scheduling state.

## Frozen baseline

The [acceptance JSON](incremental-scheduling-acceptance.v1.json) was written
before production edits. It fixes graph sizes, seven independent processes,
one warmup/three measured samples, alternating comparison pairs and medians of
process medians. The complete workstream targets a 10 percent reduction in
1,024-node history-validation thread CPU; a control regression over 5 percent
requires investigation. Those are not a claim of current release acceptance.

Baseline qualification compares the current production source with commit
`70a5d4cc484b6dca4e03aaf3f8aea1bd7127784b`, allowing only explicit `cfg(test)`
capture registration/counter probes and the new test module. Optimized build
receipts retain all 939 input-file hashes, the source patch, the capture
harness/contract and copies of readiness/history sources. This permits review
after the uncommitted test adapter changes for the candidate. This is local
provenance, not independent signed build attestation.

Initial baseline receipt: `validated-state-build-LSn0lW/isq-receipt.json` under
`woml-cli/.woml/performance/`. Baseline executable SHA-256:

```text
dcd04b1db65e2153ae441531273f09aad627d67eaed4d334a43d80dca02b9b15
```

Source manifest SHA-256:

```text
3cb433f0f405625da1125664752b46ea075ccdba7bf7cbbd308c300d75a5a0f7
```

Initial baseline capture: `isq-baseline-8TQsxS` in the same directory. All seven
processes completed before production membership edits. Initial history CPU
medians in milliseconds were 0.005777, 0.031955, 0.207393, 2.423379, 38.909635
and 660.385365 for 1, 4, 16, 64, 256 and 1,024 nodes respectively. Linux GNU
time whole-process maximum RSS had a median of 19,611,648 bytes.

These synthetic histories contain recorded attempts, not actual execution of
1,024 scripts. Allocation totals and scoped peak-live bytes remain unmeasured;
the reports preserve null rather than fabricated zeros. Disabled test-counter
branches remain in optimized test binaries and their execution frequency can
change; these timings cannot substitute for production native-addon evidence.

## Commands

From `woml-cli`:

```bash
bun run test:incremental-scheduling
bun run build:incremental-scheduling baseline
bun run capture:incremental-scheduling-baseline --receipt <isq-receipt.json>

# After the reviewed production change, build separately and then compare:
bun run build:incremental-scheduling candidate
bun run compare:incremental-scheduling \
  --baseline <baseline-isq-receipt.json> \
  --candidate <candidate-isq-receipt.json>
```

The strict baseline build is intentionally only valid before production edits.
Use the retained original receipt afterward; do not regenerate a candidate as
its own baseline. The comparison checks source, contract, harness, toolchain,
host, fixture and executable identities. Builds use the standard release
profile, serially; captures never build or install. Normal CI runs contracts
and deterministic engine tests, not these timing captures.

The initial runner binds both build receipts to this source checkpoint. Later
committed phases need an explicit historical-baseline provenance extension;
do not work around that guard by rewriting the frozen acceptance values.

## Verification status

Recorded local Linux checks:

- All 181 engine unit tests pass; five explicit measurement captures are
  ignored in the ordinary suite. This includes the existing prepared-state,
  rollback, cache and validated-history regressions.
- All 87 selected integration tests pass across DAG execution, retry state and
  hardening, approvals, branches, forks, parallel events, loop models, policy
  storage and durable waits. Counts do not include overlapping focused reruns.
- All 24 portable measurement/contract tests and all 34 source, architecture,
  CI and modularization guards pass. TypeScript checking and Rust formatting
  pass.

The new cases compare membership against independent full-scan results at
every recorded/historical prefix, with good/bad bindings and clocks. Predicate
snapshots cover both approval/choice routes, completion subsets and settlement
failure ownership. Focused cases cover duplicate source IDs with different
handlers, all non-running status gates, replacement projections, legacy
binding rules, missing IDs and before/at/after/backward retry clocks. Existing
mutated-history tests retain exact structural/graph rejection precedence.

The established Clippy policy also passes without new exemptions.

## Matched optimized comparison

All seven alternating baseline/candidate pairs completed. Raw evidence is
retained in `isq-comparison-kbrf87` under `woml-cli/.woml/performance/`.
Do not combine these matched baseline medians with the earlier baseline-only
cohort above. No failed sample or outlier was retried or omitted.

The measured candidate receipt is `validated-state-build-uKPiAN/isq-receipt.json`.
Its executable SHA-256 is
`d58954cfea9d118d48ef92304b9f46db1f6b5e98718796fb780ffbeab18cb622`;
its source manifest SHA-256 is
`4477bedab6d681a5c05affa835074c51a050724c4d785e061e51324c693b9ae5`.
The contract and measurement harness hashes are unchanged from the frozen
baseline. Both use the repository's standard optimized release test profile,
Rust 1.88.0 and Bun 1.3.14 on the same local Linux host.

History-validation thread CPU, median of process medians:

| Nodes / recorded starts | Baseline ms | ISQ1 ms | Observed reduction |
| --- | ---: | ---: | ---: |
| 1 | 0.005620 | 0.005715 | -1.69% |
| 4 | 0.029469 | 0.027485 | 6.73% |
| 16 | 0.200753 | 0.154160 | 23.21% |
| 64 | 2.150112 | 1.415854 | 34.15% |
| 256 | 38.715973 | 20.120273 | 48.03% |
| 1,024 | 564.276667 | 346.067731 | 38.67% |

The primary improves in all seven pairs. Its process medians span 542–749 ms
before and 285–451 ms after. Wall medians change from 564.281 to 346.083 ms.
This exceeds the frozen numeric primary target but is still a synthetic
history-query observation, not real-step throughput or release acceptance.

### Control regressions remain visible

Five CPU controls exceed the frozen 5 percent investigation trigger; their
wall counterparts do too, giving ten flagged summary rows:

| Control | Nodes | Baseline CPU ms | Candidate CPU ms | Increase |
| --- | ---: | ---: | ---: | ---: |
| Full query including preparation | 4 | 0.002269 | 0.002403 | 5.90% |
| Full query including preparation | 64 | 0.050858 | 0.053789 | 5.76% |
| Topology preparation | 64 | 0.022896 | 0.024986 | 9.13% |
| Full query including preparation | 1,024 | 0.949749 | 1.147908 | 20.86% |
| Topology preparation | 1,024 | 0.431450 | 0.509536 | 18.10% |

The topology/latest-index source is byte-identical between the two captured
artifacts (SHA-256 `884c8b8dd7dadfadd7684bc0976c831572677699398254a7058b97bd9f9d40ef`).
The large full-query and topology per-pair ratios closely track one another,
but unchanged source does not prove a slowdown harmless. These controls run
after history validation, whose allocation pattern and duration changed.
Allocator state, code layout and host variability are possible influences;
none is assumed to be the explanation without separate evidence.

Linux GNU time whole-process maximum RSS medians are 19,476,480 versus
19,521,536 bytes, an increase of 45,056 bytes, below the frozen investigation
trigger. This is not scoped peak-live heap or proof of lower allocations.

The comparison preserves `primaryAccepted: true` and `releaseAccepted: false`.
No threshold was raised, original cohort discarded or control suppressed.

### Separate control diagnosis

`isq-control-diagnosis-RbckW6` retains seven alternating pairs in **one identical
optimized executable**. Both modes run identical full-query and topology code
on the same 1,024-node fixture, before and after four historical validations.
Only historical validation's membership strategy differs: the old full-list
strategy versus the new membership strategy. Each control stage has one warmup
and five measured batches of 100 queries. This is a separate diagnostic protocol,
not a rerun of the acceptance cohort or a replacement for a failed control.

Per-query thread CPU medians:

| Stage / control | Full-list history mode ms | Membership history mode ms | Difference |
| --- | ---: | ---: | ---: |
| Before history / full query | 1.183973 | 1.145772 | -3.23% |
| Before history / topology | 0.558655 | 0.540072 | -3.33% |
| After history / full query | 1.179478 | 1.165677 | -1.17% |
| After history / topology | 0.562594 | 0.556932 | -1.01% |

This diagnosis does not reproduce the large control slowdown solely from the
different preceding membership strategy. It does not identify its cause or
rule out cross-binary code-generation/layout, allocator or host effects. In
particular, it does not compare the original predicate's machine code with the
extracted shared predicate. The original small and large control flags remain
open. A next focused experiment should compare **isolated controls before any
history work across both exact binaries**, with an explicitly reviewed common
test harness; the original capture cannot answer that question.

The separate diagnostic build receipt is `validated-state-build-XelqOx/receipt.json`.
Its executable SHA-256 is
`3529676e1a434fc2f4f7d84f7046d185f8b6026bbd8b2eccd22e08837b4e132f`.
The post-comparison source addition is test-only: a scoped full-membership
reference switch and ignored capture. Production query code is byte-identical
to the measured candidate after removing that explicit `cfg(test)` branch.
No operator-facing flag or production runtime behavior was added.

```bash
# Explicit diagnostic build, separate from the frozen primary comparison:
bun scripts/validated-state-baseline.ts build
bun scripts/investigate-incremental-scheduling-controls.ts --receipt <receipt.json>
```

The original comparison remains `releaseAccepted: false`. ISQ0 is complete;
ISQ1's implementation, deterministic work reduction and semantic checks are
complete, but independent performance acceptance remains open on the control
investigation. Further implementation must not treat those flags as resolved.

## Remaining work and release blockers

ISQ2/ISQ3 have a separate [reuse record](incremental-scheduling-reuse.md).
ISQ4/ISQ5 have a [cursor frontier and safety record](incremental-scheduling-frontier.md).
Live frontier integration remains deferred; final broad release evidence remains
ISQ6 work. Neither later record closes the ISQ1 control flags preserved here.

The [previous verification record](validated-state-reuse-verification.md)
still governs the existing prepared-runtime file-lock lifecycle blocker,
failed installed repeated-request/live-inspection journey, cache-hit control
investigation and incomplete scoped allocation/peak-live attribution. This
change neither repairs nor closes those findings. Actual fresh native,
installed-package and macOS/Windows execution are not established by local
Linux graph tests.
