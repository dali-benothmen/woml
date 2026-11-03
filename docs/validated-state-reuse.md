# Validated execution state reuse: inventory and baseline protocol

Status: VSR0/VSR1 preparation. No production validation/fold/cache behavior is
changed. Continue on `perf/runtime-performance`. The reference checkpoint is
`d9338480f1888808ddcc9d349d97ff0f29e77b40`, **after** startup separation.
Earlier startup-separation performance/platform acceptance remains separate.

## Exact caller inventory

`F(N)` denotes a complete structural fold; definition validation also performs
its existing monotone prefix work. A redundant fold saved is not all validation
removed, nor a promise to halve total workflow time.

| Function / path | Exact input and boundary | Errors / publication | Decision and possible saving |
| --- | --- | --- | --- |
| `DurableEventStore::projection`, cache miss (`durable/events/cache.rs`) | Same local `events`, loaded definition and binding hash; no explicit read transaction around this method today | Loader indexed/JSON checks; validator errors become `DurableStoreError::Contract`, second fold errors retain `Fold` mapping; clone/enrich/hydrate before cache insertion | First VSR2 target: replace the second `from_events` only, saving one `F(N)`. Preserve current read consistency; do not claim this adds a transactional snapshot across its separate SQL reads. |
| `DurableEventStore::projection`, ordinary cache hit | Existing per-connection entry, count and last-event identity | Still clone/enrich/hydrate; no definition history pass on this branch | Control, not a conversion. Zero full folds today. Keep bounds (256), terminal exclusion and existing freshness limits. |
| `register_durable_wait_atomically` (`durable/transactions/waits.rs`) | Resulting local history **after** compatibility-aware append, same definition/hash, one immediate transaction | Pre-version/payload checks unchanged; validator maps to `Contract`, following fold to `Fold`; wait/index checks before commit | Later VSR3 candidate: one immediate `F(N)` only. Do not remove the helper's other required validation. |
| `try_claim_policy_run_at` (`durable/transactions/policy.rs`) | Loaded local events in the immediate claim transaction after owner/lease/policy prechecks | Validate then fold before capacity/status/claim decisions. Later execution-start/lifecycle appends require their own final validation | Later VSR3 candidate: one `F(N)` on the initial pair, not reusable for the later appended history. |
| `settle_timer_wait` / `settle_signal_wait_timeout` (`waits.rs`) | Validate resulting local history, but duplicate/race branches can roll back and reload events | Some returned projections come from a **different reloaded slice** after rollback | Excluded from first delivery. Audit each branch separately, never reuse a pre-rollback witness for a reloaded history. |
| `append_payload_scoped` / `append_payloads_atomically` (`events/commits.rs`) | Existing incremental candidate plus final definition validation in one writer transaction | SQL/events/indices commit before cache publication; failed candidate may already be mutated | Preserve incremental apply and final validation. Not a blanket validate-then-fold replacement. |
| `InMemoryDagEngine::append_event` + `InMemoryEventStore::append` | Engine readiness checks and candidate history; store additionally checks globally unique event IDs | Candidate cloned/applied before publication; global duplicate checks independent of one-run folding | Exclude initially: replacing incremental store apply with a full result/clone can cost more and bypass another guard. |
| `DurableDagEngine::ready_node_ids*`, runtime `run_loop` | Projection/readiness can read separately; another writer or cancellation can intervene | Authoritative reads/clock checks cannot be merged on an assumed shared snapshot | Explicitly excluded; requires separate scheduling/freshness design. |
| Public rich-review hydration (`events/cache.rs`) | Queries immutable review records and enriches a cloned public projection | Missing review/output remains an error before cache publication | Not part of the raw result. Preserve position, errors and view semantics; test separately in VSR4. |

The result/lifetime/error specification is in the
[private protocol](protocols/validated-history-internal-v1.md). No cache entry,
startup token or projection-only value may manufacture a validated result.

## Frozen measurement cohort

The machine-readable parameters are
[`validated-state-reuse-acceptance.v1.json`](validated-state-reuse-acceptance.v1.json).
Freeze them before VSR2, keep this baseline binary separate, and retain raw
samples and unsuccessful batches. Never tune the cohort after seeing a gain.

- Synthetic active histories: exactly **128, 512, 2,048** Event v1 events,
  sequential definitions with 64/256/1,024 nodes. Each ends in a valid started
  final attempt. It is deliberately nonterminal so normal cache hits occur.
- Deterministic fixture generation, canonical model hashing, complete model/
  history validation, SQL seeding and indexed-row verification are outside
  timing. Original fixtures are not rewritten. Direct SQL seeding is a private
  test setup, not a new production ingestion path.
- Seven independent processes per artifact. Each length has two warmup
  connections, five measured connections, each with its natural first cache
  miss and five subsequent hits. Never clear/disable a global cache to create
  a win. Opening is outside the scoped projection timer and remains a separate
  startup control.
- Time the actual `DurableEventStore::projection` call including load, binding,
  model/history checks, public clone and normal cache publication. A separate
  pure fold sample is attribution, not evidence of an end-to-end gain.
- Unix captures thread CPU via its thread CPU clock. Unsupported clocks report
  null, never wall-as-CPU. Warmups and raw rows stay in the report. Use the
  median of process medians; process batches, not individual repeated reads,
  are independent units. With seven batches, report ranges, not robust p95.
- Later matched comparison: B/C then C/B alternating across seven pairs;
  preserve baseline and candidate source/binary/fixture/compiler/host identities.
  No builds, installs or unrelated suites during accepted captures. Traced
  attribution is a separate capture and cannot enter timing summaries.

Primary target: at least **10% lower median scoped thread CPU** for the 2,048
event cache miss, with unchanged errors/state and one full structural fold
removed. If baseline attribution indicates another fold is too small a cost,
narrow/defer rather than weaken the target after production implementation.

Investigate any >5% median regression in a control, >5% allocated-byte increase,
or >16 MiB own-process RSS/peak-live increase. Investigation thresholds are not
automatic permission to regress or replacements for existing product budgets.
Overlapping per-process ranges make wall-time findings inconclusive; do not
call seven process maxima a trustworthy p95. Peak allocation/retained-state
capture must distinguish the complete state and simultaneously live prefix state.

### Counters and honest attribution

The unchanged source establishes two complete folds on the selected miss and
zero on its ordinary hit; one explicit public projection clone per path, plus
transition-owned clones/allocations. These are **source-derived counts**, not
measured allocator totals. A traced isolated process also records
`sqlite.project_run` hit/miss, `runtime.fold_events` and
`runtime.history_prefix_cursor` prefix request/visit counters. The direct second
`from_events` has no fold span today: do not incorrectly count spans as total
structural passes or attribute overlapping global spans to a single caller.

The baseline tool deliberately leaves allocator totals and peak live bytes
null until a real allocation capture is available. It must not claim that RSS
or serialized fixture size substitutes for them. **VSR0 remains open without
that capture and the native controls.**

## Build and capture

From the repository root:

```bash
# Portable protocol/tooling tests (no native build):
bun run --cwd woml-cli test:validated-state-reuse-contracts

# Serial Rust correctness tests (ordinary development build):
bun run --cwd woml-cli test:validated-state-reuse

# Explicit optimized test-binary build, separate from timing:
bun run --cwd woml-cli build:validated-state-reuse-baseline

# Paste the printed receipt path. Capture NEVER invokes Cargo:
bun run --cwd woml-cli capture:validated-state-reuse-baseline --receipt /absolute/path/receipt.json
# Separate attribution, not accepted timing:
bun run --cwd woml-cli capture:validated-state-reuse-baseline --receipt /absolute/path/receipt.json --trace
```

The build pins HEAD plus exact tracked/untracked source bytes under `core`,
`woml/src`, `woml/tests/fixtures` and `woml-cli/src`, the lockfile, standard release profile, compiler,
and harness/acceptance bytes before and after compilation. It copies the exact
test executable to an ignored receipt directory. The capture rechecks these
identities and binary bytes; it refuses resource pressure (<2 GiB available RAM
or disk), profiler contamination and source changes. This is local provenance,
not a signed independent build attestation. Subprocess resource getters are
materialized explicitly and BigInt CPU totals retained. RSS remains raw with an
unverified runtime unit; it cannot satisfy a byte budget until its unit is
verified. The first cohort's resource-serialization omission and reporting-only
repair are recorded in the baseline report without replacing its raw samples.

## End-to-end controls required before implementation

Keep the post-startup-separation optimized **native** artifact independently
receipted. The Rust microbenchmark executable is not a N-API runtime or clean
installed CLI. Capture real one-shot and activated short/sequential workflows,
for-each work, StateService and storage calls, retained-run stores, first-request
and shutdown controls. Active history length and unrelated retained runs are
different dimensions. Reuse the existing session/profiling harness where it
supports the case; unsupported cases need explicit fixtures, not substitute
timer delays. Do not use the pre-startup-separation binary or npm installation
as this proposal's baseline.

See the [baseline record](validated-state-reuse-baseline.md) for what actually
ran and what remains open. VSR1 reference tests do not close VSR2–VSR5 gates or
earlier startup-separation acceptance.
