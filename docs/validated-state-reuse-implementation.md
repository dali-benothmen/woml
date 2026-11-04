# Validated execution state reuse implementation

VSR2/VSR3 reuse one complete structural state after all existing definition
checks succeed. This removes an immediate duplicate fold at three reviewed
boundaries; it does not skip graph checks, extend cache freshness or certify
later writes. Work continues on `perf/runtime-performance` after the
[readiness indexes](readiness-indexes.md).

The [verification record](validated-state-reuse-verification.md) covers the
subsequent safety and performance work. The
[original acceptance contract](validated-state-reuse-acceptance.v1.json) remains
unchanged. Allocation/peak-live and platform evidence are separate shipping
gates. The earlier readiness-index speedup must not be attributed to removing
this fold.

## Checked result and error order

[`ValidatedHistory`](../core/woml-engine/src/engine/history/validated.rs) owns the
complete `ProjectionFoldState` and borrows the exact events, compiled definition
and definition-hash inputs. Construction and fields are private to the history
validator. Consumers can borrow its projection or consume the witness to move
out the state/projection. There is no Clone, serialization, mutable projection
accessor, database token or external API.

The complete structural fold still runs first. Definition checks and historical
prefix replay follow in their existing order; a late structural corruption
still wins over an earlier readiness error. The result includes the suffix
after the last prefix requested by the checker. Failure drops all candidates.
The validation-only entrypoint remains a compatibility wrapper returning unit.
The opt-in `runtime.fold_events` span still describes the full structural pass.

Existing callers still validate models at their normal loading/construction
boundary. An empty checked history does not certify a model, start a run or
grant authority. Consuming the fold state destroys the witness; subsequent
mutation still needs the existing append checks.

## Converted boundaries

| Consumer | Exact snapshot and consumption | Unchanged safety boundary |
| --- | --- | --- |
| `DurableEventStore::projection`, cache miss | One loaded immutable event vector and loaded definition/hash; public projection is cloned from the checked state, then the raw state is moved into the existing cache | Indexed JSON loading, workflow-ID enrichment, immutable rich-review hydration, error mapping and cache publication order. Cache hits, 256-entry bound, terminal eviction and count/tail freshness remain unchanged. |
| `register_durable_wait_atomically` | Check the resulting local slice after compatibility-aware append and consume its projection before further work in the same immediate transaction | Registration-state checks, wait index insertion, early-signal processing, sequence allocation and commit. A later early-signal append is not certified by the old witness. |
| `try_claim_policy_run_at`, initial pair only | Check the initially loaded transaction-local history and consume its projection before claim/capacity decisions | Owner/lease checks, capacity/rate rules, ambiguity checks, first-start/lifecycle appends and complete final validation of the resulting history before commit. |

Structural errors still map to the existing Contract error through the history
validator. The former immediate second fold could not produce a different fold
error after successful validation of the identical immutable slice: projection
folding is event-driven, not dependent on an external clock or database read.
No new error category replaces a reachable old failure.

There is no added database connection, await, transaction widening or cache
publication inside these borrowed checks. Compatibility enrichment and review
hydration affect only the public clone, not the raw cached fold state. A failed
hydration cannot publish a new cache entry.

## Deliberate exclusions

- Incremental append and batch-commit paths keep their existing candidate apply,
  final full history validation, SQL/index commit and cache publication.
- Timer/signal-timeout settlement branches that roll back and reload remain
  unchanged. A pre-rollback result is not proof for a reloaded slice.
- Approval, notification, correction and outcome transactions are not swept
  into this conversion. Their independent authority and append paths remain.
- In-memory stores retain incremental publication and global event-ID checks.
  Replacing cheap incremental apply with a full checked result is not justified.
- Separate scheduler projection/readiness reads remain separate: another
  writer, cancellation, timeout or approval may intervene.
- No persistent/global cache, snapshot schema, future-append certificate,
  native export, dependency, public format or budget changes were introduced.

## Replay and failure regressions

The independent VSR1 reference retains the old complete structural gate, original
scan readiness and complete prefix replay. Production comparisons cover all
recorded/historical prefixes, exact errors, bindings, unsupported versions,
suffix/scoped events and state consumed for later append bookkeeping.

Test-only thread-local counters observe `ProjectionFoldState::from_events`
within one selected synchronous operation. They are not global profiling-span
counts or allocation estimates. The real durable boundary tests assert:

- A cache miss folds once; its ordinary hit folds zero times. Terminal results
  remain uncached. Indexed JSON corruption still fails before construction.
- A policy first claim folds the initial one-event snapshot once and separately
  validates the two-event result. Reattachment and a competing-owner check each
  fold their exact current snapshot once.
- Wait registration retains append/summary/policy-index folds and final checked
  validation, removing only the following duplicate replay. A forced wait-index
  insert failure rolls back the already-appended events and leaves cached truth
  unchanged; retry with the same event identity succeeds.
- Hydrated title/risk/artifact fields stay out of the raw cached approval
  result, which retains its event-recorded review digest. A missing immutable
  review fails on hits and misses, and a failed miss publishes no state.

The separate [review overlay](refactoring/validated-state-reuse-repairs.json)
pins the three changed consumer declarations. Original refactor snapshots,
startup-separation repairs and previous compatibility fixes remain unchanged.

```bash
bun run --cwd woml-cli test:validated-state-reuse
bun run --cwd woml-cli test:rust-core-modularization-contracts
```

The focused command is also wired into the existing Linux/macOS/Windows
Combined Developer Experience matrix. Captures are not normal CI timing gates.

## Memory and remaining acceptance

The witness adds only input references and moves the already-computed state.
It does not clone events/definitions/state or hash them again. The existing
public cache-miss view still needs its one clone for enrichment/hydration.

One complete state now remains live while definition checking builds its prefix
state. This can increase peak live memory; counting fewer folds does not prove
lower peak memory or a latency win. Allocated bytes, retained-state peak, matched
post-index optimized timings, native/installed controls and actual platform CI
still need the VSR0/VSR4/VSR5 work. Existing acceptance values are not relaxed or
treated as passed by these implementation tests.

## Local verification

Implementation checkpoint: `471e926` plus this uncommitted VSR2/VSR3 patch, on
Linux with the repository's serial development build configuration.

- All **169 engine unit tests** pass; three explicit measurement captures are
  ignored in the ordinary suite. This includes the new constructor/reference,
  replay-count, hydration, corruption and rollback cases.
- All **41 selected integration tests** pass: `runtime_policy_store` (13),
  `durable_wait_runtime` (14), `rich_plan_approval_authority` (12) and
  `rich_plan_approval_runtime` (2).
- All **34** modularization/source/architecture/CI guards and **7** frozen
  baseline/tooling contract tests pass, along with TypeScript checking.
- The established engine Clippy policy passes without new exemptions. Rust
  formatting, documentation/public-example validation and diff checks pass.

No optimized VSR candidate timing, allocator/peak-live capture, fresh native
addon/package journey or actual macOS/Windows run is claimed by these results.
