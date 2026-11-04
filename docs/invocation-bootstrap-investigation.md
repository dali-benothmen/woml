# Worker bootstrap: reopened invocation-setup investigation

Recorded on 2026-11-04, `perf/runtime-performance`, source checkpoint
`c1a4440ac5bafec5db9f4f647a27daa3f6e7b3b7`.

**Result:** bootstrap is now better separated from request execution, but no
production optimization is established. Keep fresh workers. The tested cache
ideas remain unjustified; a lean worker-entry design is an investigation area,
not an approved ISR2 prototype or proven ISR3 integration.

Follow-up: the [private function-boundary prototype](invocation-facade-prototype-investigation.md)
was built and tested against the independent reference. It failed the frozen
primary target and was not integrated. This earlier diagnostic is unchanged.

This follows the [original ISR0/ISR1 record](invocation-setup-investigation.md).
The original baseline, profiler captures, acceptance contract and runner guards
remain unchanged. The existing profiler duplication/realm-ID issues are not
fixed under this diagnostic task.

## Question and method

The synchronous `new Worker()` call was cheap, but did not tell us when the
worker had actually loaded its execution listener. Earlier profiler spans did
not reliably separate the asynchronous startup, loading and delivery phases.

The [separate diagnostic contract](invocation-bootstrap-diagnostic.v1.json)
was written before capture. It uses seven independent observer processes,
three declared warmup samples and twelve measured samples per case. Cases
rotate by process/sample and reverse on odd process indexes. All workers are
fresh, execute at most one request and are terminated afterward.

Two paths run side by side:

- **Raw:** launch the exact archived built production worker and post its
  request immediately. Its parent-observed journey includes asynchronous
  startup, loading, delivery, execution and completion. No readiness marker
  exists in this unchanged entry, so its internal stages remain null.
- **Wrapped:** launch a diagnostic entry that announces its own realm ID and
  entry, receives a configuration, dynamically imports the production worker,
  then announces listener readiness. Only then does the parent post execution.
  The wrapper forwards one request to the loaded listener and intercepts the
  completion to report its worker-local execution interval. The production
  source/artifact bytes are unchanged, but the wrapper changes the loading and
  dispatch path. This is a perturbing diagnostic, not the normal host path.

An empty-wrapper control uses the same configure/ready protocol but imports no
production entry and echoes the synthetic result. It does **not** execute a
workflow script. The concurrency control launches four separate one-use
workers, never four invocations in one reused worker.

Parent intervals use only the parent's monotonic clock. Import and execution
intervals use only the worker's clock. We do not subtract absolute timestamps
across realms or subtract independent medians to assign an exact residual.
The small fixture imports the normal verified module and checks its fresh
counter; every production execution must return exactly `1` and reject leaked
worker globals as in the original synthetic fixture.

The wrapper deliberately supports only these no-effect fixtures. It does not
stand in for the full host's capability, Fetch, progress, logging, cancellation
or durable-settlement transport. Overriding diagnostic message handlers is
not a production isolation redesign.

## Provenance and retained evidence

Bun 1.3.14, Linux x64, Intel i7-9750H, kernel `6.8.0-138-generic`.
The direct raw and imported worker copies have the exact original SHA-256:

```text
af632902e850fdba61b7e40dd3c66e3cb21f32d797698e9abd788e83f6e83f08
```

The captured diagnostic wrapper SHA-256 is:

```text
36d105a5e367c253885dcf4d537f0ca72506e0010a86b201fc7ad876a35784e2
```

Original and current production manifests match file for file, digest
`8dab1f490b1adca2c8e4d7c3c3ff9da8ec8afc840b608bd4f9e032201347ce94`.
The new receipt records current HEAD separately from the original receipt;
no old receipt is rewritten. This is local provenance, not signed attestation.

Retained under ignored `woml-cli/.woml/performance/`:

- Diagnostic build and archived collector: `invocation-bootstrap-build-YEUEd5`.
- Raw observations/logs/summary: `invocation-bootstrap-capture-2lwSR2`.
- Original independent built worker: `invocation-setup-build-PysNNm`.

All seven processes completed. There were **1,155 fresh workers**: 1,050 ran
the production worker code and 105 were empty controls, including warmups.
Every process records 120 case observations and 165 workers. No failures,
outliers or slow samples were discarded or retried. No compiler/test workload
overlapped capture. The original profiler was disabled throughout.

After capture, validation was strengthened to compare exact fixture hashes and
reject duplicate realm IDs. All retained observations pass the stronger reader
without a measurement rerun or raw-file rewrite. The captured collector's
original source remains archived; a subsequent capture requires a fresh receipt
for the current harness. Only validation metadata/qualification changed, not
the worker measurement behavior or frozen sample protocol.

## Results

Values are medians of seven process medians. Sequential cases have twelve
measured workers per process; the concurrent case has twelve groups of four,
with individual worker intervals retained. These are diagnostic distributions,
not production latency percentiles or baseline/candidate speedups.

### Parent-observed intervals, milliseconds

| Case | Constructor call | Wrapper entry arrival | Listener-ready arrival | Request round trip | Launch-to-completion journey |
| --- | ---: | ---: | ---: | ---: | ---: |
| Raw inline | 0.075 | — | — | 7.372 | 7.466 |
| Raw module | 0.069 | — | — | 7.523 | 7.608 |
| Wrapped empty | 0.073 | 2.478 | 3.029 | 0.527 | 3.428 |
| Wrapped inline | 0.067 | 2.330 | 4.815 | 3.075 | 7.989 |
| Wrapped module | 0.071 | 2.485 | 5.073 | 3.916 | 8.801 |
| Wrapped 256 KiB context | 0.072 | 2.409 | 5.207 | 3.550 | 8.783 |
| Wrapped padded module/map | 0.066 | 2.356 | 5.147 | 7.340 | 12.629 |
| Wrapped module, concurrency 4 | 0.050 | 2.959 | 5.982 | 4.827 | 10.957 |

“Arrival” means the parent received the marker. It includes scheduling and
message delivery, not just the worker emitting it. The wrapper adds a configure
round trip before import. Constructor timing remains synchronous API timing,
not fully initialized worker startup. Concurrent individual latency is not
the group's batch throughput; raw group timings remain in the captures.

### Worker-local intervals, milliseconds

| Wrapped case | Dynamic production-entry import | Execute-listener entry to completion post |
| --- | ---: | ---: |
| Empty control | Not imported | 0.002 |
| Inline | 1.954 | 2.738 |
| Module | 1.943 | 3.343 |
| Context padding | 1.948 | 2.897 |
| Padded module/map | 1.957 | 5.922 |
| Module, concurrency 4 | 2.401 | 4.086 |

The import interval includes loader/file/parse/initialization work for this
dynamic path; it is not a measured parse-only interval or an allocation count.
The execution interval still includes binding setup, verification, per-worker
module import, inline compilation and execution. It is not pure user-code time.

Raw-inline journey process medians ranged 6.965–8.238 ms; raw-module medians
ranged 7.013–8.432 ms. The large-input and context cases are not perfectly
matched effect-isolation experiments: even their execution medians vary. Do
not turn differences between independent medians into exact transfer/freezing
costs or assume padding always increases every measured interval.

## Interpretation and next decision

1. **A fast Worker constructor is not fast readiness.** The wrapper starts
   announcing itself after roughly 2.3–2.5 ms in the serial cases; production-
   loaded listener readiness arrives around 4.8–5.2 ms on this diagnostic path.
2. **Entry loading and request-side setup are both visible.** Dynamic import
   takes roughly 1.9 ms locally; subsequent actual execution takes roughly
   2.7–3.3 ms for the small sequential fixtures. Neither is represented by the
   tiny inline-compilation span alone.
3. **Moving preparation ahead of the request is not eliminating it.** Ready-
   gated request times look smaller because startup/import precede the request.
   The full launch-to-result journey still includes that work. An eager worker
   pool would add startup/memory/lifecycle trade-offs and remains out of scope.
4. **Large module/map costs remain after listener readiness.** The parent ready
   marker precedes delivery of those fixture bytes; verification and fresh
   user-module loading still occur during execution. Caching a namespace or
   skipping integrity checks would not be an acceptable shortcut.

The roughly 7.5 ms raw-worker journey is **not** an improvement from the earlier
9.6 ms framed-host baseline. This harness bypasses host framing, module
registration, host request checks and routing, and uses a different sampling
boundary. Likewise, wrapper minus raw or loaded minus empty is not an exact
attribution: the entries, imports, handshakes and scheduling differ.

The new evidence justifies examining a leaner worker-entry/setup boundary,
but it does not select a concrete safe transformation or show its benefit.
Current production has only a roughly 57 KiB built worker entry; fewer bytes
alone would not prove faster readiness. Any proposed split/lazy path must
preserve early checks, guarded globals, module initialization, protocol modes,
package contracts and lazy optional activation, then beat an independently
retained reference. No such prototype was implemented or accepted here.

**Decision remains gated:** do not proceed automatically with ISR2/ISR3, change
worker lifetime or add a preparation cache based on these numbers. Reliable
startup versus execution boundaries are now available for evaluating a named
future candidate. Full bootstrap sub-attribution, managed/native settlement,
realistic churn/memory and installed/platform acceptance remain open.

## Validation, limits and reproduction

Ten ISR/diagnostic contract tests and TypeScript checking pass. The 34 existing
source/modularization/architecture/CI guards pass. CI's existing ISR-contract
step discovers the new deterministic tests; it does not run timing captures.
The prior full host/settlement test record remains historical, not a claimed
rerun here. No production Rust/JS, public format, native export, dependency,
old frozen budget or original ISR capture runner was changed.

CPU, scoped allocated bytes and peak-live bytes remain null. Raw process usage
keeps its existing unverified RSS-unit label. Termination is requested per
worker; it is not timed as synchronous thread reclamation. Observer process
exit bounds final cleanup. These short no-effect fixtures do not certify
memory-plateau, effects, cancellation or remote-worker behavior.

```bash
# From woml-cli. Build explicitly; capture never builds.
bun run build:invocation-bootstrap --baseline <original-ISR-build-receipt.json>
bun run capture:invocation-bootstrap --receipt <new-diagnostic-build-receipt.json>
bun run test:invocation-setup-contracts
```

Build qualification permits a later metadata-only descendant of the diagnostic
checkpoint only when the complete production manifest still matches the
original worker receipt. Captures require exact current HEAD/harness/artifact
identity. A changed production tree or harness needs explicitly reviewed new
provenance; never relabel a candidate as the original baseline. Linux results
do not certify macOS/Windows or the unresolved prepared-runtime locking issue.
