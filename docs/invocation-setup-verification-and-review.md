# Invocation setup: isolation verification and performance review

ISR4/ISR5 follow-up on `perf/runtime-performance`, 2026-11-04. The
[ISR2 prototype](invocation-facade-prototype-investigation.md) remains rejected;
ISR3 was not integrated. This checkpoint adds tests and evidence tools, not a
production optimization.

**Outcome:** the selected real-worker/Rust regressions pass for both retained
artifacts. The frozen performance target still fails. ISR5 has a documented
no-benefit outcome; exhaustive ISR4/native/platform/resource acceptance remains
incomplete. No public schema, production source, dependency, stack setting,
cache limit, historical fixture or performance threshold changed.

## Isolation checks

Eight new cases extend the existing host suite:

- Sequential and concurrent runs on one host receive their own secrets,
  deeply frozen context/attempt, module counters and export allowlists.
  Two different digests use the same module alias without namespace reuse.
- Mutations to globals, `Object.prototype` and `Array.prototype` do not reach
  the next worker. Module state also resets after cancellation, timeout and
  an actual `process.exit(17)` inside the worker; the outer host survives.
- Pending managed-call replies retain invocation ownership. One known late
  reply after cancellation/timeout is consumed; duplicates and unknown replies
  are rejected. Late replies do not manufacture a second terminal outcome.
- Progress acknowledgements cannot cross invocation identity. Completion waits
  for the right acknowledgement; a missing acknowledgement times out. Logs and
  progress remain redacted and a subsequent run can complete normally.
- A completed worker's background timer cannot emit output during a later run.
- At 64 cached module entries, another registration is rejected. A substituted
  bundle cannot overwrite the admitted artifact. A replacement host must
  re-register that artifact instead of inheriting the previous host's cache.

These run through real workers, not a fake completion. Existing tests additionally
cover native Fetch admission, out-of-order calls, module/source-map integrity,
historical protocols, lifecycle, correction, lock/artifact facades and result
bounds. Expectations are applied independently to reference and candidate;
there is no claim of byte-identical internal stacks or nondeterministic timings.

### Actual Rust transport and authority

The test helpers in `script_host_protocol`, `services_script_host_v4`,
`cancellation_runtime` and `workflow_call_composition` now accept an explicit
test-only built-host path. An invalid explicit path fails; it does not silently
fall back to source. Normal test invocation remains unchanged.

The [serial verifier](../woml-cli/scripts/verify-invocation-setup.ts) checks the
retained artifact hashes, matching production source and exact Bun executable,
including Bun on Rust's PATH. It archives test/tool sources, arguments, selected
paths, logs, hashes and raw resource observations. Checkpoints remain incomplete
until all commands finish. No suite may pass the verifier by running zero tests.

| Selection | Reference | Candidate |
| --- | ---: | ---: |
| Full Bun host suite | 72 passed | 72 passed |
| Rust host protocol | 6 passed | 6 passed |
| Rust services/secret dispatch | 2 passed | 2 passed |
| Rust cancellation runtime | 6 passed | 6 passed |
| Rust workflow-call composition | 4 passed | 4 passed |

Additionally, **4 Rust settlement tests** and **25 authority tests** pass once
against the unchanged Rust implementation. They cover cancellation/settlement
ordering, lost acknowledgements, durable operation identities and corrective
authority. Counts across variants are repeated selections, not distinct new
tests. Rust tests use the ordinary cached development test profile, one job
and one test thread—not an optimized native addon or installed package.

The full new host suite is wired into the existing combined-experience
Linux/macOS/Windows CI matrix. Only local Linux execution is recorded here.
CI configuration does not establish that another platform passed.

All 18 diagnostic contract tests, 34 architecture/source/CI guards, TypeScript
checking, Rust formatting, documentation/public-example checks and diff checks
also pass. An offline audit verified the six finalized command logs, seven
archived test/tool sources and all 42 original per-process raw evidence files.

## Offline review of the frozen capture

The [reviewer](../woml-cli/scripts/invocation-setup-review.ts) revalidates the
original eight artifact hashes, ten archived harness files, source identities,
raw captures and successful process results. It reproduces the original summary
before writing a separate, non-overwriting review. No timing is rerun, trimmed,
relabelled, or substituted.

All eight cohorts and seven alternating pairs from the 5,040-invocation capture
remain included. The declared statistic is still median of process medians,
not whichever estimator gives a better answer.

### Cold, warm and lifetime costs

Mixed-primary values, milliseconds:

| Boundary | Reference | Candidate |
| --- | ---: | ---: |
| Host ready | 398.51 | 395.72 |
| Registration | 17.69 | 17.64 |
| First invocation | 12.89 | 13.44 |
| Declared warmup batch | 335.02 | 314.78 |
| Measured warm batch, 32 invocations | 303.52 | 311.46 |
| Teardown | 16.60 | 16.24 |
| Whole host lifetime, including first request/warmup/all batches | 1,710.66 | 1,694.59 |

These are separately aggregated observations; do not add or subtract independent
medians to attribute an exact component. The reviewer checks non-overlapping
component accounting within each original observer's lifetime before aggregation.
The slightly lower lifetime includes warmup/cold variation and does not replace
the declared warm-primary gate. There is no positive measured primary batch
saving from which to establish a finite amortization break-even.

### Regression investigation

The repeated-module control remains **6.99% slower** by the frozen estimator,
above the 5% investigation trigger. It is slower in five of seven adjacent
pairs; individual paired changes range from −27.33% to +25.27%. Its whole-lifetime
median is also higher: 776.97 ms versus 743.23 ms. Inputs and completion counts
match; a causal slowdown mechanism is not isolated. The trigger stays unresolved
for acceptance, rather than being dismissed as noise.

The mixed primary is slower in three pairs and faster in four. A paired-median
percentage is a different statistic from the frozen median-of-process-medians
comparison; it cannot be substituted after seeing the result. Neither supports
the required 10% improvement. The distinct-module control's earlier component
win likewise cannot replace the mixed-primary target.

### Bytes and resource limits

Reconstructed from the exact unchanged fixtures, per host including first
requests and warmups; both variants deliver the same content:

| Cohort | Requests | Request-direction framed bytes | Worker bundle UTF-8 bytes | Worker map UTF-8 bytes |
| --- | ---: | ---: | ---: | ---: |
| Mixed | 129 | 91,548 | 4,608 | 6,528 |
| Repeated module | 33 | 26,275 | 2,376 | 3,366 |
| Larger module/map | 33 | 550,747 | 8,653,128 | 8,653,557 |

All other cohorts are retained in the machine-readable review. Wire-frame bytes
are reconstructed request-direction traffic, not observed network packet sizes.
Logical module bytes are **not** structured-clone allocation or resident memory.
The prototype does not remove these transfers or introduce a preparation cache.

Raw process high-water observations stay attached to every cohort. Their RSS
units were not independently established, so the 16 MiB trigger cannot be
evaluated in bytes from them. Scoped CPU/allocation, retained host heap,
active-worker peak, physical clone cost and repeated-run plateau remain **null**.
Three warm batches plus a whole-process high-water mark do not measure a heap
plateau. No memory/resource acceptance is inferred from logical byte counts.

## Reproduction and evidence

From `woml-cli`, with the original archived prototype build/capture available:

```bash
bun run verify:invocation-setup --receipt <retained-prototype-build-receipt.json>
bun run review:invocation-setup <original-matched-capture-directory>
bun run test:invocation-setup-contracts
bun run test:invocation-setup-isolation
```

Verification compiles selected Rust tests explicitly, offline and serially; it
does not build a native addon or run the blocked installed-runtime journey.
Timing capture is not part of these commands or routine CI.

The original comparison remains `invocation-facade-capture-5K0gjQ`; its additional
`isr5-review.json` records all cold/lifetime/process distributions, reconstructed
volumes, raw-file hashes and unmeasured fields. Correctness verification logs
are retained under ignored `woml-cli/.woml/performance/invocation-setup-verification-*`.
The finalized local run is `invocation-setup-verification-zDKZ8Q`, with
`completed: true` and all six command results. The earlier successful
`invocation-setup-verification-kG4qRb` run predates the finalized harness-copy,
PATH qualification and completion-marker additions; both records are retained.
These are local provenance records, not independent signed attestations.

## Remaining boundaries

ISR4 has selected differential coverage, not universal equivalence proof.
Full same-pool policy/authority reassignment, all remote/artifact/process-tree
failure combinations, exhaustive composition and physical resource-leak checks
are not certified by these selections. Existing prepared-runtime locking and
installed inspection failures remain open. ISR6 optimized-addon, clean-package
and actual platform acceptance remain unrun here.

ISR5's valid outcome is **reject/retain evidence**. Do not integrate this
candidate, change the target, count existing pooling as a new optimization, or
mark the shared performance branch release-ready from these tests.
