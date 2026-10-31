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

## Post-handoff repairs: schema assertions and parallel recovery

The five failures reproduced in the subsequent local test run are now repaired.
This is a targeted behavior correction after the structural extraction; the
original refactor baselines and historical results above remain unchanged.

- Durable-state and runtime-ownership migration tests compare the persisted
  schema marker with `DURABLE_STORE_SCHEMA_VERSION`, without additionally pinning
  it to the obsolete value 15. Workflow-call admission checks the persisted
  marker too. Lifecycle coverage still verifies the historical model-v11/event-v10
  mapping on the current store, independently of the store version.
- Parallel dispatch can select only ready children, but group settlement now
  reloads the complete compiled group. A final failure recorded before restart
  cannot disappear merely because only its pending sibling was dispatched.
- The recovery regression covers either sibling failing first, one execution
  per child, no downstream execution, one failed group completion, coherent
  persisted history, and an unchanged history after another resume.

Fresh serial Linux verification passed all 98 engine unit tests and 270
integration tests across 33 targets, including the five previously failing
targets, the CI engine baseline, and neighboring parallel/retry/fork tests.
All 25 modularization/source/architecture/CI contract checks passed as well.
Rust formatting and the exact CI lint command,
`bun run test:communication-provider-clippy`, passed. A separate unrestricted
`-D warnings` Clippy run failed with 201 library diagnostics; CI already has
an explicit legacy-lint allowance list. That list was not expanded, and the
unrestricted lint run is not being reported as a pass.
No fixture, fingerprint, contract version, dependency, or performance budget
was changed. A new native addon/release package was not built for this check.

These results do not certify the separate workflow-call composition and retry
publication-fixture issues, the full CLI operations shard, or the unrun
cross-platform and optimized-release gates recorded above.

## Manual feature-lab integration findings

The follow-up laboratory is in `manual-tests/feature-lab/README.md`, with one
main WOML workflow, small TypeScript helpers, reusable step tags, operator
configuration, and separate payload scenarios. It is a local manual rehearsal,
not a replacement for the feature suites or cross-platform CI.

Running its combined default path exposed compatibility defects that isolated
historical fixtures did not cover. These are explicit post-refactor fixes:

- Project-aware service discovery now inspects resolved reusable steps and
  their imported services, and recognizes `workflow.json` as a module boundary.
- Public JS infrastructure/emergency authority options are encoded with the
  native decoder's existing snake-case wire keys; native schemas are unchanged.
- Verified module bundles load through short Worker-local blob URLs rather
  than filename-length-sensitive data URLs. Imported module console calls now
  use the same redacted facade as inline scripts, keeping stdout protocol-only.
- Durable wait registration accepts supported inherited Event v17–v20 histories.
  Newer model hooks, cancellation, outcome admission, and finalization retain
  their historical lifecycle semantics rather than skipping work or rejecting
  supported versions. The JS outcome decoder accepts Event v18–v20 and bounded
  iteration envelopes while rejecting unsupported future versions.

The original refactor baselines remain unchanged. The two affected ordered
transaction fingerprints are separately pinned, with their original hashes and
regression targets, in
`post-refactor-compatibility-repairs.json`. Their guard predicates changed, not
their transaction ordering; the contract suite checks these explicit amendments.
No contract version, fixture inventory, dependency, performance threshold, native
export, or legacy lint allowance was changed.

The lab deliberately leaves real remote/quorum/collector setup opt-in and uses
simulated network/deployment handles. It also documents unresolved boundaries:
managed corrective service-call authorization, policy support for StateService,
and missing run-start hook admission on the policy-managed path. Those are not
silently bypassed or counted as verified features.

## Windows compilation repair after CI

The PR's Windows feature lanes shared one compile blocker: extraction had added
`pub(in crate::process)` to `WindowsJob`'s `Drop::drop` implementation. Trait
implementation methods cannot declare visibility. The pre-refactor destructor
used plain `fn drop`; the repair restores that spelling without changing handle
ownership or kill-on-close behavior. This was an extraction regression, unlike
the separately deferred run-start admission bug.

The Unix-only containment grace-period import and its facade re-export are now
both cfg-gated. A new negative source-contract regression rejects visibility on
trait methods before fingerprint normalization can erase the invalid qualifier;
inherent helper visibility remains supported. Original baselines are unchanged.

Verification includes 24 passing Linux process tests and 26 passing
modularization/source/architecture/CI checks. The actual Windows Job Object and
containment modules also type-check under `x86_64-pc-windows-msvc`, using cached
real Tokio/windows-sys metadata and a small temporary harness with warnings
denied. That is not Windows execution or a full engine build. The full Windows
engine/test cross-check stops in ring's build script because this Linux machine
lacks MSVC C/archive tools (`lib.exe`); the Windows CI build and runtime suites
must still be rerun.

## Windows verifier cleanup ordering repair

After the Windows engine compiled, the real-addon verifier failed its cleanup
assertion with the loaded `candidate.node` and several runtime data files still
present. Recursive directory removal can stop on the DLL lock before visiting
its siblings; those untouched files are not by themselves proof of leaked
database handles.

The verifier now explicitly removes every non-addon entry first, propagating
database/artifact removal failures. It then removes the directory and retains
the existing narrow Windows exception only when a loaded addon is the sole
remaining entry. Unexpected errors, an addon that was never loaded, and new
runtime files appearing during final removal still fail verification.

Five regression tests use real temporary files and injected Windows-style
removal failures, and run in the dedicated modularization-contract CI command.
The existing Linux debug addon also passed all 68 export checks and the real
sync/async/error/callback, activation, telemetry, shutdown and rebind journey.
No Rust rebuild or Windows runtime execution is claimed for this cleanup fix;
Windows CI must confirm the native loaded-DLL behavior on its runner.

## Cold-activation budget repair: atomic empty-store initialization

The next Windows combined gate passed its contracts but measured 3,797.69 ms
against the unchanged 3,000 ms cold-activation ceiling. This is a timing failure,
not another DLL cleanup or Windows compilation failure. The single CI number
does not identify a Windows-specific cause by itself.

Local native profiling identified avoidable first-store work: an empty database
autocommitted the base DDL and then ran nineteen individually committed
historical migrations, including repeated nested layout validation. The new
private `durable/bootstrap.rs` path runs the exact same frozen DDL in order,
validates the complete layout, and publishes the current version marker in one
`Immediate` transaction. Only a database with no schema objects qualifies; it is
checked again after acquiring the writer lock. Partial, foreign, old, and future
stores retain the existing opening/migration path. Historical migration bodies,
DDL literals, schema versions, public APIs, synchronization settings and frozen
performance budgets are unchanged. This is an explicit startup optimization
after the structural refactor, not a claim that only file movement occurred.

Regression coverage compares every `sqlite_master` object and SQL definition
against an actual v1-to-v20 upgrade, checks FULL synchronization and foreign-key
enforcement, injects disk-full failure and verifies complete rollback/retry,
rejects partial/foreign schemas from the fast path, and races four first openers.
The existing recovery, state, lock, compensation, backup and remote-dispatch
integration tests also remain required.

An illustrative Linux before/after comparison used separately loaded debug
addons and a fresh database in each process, with the same JavaScript host:
initial store opening measured 262.7 -> 27.0 ms and cold execution measured
928.1 -> 758.2 ms. These are single diagnostic samples, not cross-platform p95
proof or optimized-release results. Windows CI must still confirm that its
complete cold activation meets the frozen 3-second gate. No retries, prewarmed
database, raised budget, or reduced durability were added to the benchmark.

Local verification passed 10 store boundary tests, 66 existing integration
tests across seven storage/recovery targets, 31 modularization contract tests,
the exact CI Clippy command, formatting and diff checks. The rebuilt debug
addon passed the actual 68-export sync/async/error/callback, activation,
telemetry, shutdown and rebind verifier. The separate unrestricted Clippy
attempt still reports the 201 documented legacy library diagnostics; no lint
allowance was added. Windows execution and optimized-release timing remain
unverified locally.
