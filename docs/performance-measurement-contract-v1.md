# WOML Performance Measurement Contract v1

Status: frozen for the performance investigation.

This contract makes every WOML latency number describe an exact boundary. It is an internal engineering contract, not a public CLI compatibility promise.

## Canonical fixture

The baseline fixture is `woml/tests/fixtures/performance-two-step-manual.woml`.

It contains one manual trigger, two sequential script steps, one `context.steps` reference, no external services, no modules, no lifecycle hooks, no retry, and no control flow. Its expected final result is:

```json
{"message":"Hello World"}
```

## Registered v1 metrics

| Metric | Temperature | Start | End |
| --- | --- | --- | --- |
| `harness.process` | calibration | benchmark child process is spawned | child reports that its control protocol is ready |
| `activation.cold` | cold | release-shaped runtime child is spawned | `runCli` invokes its runtime-ready callback after durable activation |
| `manual.visible` | warm | benchmark controller submits the manual trigger | terminal run presentation is emitted for that request |
| `engine.durable` | warm | compiled model is passed to the N-API durable executor | the durable executor returns its terminal result |

`manual.visible` includes trigger admission, Rust execution, script-host work, durable settlement, authoritative presentation inspection, JSON rendering, and delivery to the benchmark controller. It does not include activation.

`engine.durable` starts with an already parsed and compiled model. Its configured warm-up iterations load the native addon and warm relevant operating-system caches before measured samples begin.

`activation.cold` is release-shaped: a fresh Bun process dynamically loads the built `dist/cli.js` and invokes its exported `runCli`. The reported `harness.process` calibration exposes the extra benchmark-child bootstrap overhead. Calibration is reported, never silently subtracted.

## Temperature definitions

- **Cold:** a new operating-system process and WOML runtime are created.
- **Warm:** the same active runtime receives another trigger, or prior unmeasured iterations have warmed the engine-only path.
- **Calibration:** measures harness machinery and is not a WOML product result.

## Sampling rules

- Cold activation creates a fresh runtime and temporary state database.
- Warm manual samples use one active runtime and are sequential.
- Warm-up samples are executed but excluded from summaries.
- Engine-only samples use a compiled model resident in the benchmark process.
- Reports contain raw samples plus minimum, median, p95, maximum, and median absolute deviation.
- A single sample is diagnostic only. Optimization decisions require at least three independent batches.
- External network services are forbidden in the canonical fixture.

## Result artifact

Benchmark results conform to `docs/schemas/performance-measurement.v1.schema.json` and use the profile:

```text
woml.performance-measurement/v1
```

Durations are milliseconds from a monotonic clock. Wall-clock time is used only for the artifact creation timestamp.

## Future profiler spans

PERF2 and later will emit spans conforming to `docs/schemas/performance-span.v1.schema.json` with profile:

```text
woml.performance-span/v1
```

Spans contain timings, identities, counts, and byte sizes only. They must never contain secrets, workflow payloads, script source, request headers, or complete context values. They are diagnostic files and never durable workflow events.

Monotonic timestamps from different processes must not be compared directly. Cross-process analysis joins paired spans by trace, run, and invocation identity and uses durations recorded by each process.

Generate the PERF2 frontend trace from the repository root:

```bash
bun run build
bun run profile:workflow:frontend
bun run profile:workflow:frontend:source
```

The commands append schema-valid NDJSON spans for the release-shaped bundle and source entrypoint to `woml-cli/.woml/performance/frontend.ndjson` and `frontend-source.ndjson`. Set `--cli-artifact built|source` and `--profile-output <path>` on `measure-workflow.ts` when isolated artifacts are required. Profiling remains disabled for ordinary WOML commands.

Generate a PERF3 native-runtime trace with one unreported warm-up and one measured durable run:

```bash
bun run build
bun run profile:workflow:runtime
```

The trace is written to `woml-cli/.woml/performance/runtime.ndjson`. The engine measurement runs in a fresh child process so the same profiling environment is visible to Bun, N-API, and Rust from process startup. TypeScript/native pairs are joined by invocation or run ID; their separate monotonic clocks are never compared directly.

PERF4 adds release-shaped step-count and growing-context diagnostics:

```bash
bun run profile:workflow:steps
bun run profile:workflow:context
```

These write `steps.ndjson` and `context.ndjson` beside the runtime trace. Every script invocation is joined across the Rust host client, long-lived Bun host, and isolated Bun worker by run and invocation IDs. Only byte sizes and counts cross the profiler; script source, context values, results, and secrets remain absent.

PERF5 adds terminal-presentation and runtime-service diagnostics:

```bash
bun run profile:workflow:presentation
bun run profile:presentation:modes
```

The first command writes `presentation.ndjson` beside the other traces and correlates a durable terminal run with its authoritative inspection, projection, decoding, rendering, and output write. It also measures runtime-control, observability, retention, provider-host, source-revalidation, and readiness work during activation. The second command compares the same real eight-step presentation in normal, colored, JSON, and verbose modes without changing any public CLI behavior.

## PERF6 controlled baseline

Run the complete controlled matrix from the repository root:

```bash
bun run build
bun run benchmark:performance-baseline
```

Use `bun run benchmark:performance-baseline:quick` for a bounded diagnostic run. Both commands produce `woml-cli/.woml/performance/baseline-v1.json` with profile `woml.performance-baseline/v1`.

The runner prepares every runtime case before measurement, then interleaves scenarios across at least three rotated and reversed batches. This prevents every large case from inheriting the same fixed position in the machine's thermal and cache history. Memory measurement runs separately after latency collection. Each case records raw latency and throughput samples, median, p95, median absolute deviation, event counts, graph size, model/source bytes, durable-store bytes, process-tree RSS on Linux, and its position in every batch.

The baseline is machine-specific evidence rather than a public performance promise. It is ignored by Git; the reviewed interpretation and bottleneck ranking are committed in `docs/performance-baseline-report.md`.

## Historical Cronflow comparison

The remembered Cronflow latency of roughly 20 ms is a product reference, not a valid baseline until the same semantic workflow and the same timing boundary are reproduced from a historical revision. Historical comparisons must run from an isolated checkout and must not restore legacy execution code to WOML.
