# Invocation setup: private function-boundary prototype

Recorded on 2026-11-04 on `perf/runtime-performance`, production checkpoint
`0c683fd13c1ba82a3241122cf1e7969127dfb459`.

**Decision: reject this prototype for production integration.** The mixed
primary batch took 303.52 ms for the independent reference and 311.46 ms for
the candidate: 2.62% slower in this capture, not the required 10% improvement.
The repeated-module control also crossed the 5% regression investigation
threshold. Correctness tests passing does not make a slower candidate an
optimization. ISR2 experimentation is recorded; ISR3 integration is deferred.

No Rust or production TypeScript source, dependency, public contract, worker
lifetime, cache limit or product budget changed. This is a measured no-go, not
completion of the invocation-setup or combined performance workstream.

## Candidate and ownership design

The [bootstrap investigation](invocation-bootstrap-investigation.md) left
worker-entry/setup simplification open. This candidate changes one private
function boundary, not worker lifetime or the existing module cache.

`deeplyReadonlyServiceFacade` eagerly captures the request, attempt, resolved
secret values and deadline. Its nested `invokeCapability` has a large dispatch
body. The candidate moves that body verbatim into a private top-level async
function, passing those same four values explicitly. A synchronous forwarding
closure returns its Promise directly, without an extra async wrapper/`await`.

Hypothesis: a smaller eagerly entered facade factory might reduce fresh-worker
parsing/closure preparation. This does not assume JSC parses every nested body
eagerly. The dispatch body has 7,824 source characters; its size is not a
measurement of startup cost.

The [contract](invocation-facade-prototype.v1.json) and
[transform](../woml-cli/scripts/invocation-facade-transform.ts) require the exact
reviewed source hash. The four captures were manually audited; there is no
claim of general free-variable analysis. Tests check the copied body and that
all remaining bytes outside the helper insertion/closure replacement are
unchanged. Any source drift requires re-review.

- **Retention:** no new host-retained preparation object, cache, key, hit/miss
  path, eviction or disposal API. Cache-specific phase items do not apply to
  this code-layout candidate; existing cache limits stay unchanged.
- **Lifetime:** private code and its invocation-bound closure live in one fresh
  worker. Context, secrets, module namespaces, results, handles and authority
  are never cached/shared. Module initialization still happens per worker.
- **Ordering:** `Object.values` secret snapshots and facade `Map` creation stay
  eager, before any module/user code. Delaying these until after user code could
  observe changed globals; that alternative was not implemented.
- **Effects:** dispatch checks, normalization, operation IDs, deadlines, pending
  replies, cancellation and Rust-side settlement rules stay on their established
  paths. The helper is not a new authorization boundary or OS sandbox.
- **Bounds:** no new runtime retention; the minified worker grows by 53 bytes.
  No prewarming or registration work is hidden outside measurement.

This is a narrow isolation argument and differential coverage, not a proof of
all adversarial behavior or native/installed failure equivalence.

## Build and independent reference

The [ISR0 acceptance contract](invocation-setup-acceptance.v1.json), fixtures,
collector and original receipt are unchanged. The
[driver](../woml-cli/scripts/invocation-facade-prototype.ts) requires reviewed
checkpoint ancestry and an exact match with the original production manifest.
Machine, Bun/executable, harness and artifact identities are recorded/checked.

Candidate source is generated through a diagnostic build plugin, never written
into `src/`. A separate generated tree passes TypeScript checking. Both variants
use the existing eight build entrypoints, minification and external maps.
Build and capture are separate explicit commands; capture never builds.

Relocation changes Bun's output-relative source paths and debug IDs. The
driver requires exact executable bytes except the final debug-ID comment;
maps may differ only in that ID and equivalent resolved source paths. Tests
reject executable or mapping drift. Both measured hosts and the measured
reference worker/maps are then copied from the **untouched original archive**.
Those hashes remain exact, not rewritten. Generated builds are also retained.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Host, both measured variants | 239,319 | `d81ca54095ff5d040c6ff2dd96ba0f18b43f45b56f57934d1271efac13e15cbf` |
| Independent reference worker | 57,513 | `af632902e850fdba61b7e40dd3c66e3cb21f32d797698e9abd788e83f6e83f08` |
| Prototype worker | 57,566 | `ad26b8caaaf3a13ccecb36fd4d9fe75775a94e74fa3f975273719952795df05b` |

Receipts additionally retain original/generated source, body/source-map hashes,
harness files/hashes and the baseline receipt. They are local provenance, not
independent signed attestations.

## Matched results

Bun 1.3.14, Linux x64, Intel i7-9750H, kernel `6.8.0-138-generic`.
Seven adjacent process pairs alternate reference/candidate order. The unchanged
collector runs eight original cohorts: one first request, one warmup batch and
three measured batches per host. **5,040 actual invocations across 112 hosts**
completed, including first requests/warmups. Each returned the required value
and passed worker-global/module-state freshness checks. No timing failure,
outlier or slow sample was discarded, retried or relabelled. No builds/tests
ran alongside timing.

Values are the frozen median of seven process medians, each from three measured
batches. Positive improvement means faster; negative means slower. Concurrent
batches measure overlapped throughput, not individual latency.

| Cohort | Invocations/batch | Reference ms | Prototype ms | Improvement |
| --- | ---: | ---: | ---: | ---: |
| Mixed primary | 32 | 303.52 | 311.46 | −2.62% |
| Inline | 8 | 72.38 | 75.22 | −3.92% |
| Repeated module | 8 | 80.55 | 86.18 | −6.99% |
| Eight distinct modules | 8 | 88.64 | 76.93 | +13.21% |
| Larger context | 8 | 92.56 | 92.30 | +0.28% |
| Larger module/map | 8 | 103.85 | 103.70 | +0.15% |
| Concurrency 2 | 8 | 44.94 | 46.65 | −3.81% |
| Concurrency 4 | 8 | 30.18 | 29.99 | +0.63% |

The distinct-module improvement cannot replace the declared mixed primary.
Distributions vary; this capture does not establish a causal explanation for
each control difference. The module regression stays flagged, not explained
away as noise. No repeated capture or higher budget was used to make it pass.

Do not compare the new reference with an older session and attribute differences
to this candidate. Dividing the primary batch by 32 gives a batch mean, not an
individual latency median or durable workflow benchmark. The distinct-module
fixture repeats eight bundles across batches, not sustained cache churn.

Allocation totals, scoped peak live bytes and scoped CPU remain **null**. Raw
process resource observations/headroom are retained, with unverified RSS units
labelled. This run certifies no cache-memory, native-addon, SQLite, managed-effect,
installed-workflow or cross-platform release budgets.

## Verification and evidence

- Generated candidate TypeScript checking passed.
- Existing host suite: **64 passed against the archived reference**, then
  **64 passed against the candidate**. Includes managed services, secrets,
  fresh state, module integrity/maps, cancellation, lifecycle, correction,
  progress and old/current protocol cases. Test-only worker selection makes
  in-process tests use the chosen built worker; subprocess tests use the chosen
  built host. The deliberately missing-worker fixture remains unchanged.
- Diagnostic contracts: **14 passed**, including transformation identity,
  relocated metadata, thresholds and rejection cases. TypeScript checking passed.
- All **34 architecture/modularization/source/CI contract tests**, documentation
  links/public-example validation and `git diff --check` passed. An offline
  audit rechecked all eight measured artifact hashes and ten harness hashes,
  recounted 5,040 completions and reproduced the summary from raw captures.

Retained under ignored `woml-cli/.woml/performance/`:

- `invocation-facade-build-Oanw3S/receipt.json`: qualified build and archives.
- `invocation-facade-capture-5K0gjQ/inputs.json` and `summary.json`: exact inputs,
  aggregate results and all process medians.
- The capture's `pair0-reference` through `pair6-candidate` directories: raw
  captures, logs and resources, with reversed order in odd-numbered pairs.

One build was refused before compilation due to insufficient available RAM.
After Chrome closed, headroom exceeded the unchanged 2 GiB requirement.
The retained `invocation-facade-build-QaJ4cj` attempt stopped at reference
qualification on relocated debug metadata, before timing. The qualified build
permits only the verified metadata relocation above, without altering the
independent archive. No failing timing was hidden by that correction.

From `woml-cli`, run serially with the original archive available:

```bash
bun run build:invocation-facade-prototype --baseline <original-ISR0-receipt.json>
bun run capture:invocation-facade-prototype --receipt <prototype-build-receipt.json>
bun run test:invocation-setup-contracts
```

For either built variant:

```bash
WOML_SCRIPT_HOST_TEST_ENTRY=<variant>/script-host.js \
WOML_SCRIPT_HOST_TEST_WORKER=<variant>/script-host-worker.js \
bun test tests/script-host.test.ts --max-concurrency=1
```

These are diagnostic/test controls, not production CLI options. Timing capture
is not added to routine CI; deterministic tests join the existing ISR contract
command in the cross-platform matrix.

## Handoff

Keep the production worker. ISR3 has **no proven change to integrate**. This
does not rule out all bootstrap optimizations; any new candidate needs its own
reviewed mechanism and matched evidence. Existing prepared-runtime locking,
installed inspection, earlier controls, resource attribution and native/platform
prerequisites remain open. This experiment does not close ISR4–ISR6 or certify
the combined performance branch for release.
