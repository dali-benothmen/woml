# Rust core modularization: RCM0 baseline and RCM1 gates

This is an internal refactor, not a language, schema, or runtime revision.
Branch: `refactor/rust-core-modularization`. Baseline source:
`e8a62490fd15c40c152c8440ff12fd2bdb75120f` (WOML 1.1.0).
The [machine-readable baseline](rcm0-baseline.json) records original contract
hashes/public types, fixture-tree identity, native exports, test targets,
and local tool versions. Its test inventory includes the new safety test;
no existing integration target was renamed.

## RCM0: results before extraction

| Serial check | Result |
|---|---:|
| Existing DAG, loop, rich approval, remote model, script-host contract suites | 34 passed |
| New public type paths and diagnostic-order tests | 4 passed |
| Native separation, including hidden private dependency rejection | 4 passed |
| Bun architecture and recursive source-reader tests | 6 passed |

No failure was observed in this selected baseline. This is not a claim that
the full release/platform matrix was run. Rust used `--locked --offline`,
one job, debug information/incremental compilation disabled, and serial
tests. Native separation used the debug test profile, not a release build.
Bun 1.3.14 was available, so script-host tests exercised the actual host.
Local rustc/cargo are 1.88.0; macOS/Windows and CI's newer Rust remain
independent verification gates.

## Frozen boundaries

Preserve root re-exports and direct `model`, `event`, and `protocol` imports.
`core_contract_paths.rs` names every original public type in these modules
and checks root/direct-module type identity where a root export existed.
It also freezes multiple-issue ordering and the first diagnostic message.
The native crate remains an actual compile-time consumer of the engine.
JavaScript exports remain governed by `verify-native-exports.ts`; neither
its expected list nor the native bridge implementation changes in RCM1.

Preserve Serde attributes, enum order (especially untagged failures), version
dispatch, JSON shapes, hash inputs, constructor behavior, and borrowed message
lifetimes. Keep ordinary/durable executable validators distinct. No model,
event, host, package, or store version is raised.

## Source guards and discovery

| Guard | Sources it inspects | Treatment |
|---|---|---|
| Native separation (Rust and Bun) | Native `src` | Scan all nested Rust files, retaining the existing prohibited-import assertions. |
| Infrastructure Policy release | Capability/durable/event/policy/runtime/bridge | Read each Rust facade plus its private implementation directory; keep all marker assertions. |
| Streaming Output release | Output/process/runtime/artifact/capability/bridge; event vocabulary | Include nested event payloads in negative output-vocabulary checks. |
| Waits/locks/compensation/rich approvals | Durable/runtime/feature/bridge source readers | Recorded for update before those later extractions. |
| Runtime activation/ownership/observation | Webhook/durable/bridge | Recorded for update before later extractions. |
| Native and package release | Exact exports, actual addon, package allowlist | Do not relax or rebaseline because of a file move. |

`scripts/rust-source.ts` reads a facade and its same-named private tree in
deterministic order and rejects symlinks. Negative tests hide forbidden
content in a nested child. Native checks scan the whole source directory.
These are source guards, not a complete Rust module resolver; keep private
implementation inside its owned directory and review unusual path attributes.
The native `crate::` prohibition is retained; legitimate native-local import
changes, if needed, belong to reviewed RCM13 guard work.

Preserve Cargo's top-level integration entrypoints and CLI shard discovery.
Important unit filters include `process::tests` and `artifacts::tests`;
relocation must never produce a falsely green zero-test suite. The three
RCM1 modules contain no inline unit-test modules or relative `include_str!`
fixtures; all integration fixtures retain their paths.

## Ownership and transaction map: unchanged by RCM1

| Boundary | Owner / reading stop | Regression evidence |
|---|---|---|
| Event validation, sequence, commit, then cache publication | `DurableEventStore::append_payloads_scoped_atomically` | `durable_event_store` rollback and cross-connection cache tests. |
| Startup migrations versus prepared request opening | `initialize` / `open_ready` | Migration/future-schema and webhook contention cases. |
| Occurrence deduplication and clock cursor movement | Durable admission and schedule/interval claim transactions | `trigger_occurrence`, `schedule_runtime`, `interval_runtime`. |
| Approval/vote/notification/artifact-pin settlement | Durable approval/rich-review transactions | Approval resolution, notification, rich approval authority suites. |
| Wait registration, early signals, terminal winner | Wait/signal transactions | `durable_wait_runtime`. |
| Cross-run lock handoff and greater fence | Shared durable lock authority | `locks_and_leases`. |
| Forward success and compensation registration | Atomic append/correction registration | Compensation runtime/store suites. |
| Per-run recovery and externally resolved decisions | Recovery transaction; preserving correction rebuild | Durable event, correction, backup, and retention suites. |
| State mutation and immutable result proof | Durable state mutation transaction | State store/hardening suites. |
| Policy capacity and live ownership | Store claim/owner plus runtime coordinator/lease | Policy scheduler and runtime ownership/hardening suites. |
| External effects and ambiguity | Capability authority plus host/process/remote supervision | Capability, script-host, worker, and dispatch tests. |

Keep `Box::pin` boundaries around continuation, lifecycle recursion, forks,
parallel children, and host execution. RCM1 does not change future shape,
mutex/lease scope, cancellation polling, drop guards, or worker ownership.
Prior stack overflow/cancellation regressions make those later changes risky.

## RCM1 review

The stable facades explicitly re-export original types and functions.
Model contracts, expressions, policies, graph lookups, diagnostics, central
validation orchestration, and feature validators have separate private homes.
Event envelope and dispatch enum stay central; payload families, identities,
failure types, and ordered version validation are private implementation.
Host control, invocation, bindings, modules, outcomes, capabilities, and
operational messages are separate protocol modules.

Only required internal sibling access is widened to `pub(super)`; no private
helper becomes public. In particular, model error construction and protocol
context constructors remain outside external consumers' API.

Repeat contract/path tests after extraction; broaden coverage with approvals,
fork/choice/parallel/retry/lifecycle history, waits, and remote contracts.
Check formatting, the native consumer, affected release guards, and TypeScript.
Actual addon loading, clean installed-package tests, and platform release
lanes remain independent gates; do not label those passed from a source check.

The developer book keeps stable facade links and adds implementation routes.
Its generated atlas is a tracked-source snapshot; additions must not be made
invisible merely because local source files have not been committed yet.

## RCM1 results after extraction

| Check | Local result |
|---|---|
| Primary model/event/protocol regression set (13 targets) | 78 passed |
| Additional provider/model/policy/store/wait/correction set (9 targets) | 54 passed |
| Native separation rebuilt against the extracted engine | 4 passed |
| Frozen type/export/fixture inventory and source scanner checks | 10 passed |
| Portable CLI contract shard | 17 files passed |
| Engine and native consumer compilation | Passed |
| Established engine Clippy gate | Passed with its existing policy; no new allowances |
| TypeScript typecheck | Passed |
| Infrastructure Policy and Streaming Output source release verifiers | Passed with original assertions intact |

The contract inventories contain 48 model, 126 event, and 40 protocol public
types, 68 native export names, and 96 engine integration targets (including
the new guard). All 356 reviewed text fixtures retain their canonical LF
contents; the test permits Git's Windows CRLF checkout conversion, not a
content change. Future intentional fixture changes require explicit review
of this refactor baseline rather than silently refreshing it.

A one-time mechanical comparison against the base revision audited 396
declaration/method token streams. Attributes, literals, fields, enum order,
and function bodies were unchanged, allowing whitespace, Rustfmt's optional
argument trailing commas, and the documented parent-only helper visibility.
The checker used for this comparison is not a general Rust parser or a
replacement for contract tests.

The former 5,479-line model, 3,643-line event, and 1,242-line protocol files
are now 80-, 200-, and 33-line facades, respectively, with 47 private modules.
This is organization, not a reduction of runtime functionality or a measured
performance improvement. The central event vocabulary and ordered validation
remain intact. The CI engine baseline explicitly includes the new
`core_contract_paths` target; CLI shard discovery includes the new guard tests.

Book source routes and edition metadata describe this working-tree overlay;
the atlas includes explicitly reviewed additions without staging unrelated
notes. The user's `developer-book/` ignore rule is preserved. Cross-platform
release execution, installed-package journeys, actual rebuilt-addon export
loading, and the full remaining feature matrix were not run locally.
