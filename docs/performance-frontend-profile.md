# PERF2 Bun/TypeScript Frontend Profile

Status: PERF2 completed on 2026-08-28.

This report records the first diagnostic frontend trace. It proves that the CLI and compiler can be decomposed accurately; it is not yet the multi-batch performance baseline produced by PERF6.

## What is instrumented

Profiling is disabled during ordinary WOML use. It is enabled only when both `WOML_PROFILE=1` and `WOML_PROFILE_OUTPUT=<path>` are present.

The frontend records buffered, schema-valid NDJSON spans for:

- process-to-profiler bootstrap;
- command and runtime-configuration parsing;
- workflow input discovery and source reads;
- WOML markup parsing;
- reusable-definition resolution and validation;
- module inspection and runtime-module materialization;
- definition, reusable, executable, and runtime package construction;
- compiled-model lowering and promotion;
- canonical definition hashing;
- source and directory snapshot verification;
- editor-type refresh.

Spans contain timings, counts, and byte sizes. They do not contain source text, scripts, context values, payloads, secrets, headers, or durable workflow events.

## Reproduce

From the repository root:

```bash
bun run build
bun run profile:workflow:frontend
bun run profile:workflow:frontend:source
```

The first profile uses the release-shaped `dist/cli.js` bundle. The second loads the TypeScript source entrypoint. Custom investigations can select an isolated artifact:

```bash
cd woml-cli
bun scripts/measure-workflow.ts \
  --mode manual \
  --warmups 0 \
  --iterations 1 \
  --cli-artifact built \
  --profile-output /tmp/woml-frontend.ndjson
```

## Diagnostic sample

Environment: Linux x64, Bun 1.3.14, release native core, Intel Core i7-9750H, canonical two-step manual workflow. Each value below is one fresh-process diagnostic sample, so it must not be treated as a stable product benchmark.

| Stage | Release bundle | TypeScript source |
| --- | ---: | ---: |
| Process start to profiler module | 61.37 ms | 103.98 ms |
| Complete frontend compilation | 26.84 ms | 30.72 ms |
| WOML markup parsing | 5.10 ms | 5.44 ms |
| Definition package construction | 8.45 ms | 10.39 ms |
| Model lowering | 3.73 ms | 4.85 ms |
| Cold process to runtime Ready | 485.03 ms | 572.51 ms |

For the release bundle, child spans covered 26.55 ms of the 26.84 ms compile envelope. The unexplained residual was 0.29 ms, or 1.09%, satisfying PERF2's requirement that less than 5% remain unattributed.

## What this tells us

The release bundle materially reduces early Bun/module bootstrap in this diagnostic sample. Compilation is visible but does not explain most of cold activation. On this small fixture, XML parsing accounted for about 5 ms, so replacing `fast-xml-parser` is not currently justified by end-to-end evidence.

The largest remaining interval sits below the frontend: native loading, Rust activation, durable storage, runtime services, the script host, and presentation are not decomposed by PERF2. PERF3 will instrument the N-API boundary, Rust admission, SQLite, and durable execution before any optimization is selected.

The warm manual result is intentionally omitted from conclusions here. It includes Rust, SQLite, worker isolation, script execution, settlement, inspection, and presentation, which later phases must separate.
