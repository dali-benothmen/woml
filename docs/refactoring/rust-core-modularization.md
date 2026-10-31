# Rust core modularization: RCM0–RCM13 verification

RCM14 implementation/handoff and current sign-off status are recorded in the
[final handoff report](rust-core-modularization-handoff.md). Historical phase
results below retain their original scope; they are not all fresh RCM14 runs.

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
The native `crate::` prohibition is retained. RCM13 uses legitimate private
`super::` helper imports; both guards explicitly allow these and reject
parent-relative legacy imports too. All private source files remain scanned.

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

## RCM6 and RCM7: durable opening, commits, indexes, and inspection

Baseline: `35ebd561e79f8759483293cd345f82183b0cfbb0`, after the committed
RCM4/RCM5 work. The [RCM6 baseline](rcm6-baseline.json) freezes the original
`durable.rs` hash, its public inventories (111 types, two functions, 20
constants, one trait), and exact hashes of all 19 schema/migration DDL
literals. Existing API, native export, fixture, and integration-target
baselines remain unchanged. No dependency, schema, contract, or budget bump
is part of these phases.

### RCM6 ownership

| Responsibility | Private durable implementation |
|---|---|
| Full startup initialization versus prepared request opening | `opening.rs` |
| Current/historical version markers and exact DDL literals | `schema.rs` |
| Ordered transactional layout upgrades and marker updates | `migrations.rs` |
| Historical/current required-object and column checks | `validation.rs` |
| Stable store errors/conversions/display | `errors.rs` |
| Typed admission, ownership, notification, approval, policy, wait, lock, correction, inspection, recovery, and run-control records | `records/` |

`DurableEventStore` retains the original connection and `RefCell` projection
cache in the parent facade. New inherent `impl DurableEventStore` blocks add
methods to that same type; they are not new repository objects, per-feature
connections, or transaction owners. Existing public `woml_engine::durable::*`
names and root re-exports remain available. Private types/helpers required by
siblings use durable-tree visibility, with no wildcard exports or new public
implementation modules. Record Serde attributes, variant/field order, errors,
and private evidence semantics are unchanged.

Startup `open`/`open_in_memory` still call `initialize`. The original order is
retained: configure the connection, prepare base schema, validate the stored
version/layout, perform necessary migrations, validate current layout,
rebuild correction indexes in an immediate transaction, conditionally rebuild
policy indexes and summaries, then expire scheduler claims. General
interrupted-run recovery is not part of opening.

`open_ready` still configures the prepared connection and checks the current
version marker without running DDL, historical validation, migration, index
rebuild, or interrupted-work classification. Startup readiness is its caller's
precondition. Both opening paths retain the existing five-second busy timeout
and foreign-key setup. No SQL literals were moved into external SQL files.

RCM6 compiled and its regression targets ran before RCM7 extraction. The
selected pre-extraction baseline passed 35 tests. RCM6 passed 57 tests across
six targets and exposed one unchanged stale-version assertion described below.
After Cargo stopped at `durable_state_store`, the remaining `locks_and_leases`
target was run separately; all 17 tests passed.

### RCM7 ownership and boundaries

| Responsibility | Private durable implementation |
|---|---|
| Immutable program, module artifacts, and exact run bindings | `definitions.rs` |
| Run creation and atomic event-batch authority | `events/commits.rs` |
| Integrity-checked history loading and transaction-borrowing append helpers | `events/history.rs` |
| Shared historical outcome/lifecycle semantics and warnings | `events/compatibility.rs` |
| Validated bounded projection caching and rich-review output hydration | `events/cache.rs` |
| Event-derived summaries | `indexes/run_summaries.rs` |
| Policy binding, queue, start indexes and rebuild detection | `indexes/policy.rs` |
| Correction read models, retaining external operator decisions | `indexes/correction.rs` |
| Bounded versioned run/review/wait/lock/correction reads | `inspection/` |

The original immediate transaction still encloses run/binding checks,
history loading, candidate state, compatibility expansion, definition/scope
validation, actual event sequencing, row writes, history validation, summaries,
and relevant index writes. Commit still precedes cache publication. Low-level
history helpers borrow the existing caller transaction and do not independently
commit. Feature-specific commits remain with their original feature authority
in the parent for RCM8.

Cache freshness still compares event count/sequence and last event identity.
Tentative append preparation removes the old cached entry; failures cannot
publish speculative candidate state. Reload validates against the exact
definition. Rich-approval output hydration still happens on both cache hits
and misses. Terminal runs remain outside the bounded active cache.

`expand_model_v11_payload` and lifecycle warning reconstruction retain their
original crate-visible durable-facade paths. The in-memory runtime adapter
continues using the same compatibility semantics; this code was not hidden
behind a database-only interface. Both ordinary correction-index maintenance
and startup rebuild preserve `externally_resolved` operator decisions. Those
mixed rows were not reclassified as wholly disposable projections.

The parent durable file is now 11,374 lines, down from 18,315. There are 34
new private production Rust files and one new characterization-test file.
Feature transactions, recovery, and `DurableDagEngine` intentionally remain
for RCM8/RCM9; this is not a claim that the whole durable facade is finished.
Line counts measure organization rather than execution performance.

### Characterization and preservation evidence

The one-time token audit matches all 516 original production durable
declarations/methods across the parent and 34 private production files. It
retains attributes, literal contents (including inline SQL), and statement
order, permitting formatting/trailing commas and documented durable-scoped
visibility. The existing notification-capability test module is retained;
new test-only code is outside the production token comparison. No algorithm,
synchronization, transaction, or cache-publication substitution was needed.
This supports, rather than replaces, executable tests.

Five new `durable::boundary_tests` characterize:

1. Prepared opening alongside an existing writer without DDL/schema repair.
   This deliberately damages a test schema to distinguish startup validation
   from the ready caller's precondition; it is not a supported way to admit
   an arbitrary damaged production database.
2. Rejection of old/future markers without changing the marker or migrating.
3. Rollback of an entire batch when its second payload is invalid, including
   removal of speculative cache state and reuse of the unconsumed event ID.
4. A forced summary-write failure after candidate history processing, proving
   history, summary, sequence/event ID, and cache publication remain coherent.
5. Rejection of a cached history with equal count but a different last event
   identity, without letting its fabricated trigger data replace durable truth.

These complement the existing invalid-append, cross-connection refresh,
failed-migration, admission rollback, rich-review race, correction evidence,
backup/restore, and retention suites. No assertion, timeout, stack size,
inspection limit, or performance budget was relaxed.

### Source verifiers and developer navigation

The waits, locks, rich approvals, runtime-ownership, and observability source
verifiers now use the existing owned-tree Rust reader. Their required and
forbidden assertions remain intact. Compensation, policy, and streaming-output
verifiers already read recursively and still pass. These are source/artifact
contract checks, not evidence that installed-package or platform execution was
performed locally.

`core_contract_paths` now compiles every original durable type/function/constant
and the provider trait, checking root/direct-module type identity where the
root export existed. The Bun contract guard includes durable public inventories
and exact DDL literal hashes, while preserving prior native names, fixture
bytes, boxed call sites, and target discovery checks.

The ignored local developer book now follows actual opening, schema,
definition, commit, cache, index, inspection, and record owners. Chapter 14
explains who controls each boundary and why it exists; the beginner story,
architecture map, schema map, reading routes, version references, reviewed
edition inventory, and source atlas were updated together. Feature/recovery
work still pending is explicitly distinguished from implemented modules.
The user's ignore rule and unrelated untracked roadmaps are preserved.

### Verification scope and known baseline assertion

All local Cargo build/test commands run one at a time with one job; test/check
commands use locked offline dependencies and disabled dev debug/incremental
output. Durable/process/port tests are serial. The established Clippy command
retains its existing policy and allowlist. No release build was run.

| Check | Local RCM6/RCM7 result |
|---|---|
| Pre-extraction store/correction/restore/presentation selection | 35 passed |
| RCM6 store/state/migration/contracts/lock/restore selection | 57 passed; 1 unchanged stale-version assertion failed |
| New opening/transaction/cache characterization | 5 passed |
| RCM7 store/authority/index/inspection selection (11 targets) | 105 passed |
| Engine plus native consumer checks | Passed without warnings |
| Native separation rebuilt against the final engine | 4 passed |
| Final durable unit filter | 6 passed (5 new boundary tests and the retained capability-token test) |
| Established engine Clippy policy | Passed; no new allowances |
| Portable CLI contracts | 17-file shard passed |
| Source/API/DDL/fixture/boxing/architecture guards | 13 passed |
| TypeScript and eight affected source release/ownership/observability verifiers | Passed |
| Rust formatting, token audit, and diff validation | Passed |
| Ignored developer book and source atlas | Passed; 1,691 files, 77 pages, 3,252 local links |

Selections overlap; do not sum them as distinct tests. RCM7 targets were
`backup_restore`, `compensation_store`, `core_contract_paths`,
`durable_event_store`, `locks_and_leases`, `notification_infrastructure`,
`retention`, `rich_plan_approval_authority`, `run_presentation`,
`runtime_policy_store`, and `trigger_occurrence`, with `--no-fail-fast` so one
target could not conceal later coverage.

The RCM6 failure is
`durable_state_store::store_v12_migrates_transactionally_through_state_v1_to_store_v14`,
line 110: `assert_eq!(DURABLE_STORE_SCHEMA_VERSION, 15)`. The immediately
preceding check that the actual migrated version equals the current constant
passes; the current constant is 20. The exact committed `35ebd56` test has
that same unconditional assertion, and the committed source defines 20.
Neither file contained an intervening version fix before this extraction;
their original contents also match the earlier pre-RCM4 snapshot for this
comparison. This is direct static proof of a pre-existing assertion, not a
claim that an archived executable was rerun in this turn. The test remains
unchanged, was not skipped in its run, and no schema change was used to hide
it. Its other seven tests, including failed-v12-migration rollback, passed.

Earlier RCM4/RCM5 baseline findings remain documented above. Their three
specific failing tests were not rerun or fixed as part of these code moves.
The next implementation phase is RCM8. macOS/Windows, actual-addon export
loading, installed-package journeys, and full release performance/security
gates remain independent verification work, not implied passing results.

## RCM8 and RCM9: feature transactions, ordered recovery, and adapters

These phases start from committed RCM6/RCM7 revision
`6daaff62c4c317338402ae9a3380361688c62e81` on the existing implementation
branch. The [RCM8 baseline](rcm8-baseline.json) records the owned durable/runtime
source hashes before extraction and ordered fingerprints for 12 critical
transaction/recovery declarations. Earlier public API, native-name, fixture,
DDL, boxing, and target-discovery baselines remain unchanged.

### RCM8: five independently tested transaction checkpoints

`DurableEventStore` remains the only connection/cache owner. The new private
`durable/transactions/` modules contain inherent methods on that same type;
they do not construct per-feature stores. A helper taking `&Transaction` still
borrows its caller's writer transaction. Supporting credential parsing and
stored timestamp/decision conversion live in `credentials.rs` and
`record_values.rs`; their algorithms and public errors are unchanged.

The following map identifies affected families, not a claim that every listed
table is written on every call. Common event append helpers also maintain the
existing run-summary, policy, and correction indexes at their original boundary.

| Checkpoint and owner | Tables and event responsibilities | Validation and commit/rollback boundary |
|---|---|---|
| 1. `transactions/admission.rs`, `schedules.rs`, `intervals.rs`, `workflow_calls.rs` | Run/definition binding, trigger occurrences, internal-event deliveries, schedule/interval cursors, child-call links, admission/initial history, policy queue bindings. | Exact occurrence/payload/definition/parent identity, call depth and cycle checks, and cursor compare/update stay in their original immediate writer transactions. Clock callers share `admit_trigger_occurrence_in_transaction`; a late cursor/link error rolls back the proposed admission rather than committing a detached run. Duplicate/conflict results retain their original identities. |
| 2. `ownership.rs`, `policy.rs`, `infrastructure_policy.rs` | Runtime owner/routes, scheduler claims, queue/start records, bound policy bundles/evidence, and applicable execution-start facts. | Exact owner lease, current claim, policy hash, concurrency, rate and FIFO eligibility remain checked before the original updates/commit. Bundle/evidence readers do not gain admission authority. Failed validation/write leaves the proposed transaction uncommitted; no new connection or independent policy commit was introduced. |
| 3. `approvals.rs`, `rich_approvals.rs`, `notifications/{delivery,updates,approval}.rs` | Approval credentials, notification capabilities/intents/outcomes, review evidence, reviewer votes, quorum, artifact pins, decision and message-update facts. | Exact request/token/decision identity, expiry, reviewer/evidence/quorum checks and single terminal winner are retained. Request-plus-intents and decision-plus-updates/pin changes stay within their original boundaries. Cache publication/invalidation still follows commit; late credential/vote/pin failures cannot leave an accepted partial decision. Provider I/O is not claimed to be atomic with SQLite. |
| 4. `waits.rs`, `signals.rs`, `locks/{queue,leases,programmatic,scope}.rs` | Durable waits, early-signal/idempotency receipts, lock waiters/resources and fencing high-water; wait-resolution and lock lifecycle facts. | Exact active execution/correlation/payload and lease/fence/scope validation remain unchanged. Wait registration consumes eligible early signals in one transaction. Lock expiry and successor grant share the caller's transaction, including facts for different runs. Queue/fence exhaustion fails closed. Late stale release deliberately commits expiry/successor transfer before returning its rejection; it must not be rewritten into unconditional error rollback. |
| 5. `correction.rs`, `run_control/{identity,cancellation,outcomes,timeouts}.rs` | Audited correction/operator decisions, interruption/outcome/deadline/cancellation facts and affected wait/index state. | Bound run/action/request identity, correction eligibility and primary failure rules remain intact. Forward success plus compensation registration still uses the same grouped writer append through the DAG adapter. Cancellation and timeout use the original shared helpers/transactions; helper extraction cannot commit half a terminal or corrective transition. |

Each checkpoint compiled and ran its relevant tests **before** the next
family moved. No transaction was narrowed to one run where it previously
changed several runs. In particular, `grant_oldest_lock_waiter` and
`expire_lock_owner` still use their caller's transaction and preserve the
fencing high-water conditional update. `locks/leases.rs` retains the explicit
commit-before-stale-release-error branch, covered by the late-release test.

| Checkpoint | Targets | Result |
|---|---|---|
| Pre-extraction baseline | `backup_restore`, `compensation_store`, `durable_wait_runtime`, `locks_and_leases`, `notification_infrastructure`, `rich_plan_approval_authority`, `runtime_policy_store`, `trigger_occurrence` | 84 passed |
| 1. Admission/clocks/child links | `interval_runtime`, `schedule_runtime`, `trigger_occurrence`, `workflow_call_admission` | 29 passed; 1 unchanged stale-version assertion |
| 2. Ownership/policy | `infrastructure_policy_authority`, `runtime_ownership`, `runtime_policy_store` | 21 passed; 1 unchanged stale-version assertion |
| 3. Review/notification | `lifecycle_notifications`, `notification_infrastructure`, `rich_plan_approval_authority` | 22 passed |
| 4. Wait/signal/lock | `durable_wait_runtime`, `locks_and_leases` | 30 passed |
| 5. Correction/run control | `cancellation_runtime`, `compensation_runtime`, `compensation_store`, `workflow_timeouts` | 28 passed |

These are 130 passing checkpoint tests and two baseline assertions, not a
claim that every historical test is green. Selections overlap other baseline
and final runs and must not be summed as distinct coverage.

### RCM9: move recovery intact, then separate support and adapters

The first RCM9 checkpoint moved recovery with its helpers intact into
`durable/recovery.rs`. Before splitting helpers, `compensation_store`,
`durable_event_store`, `durable_state_hardening`, and
`notification_infrastructure` passed 33 tests. The state-hardening suite's
ignored worker entrypoint is deliberately invoked in four child processes
by its parent concurrency case; those helper processes passed too.

Only then were these private responsibilities separated:

- `recovery.rs`: the ordered per-run algorithm, report aggregation,
  ownership-scoped child-call recovery, and call-index reconciliation.
- `recovery/state_mutation.rs`: immutable managed-state mutation proof.
- `recovery/interruption.rs`: existing interrupted/failure payload helpers.
- `recovery/for_each.rs`: shared iteration-failure settlement, also used by
  timeout handling through the caller's transaction.
- `recovery/queries.rs`: existing bounded discovery of undispatched, correction,
  lock and fork work.
- `dag_adapter.rs`: `DurableDagEngine`, durable errors and failure dispositions,
  bound execution methods, grouped commits and retry/deferred-failure behavior.
- `integrity.rs`: the existing store integrity audit, not a new startup repair.
- `runtime/engine_adapter/{mod,in_memory,durable}.rs`: the existing internal
  trait and its two implementations. Public runtime entrypoints remain in
  `runtime.rs`; the scoped iteration adapter remains with its coordinator.

`recover_run` retains its immediate transaction and branch order: bound
definition/history validation and folding; corrective interruption/manual or
resumable state; open iterations; lifecycle obligations; waiting notification
sends; bounded known-message updates; non-running exits; managed-operation
proof/failure; interrupted attempts; fork/parallel state; pending retries;
latest-attempt failure; justified terminal settlement or resumable work.
Version checks and early returns remain part of that algorithm. The module
split does not authorize each helper to run its own recovery loop.

Managed-state proof is deliberately narrow. The original stable operation
key, supported mutation operation, key/input digests, result contract/version,
mutation result/version validation, and domain-separated canonical receipt
digest must match. The operation result's encoded-byte digest remains a
different hash input. A verified receipt may settle the operation; it does not
prove that the containing interrupted script completed or is safe to replay.
Unknown notification sends remain non-retryable/ambiguous, while updates of
an already known message retain their bounded retry contract. Remote dispatch
keeps its separate authority and ambiguity/reconciliation rules. Local child
recovery remains scoped to the supplied owned target definitions rather than
interrupting every other live process's children.

### Characterization, preservation guards, and navigation

The new boundary test
`failed_recovery_write_rolls_back_interruption_and_retries_idempotently`
injects an SQLite error at the second proposed recovery event. It verifies
that history and projection remain unchanged, removes the fault, checks one
interruption/terminal repair, then confirms another recovery adds no events.
All seven durable unit checks pass, including the six boundary tests and the
retained capability-token parser test.

The final one-time ordered token audits match all **516 original durable**
and **357 original runtime** declarations/methods across their owned trees.
They retain attributes, literals/SQL and statement order; formatting, trailing
commas and the documented private visibility routes are the allowed changes.
The persistent Bun guard also freezes 12 critical declarations against the
new baseline, including `recover_run`, state proof, lock expiry/grant/release,
admission, rich review, wait registration, cancellation and timeout settlement.
It distinguishes store methods from same-named DAG adapter methods and includes
a negative mutation of the state proof's domain separator. These guards
support executable tests; they do not prove algorithms correct by themselves.

The durable facade is now 181 physical lines (including its retained test)
instead of 11,374 at this phase's starting checkpoint; `runtime.rs` is 1,045
lines after extraction. Recovery remains a cohesive ordered coordinator, not
arbitrarily split to meet a line target. Public Rust paths and native export
names, schema/event/protocol versions, inline SQL, synchronized ownership,
future boxing, secret boundaries, limits and timeouts did not change.

The codebase-documenter workflow guided updates to the ignored local book:
architecture and scheduler/adapter chapters, transactions/recovery chapters,
schema and reading maps, responsibilities/coverage, beginner story and source
atlas. They explain caller ownership and proof versus replay rather than only
listing relocated filenames. Edition metadata identifies RCM0–RCM9 as a
working-tree overlay. The user's ignore rule and unrelated roadmaps remain
untouched; the book was not force-added.

### Local verification and remaining baseline failures

All Rust builds are serial, one Cargo command at a time and one build job,
with dev debug/incremental disabled. Test/check commands use locked offline
dependencies and serial durable/process tests. The established Clippy policy
was retained without new allowances. No release build was run.

| Check | RCM8/RCM9 result |
|---|---|
| Five RCM8 checkpoints | 130 passed; 2 unchanged stale-version assertions |
| Recovery moved intact before helper split | 33 passed |
| Broader final recovery/adapter selection, 13 targets | 88 passed; 1 known parallel-recovery failure |
| Durable unit filter, including injected recovery rollback | 7 passed |
| Runtime foundation and infrastructure-policy unit filter | 15 passed |
| Native separation rebuilt against the final engine | 4 passed; build without warnings |
| Public contracts, DDL/fixtures/boxing, recursive-reader and architecture guards | 14 passed |
| Established engine Clippy policy | Passed, no new allowances |
| Portable CLI contracts | 17-file shard passed |
| TypeScript check and eight affected source verifiers | Passed |
| Ordered durable/runtime preservation audits and Rust formatting | Passed |
| Ignored developer book and regenerated atlas | Passed; 1,731 source files, 77 pages, 3,364 local links |

The broader final selection was `core_contract_paths`, `dag_engine`,
`module_recovery`, `workflow_call_recovery`, `fork_recovery_runtime`,
`parallel_recovery`, `for_each_recovery`, `compensation_runtime`,
`remote_dispatch_authority`, `backup_restore`, `retention`, `approval_runtime`,
and `approval_resolution`, using `--no-fail-fast` so one target would not
conceal later results.

Two RCM8 selections found unchanged assertions:

1. `workflow_call_admission::admission_creates_one_truthfully_bound_child_and_reuses_it`,
   line 216, requires `DURABLE_STORE_SCHEMA_VERSION == 15` although it is 20.
2. `runtime_ownership::store_v14_migrates_transactionally_from_v13`, line 68,
   has the same unconditional assertion. Its preceding comparison of the
   actual migrated marker to the current constant passes; migration rollback
   coverage passes too.

The exact committed `6daaff6` tests contain those assertions and its schema
constant is already 20. This is direct static evidence, not a claim that an
archived executable was rerun in this turn. Both tests remain unchanged and
were executed, not skipped. No schema bump or loosened assertion hides them.

The final broader run reproduces the previously documented
`parallel_recovery::wait_all_resumes_pending_siblings_after_a_recorded_failure_then_fails_the_group`
failure: resumed `fieldData` completion does not match its durable child
outcomes. Earlier independently established baseline failures remain recorded
in their own sections; this refactor does not fix them or claim fully green CI.
One cancellation checkpoint also emitted a Bun 1.3.14 child-host segmentation
fault diagnostic during cancellation; the Rust test and command passed. No
host version or timing policy was changed to suppress that diagnostic.

Source verifier output describes configured release/platform contracts, not
executed platform coverage. macOS/Windows, actual-addon export loading,
installed-package journeys and full performance/security release gates remain
unrun locally for these phases. No commit or push was made. The next planned
implementation phase is RCM10, capability authority and process supervision.

## RCM10 and RCM11: capability, process, and artifact ownership

These phases start from committed RCM8/RCM9 revision `a99d1fa` on
`refactor/rust-core-modularization`. The exact revision, source hashes, public
inventories, production declaration fingerprints, and original test bodies
are frozen in [rcm10-baseline.json](rcm10-baseline.json). No dependency, schema,
language, protocol, native export, or retry-policy revision is introduced.

### RCM10: lookup is not permission; supervision is not replay safety

`capability.rs` is now a stable 50-line facade. Its private modules retain
these distinct responsibilities:

| Owner | Responsibility and preserved boundary |
|---|---|
| `contracts.rs`, `handler.rs` | Frozen call/result/descriptor shapes, Serde behavior, constants and handler defaults. A valid request is not permission to invoke it. |
| `registry.rs`, `validation.rs` | Handler selection, contract/version/input/result checks, safe metadata, bounded timeout/cancellation/crash supervision. This is not a second durable authority. |
| `concurrency.rs`, `cancellation.rs` | Existing invocation/global permit accounting, Drop release, shared atomics and cooperative cancellation notification. No new queue or synchronization primitive. |
| `authority.rs`, `identity.rs`, `policy.rs` | Exact active attempt/lifecycle/iteration/correction identity, stable operation key, current locks, trusted policy evidence, managed operation start and terminal facts. Start-before-effect and settlement-before-return ordering are unchanged. |
| `native_fetch.rs` | Frozen observations of fetch performed by Bun, durable observation checks and closure. Observation does not turn the operation into replayable managed I/O. |
| `operational_output.rs` | Authorized, redacted operational records and progress snapshots, separate from authoritative workflow events. |
| `failures.rs`, `test_handler.rs` | Existing safe envelopes and the existing deterministic exported test capability. |

`process.rs` is a stable 69-line facade. Contracts, prepared policy, request
validation, executable identity, handler authorization, live execution,
environment bootstrap, containment, platform operations, output draining,
incremental redaction, and failures have named private homes. The common
containment owner still delegates to Unix process groups or Windows Jobs;
the OS-specific functions live in `platform/unix.rs` and `platform/windows.rs`.

The extraction preserves canonical executable pinning and replacement checks,
literal argument arrays, cwd containment, policy-only application environment,
and the Windows kernel-derived `SystemRoot` exception after `env_clear()`.
It retains Job kill-on-close/handle ownership, Unix group signals, cancellation,
timeout, normal leader-exit and dropped-future cleanup. Concurrent pipe readers
still drain after retained capture fills; redaction keeps split-secret and
incomplete UTF-8 state. Operational-output authority failure still terminates
the supervised tree rather than silently continuing unobserved.

The original process tests moved to `process/tests.rs`. The enclosing filter
`process::tests` and exact `process::tests::fixture_child` subprocess entrypoint
are preserved. Only relative fixture include paths/imports changed; the
assertions, attributes, test bodies and Windows-only case remain frozen.
The RCM10 checkpoint passed all 24 Linux process tests and 21 capability,
native-fetch and infrastructure-policy-authority integration tests **before**
artifact implementation moved.

### RCM11: the ledger decides access; backends preserve bytes

`artifacts.rs` is a stable 93-line facade. The existing backend trait is
retained, not replaced with a universal filesystem framework:

| Owner | Responsibility and preserved boundary |
|---|---|
| `contracts.rs`, `policy.rs` | Public reference/request/inspection shapes, operator modes/limits, prepared roots and local/S3 configuration, symbolic credentials and safe debugging. |
| `store.rs`, `schema.rs` | Managed store binding, selected database opening, original artifact DDL/version checks and store identity. No new metadata database or schema bump. |
| `authorization.rs` | Run grant, status, expiry and exact supplied-reference equality before backend content use. Digest knowledge and blob existence are not authorization. |
| `transfer.rs`, `filesystem.rs` | Stable operation receipts, source containment/reparse/identity checks, scanning, quotas, publication and logical settlement. |
| `backend/{mod,local,s3}.rs` | Existing verified-byte interface, local/S3 publication, deduplication, integrity, inventory and store binding; staged-file and temporary-blob Drop ownership. |
| `handlers/{put,read}.rs` | Validated identity-bound capability calls, transfer concurrency, timeout/cancellation and safe metadata. |
| `materialization.rs` | Authorized invocation-private copies, byte/digest limits, cancellation, private `.part` files, sync/publication/final verification and cleanup. |
| `maintenance.rs` | Bounded retention, scrub and orphan passes protecting active access and unreleased rich-review pins. |
| `backup.rs`, `inspection.rs` | Metadata-derived verified inventory, supported export/restore/verification, bounded inspection, explicit destination-safe export and audit. |
| `output.rs` | Explicit bounded redacted operational-output rollover, ordered settlement and artifact publication. Denial does not create an alternate stage. |
| `validation.rs`, `identity.rs`, `failures.rs` | Strict requests/names/content types, existing identifiers/digests and unchanged safe failures. |

Transfer-start bookkeeping and later settlement keep their original separate
transactions. `put_started` still publishes backend bytes inside its metadata
settlement transaction before writing the successful logical relationships
and committing. SQLite cannot roll back already-published remote/local bytes:
a failed commit may leave an orphan, never invented logical success. Likewise,
maintenance commits logical removal before deleting unreachable bytes. No
extraction turned these boundaries into a claimed distributed transaction.

All original artifact tests moved into focused `tests/` modules for publication,
references/materialization, lifecycle, backup, backend conformance, operator
inspection/budgets and output. Shared fixtures and the controlled local S3
server live in `tests/mod.rs`. The existing `artifacts::tests` CI filter still
discovers all cases. Individual artifact case paths now include their focused
module; no script invokes them by an old exact path. All 16 tests pass,
including both-backend conformance, corrupt/foreign/expired references,
partial-file cancellation, verified backup, active-run/review protection,
bounded maintenance and output rollover. The controlled S3 fixture is not
certification of every real S3-compatible provider.

### Preservation, source verifiers, and developer navigation

The persistent Bun guard matches **all 422 original production declarations**:
99 capability, 103 process, and 220 artifact declarations/methods. It retains
literal contents, SQL, attributes and statement order; only formatting,
trailing commas and documented family-private visibility are normalized.
It also matches 65 original test/helper declarations, normalizing only fixture
include relocation in addition to formatting. This includes the Windows-only
process case, which cannot execute in this Linux environment. Earlier durable,
runtime, fixture, boxing, public API and native-name guards remain intact.

Three new compile-only `core_contract_paths` cases cover public capability,
process and artifact types, root/direct-module type identity where originally
exported, constants, traits and free-function paths. Generic artifact functions
are checked through typed closures that are never invoked; an API-path test
must not perform artifact I/O or maintenance. All 12 contract-path cases pass.

The artifact release verifier now uses the established owned-tree Rust reader
instead of reading only the facade. Its required markers and forbidden
telemetry assertions are unchanged. Process/policy/streaming verifiers retain
their assertions and configured platform/native-target inventories. Passing
these source checks is not the same as executing installed platform packages.

The codebase-documenter workflow guided the ignored local book updates:
registry versus permit-office explanations; process policy, cleared environment,
platform containment, draining and redaction; artifact ledger versus warehouse,
publication/orphan boundaries and cleanup; concrete source-reading routes and
focused test locations. Architecture, services, policies, streaming, recovery,
schema and responsibility maps, beginner story, edition inventory and atlas
were updated together. The existing book ignore rule is unchanged, and no
unrelated roadmaps were modified or force-added.

### Verification scope

One Cargo command and one build job run at a time, with serial durable/process
tests, locked offline dependencies, and dev debug/incremental output disabled.
No release/native-addon rebuild, package installation or dependency update was
performed. The original Windows x64/ARM64 and Unix packaged containment and
artifact lanes remain required CI/release verification, along with real
macOS/Windows filesystem and process behavior. RCM14 remains pending.

| Check | Local result |
|---|---|
| Before extraction: process/artifact units and capability/fetch/policy authority | 61 passed |
| RCM10 process/authority checkpoint | 45 passed |
| RCM11 full artifact unit/conformance filter | 16 passed |
| Final rebuilt process/artifact unit filters | 40 passed (24 process, 16 artifacts) |
| Native separation rebuilt against the final engine | 4 passed; build without warnings |
| Expanded root/direct-module contract-path tests | 12 passed |
| Final capability/artifact/output/worker/review/backup/retention integration selection | 86 passed across 10 targets |
| Production/test-body/API/DDL/fixture/boxing/native-name/source-reader/architecture guards | 16 passed |
| Portable CLI contracts | 17-file shard passed |
| TypeScript and ten affected source verifiers | Passed |
| Established engine Clippy policy | Passed without new allowances |
| Rust formatting and diff validation | Passed |
| Ignored book/atlas validation | Passed; 1,790 source files, 77 pages, 3,510 local links |

The final integration targets were `core_contract_paths`, `capability_authority`,
`native_fetch`, `infrastructure_policy_authority`, `operational_output_store`,
`services_script_host_v4`, `remote_worker_attempt`, `backup_restore`, `retention`,
and `rich_plan_approval_authority`, run with `--no-fail-fast`. The 17 output-store
cases cover ordering, bounded records, retention, recovery and publication
budgets; artifact unit cases separately check governed rollover. The authority tests include both rejected
calls that never reach a handler and terminal event commit before returning.

Selections overlap and should not be added as distinct tests. Earlier baseline
failures documented above remain untouched; these phases do not fix them or
claim the whole CI matrix is green. No timeout, output/resource limit,
performance budget, test assertion or Clippy allowance was relaxed. No commit
or push was made. The next implementation phase is RCM12, the remaining
feature-family ownership audit. (This is the RCM11 handoff; see the completed
RCM12/RCM13 checkpoint below for the current next step.)

## RCM12: remaining-family ownership audit

These phases start from `b43f129` on the existing refactor branch. The
[pre-extraction baseline](rcm12-baseline.json) freezes 410 production
declarations/methods across six families, their public inventories, the original
webhook tests and 35 explicitly retained source files. Literal contents,
attributes, SQL and statement order are included. The guard normalizes only
comments, formatting/trailing commas and documented family-private visibility.
Earlier schema, fixture, API, boxed-future and native-name baselines are not
replaced or regenerated to accept a changed implementation.

Before extraction, the selected remote/notification/webhook baseline passed
52 tests. After the RCM12 engine checkpoint, the expanded selection passed
75 tests, including authenticated mTLS loopback, worker cancellation and terminal
redelivery, output deduplication, dispatch recovery/ambiguity, event fan-out,
schedule/interval admission, webhook contention and notification recovery.

### Extracted ownership

| Stable entry | Private owners and why |
|---|---|
| `remote_worker_journal.rs` | `contracts`, `schema`, `persistence`, `preparation`, `secrets`, `artifacts`, `execution`, `identity`: separate journal evidence, permission/integrity preparation and live attempt supervision. |
| `remote_dispatch.rs` | `contracts`, `schema`, `identity`, `validation`, `transactions`: coordinator transitions stay on the same `DurableEventStore` connection, not a new feature repository. |
| `remote_coordinator_server.rs` | Parent owns listener, stop/join and connection accounting; `contracts`, `connection`, `http`, `tls` separate bounds, authenticated routing, framing and trust loading. |
| `webhook.rs` | Parent owns prepare/activate/stop. Children own shared `state`, `contracts`, `preparation`, `authentication`, `responses`, `admission`, `signals`, `events`, `whatsapp`, `schedules`, `intervals`, `routing`, `waits`, and `dispatch`. Original authentication cases retain `webhook::tests` discovery. |
| `notification_runtime.rs` | `contracts`, `journey`, `builtin`, `custom`, `retry`, `diagnostics`: outbox orchestration stays distinct from transport hosts and provider-specific message/result adaptation. |

Journal acceptance still precedes acknowledgement; start remains immediately
before the execute frame. Terminal replay precedes drain rejection. Validation
and authority intersection remain before acceptance, secret values resolve at
the worker, and unknown started effects remain ambiguous. Capacity guards and
Drop behavior retain their original owners and ordering.

Dispatch transaction bodies remain together with their exact identity checks,
event append, dispatch/output-sequence writes, audit and commit. No per-feature
connection or generic transaction framework was introduced. Worker facts do
not authorize retries or downstream scheduling independently of the coordinator.

Accepted coordinator sockets still explicitly become blocking under the same
read/write deadlines. Certificate-derived identity, message limits, HTTP/TLS
response flush/close, listener shutdown and connection Drop accounting are
unchanged. Ingress preparation/activation retains its admission gate, routing
registration/rollback, targeted recovery and bounded stop. Schedule/interval
producers are distinct from DAG eligibility. Provider adaptation retains
ambiguous-send/error classification and cannot manufacture a human decision.

### Explicit retain-as-is decisions

All 35 files below are hash-pinned to the pre-RCM12 baseline. Retaining a file
is an ownership decision, not a promise that it will never benefit from future
work. A meaningful new boundary requires its own evidence and review.

| Retained modules | Existing responsibility and rationale |
|---|---|
| `remote_protocol`, `remote_coordinator`, `remote_transport`, `remote_execution`, `remote_worker`, `target_registry` | Versioned wire contracts, authenticated sessions, offer/terminal transport, output ingestion, worker configuration and target registry already have distinct owners. They remain separate from journal persistence/execution and HTTP/TLS. |
| `notification_host`, `custom_notification_host`, `notification_protocol`, `custom_notification_provider_protocol` | Host supervision and two explicit framed protocols are cohesive boundaries. Do not merge them with the journey or messaging capabilities. |
| `http`, `database`, `database_postgres`, `storage`, `cache`, `durable_state` | Separate capability handlers, storage adapters and mutation authority already exist. SQLite and PostgreSQL differences and state mutation proof deserve explicit APIs, not a new generic service layer. |
| `infrastructure_policy`, `infrastructure_policy_runtime` | Contract/trust preparation and executable evaluation/evidence are already distinct. Further splitting would scatter the deny/authority decision without a new consumer boundary. |
| `operational_output` | One separate bounded output-store authority owns sequences, coalescing, retention and gap receipts. Keep its transaction semantics together; do not fold it into workflow events. |
| `telemetry`, `telemetry_propagation` | Safe signal generation and inbound/outbound propagation already have separate boundaries. Export is observation, not workflow authority. |
| `presentation` | One bounded read-model/presentation vocabulary belongs together; rendering cannot settle runs. Its size alone does not justify splitting every record family. |
| `backup`, `retention` | Supported snapshot/restore and governed collection already have separate owners, inventories and maintenance leases. Preserve their mixed evidence/coordination protection rules. |
| `events_service`, `workflow_calls`, `schedule`, `interval` | Internal publication, child-call coordination and two clock semantics are distinct capabilities. Trigger-host adapters moved; these underlying contracts and authorities did not. |
| `telegram`, `discord`, `whatsapp` | Provider-specific messaging capabilities remain distinct from ingress callbacks and notification transports. Their differences are meaningful, not duplicate boilerplate to erase. |
| `locks`, `rich_approval`, `host`, `performance` | Lease contracts/authority, review authority, supervised script-host protocol and opt-in profiling are separate focused modules. Do not extract them into a miscellaneous shared utility layer. |

## RCM13: private native bridge owners, frozen JavaScript doors

All 68 original `#[napi]` attributes, argument lists and export bodies remain
in `core/woml-native/src/bridge.rs`. It is still approximately 2,675 lines
by design: relocating exports would add generated-interface risk without
being needed to expose the helper responsibilities.

| Private owner | Responsibility |
|---|---|
| `configuration.rs` | Runtime options, modules, secret maps, process/artifact/reviewer authority and infrastructure-policy conversion. |
| `contracts.rs` | Native request/response, registration, outcome and inspection JSON records with unchanged Serde attributes. |
| `errors.rs` | Existing structured error envelopes, safe classifications and N-API conversion. |
| `progress.rs` | Ordinary callbacks, output wakeup coalescing, enabled telemetry and original lifetime/atomic rules. |
| `registries.rs` | Single webhook and remote-coordinator registries and their exact resource owners. |
| `remote.rs` | Adapter preparation of one remote attempt, delegated to engine verification/authority. |
| `validation.rs` | Existing definition-binding checks, manual rejection encoding and output cursor validation. |

Imports in the parent remain private; child helpers use `pub(super)` only
within the bridge tree. No private helper becomes an external Rust or N-API
API. The engine-only dependency boundary remains unchanged.

Ordinary progress retains queue size zero (unbounded) and `Blocking` delivery;
output wakeups retain one `NonBlocking` slot, `fetch_max`, compare/exchange and
reset ordering; enabled telemetry retains its bounded 1,024-slot `NonBlocking`
lane. Existing `unref` calls remain. Do not describe every callback lane as
interchangeable best-effort delivery or silently change its backpressure.

There is still exactly one `WEBHOOK_RUNTIMES` map and one `REMOTE_COORDINATORS`
map. Webhook stop removes the registered owner before signalling and joins
through `spawn_blocking`. Remote stop retains explicit shutdown and server
Drop. Startup audits, ownership acquisition, preparation, activation, heartbeat
and release keep their original order. The new files do not own detached copies
of channels, connections, transports or lifetime registries.

Both native separation guards scan every nested Rust source, permit the
legitimate private `super::` imports and reject parent-relative legacy imports
as well as the original forbidden dependencies. Negative nested-module tests
remain, and Rust adds positive private-owner and negative parent-legacy cases.
Waits and atomic-activation source verifiers now read the owned trees rather
than facade text alone, without dropping marker/privacy assertions.

### Actual addon and installed-package evidence

A single-job **debug** addon was built after the RCM12 checkpoint and before
the bridge move, copied to a unique temporary location, then rebuilt after
RCM13. [verify-rust-core-native.ts](../../woml-cli/scripts/verify-rust-core-native.ts)
loads real addons and optionally compares both:

```bash
bun woml-cli/scripts/verify-rust-core-native.ts <before.node> <rebuilt-library>
```

Both passed identically: exact 68 names, function types and observable `.length`,
ordinary async execution/rejection, durable sync inspection/error envelopes,
synchronous callback-API decode failure, Promise-returning durable execution,
lifecycle progress, coalesced operational wakeups, waiting outcomes, prepared
ingress returning 503, activation admitting 202, trigger/telemetry callbacks,
stop removing ownership, and rebinding the same address/store. N-API reports
`.length === 0` for these functions; **that alone does not prove positional
argument compatibility**. Frozen source signatures/bodies and actual positional
invocations provide the complementary evidence. Not every export's full
business workflow is exercised by this harness.

Fresh portable/native npm candidates were staged under a unique temporary
directory using the normal packaging/sealing/load-test helpers and the rebuilt
debug addon, without overwriting the existing staged addon or publishing.
The existing `smoke-release-candidate.ts` passed its clean installed Linux
journey, including real workflow/process/durable-wait behavior. The full
architecture-separation verifier also passed. Its default addon check uses
the existing staged path; the dedicated comparison and temporary candidate
above are the checks proving the newly rebuilt binary.

This is not an optimized-release performance result or Windows/macOS
certification. Full platform/native-architecture/package/budget gates remain
RCM14 work. No dependency, protocol/schema, package allowlist, resource limit,
performance budget or existing assertion was relaxed.

### Separate unchanged release-verifier failure

`verify-production-triggers-release.ts` passes its T5–T13 checks, then its
transitive retry verifier fails RI7 because `examples/retryWorkflow.woml`
does not exactly match `woml/tests/fixtures/retry.woml`. Both files and
`verify-retry-release.ts` are byte-for-byte unchanged from `b43f129`:

| File | Pre-refactor/current SHA-256 |
|---|---|
| Product retry example | `4e5812cef003b1f46c033a1c873cb7936459ec06ccb56a9554173518d4a9fb91` |
| Frozen retry fixture | `6ad7eba956c9996f66c14bd7dba2e427e72a926564b02fbce8974faf9130f375` |
| Retry verifier | `d70b607decd91b73da09398097411c7cdab554b9b79f9e151ecf5a21728f1849` |

This input mismatch is not a Rust relocation regression. It remains visible,
as do the earlier baseline failures documented above; do not silently update
a golden fixture or weaken equality as part of this refactor.

The codebase-documenter workflow guided updated native, ingress, notification
and remote reading routes, callback/lifetime explanations, architecture/schema
maps, beginner story, edition metadata and per-file atlas roles. The book's
existing ignore rule remains unchanged, and unrelated untracked roadmaps are
not part of this work. No commit or push was made. Next: **RCM14**.

### RCM12/RCM13 verification summary

All Cargo commands ran one at a time with `-j 1`, `--locked --offline`, dev
debug information and incremental compilation disabled; durable/port/process
tests ran serially. Debug addons were built deliberately instead of starting
an expensive optimized release build on the developer machine.

| Check | Local result |
|---|---|
| Before extraction: worker protocol/attempt, dispatch/transport/mTLS, webhook and notifications | 52 passed |
| RCM12 checkpoint: above plus event, schedule, interval, lifecycle notification and custom-provider protocol | 75 passed across 12 targets |
| Final service/policy/output/backup/retention/public-path selection | 56 passed across 8 targets, including 13 public-path and all 17 output-store budget cases |
| Original `webhook::tests` filter after relocation | Exactly 2 tests discovered and passed |
| Rebuilt native separation and parent-relative negative checks | 6 passed without compiler warnings |
| Frozen production/test/API/DDL/fixture/boxing/native/source-reader/architecture unit guards | 17 passed |
| Actual before/after debug addons | Exact 68-export surface and selected positional sync/async/error/callback/lifecycle contracts passed identically |
| Fresh temporary Linux portable/native installed candidate | Existing release-candidate smoke passed with the rebuilt debug addon |
| Complete architecture-separation verifier | Passed; see the addon provenance distinction above |
| Portable CLI contract shard | All 17 files passed; 138 test files remain assigned to bounded shards |
| TypeScript, exact rebuilt-addon export verifier | Passed |
| Affected waits/activation/ownership/observation/policy/streaming/review/correction source verifiers | 8 passed with original assertions |
| Production-trigger verifier | Its T5–T13 checks passed; transitive RI7 retry-example mismatch remains an unchanged baseline failure |
| Established Clippy policy applied to both engine and native libraries | Passed; no new allowances |
| Rust formatting and diff hygiene | Passed |
| Ignored developer book and regenerated atlas | Passed; 1,837 source files, 77 pages, 3,626 local links |

Selections overlap and are not counts of distinct tests. Optimized release,
the complete runtime operations/package matrix, Windows/macOS, all native
architectures and final full-release budgets remain independent RCM14 gates.

## Post-merge F02: corrective managed-service authority

This is a behavioral follow-up, not another file extraction. The original RCM
snapshots remain unchanged. The scoped
[compatibility repair record](post-refactor-compatibility-repairs.json) pins the
exact F02 replacements and added private identity helper. It records why the
capability declarations and `recover_run` transaction differ from the frozen
refactor and points to executable regressions.

Corrective calls now prove their exact active run/saga/action/attempt/invocation
and derived operation key. The existing policy and lock gates remain in place.
Operation events recognize compiled corrective actions, including per-iteration
identities, without pretending those actions are ordinary DAG nodes. Only active
corrective work gains the operation-runtime exception during cancellation.

Open operations must settle before their corrective action. Recovery performs
those terminals, interruption, and manual-recovery marking in the existing
single transaction. Immutable StateService receipts retain their narrow proof:
the mutation can be proven committed while the script still requires manual
recovery. F03 policy-adapter support is not added by this fix.

The correction CI matrix also runs `capability_authority` on Linux, macOS, and
Windows. Local execution does not certify those remote platform results.

## Post-merge F03: StateService infrastructure-policy adapter

This behavioral repair adds the six existing State v1 operations to the trusted
policy adapter list. It does not alter the original modularization snapshots.
The [compatibility repair record](post-refactor-compatibility-repairs.json)
explicitly pins the reviewed replacements for the retained `durable_state.rs`
and `infrastructure_policy_runtime.rs` files, including their original hashes
and regression path. Every other retained-file hash and the public API, schema,
transaction, fixture, and native-export guards stay in force.

The adapter shares the State v1 parser and produces bounded operation/scope
facts and hashed key identifiers, never state values. Reads and idempotent
writes retain their handler-owned effects. Policy allow/deny, required evidence,
and emergency overlays run through the existing authority; state scoping,
mutation receipts, version checks, and quotas remain unchanged.

The `state_policy` regressions are part of the existing
`infrastructure_policy_authority` integration target, so the Infrastructure
Policies matrix runs them serially on Linux, macOS, and Windows. The correction
matrix also exercises StateService policy decisions during compensation and
cleanup. Local Linux results do not certify those other platform results.

## Post-merge F04: nested workflow calls and parent completion

The nested-call failure was reproduced, then separated into invalid activation
setup and a real runtime ordering defect. The Rust composition test registered
raw Model v7/v8 targets, whose events cannot represent workflow-call ingress.
Its helpers now mirror CLI activation with Model v11 and required script
bindings; historical fixtures and event validators are unchanged.

A separate short-deadline regression reproduced the parent-left-Running issue:
the Script Host completion cancelled the managed call but did not wait for its
durable terminal event. The attempt-failure validator correctly rejected that
ordering. The host now tracks per-call settlement acknowledgements, signals
cancellation immediately, and waits without holding its shared transport mutex
before publishing completion. The ten-second barrier fails closed on expiry or
lost acknowledgement. It does not relax active-operation guards, assume external
rollback, or replay uncertain work.

Child duplicate validation also used an Event v11-only check for every
policy-managed model. It now requires the event version selected by the stored
definition, retaining the complete identity and history validation. The
admission transaction and its atomic ordering are unchanged.

The [repair record](post-refactor-compatibility-repairs.json) pins the exact
reviewed host replacement against its original retained-file hash; no refactor
baseline is regenerated. Regression targets cover nested branch/parallel/retry
composition, retry/transport reattachment, modern child admission after reopening,
policy-slot release/resume, explicit parent deadlines with an independent child,
and concurrent/lost host settlement acknowledgements. The existing correction
matrix runs workflow-call, policy-scheduler, and host integration targets serially
on Linux, macOS, and Windows. Local results do not certify the remote platforms.
