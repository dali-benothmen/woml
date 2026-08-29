# PERF5 Presentation and Runtime Services Profile

Status: PERF5 completed on 2026-08-28.

This report separates durable workflow completion from the moment the user sees the result, then decomposes the production services initialized before the runtime reports that it is ready. The measurements are profiler-enabled diagnostics; the controlled multi-batch baseline remains PERF6.

## What is instrumented

Terminal presentation now records secret-free spans for:

- workflow startup rendering;
- trigger and execution progress handling;
- authoritative durable run inspection through N-API;
- Rust event folding, result summarization, and bounded presentation projection;
- TypeScript presentation decoding;
- final and intermediate rendering;
- the single atomic stdout or stderr write.

Every settled-run span carries the same run ID. This establishes a direct measured boundary from receipt of Rust's durable `run_terminal` fact through the write that makes the result visible.

Runtime activation now records:

- workflow registration and secret resolution;
- Rust trigger-runtime startup;
- observability initialization;
- runtime-control server startup;
- communication-provider host startup;
- final source and input-set revalidation;
- opening durable trigger admission;
- retention scheduling;
- runtime descriptor publication and the ready receipt.

The canonical fixture has no external communication provider. Its provider-host span therefore measures the zero-provider path. Real provider handshakes remain visible through the same span, but they are excluded from the canonical result because external network latency is not WOML engine latency.

## Reproduce

```bash
bun run build
bun run profile:workflow:presentation
bun run profile:presentation:modes
```

The trace is written to `woml-cli/.woml/performance/presentation.ndjson`. Profiling is opt-in and ordinary `woml` output is unchanged.

## Cold activation diagnostic

Environment: Linux x64, Bun 1.3.14, release native core, Intel Core i7-9750H. The fresh-process activation sample was 874.61 ms. The benchmark-child calibration median was 116.89 ms and is reported separately rather than subtracted.

| Activation stage | Duration |
| --- | ---: |
| Bun process to profiler availability | 186.73 ms |
| Compile complete input set | 79.14 ms |
| Start Rust core services | 498.32 ms |
| Start runtime control | 25.71 ms |
| Open durable admission | 24.72 ms |
| Publish runtime descriptor | 6.64 ms |
| Final source revalidation | 4.35 ms |
| Initialize observability | 3.60 ms |
| Provider-host path, zero configured providers | 1.72 ms |
| Resolve registrations and secrets | 1.29 ms |
| Schedule disabled retention policy | 0.60 ms |
| Report ready | 0.76 ms |
| Render startup presentation | 1.10 ms |

The stages are not all additive: several N-API and Rust child spans sit inside `runtime.start_core_services`. Inside that 498.32 ms envelope, opening and initializing the fresh SQLite store took 435.05 ms and Rust trigger-host preparation took 32.78 ms. Fresh durable-store initialization, not terminal rendering or observability, dominates this cold activation sample.

## Durable completion to visible result

The trace contains one warm-up and three measured two-step runs. Across all four terminal results:

| Post-completion stage | Average |
| --- | ---: |
| Complete durable-terminal presentation journey | 24.63 ms |
| Authoritative inspection as observed by TypeScript | 24.56 ms |
| N-API presentation inspection | 23.15 ms |
| Rust store inspection after the bridge opened the store | 1.97 ms |
| Rust bounded presentation projection | 0.63 ms |
| Result summary inside the projection | 0.003 ms |
| TypeScript decode and contract validation | 1.14 ms |
| Final terminal rendering | 1.71 ms |
| Atomic output write | 0.27 ms |

The approximately 21 ms gap between the TypeScript N-API call and Rust's inner store-inspection span includes opening the SQLite store in the native bridge plus boundary conversion. Together with PERF3's store-opening measurements, repeated database opening is the leading explanation, but the exact split will be finalized in PERF6 rather than silently assumed.

The measured warm `manual.visible` median was 1,644.43 ms. Only about 24.63 ms occurred after durable completion, so presentation accounts for roughly 1.5% of that observed result latency. The workflow engine and script-host lifecycle identified in PERF4 remain the dominant short-workflow cost.

## Output-mode comparison

The mode benchmark uses one real durable eight-step presentation, a warm store, ten batches of ten complete foreground journeys, and no profiler. Each journey includes startup, admission, started progress, terminal inspection, rendering, and captured output.

| Mode | Median journey | Durable inspections | Output size |
| --- | ---: | ---: | ---: |
| Normal plain output | 5.70 ms | 1 | 2,412 B |
| Colored terminal output | 6.15 ms | 1 | 2,803 B |
| `--json` | 10.25 ms | 2 | 5,124 B |
| Verbose colored output | 5.45 ms | 1 | 2,868 B |

Color and verbose formatting do not materially change the result. JSON mode is slower here because it deliberately emits an authoritative presentation snapshot at admission as well as at terminal completion. The additional durable inspection—not JSON serialization—is the meaningful difference. Direct rendering of every mode remained below 1.5 ms median in this diagnostic.

## What PERF5 tells us

1. Terminal formatting is not responsible for WOML's current short-workflow latency.
2. The user sees a terminal result roughly 25 ms after durable completion in the profiler-enabled canonical run.
3. Reopening the durable store for presentation appears more expensive than folding, summarizing, decoding, rendering, and writing combined.
4. Fresh SQLite initialization dominates the measured cold runtime-service activation.
5. Runtime control, observability, source revalidation, retention scheduling, and the zero-provider host path are individually small.
6. JSON progress intentionally pays for extra authoritative snapshots; PERF6 should judge whether that behavior is worth optimizing without weakening trustworthy output.

No optimization is implemented in PERF5. Durable authority, event folding, result bounds, terminal sanitization, atomic output, provider behavior, and source-integrity checks remain unchanged.

## Safety and conformance evidence

- Performance spans contain identifiers, durations, counts, and byte sizes only.
- Workflow result values, script source, secrets, and provider credentials are absent from traces.
- The focused performance contract suite validates every span against `performance-span.v1.schema.json`.
- Existing foreground presentation tests preserve plain, JSON, warning, retry, and for-each output behavior.
- Rust presentation tests preserve deterministic projection, redaction, truncation, lifecycle, branch, parallel, approval, retry, and workflow-call behavior.
