# Rust core modularization: developer handoff and release checklist

## Decision and current status

The structural refactor is implemented through RCM13. RCM14 adds a tracked
[core contributor guide](../../core/README.md), stronger source/API guards,
cross-platform verification wiring and this final ownership map. The code review
base for this checkpoint is `d243de4` (the RCM12/RCM13 commit).

**RCM14 release sign-off remains open.** Local Linux results are not a claim of
successful macOS/Windows, all six native architectures, optimized-release
budgets or the complete affected test matrix. Existing baseline failures also
remain unresolved. Do not tick the phase's final exit criteria until those
gates have actual results. No release tag, publication, remote CI dispatch,
commit or push is part of this handoff.

The [phase-by-phase verification record](rust-core-modularization.md) retains
commands, preserved boundaries and independently established baseline failures.
This page is the concise final map, not a replacement baseline or a blanket
green-CI assertion.

## Before and after: where the responsibility moved

Physical line counts include comments/tests and measure navigation, not speed,
binary size or correctness. Before values are the pre-extraction source
snapshots recorded by RCM0–RCM12, not fabricated counts of a new architecture.
After values refer to this checkpoint, before any future behavior work.

| Stable source | Before → after lines | Private responsibility owners |
|---|---:|---|
| `durable.rs` | 18,315 → 181 | Opening/layout/upgrades, records, immutable definitions, event transactions/cache, indexes/inspection, feature transactions, recovery and DAG adapter. |
| `runtime.rs` | 14,065 → 1,045 | Options/clocks/context/hosts, scheduling, execution families, lifecycle, cancellation, failure reconstruction and correction. Public orchestration entrypoints remain. |
| `artifacts.rs` | 6,249 → 93 | Contracts/policy, logical store/authorization, transfer, local/S3 verified bytes, materialization, maintenance, output rollover, backup and tests. |
| `model.rs` | 5,479 → 80 | Contracts/expressions, graph queries, ordered diagnostics and named feature validators. |
| `webhook.rs` | 4,029 → 335 | Prepare/activate/stop stays central; request/auth/response/admission, signals/events/provider ingress, clock production, routing/wait scans and dispatch have named owners. |
| `projection.rs` | 3,919 → 114 | Contracts/context and typed transitions; the central fold gate preserves global ordering and identity validation. |
| `event.rs` | 3,643 → 200 | Payload families, identities, failures and version checks; envelope/dispatch remain central. |
| `engine.rs` | 3,069 → 209 | Readiness, routes, context resolution, history validation and invocation/effect identities. |
| `process.rs` | 2,967 → 69 | Policy/executable pinning, request validation, supervision, platform containment, pipe draining/redaction and tests. |
| `remote_worker_journal.rs` | 1,394 → 36 | Journal evidence versus package/permission preparation, local secrets/grants and supervised attempt execution. |
| `remote_dispatch.rs` | 1,236 → 24 | Contracts/layout/identity/validation and coordinator transactions extending the existing store. |
| `notification_runtime.rs` | 984 → 28 | Durable journey, built-in/custom adaptation, retry waits and diagnostics. |
| `remote_coordinator_server.rs` | 586 → 172 | Listener/shutdown/capacity stays central; connection identity/routing, HTTP framing and TLS trust are private. |
| Native `bridge.rs` | 3,721 → 2,673 | Frozen N-API exports stay central; configuration, wire records, errors, callbacks, registries, remote preparation and validation move privately. |

Use the [core guide's source map](../../core/README.md#where-to-change-what)
to navigate these paths. The workspace still contains two crates. There is no
new language/model/event/store/host version, dependency, public implementation
module, per-feature database owner or alternate workflow scheduler.

### Why some files are still large

`bridge.rs` retains all 68 exported bodies to avoid changing generated N-API
registration/signatures. `durable/recovery.rs` retains one ordered recovery
coordinator; extracting independent recovery loops would obscure precedence.
Existing `presentation.rs`, `infrastructure_policy_runtime.rs` and
`operational_output.rs` remain coherent read-model, deny/evidence and bounded
output-store authorities. Their size is not a reason to scatter atomic decisions.

The RCM12 audit explicitly records 35 retained files and their hashes. Further
splits need a new ownership/consumer boundary and targeted evidence, not an
arbitrary maximum number of lines.

## Compatibility evidence and guard limits

- Root and direct-module Rust imports remain compile-tested; original contract,
  fixture, DDL and ordered algorithm fingerprints remain frozen. No old baseline
  is regenerated to conceal a mismatch.
- The original RCM0 fixture aggregate accidentally included seven ignored,
  generated `woml-env.d.ts` files, so CLI regeneration could fail it without a
  tracked-source change. [rcm14-fixtures-baseline.json](rcm14-fixtures-baseline.json)
  instead records all 349 tracked author fixtures directly from the exact
  original pre-refactor Git tree. Each file/hash and the aggregate are checked;
  only the seven named generated paths are permitted outside that inventory,
  and unexpected additions/symlinks fail. The original RCM0 record is retained.
  A separate comparison found zero differences across all 349 original fixtures.
- Public implementation modules and unreviewed re-export leaves are rejected.
  Ordinary visibility review is still required: regex inventories are not a
  complete rustdoc/API diff, and scoped-visibility normalization is not proof
  of every helper's exact visibility.
- Source readers scan the facade and same-named owned tree, including private
  descendants, reject symlinks and now fail on `#[path]`/conditional `path`
  attributes and `include!` implementation escapes. Fixture includes remain
  supported. This is a conservative source policy, not a Rust module resolver.
- Actual addons are loaded and called. Exact 68 names, function types and
  `.length` supplement, not replace, source signatures and selected positional
  sync/async/error/callback/lifecycle calls.
- Test entrypoint/filter discovery remains explicit. Boxed-callee inventories
  supplement, not replace, ordinary-stack runtime and cancellation tests.
- Clean installed-package tests can select a reviewed debug addon with the
  **test-only** `WOML_TEST_NATIVE_ARTIFACT` helper input. Production loading and
  the default CI release-artifact path are unchanged. A debug result does not
  certify release-profile performance or binary size.

## CI ownership and remaining sign-off

No new parallel release build/job was added. Existing workflows retain
least-privileged read-only CI and frozen target inventories. New checks reuse
the already-built candidate:

| Gate | Owner / required evidence |
|---|---|
| Language and TypeScript | Existing frontend lane. |
| Engine units and durable baseline | Existing Rust lane, now also running native separation. Existing test selection is not a claim that all integration targets passed. |
| Public Rust paths | Linux baseline plus the existing three-OS lock-authority matrix. |
| Frozen source/API/fixture/algorithm guards | `test:rust-core-modularization-contracts` in the three-OS combined matrix and release-source validation. |
| Actual Promise/callback/activation/stop/rebind behavior | `test:rust-core-native` after the combined matrix's fresh native build and in release-source validation. Windows loaded-addon cleanup tolerates only the exact temporary DLL leftover; other leaked resources/errors still fail. |
| Processes, artifacts, locks, correction, review, policy, output, remote faults | Existing dedicated Linux/macOS/Windows feature matrices and their current serialized execution. |
| Projects, artifact bytes, telemetry, remote coordinator/worker | Existing clean installed combined-package matrix. |
| CLI operations, backup, background log following | Existing bounded CLI lane. |
| Frozen component/package budgets | Existing feature/combined performance gates. Do not rebaseline because a file moved. |
| Six native architectures, ABI/glibc, installation and release sizes | Existing matching-runner release build/load/smoke lanes. Source inventory alone is not successful execution. |
| Current source/dependency security and exact package contents | Existing final review, JavaScript audit and RustSec audit. Missing local tooling is recorded, not silently installed through an expensive build. |

To finish sign-off, push the reviewed branch through the normal user-controlled
PR workflow, inspect every required CI result, and attach results for unrun
release/native-architecture checks. This task does not authorize pushing a tag
or publishing npm packages. Address existing behavior/fixture defects in
separate reviewed changes rather than changing this refactor's expectations.

## Known blockers carried forward

The detailed record documents stale store-version assertions in lifecycle,
durable state, child admission and runtime ownership; a wait-all parallel
recovery outcome mismatch; a cross-process workflow-call authority failure;
and the exact retry-example/frozen-fixture mismatch in transitive publication
verification. Some were independently reproduced against archived pre-extraction
source; others have exact unchanged source/constant evidence. Preserve those
distinctions. They are not newly passing checks and are not all rerun here.

The developer machine had roughly 7 GB disk headroom and nearly full swap at
the start of RCM14. Local verification reuses cached artifacts and runs
serially. The broad CLI operations suite unexpectedly launched native release
builds from `beforeAll` hooks in `production-triggers-cli` and `trigger-runtime-cli`.
Hooks timed out, builds became orphaned/contended, and Bun subsequently crashed
at `manual-trigger-pty` (segmentation fault). That shard is a failed run, not
a blanket application failure or a pass. Its output includes passing backup
and background-follow cases, but isolated reruns provide clearer evidence.

The leftover build roots were positively identified and checked for termination;
both had already exited. A release library and staged addon were regenerated
by those hooks. The actual-addon verifier subsequently passed against that
release addon too. No additional release rebuild was requested afterward;
optimized performance/architecture certification still requires the relevant
gates. No user files or build caches were deleted to make a verification claim.

## Local RCM14 evidence

Results below are filled from commands actually run for this checkpoint.
Earlier RCM12/RCM13 selections remain separate historical evidence, not fresh
RCM14 runs. Local platform: Linux x64, Bun 1.3.14, Rust/Cargo 1.88.0.

| Check | Result |
|---|---|
| Cached full engine unit suite, serial/locked/offline | 98 passed, including artifact/process, policy/output budgets, recovery transaction regressions and webhook unit discovery. |
| Expanded modularization/source/architecture/CI contracts | 25 passed. |
| Actual reviewed RCM13-built debug addon with final verifier | 68 exports and selected sync/async/error/callback/activation/telemetry/stop/rebind contracts passed; no fresh Rust build claimed. |
| Four clean installed journeys using that addon | 4 passed: project/backup/restore, artifacts, telemetry and remote attempt/terminal replay. Existing staged release addon was not overwritten. |
| Final source/security/package review | 3 tests passed; verifier passed exact 19-file staged package review. This is not the dependency advisory audit. |
| TypeScript and public documentation links/examples | Passed. |
| Ten feature publication/source verifiers | Passed; presence of platform CI in these checks is not a platform execution result. |
| Disabled telemetry and JS package budgets | Passed; CLI bundle 1,190,210 bytes; optional runtime/SDK 244,719 bytes; no threshold raised. |
| Deterministic performance regression gate | 13 tests and hard compiler/profiler gates passed. Runtime measurement tests use the existing staged addon; do not attribute those to a new release build. |
| Regenerated Linux release addon after the test-hook build | Exact 68 exports and selected lifecycle/callback contracts passed with the final verifier. |
| Locked JavaScript advisory audit | Passed: no vulnerabilities reported. |
| Locked RustSec advisory audit using existing cargo-audit | Exit 0, no vulnerability failures; warnings remain for unmaintained rustls-pemfile, Tokio I/O unsoundness and yanked chacha20. Existing RUSTSEC-2026-0258 ignore retained; no new ignore added. |
| Full CLI operations shard | Failed: nested native-build hook timeouts and Bun PTY segmentation fault. No skip/timeout/assertion change hides the failure. |
| Isolated no-build runtime/backup/background/log/retention/activation selection | 30 passed across 6 files against the regenerated Linux release addon. This does not turn the failed full operations shard into a pass. |
| Portable CLI contracts | All 17 files passed; all 138 test files remain assigned to bounded shards. |
| Frozen combined project/runtime/package budgets | Passed against the regenerated Linux release addon: project resolution p95 30.5 ms; preflight p95 317.2 ms; cold activation 799.2 ms; warm p95 653.0 ms; remote preparation p95 14.0 ms; streamed-output overhead 80.3%; installed JavaScript 2,537,319 bytes. Existing thresholds unchanged. |
| Formatting/diff hygiene and ignored developer book | Passed; atlas covers 1,840 source files, 77 pages and 3,637 local links. |

The local book is intentionally ignored; essential contribution rules are in
the tracked core guide. Its edition, maps, walkthrough and generated atlas are
updated without changing the ignore rule. Final full CI/platform/optimized
release completion remains pending, regardless of this handoff's local passes.
