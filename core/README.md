# Working on the WOML Rust core

Rust owns execution and durable truth. The TypeScript frontend compiles WOML;
Bun hosts execute bounded JavaScript invocations. A host, HTTP listener,
renderer or native callback must not become another workflow scheduler.

## Two crates, stable public doors

- [woml-engine](woml-engine/src/lib.rs): model validation, graph decisions,
  projections, runtime coordination, durable storage and governed effects.
- [woml-native](woml-native/src/lib.rs): N-API conversion and resource-lifetime
  adaptation. It depends on the engine, not retired orchestration code.

Keep public facades such as `runtime.rs` and their existing root re-exports.
Implementations belong in private same-named directories. Prefer explicit
imports/re-exports and the narrowest family-scoped visibility. Do not publish
private modules, introduce wildcard exports, or scatter shared utilities
merely to shorten a file. Avoid `#[path]` and `include!` implementation escapes:
source guards intentionally require owned trees. Text/binary fixtures may use
`include_str!`/`include_bytes!`.

## Where to change what

| Responsibility | Start here |
|---|---|
| Compiled shapes and ordered validation | [model/](woml-engine/src/model.rs) |
| Event envelope/payloads and version validation | [event.rs](woml-engine/src/event.rs), its private children |
| Host messages, bindings and bounded protocol | [protocol/](woml-engine/src/protocol.rs), [host.rs](woml-engine/src/host.rs) |
| Graph readiness, routes and history binding | [engine/](woml-engine/src/engine.rs) |
| Global event gate and feature transitions | [projection/fold.rs](woml-engine/src/projection/fold.rs), `fold_*.rs`, `iteration_fold.rs` |
| Run scheduling, execution families, lifecycle, correction | [runtime.rs](woml-engine/src/runtime.rs), `runtime/scheduling/`, `control_flow/`, `lifecycle/`, `correction/` |
| Runtime storage adaptation | [runtime/engine_adapter/](woml-engine/src/runtime/engine_adapter/mod.rs); scoped iteration adapter in `control_flow/for_each/shared.rs` |
| Store startup/layout and supported upgrades | [durable/opening.rs](woml-engine/src/durable/opening.rs), `schema.rs`, `migrations.rs`, `validation.rs` |
| Atomic history, related indexes and cache | [durable/events/commits.rs](woml-engine/src/durable/events/commits.rs), `history.rs`, `cache.rs` |
| Feature transactions and ordered recovery | [durable/transactions/](woml-engine/src/durable/transactions/mod.rs), [recovery.rs](woml-engine/src/durable/recovery.rs), `recovery/` |
| Handler selection versus permission to perform an effect | [capability/registry.rs](woml-engine/src/capability/registry.rs) versus [authority.rs](woml-engine/src/capability/authority.rs) |
| Process policy, tree lifetime and redacted draining | [process/](woml-engine/src/process.rs), `containment.rs`, `platform/`, `output/` |
| Artifact metadata permission versus verified backend bytes | [artifacts/authorization.rs](woml-engine/src/artifacts/authorization.rs) versus [backend/](woml-engine/src/artifacts/backend/mod.rs) |
| Remote journal/preparation/execution and coordinator commits | [remote_worker_journal/](woml-engine/src/remote_worker_journal.rs), [remote_dispatch/transactions.rs](woml-engine/src/remote_dispatch/transactions.rs) |
| Ingress prepare/activate/stop, auth, admission and clock arrivals | [webhook.rs](woml-engine/src/webhook.rs), `webhook/` |
| Durable notification journey versus transport | [notification_runtime/](woml-engine/src/notification_runtime.rs), explicit notification hosts/protocols |
| Native configuration, errors, callbacks, registries | [bridge/](woml-native/src/bridge.rs); exported bodies stay in `bridge.rs` |

These are responsibility boundaries, not a claim that the current Rust import
graph is a perfectly one-way layering. Keep the two-crate workspace rather
than extracting a crate for every feature without a separate dependency design.

## Rules that a file move must preserve

1. **One durable owner.** Private inherent `impl DurableEventStore` blocks use
   the same connection and projection cache. A helper borrowing a transaction
   must not acquire an independent connection/commit inside it. Keep immediate
   transaction scope, validation, sequence allocation, SQL/index updates,
   rollback and commit-before-cache-publication ordering. A rejected tentative
   projection is not truth, even if applying an event already mutated it.
2. **Startup is different from a request.** `open` initializes/migrates;
   `open_ready` assumes prepared startup. Do not move DDL, rebuilds or general
   interrupted-run recovery into the prepared request path.
3. **Wakeups are not authority.** Preserve claims, leases, fencing tokens,
   mutex scope, capacity guards, heartbeat/renewal and Drop ordering. Timer
   trigger production is not DAG eligibility. Remote workers report one exact
   attempt; only the coordinator schedules and settles the workflow.
4. **Context is a scoped snapshot.** Build it from accepted history and compiled
   visibility, including the correct fork-opening snapshot. A result that
   finished earlier is not automatically visible to a sibling branch. Preserve
   ordinary, iteration, lifecycle, compensation and cleanup bindings. Runtime
   options and resolved secrets are not ordinary workflow input.
5. **Cancellation is truthful settlement.** Preserve polling/deadlines, host,
   process-tree and worker ownership, and correction/cleanup obligations.
   Unknown external effects remain ambiguous, not automatically retryable.
   Proof of a state mutation does not prove the rest of a script completed.
6. **Future size matters.** Keep intentional `Box::pin` boundaries around
   recursive/large continuations and ordinary-stack regression tests. Increasing
   stack size is not a structural refactor or a proof of correctness.
7. **Observations are not workflow commits.** Capability lookup is not durable
   permission. Operational output, telemetry and presentation cannot settle a
   run. Artifact SQL cannot roll back already-published backend bytes: failed
   settlement can leave an orphan, never logical success.
8. **Native lifetimes are observable.** Preserve all existing N-API names,
   parameter order, attributes, Promise/synchronous behavior and error envelopes.
   Keep one webhook registry and one remote-coordinator registry. Stop removes
   the owner before signalling/joining. Ordinary callbacks retain unbounded/
   `Blocking` delivery; output retains one-slot `NonBlocking` coalescing;
   enabled telemetry retains bounded 1,024-slot `NonBlocking` delivery. Preserve
   atomics and `unref`; these lanes are not interchangeable.

## Adding a feature or making a correction

Begin with a concrete failure/behavior and identify its authority. Trace source
compilation, model, protocol, event, projection, runtime and storage only where
the feature crosses those boundaries. Version serialized changes explicitly;
do not raise versions for a file move. Add a regression at the real boundary,
including duplicate delivery, interruption and rollback where applicable.

Keep existing integration entrypoints under `tests/`. Existing filtered unit
paths such as `process::tests`, `artifacts::tests` and `webhook::tests` must run
real tests, not silently become empty. Read relative fixture paths after moves.
Do not rewrite golden files, thresholds or unsupported-future-version checks
to hide failures. Isolate behavioral fixes from mechanical reorganization.

## Verification without exhausting a developer machine

From the repository root, run selected checks serially:

```bash
cargo fmt --manifest-path core/Cargo.toml --all -- --check
cargo test --config profile.dev.debug=0 --config profile.dev.incremental=false -j 1 --locked --manifest-path core/Cargo.toml -p woml-engine --test core_contract_paths -- --test-threads=1
cargo test --config profile.dev.debug=0 --config profile.dev.incremental=false -j 1 --locked --manifest-path core/Cargo.toml -p woml-native --test separation -- --test-threads=1
bun run --cwd woml-cli test:rust-core-modularization-contracts
```

Choose the real feature tests as well; this small selection is not the full
release gate. Use `--offline` when dependencies are already cached. Check free
memory/disk before starting a native build; do not run simultaneous Cargo or
release builds. The established Clippy policy is
`bun run --cwd woml-cli test:communication-provider-clippy`.

After building/staging a native addon, verify its actual JavaScript interface:

```bash
bun run --cwd woml-cli test:rust-core-native
# Or provide a reviewed library explicitly without replacing the staged addon:
bun woml-cli/scripts/verify-rust-core-native.ts path/to/library
```

The optional second library argument compares before/after surfaces and selected
execution/lifecycle contracts. N-API `.length` alone is not positional arity
proof; source signatures and real calls matter. Loaded Windows DLLs can remain
locked until process exit; the verifier tolerates only the exact addon leftover.

For local clean-package tests only, `WOML_TEST_NATIVE_ARTIFACT` may point to a
reviewed debug addon instead of the default staged release artifact. This is a
test-helper input, not a production runtime bypass. Debug execution evidence does
not certify optimized-release size/speed or another OS.

The [verification record](../docs/refactoring/rust-core-modularization.md) and
its baseline JSON files explain preserved contracts, known failures and unrun
gates. [CI](../.github/workflows/ci.yml) builds and tests the combined experience
on Linux/macOS/Windows; [release automation](../.github/workflows/release.yml)
builds the six frozen native targets. Neither a source scan nor one Linux run
proves the entire matrix passed. See [CONTRIBUTING](../CONTRIBUTING.md) for
repository-wide setup, reviews and release responsibilities.
