# Rust core modularization: RCM0–RCM5 verification

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

## RCM2: graph decisions and validated projection

This phase starts from `2c485abca716b7bf7c5c9cc1aca02bbf61a361c7`, the
committed RCM0/RCM1 implementation. The additional
[execution API baseline](rcm2-baseline.json) was captured before extraction;
the new compile-time path checks also passed against the original files.
It freezes 2 engine, 46 projection, and 27 runtime public types, 16 public
functions, 6 constants, and the `EngineClock` trait. Public root exports and
direct-module imports remain as before; private children are not public APIs.

| Responsibility | Private owner |
|---|---|
| Graph readiness, active routes, completion | `engine/readiness.rs` |
| Branch/choice evaluation and diagnostics | `engine/routing.rs` |
| Context-reference lookup | `engine/context.rs` |
| Definition-bound payload/scoped validation | `engine/validation.rs` |
| Prefix-history checks and their cross-event rules | `engine/history.rs` |
| Stable effect identity | `engine/identity.rs` |
| Projected context/status/feature shapes | `projection/{context,run,attempts,control_flow,...}.rs` |
| Global sequence/identity/version/terminal gate and exhaustive dispatch | `projection/fold.rs` |
| Typed feature transition handlers | `projection/fold_*.rs` |
| Scoped iteration fold | `projection/iteration_fold.rs` |
| Shared transition preconditions | `projection/guards.rs` |

The in-memory engine remains in its public facade. It validates the candidate
history against its definition before appending. The fold's 77 extracted
transition bodies receive typed borrowed payloads behind one exhaustive
dispatch, not a second permissive event decoder. The remote transition stays
the original projection no-op: durable dispatch authority still owns that
state. Iteration dispatch still passes through the common gate first.

`ProjectionFoldState::apply` itself is not transactional. It may change a
candidate before rejecting an event. `InMemoryEventStore::append` still clones
the cache, applies to the candidate, and publishes history, IDs, and projection
only on success. Durable commit/cache boundaries were not moved. New graph
tests reject global gate drift and invalid feature transitions, then reuse the
rejected ID for a valid append to prove committed context and indexes remain
usable. Existing prefix-history, fork, parallel, choice, and retry tests retain
their entrypoints.

## RCM3: foundations and ordinary script invocation

| Responsibility | Private owner |
|---|---|
| Clock contract and implementations | `runtime/clock.rs` |
| Options, reviewed settings, and safe debugging | `runtime/options.rs` |
| Explicit built-in registration and durable capability attachment | `runtime/composition.rs` |
| Script-host pool acquisition, return, retirement, and shutdown | `runtime/script_host_pool.rs` |
| Attempt-local artifact cleanup guard | `runtime/cleanup.rs` |
| Public outcomes and errors | `runtime/outcomes.rs`, `runtime/errors.rs` |
| Progress contracts and delivery helpers | `runtime/progress.rs` |
| Stored module recovery and module validation | `runtime/modules.rs` |
| Node context snapshots, declared secrets, reusable bindings | `runtime/context.rs` |
| Ordinary invocation, placement guard, interruption, failure settlement | `runtime/script_execution.rs` |

`runtime.rs` retains execution/resume entrypoints, policy coordination,
continuation and control-flow algorithms, lifecycle/correction orchestration,
and engine adapters for RCM4, RCM5, and RCM9. This is intentionally not the
completion of runtime modularization. The in-memory and durable adapters
still share the existing private `RuntimeDagEngine` interface.

The composition helper preserves construction and registration order. Its
small typed resource bundle transfers the same owned `Arc`s back to the
options constructor; it is not a new generic capability framework. Process,
artifacts, output, and durable authority retain their existing opt-in/lazy
boundaries. Private options fields and helpers needed by siblings become
`pub(super)` within the private runtime tree, not `pub` or newly crate-wide.
Host pool mutexes, cancellation flags, deadlines, `Box::pin` boundaries,
cleanup drop order, and exact invocation/attempt identity are unchanged.

Context construction has separate owners: projection proves completed
results; callers choose the appropriate fork opening snapshot; compiled
visibility filters node inputs; protocol constructors attach iteration or
correction-specific bindings. Moving these helpers does not grant a sibling's
result merely because it finished earlier. The developer book's context deep
dive and execution reading map now link to these implementation owners.

The Compensation and Cleanup release reader now reads Rust facades together
with their owned private trees, as the policy and output verifiers already
do. Its required and forbidden assertions were not weakened or satisfied by
adding marker comments. Existing native and package export lists are unchanged.

A one-time source audit compared 455 original declaration/method token streams
and all 77 extracted transition bodies against the committed base. It allows
parent-only visibility and formatting; both transition bodies were normalized
with rustfmt to account for single-expression closure wrapping. Inlining the
registration helper reproduces the constructor's original token order. This
audit is supporting evidence, not a substitute for executable regression tests.

### Verification scope

All local Cargo commands use one job, disabled dev debug/incremental output,
locked offline dependencies, and serial tests. The focused pre-extraction
baseline passed 21 tests (`core_contract_paths`, `dag_engine`). After extraction,
the planned 11-target RCM2/RCM3 regression set passed 72 tests; the foundation
unit filter passed 15 tests (5 new runtime tests and 10 existing infrastructure
policy tests also matched by the filter).

The 17-file CLI contract shard passed with ordinary workspace write access;
its first sandboxed run rejected the existing editor-types write with EROFS.
No CLI behavior was changed to hide that environment restriction. The focused
Bun source/API/fixture guards passed 11 tests, TypeScript checking passed, and
the policy, compensation, and streaming-output source release verifiers passed
with their original assertions. Broader feature/native/lint results are
recorded below.

| Check | RCM2/RCM3 local result |
|---|---|
| Planned compatibility/graph/script/services/worker set (11 targets) | 72 passed |
| Broader cancellation/store/wait/loop/fork/lock/notification/parallel/retry/review set (18 targets) | 126 passed; 1 unchanged stale-version assertion failed |
| Foundation unit filter | 15 passed, including all 5 new runtime tests |
| Native separation rebuilt against the final engine | 4 passed |
| Final post-format API/DAG/iteration smoke rerun | 30 passed (repeat coverage) |
| Portable CLI contract shard | 96 tests across 17 files passed |
| Recursive source/API/native-name/fixture guards | 11 passed |
| Established engine Clippy policy | Passed; no new lint allowances |
| TypeScript and Rust formatting | Passed |
| Policy, compensation, streaming-output source release verifiers | Passed |
| Ignored developer book and regenerated atlas | Passed; 1,627 files, 77 pages, 3,077 local file links |

The known broader-suite failure is
`lifecycle_authority::model_v11_maps_to_event_v10_while_current_store_is_v14`:
line 90 asserts `DURABLE_STORE_SCHEMA_VERSION == 15`, while the existing
`durable.rs` constant is 20. Both the assertion and the constant are unchanged
from `2c485ab`; their files have no diff in these phases. Eight other tests in
that target passed. The failure was not skipped in its run, and neither the
schema nor the assertion was changed to make this refactor look green. Review
that historical test separately (including its misleading name), preserving
its model-v11/event-v10 compatibility checks. Cargo stopped at this target;
the remaining eight targets were run in a separate command and all 55 passed.

The public engine facade is now 209 lines (from 3,069); projection is a
114-line facade (from 3,919), with its global gate/dispatch in a 340-line file.
Runtime is 11,858 lines (from 14,065) because later orchestration phases are
intentionally still pending. There are 42 new private Rust files, including
the five-test foundation module. File-size changes are navigation evidence,
not a claim of improved execution performance.

macOS/Windows, release/native rebuilds and actual-addon export loading,
installed-package journeys, and release performance budgets remain CI gates.
No schema, dependency, serialized fixture, native export, or budget baseline
is intentionally changed by these phases. The ignored developer book is
updated locally; the user's ignore rule and unrelated roadmap notes remain
untouched.

## RCM4 and RCM5: scheduling, control flow, lifecycle, and correction

These phases start from committed RCM2/RCM3 source
`6d2b7ec7bf026fd9ec852e3d4ff4ae2f7d79cb08`. The
[RCM4 source baseline](rcm4-baseline.json) records the original runtime facade
and private-file hashes plus the five intentional boxed call sites. Earlier
schema, API, fixture, native-name, and integration-target baselines remain in
force; they were not refreshed to accept extraction drift.

### Ownership after extraction

All new implementation modules are private. Explicit facade re-exports retain
existing root/direct-module APIs. Cross-sibling helpers use runtime-scoped
visibility, not new public APIs or blanket exports.

| Responsibility | Private implementation owner |
|---|---|
| Continuation wrapper and central execution loop | `runtime/scheduling/run_loop.rs` |
| Capacity coordination, durable claim acquisition, heartbeat ownership | `runtime/scheduling/policy.rs` |
| Total workflow deadlines and timeout lifecycle settlement | `runtime/scheduling/timeouts.rs` |
| Root/iteration lock renewal and key resolution | `runtime/scheduling/locks.rs` |
| Parallel invocation/group completion | `runtime/control_flow/parallel.rs` |
| Fork snapshots, branches, joins, and closure | `runtime/control_flow/forks.rs` |
| Branch/choice failure helpers | `runtime/control_flow/branches.rs` |
| Approval/signal/timer waiting outcomes and wait registration | `runtime/control_flow/waits.rs` |
| Bounded iteration and ordered aggregation | `runtime/control_flow/for_each/coordinator.rs` |
| Iteration references, module bindings, and digests | `runtime/control_flow/for_each/context.rs` |
| Iteration script supervision and settlement | `runtime/control_flow/for_each/script.rs` |
| Shared borrowed-engine iteration adapter | `runtime/control_flow/for_each/shared.rs` |
| Workflow lifecycle and settlement snapshots | `runtime/lifecycle/workflow.rs` |
| Step lifecycle admission and outcomes | `runtime/lifecycle/steps.rs` |
| Reusable/provider lifecycle | `runtime/lifecycle/reusable.rs` |
| Iteration-scoped lifecycle | `runtime/lifecycle/iteration.rs` |
| Lifecycle-specific notifications and rendering | `runtime/lifecycle/notifications.rs` |
| Atomic success/compensation registration and scope identity | `runtime/correction/registration.rs` |
| Exact corrective attempt execution | `runtime/correction/execution.rs` |
| Saga compensation, cleanup, and recovery continuation | `runtime/correction/saga.rs` |
| Cancellation closure/settlement ordering | `runtime/cancellation.rs` |
| Primary failure classification and reconstruction | `runtime/failure.rs` |

The original `runtime.rs` is now 1,986 lines, down from 11,858 at this phase's
baseline. It retains execution/resume APIs, reviewer authority registration,
the small correction-resume result, and the main `RuntimeDagEngine` interface
and adapters. Those adapters belong to RCM9. The iteration-specific shared
adapter moved with bounded iteration in RCM4 because it directly owns that
algorithm's borrowed-engine access; it still shares the same
`Arc<Mutex<&mut E>>`. Durable store, migration, and native bridge extraction
has not started. There are 27 new private Rust files, including module facades.
Line counts measure navigation, not performance improvement.

The central continuation loop moved intact first. Policy scheduling,
parallel work, forks, then iteration moved in separate compiling checkpoints.
Lifecycle/correction extraction followed that control-flow verification.
No independent scheduler, store, connection, transaction owner, or per-item
event log was introduced. A policy wakeup still only prompts a durable claim
check; suspension does not invent new ownership authority.

Cancellation still settles operations/attempts and closes active scopes in
the original order before deciding the run outcome. Saga correction still
uses proven forward completions in reverse completion order, then cleanup.
Manual recovery and ambiguity still block fabricated final success or safe
retry. Lifecycle notification behavior is not mandatory saga cleanup.
Corrective context, declared secrets, retry identities, local/remote guards,
capability intersections, cancellation flags, and drop order are unchanged.

### Preservation evidence

A one-time declaration/method audit compared the committed runtime tree with
the extracted tree: all 357 declarations/methods remain across 40 files.
It compares token streams, retaining string literals, attributes, and statement
order while allowing formatting/trailing commas and documented runtime-scoped
visibility. No wrappers, algorithm rewrites, synchronization substitutions,
deadline changes, or new boxing were needed. The audit complements executable
tests; it is not a proof of full behavioral equivalence.

The persistent source guard now freezes the intentional `Box::pin` callees:
one continuation-loop boundary, two host executions, and two parallel-child
invocations. Actual async/control-flow tests ran on their ordinary thread and
Tokio runtime configurations, without `RUST_MIN_STACK`, thread stack overrides,
increased watchdogs, or relaxed concurrency/output budgets.

The compensation, policy, and streaming-output source verifiers already scan
owned nested modules from RCM0/RCM2 preparation. Their original assertions pass
without marker-comment substitutes. The API guard still freezes public
types/functions/constants/traits, 68 expected native names, existing integration
targets, and all 356 reviewed text fixture files. No schema, lockfile, package
allowlist, native export, serialized contract, or performance baseline changed.

### Local verification

Cargo tests/checks used `-j 1 --locked --offline`, disabled dev debug information
and incremental compilation, and serial test threads. Only one Cargo build/test
ran at a time. The established Clippy command retains its existing allowlist.
Bun 1.3.14 was present, so host-dependent tests exercised real script hosts.

| Check | RCM4/RCM5 result |
|---|---|
| Pre-extraction API/policy/timeout/lifecycle/compensation selection | 38 passed |
| RCM4 control-flow/policy/rate/timeout selection (13 targets) | 55 passed; 1 baseline-confirmed failure |
| RCM5 lifecycle/cancellation/compensation/composition selection (16 targets) | 87 passed; 2 baseline-confirmed failures |
| Final engine and native consumer checks | Passed without warnings |
| Native separation rebuilt against the final engine | 4 passed |
| Established engine Clippy policy | Passed; no new allowances |
| Final foundation unit filter | 15 passed (5 runtime tests plus 10 matching policy tests) |
| Final post-RCM5 scheduler/rate/timeout/parallel/fork smoke | 25 passed (repeat coverage) |
| Portable CLI contracts | 17-file shard passed |
| Recursive source/API/fixture/boxing guards | 12 passed |
| TypeScript checking and Rust formatting | Passed |
| Compensation, infrastructure-policy, output source verifiers | Passed |
| Developer book and regenerated source atlas | Passed; 1,655 files, 77 pages, 3,151 local links |

The two phase selections overlap (notably iteration coverage); do not treat
their counts as distinct tests. RCM5 used `--no-fail-fast` so its known failed
targets did not hide the remaining tests. RCM4's remaining targets were run
separately after Cargo stopped at its failed target.

RCM4 targets: `for_each_recovery`, `for_each_runtime`,
`fork_control_flow_runtime`, `fork_failure_runtime`, `fork_join_all_runtime`,
`fork_join_modes_runtime`, `fork_recovery_runtime`, `parallel_failures`,
`parallel_recovery`, `parallel_runtime`, `policy_scheduler`, `rate_limits`,
`workflow_timeouts`.

RCM5 targets: `cancellation_runtime`, `compensation_runtime`,
`compensation_store`, `core_contract_paths`, `for_each_recovery`,
`for_each_runtime`, `lifecycle_authority`, `lifecycle_notifications`,
`lifecycle_runtime`, `module_recovery`, `remote_worker_attempt`,
`retry_composition`, `reusable_lifecycle_events`, `services_composition`,
`step_lifecycle`, `workflow_call_composition`.

### Baseline failures, not a green-release claim

Three distinct failures were observed. None was skipped, reclassified as a
passing test, or hidden with a timeout/schema/expected-result change:

1. `lifecycle_authority::model_v11_maps_to_event_v10_while_current_store_is_v14`
   expects store version 15 while the existing constant is 20. It failed before
   this phase's extraction and again afterward. This is the earlier recorded
   stale-version assertion; its other compatibility checks remain unchanged.
2. `parallel_recovery::wait_all_resumes_pending_siblings_after_a_recorded_failure_then_fails_the_group`
   returns a durable contract error: `Parallel completion for "fieldData" does
   not match its durable child outcomes.`, instead of the expected
   `ParallelFailed`. The test's panic message now includes the actual error;
   its assertion is unchanged. An independent `git archive` of the exact
   pre-RCM4 commit reproduced the same error using cached dependencies and
   the same single-job command. No source rollback or branch switch was used.
3. `workflow_call_composition::nested_cross_process_calls_compose_with_branch_and_parallel_targets`
   records `WOML_WORKFLOW_TARGET_UNAVAILABLE` / `The durable workflow call
   authority is unavailable.` for `requestRisk`. The extracted source failed
   twice; the archived pre-RCM4 commit independently produced the same failed
   run and capability failure. This establishes that it is not introduced by
   RCM4/RCM5; it does not establish whether its deeper cause is a baseline defect
   or a local cross-process/environment limitation.

The isolated historical checks used a temporary archive under `/tmp`, sharing
the existing target cache and CLI dependencies. The primary working tree and
user changes stayed intact. The only test code change is diagnostic detail
for the parallel failure. These findings need separate investigation rather
than an algorithm fix buried in a large refactor.

macOS/Windows, actual-addon export loading, installed release-package journeys,
and release performance budgets remain CI gates. The local developer book
now explains each new owner and why it exists, and its teaching/source routes
and atlas match these modules. That book remains ignored according to the
user's existing `.gitignore`; neither that rule nor unrelated roadmaps changed.
The next extraction is RCM6 (durable opening, schema, and migrations).
