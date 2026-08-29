# PERF3 Native and Durable Runtime Profile

Status: PERF3 completed on 2026-08-28.

This report proves that WOML can separate TypeScript boundary work from native decoding, Rust execution, and SQLite durability. It is a diagnostic profile, not the multi-batch optimization baseline planned for PERF6.

## What is instrumented

Profiling remains opt-in and does not change WOML's durable event vocabulary. The runtime records schema-valid, secret-free spans for:

- native-addon loading and TypeScript request serialization;
- native JSON decoding and result encoding;
- runtime startup, activation, trigger admission, dispatch, and settlement;
- durable one-shot execution and admitted-run execution;
- SQLite opening and schema initialization, definition registration, trigger admission, event transactions, projections, and event folding;
- model, trigger, module, result, and database byte sizes plus node, module, event, and transaction counts.

No workflow source, script, payload, context value, secret, credential, or request header is recorded.

## Reproduce

From the repository root:

```bash
bun run build
bun run profile:workflow:runtime
```

For a complete cold activation, manual execution, and durable-engine trace:

```bash
cd woml-cli
bun scripts/measure-workflow.ts \
  --mode all \
  --warmups 1 \
  --iterations 1 \
  --profile-output /tmp/woml-runtime.ndjson \
  --json
```

## Diagnostic sample

Environment: Linux x64, Bun 1.3.14, release native core, Intel Core i7-9750H, canonical two-step workflow. The table uses one unreported engine warm-up followed by one measured run. It is evidence about attribution, not a stable performance claim.

| Warm durable stage | Duration |
| --- | ---: |
| TypeScript `napi.execute_durable` envelope | 966.73 ms |
| Native `napi.execute_durable` envelope | 965.65 ms |
| Rust durable execution | 965.62 ms |
| Rust workflow engine | 927.59 ms |
| TypeScript request serialization | 0.29 ms |
| Native request decoding | 0.30 ms |
| Native result encoding | 0.04 ms |
| TypeScript result decoding | 0.07 ms |

The request contained a 916-byte model with two nodes, a 2-byte trigger object, and no runtime modules. The result contained 2,247 bytes and six durable events.

During that warm run, six event-append transactions took 108.70 ms in total. Twenty-six durable projections took 23.03 ms. Event folding itself was called 117 times but consumed only 2.56 ms, so folding computation is not currently the dominant cost. Repeated projection and store access remain candidates for later review because their call counts are disproportionate to a two-step workflow.

The first run also exposed a separate cold cost: initial SQLite store/schema initialization took 421.29 ms in this diagnostic trace. Later store openings were roughly 7–10 ms each. This explains a substantial portion of fresh-state startup, but it does not explain the warm engine body.

## What this tells us

The N-API boundary is not the main bottleneck. On the measured warm run, the TypeScript envelope exceeded the native envelope by about 1.07 ms, while serialization and JSON decoding were each below 0.31 ms.

Most unexplained time is inside `runtime.run_engine`, below the boundary instrumented in PERF3. That interval contains script-host communication, isolated worker creation, context transfer, script evaluation, and worker teardown. PERF4 will decompose those stages before WOML changes its runtime architecture.

SQLite also deserves targeted investigation. Cold schema initialization is expensive, and a tiny workflow performs several store openings and event transactions. WOML will not remove durability or isolation to improve these numbers; later optimization must preserve the event contract, crash behavior, and authoritative projections.

## Contract evidence

The integration test validates every emitted span against `performance-span.v1.schema.json`. It also proves:

- the CLI and native manual-admission spans share one invocation ID;
- the CLI and native durable-execution spans share one run ID and trace ID;
- policy-lease spans remain available in the implementation but are not required from the canonical fixture because it has no concurrency or rate-limit policy;
- ordinary WOML execution remains unprofiled unless explicitly enabled.
