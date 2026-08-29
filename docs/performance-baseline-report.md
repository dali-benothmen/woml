# PERF6 Baseline, PERF7 Optimization Decisions, and PERF8 Gates

Status: PERF8 completed on 2026-08-29.

This report answers where WOML currently spends time. It ranks optimization work; it is not a promise that every machine will reproduce the same absolute latency.

## Executive finding

WOML is not slow because XML parsing, N-API, user JavaScript, or terminal colors are slow. The dominant cost is the production durable execution path: starting and supervising the Bun script host, creating an isolated worker for every script invocation, and repeatedly opening, updating, and projecting durable workflow state.

Cold activation has a separate fixed cost. Fresh Rust core-service and SQLite initialization is its largest measured stage, followed by Bun startup and the complete frontend input pipeline.

The optimization order is therefore:

1. amortize script-host lifecycle across runs without weakening crash isolation;
2. reduce repeated durable-store openings, projections, and safe event transactions;
3. reduce isolated-worker lifecycle cost while retaining fresh invocation state, timeouts, cancellation, and crash containment.

Parser rewrites, N-API replacement, and terminal-rendering work are not justified by this baseline.

## Method

The baseline runner generates deterministic workflows, builds each with the authoritative TypeScript compiler, and executes each with the release Rust core and Bun script host. Runtime cases use three independent batches. Cases are rotated and reversed between batches so a large workflow cannot always run at the same point in the machine's thermal and cache history. Memory sampling runs afterward and cannot affect latency samples.

The controlled matrix covers:

- 2, 10, 50, and 100 sequential script steps;
- 2, 10, and 50 parallel branches, capped at four simultaneous branches;
- `for-each` over 10 and 100 items, capped at four simultaneous iterations;
- tiny, 100 KiB, and 1 MiB step results;
- small, 100-step, control-flow-heavy, one-module, and ten-module compilation;
- plain, colored, JSON, and verbose presentation;
- native `fetch` and managed `services.http.request()` against loopback only;
- cold activation and warm Enter-to-visible-result waterfalls.

Run the complete local matrix from the repository root:

```bash
bun run build
bun run benchmark:performance-baseline
```

For a bounded diagnostic pass:

```bash
bun run benchmark:performance-baseline:quick
```

Both commands write the raw, machine-readable artifact to `woml-cli/.woml/performance/baseline-v1.json`. The artifact is local and ignored by Git because it records machine-specific evidence.

## Environment and interpretation

The reviewed run used commit `69d643d`, Bun 1.3.14, Linux x64, a release native build, an Intel Core i7-9750H, and 16 GB of memory. It used the bounded three-batch matrix after a longer run exposed severe sustained-load variation on this machine.

The canonical cold activation median was **1,080.56 ms** with a **1,281.86 ms p95**. The warm manual median was **2,350.67 ms**, but its samples ranged from **620.13 ms to 2,908.26 ms**. That distribution is intentionally reported rather than averaged away. It means the local absolute warm number is not yet suitable as a cross-machine release budget. The component ranking, scaling curve, and attributed owners are still clear.

## Runtime scaling baseline

| Case | Median | p95 | Durable events | Median work units/s |
| --- | ---: | ---: | ---: | ---: |
| Sequential, 2 steps | 2,531.83 ms | 2,820.12 ms | 6 | 0.79 |
| Sequential, 10 steps | 3,592.51 ms | 3,911.43 ms | 22 | 2.78 |
| Sequential, 50 steps | 22,958.36 ms | 24,371.82 ms | 102 | 2.18 |
| Sequential, 100 steps | 122,628.63 ms | 129,749.22 ms | 202 | 0.82 |
| Parallel, 2 branches | 2,956.68 ms | 3,067.87 ms | 12 | 1.35 |
| Parallel, 10 branches | 3,682.93 ms | 3,889.64 ms | 28 | 3.26 |
| Parallel, 50 branches | 24,554.30 ms | 27,000.15 ms | 108 | 2.12 |
| `for-each`, 10 items | 4,186.81 ms | 4,659.18 ms | 50 | 2.87 |
| `for-each`, 100 items | 35,291.27 ms | 36,192.63 ms | 410 | 2.89 |

The 100-step sequential workflow takes far more than twice the 50-step workflow. Constant-size outputs should not create that shape, so PERF7 must inspect repeated durable work and process lifecycle before treating the curve as acceptable. Parallel and `for-each` benefit from bounded concurrency, but their event volume and isolated-worker work remain visible.

## Context, storage, and memory

| Result shape | Median | p95 | Incremental RSS during run | Durable store |
| --- | ---: | ---: | ---: | ---: |
| Tiny | 2,808.03 ms | 2,812.61 ms | 0.31 MiB | 348 KiB |
| 100 KiB per step | 3,068.99 ms | 3,234.30 ms | 1.51 MiB | 1.80 MiB |
| 1 MiB per step | 4,388.32 ms | 4,988.76 ms | 28.33 MiB | 15.36 MiB |

Large context is not the primary fixed-cost bottleneck, but it materially increases latency, resident memory, and durable storage. The RSS column is the observed increase from the shared runner's pre-run baseline; the raw artifact also retains absolute process-tree RSS. Because each later script can dynamically read all earlier step results, cumulative context transfer remains structurally quadratic. PERF7 must preserve the current JavaScript context contract and size enforcement.

## Compiler baseline

| Frontend shape | Median | p95 |
| --- | ---: | ---: |
| Small workflow | 3.23 ms | 4.47 ms |
| 100 sequential steps | 52.43 ms | 60.59 ms |
| 50 conditional branches | 20.26 ms | 24.23 ms |
| One local module | 23.61 ms | 25.53 ms |
| Ten local modules | 40.61 ms | 56.49 ms |

Even the largest compiler case is small compared with durable execution. The wider cold `compiler.compile_inputs` envelope also includes input discovery, module packaging, integrity work, and editor metadata; XML parsing alone remains a small child span. A Rust `quick-xml` experiment is therefore not selected for PERF7.

## Cold activation waterfall

The canonical cold sample was 1,000.59 ms. Named stages account for **930.51 ms (93.00%)**, exceeding PERF6's 90% acceptance threshold. The remaining **70.08 ms** is explicitly named the **cold process/runtime boundary residual**; it includes uninstrumented module evaluation, scheduling between observed envelopes, and benchmark signal delivery.

| Measured owner | Duration | Share of observed activation |
| --- | ---: | ---: |
| Rust core services, including fresh durable-store setup | 419.11 ms | 41.89% |
| Bun bootstrap to profiler availability | 311.13 ms | 31.09% |
| Complete frontend input pipeline | 141.28 ms | 14.12% |
| Runtime control | 29.94 ms | 2.99% |
| Open durable admission | 14.34 ms | 1.43% |
| Other measured readiness work | 14.71 ms | 1.47% |
| Named boundary residual | 70.08 ms | 7.00% |

## Warm execution waterfall

The profiler's measured warm-manual mean was 2,056.06 ms. The complete Rust durable-execution envelope averaged 2,048.22 ms, terminal presentation averaged 20.99 ms, and admission averaged 14.85 ms. These independently measured cross-process envelopes can overlap slightly and come from a variable local distribution, so their sum is not treated as a perfectly stacked clock. They nevertheless assign **100% of the warm journey** to named measured owners.

Durable execution owns essentially the entire critical path. Presentation is about one percent, and admission is below one percent. Prior PERF3 and PERF4 traces decompose the durable owner further: N-API serialization is sub-millisecond, actual user JavaScript is sub-millisecond for trivial scripts, while script-host startup, isolated-worker journeys, SQLite event transactions, repeated projections, and store access dominate.

## Secondary comparisons

- Direct rendering medians were 0.30–1.39 ms. Complete normal, colored, and verbose presentation journeys were about 21–23 ms. Formatting is not the problem.
- The local HTTP sample showed no managed-service penalty worth optimizing. With only four loopback requests it is diagnostic, not a general throughput claim.
- The historical Cronflow tag `v0.11.1` exists, but its benchmark simulates pooling, caching, serialization, and SDK setup. It does not execute an equivalent durable workflow, so the remembered 20 ms number is deliberately excluded rather than presented as a false comparison.

## PERF7 experiment shortlist

Only these three experiments are authorized by the baseline:

1. **Reuse the supervised Bun script host across workflow runs.** Expected impact: remove the largest fixed warm-run process cost. Required guarantees: host-crash distinction, fail-closed interrupted effects, module registration integrity, cancellation, and clean shutdown.
2. **Remove redundant durable-store work.** Inspect store reopenings, repeated projections, event folds, and transaction boundaries; reuse handles or batch only operations that are semantically safe. Durability and authoritative folding remain mandatory.
3. **Reduce isolated-worker lifecycle cost safely.** Prototype an isolation-preserving worker strategy and compare it with fresh workers. Reject it if globals, secrets, cancellation, timeout enforcement, or crash containment can leak between invocations.

Each experiment must be independent and must report before/after median, p95, memory, throughput, and correctness. A fast microbenchmark is insufficient if the user-visible workflow journey does not improve.

## PERF7 decisions

PERF7 used the same release-shaped binary for alternating before/after batches. Temporary experiment switches existed only while collecting the comparison and were removed afterward. Values below are the median of three independent batch summaries, not a single favorable sample.

### Adopted: reuse the supervised Bun script host across runs

| Metric | Previous lifecycle | Shared host | Change |
| --- | ---: | ---: | ---: |
| Canonical warm median | 852.17 ms | 110.71 ms | 87.0% lower |
| Canonical warm p95 | 874.37 ms | 280.42 ms | 67.9% lower |
| Estimated warm throughput | 1.17 runs/s | 9.03 runs/s | 7.7x |
| Peak process-tree RSS | 141.22 MB | 146.78 MB | 3.9% higher |

The production runtime now checks out one healthy host for a run and returns it afterward. Different concurrent runs still receive exclusive clients and may execute in parallel; a burst can retain at most four idle hosts. Pool entries are keyed by the complete host options, including immutable module artifacts, so one workflow cannot inherit another workflow's module registry. Capability authority is attached to each pending invocation rather than permanently trusted from the process that first created the host. A crashed host is never returned to the pool, and clean runtime shutdown drains all idle processes.

This deliberately amortizes process startup only. Every `<script>` still receives a new Bun worker with fresh globals, its own context, timeout/cancellation enforcement, and crash containment. The durable event log, folding authority, result-size enforcement, module integrity checks, and fail-closed recovery behavior are unchanged.

### Rejected: remove one repeated durable-history read

| Metric | Repeated read | Single read | Change |
| --- | ---: | ---: | ---: |
| Eight-step warm median | 348.59 ms | 366.77 ms | 5.2% slower |
| Eight-step warm p95 | 384.40 ms | 385.54 ms | effectively unchanged |

Although the second query looked redundant in isolation, removing it did not produce a repeatable end-to-end improvement. The batches alternated winners and remained dominated by other durable work. The experiment was reverted rather than retained as an unearned optimization.

### Rejected: replace fresh `smol` workers with fresh regular workers

The regular-worker prototype preserved per-invocation isolation and improved the eight-step median and p95. It did not survive the concurrent check: the parallel batch-median median changed from 333.08 ms to 364.52 ms, a roughly 9% regression, even though its p95 improved. Sequential and parallel peak process-tree RSS were effectively unchanged in this bounded test. Because the primary concurrent metric regressed, WOML keeps fresh `smol` workers.

### Correctness gates

The adopted host lifecycle is protected by a release-artifact regression test proving that three manual runs use one supervised host but six distinct isolated workers. Existing script-host protocol, durable-store, capability-authority, compiler, runtime, lifecycle, cancellation, recovery, and terminal-presentation tests remain the correctness authority.

## PERF8 regression protection

PERF8 freezes `woml.performance-regression-budgets/v1` with named compiler, runtime, presentation, and CI owners. Hard gates now cover the canonical graph shape, supervised-host process count, isolated-worker count, host shutdown, schema validity, compiler median/p95, and disabled-profiler overhead. They execute after the release-shaped runtime is built and fail CI on a real contract or component regression.

End-to-end journeys remain advisory. CI measures the canonical two-step workflow, eight sequential steps, and a parallel workflow, retains their raw versioned measurements for 30 days, and adds a readable table to the GitHub job summary. A target miss is marked for review but cannot fail CI solely because a shared runner was slow. Artifact upload is also non-blocking because network failure must not invalidate otherwise successful runtime tests.

The reviewed local PERF8 report produced:

| Journey | Cold activation | Warm median | Warm p95 | Advisory |
| --- | ---: | ---: | ---: | --- |
| Canonical two-step | 598.35 ms | 110.19 ms | 271.98 ms | within targets |
| Eight sequential steps | 574.06 ms | 327.92 ms | 547.86 ms | within targets |
| Parallel workflow | 627.58 ms | 330.66 ms | 449.59 ms | within targets |

The policy and approval requirements are documented in `docs/performance-regression-policy.md`. Local artifacts remain ignored because they contain machine-specific evidence, not source-controlled promises.
