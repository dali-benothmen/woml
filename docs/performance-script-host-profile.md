# PERF4 Script Host and Worker Profile

Status: PERF4 completed on 2026-08-28.

This report decomposes the largest unresolved PERF3 interval: communication with the Bun script host and its isolated per-invocation workers. The samples are profiler-enabled diagnostics, not the multi-batch baseline planned for PERF6.

## What is instrumented

One invocation can now be followed across Rust, the long-lived Bun host, and its isolated worker using the same run and invocation IDs. Spans cover:

- host process spawn, readiness, module registration, and shutdown;
- Content-Length frame writes, reads, decoding, and dispatch;
- complete host invocation and worker journey;
- worker creation, structured-clone request delivery, response receipt, and termination;
- context freezing, module loading, script compilation, user-code execution, JSON validation, and result-transfer preparation;
- context, source, module, frame, and result byte sizes without recording their contents.

Rust span appends are written atomically so concurrent host and worker writers cannot corrupt the NDJSON trace.

## Reproduce

```bash
bun run build
bun run profile:workflow:runtime
bun run profile:workflow:steps
bun run profile:workflow:context
```

The last two commands use reviewed eight-step and growing-context fixtures under `woml/tests/fixtures`.

## Diagnostic samples

Environment: Linux x64, Bun 1.3.14, release native core, Intel Core i7-9750H. Each row is one fresh-state, profiler-enabled run and must not be interpreted as a stable benchmark.

| Fixture | Steps | Durable total | Rust engine | Host startup | Bun worker journeys |
| --- | ---: | ---: | ---: | ---: | ---: |
| Tiny sequential | 2 | 1,892.91 ms | 1,411.53 ms | 1,093.36 ms | 141.85 ms |
| Tiny sequential | 8 | 2,748.66 ms | 2,109.43 ms | 1,120.70 ms | 495.46 ms |
| Growing context | 5 | 2,185.22 ms | 1,685.85 ms | 1,026.42 ms | 379.58 ms |

The large fixed cost is starting and preparing the Bun script-host process. Once it is ready, each script still creates a fresh isolated worker. Across the eight-step fixture, the average host-observed worker journey was 61.93 ms.

Inside those eight workers, actual user JavaScript averaged 0.58 ms and script compilation averaged 0.28 ms. Worker bootstrap to profiler averaged 43.13 ms, while the complete worker-side invocation averaged 7.23 ms. The remaining host-observed time contains worker startup, initial structured-clone transfer, scheduling, result delivery, and teardown.

This means the JavaScript written by the workflow author is not the source of the current small-workflow latency. Process and isolation lifecycle costs dominate it.

## Growing context

The growing-context fixture returns a new 32 KiB value from each of five sequential steps. The contexts delivered to the workers were:

```text
25 B → 32,813 B → 65,619 B → 98,425 B → 131,231 B
```

WOML transferred 328,113 context bytes across the five invocations. Host request delivery took 4.23 ms in total and worker context preparation took 11.35 ms, so 131 KiB is not yet a time bottleneck on this machine.

The volume is nevertheless structurally quadratic: every later step receives all earlier step outputs. With `n` equally sized outputs, cumulative transferred context approaches `n(n-1)/2` output units. Dynamic JavaScript access prevents the compiler from safely selecting only referenced fields, so any future optimization needs an explicit context contract or another semantics-preserving strategy.

## What PERF4 tells us

The next optimization candidates are now evidence-backed:

1. Avoid paying full script-host startup for every short engine execution, while preserving host-crash recovery and clean shutdown.
2. Investigate cheaper isolated worker lifecycle or safe worker reuse without sharing script globals between invocations.
3. Reduce repeated durable store openings and projection work identified in PERF3.
4. Keep context-volume growth visible in large-workflow benchmarks even though it is not the leading cost yet.

No optimization is implemented in PERF4. Isolation, durability, timeout control, cancellation, and fail-closed crash behavior remain unchanged.

## Safety and conformance evidence

- 48 script-host tests pass, including Unicode framing, multiplexing, timeout, cancellation races, worker crashes, fresh globals, environment-secret isolation, result/context limits, modules, services, lifecycle scripts, reusable steps, and for-each bindings.
- Seven performance-contract tests pass and validate every span against `performance-span.v1.schema.json`.
- The trace is checked to contain neither workflow result text nor script source.
- Host and worker spans are correlated to the Rust invocation without comparing clocks from different processes.
