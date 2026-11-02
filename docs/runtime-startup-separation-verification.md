# Startup/request separation: implementation and safety record

This records the bounded SRS2/SRS3 implementation, not release-performance
acceptance. The implementation branch is `perf/runtime-performance`. The frozen
[SRS0 baseline](runtime-startup-separation-baseline.md) remains unchanged.

## What changed

The owned native coordinator retains its acquisition connection through startup,
performs required initialization/recovery and its existing first StateService
binding, then publishes private Rust readiness for that exact file and owner.
Requests open non-creating, guarded connections instead of repeating startup.
Separate execution/authority connections and per-connection projections remain.

Prepared write transactions recheck file/schema/owner identity under the writer
lock and before commit. Rejection rolls back SQL and does not publish a candidate
projection. Guarded direct writes retain transactional checks. Readiness never
replaces policy, operation identity, claims, leases, fencing or ambiguity rules.

The state binding is deliberately moved, not eliminated: owned startup performs
one additional already-existing preparation step instead of the first request
entering full engine initialization. Its effects on activation/first-request
latency must be included in SRS4, not omitted from performance reporting.

Drain distinguishes new requests, bound runs, and exact admissions whose SQL
commit succeeded before their ingress response returned. Heartbeat continues
through drain. Stop releases only the exact owner and drops the file pin even if
a dormant continuation retains a revoked options clone. Failed preparation
uses the acquisition connection for cleanup; it does not create/repair a file.

## Safety evidence

The production tests live in
[prepared/tests.rs](../core/woml-engine/src/durable/prepared/tests.rs), separate
from the historical test-only design model. They exercise real SQLite files,
connections, event transactions, projection caches and real Bun hosts.

Coverage includes:

- successful startup receipt versus marker-only connection; wrong owner,
  activation, file and acquisition connection;
- missing file/non-creation, same-path replacement, Unix ABA, unsupported VFS,
  schema-version/cookie changes and Windows delete-denying pin behavior;
- ownership transfer after opening, exact post-expiry renewal, revoked clones,
  successor-safe release and guarded autocommit;
- pre-commit revocation, injected SQL failure, rollback/cache authority,
  cross-connection visibility and opening while another writer holds a lock;
- drain eligibility and a committed admission returned after drain began;
- preparation-store startup counters and scoped prepared counters; attempted
  global recovery through a prepared store rejects;
- direct prepared-engine success and controlled failure through policy, state,
  parallel work, branches, loops, markup locks, lifecycle, compensation and
  cleanup, compared with the unprepared engine reference.

The [actual native verifier](../woml-cli/scripts/verify-startup-separation-native.ts)
runs serial isolated child processes with the explicitly selected fresh addon.
It compares accepted fact classes, step/attempt/operation facts, final results,
history sequencing, integrity/foreign-key checks and released scheduler claims.
It does not compare variable run IDs, timestamps or parallel completion order.

The native cohort includes sequential work, hooks, StateService, StorageService,
parallel work, for-each, retry/timer/output, an independent later request,
duplicate admission, idle owner heartbeats, stop and an injected successor owner.
The verifier supplies profiling settings before process creation, never polls
native inspection during structural request checks, and asserts unchanged
full-initialization counts across selected owned request/heartbeat/stop scopes.
Unowned reference execution occurs outside those scopes. A per-child watchdog
terminates only that verifier's process tree; failures retain diagnostic files.
Successful children exit before parent cleanup, including Windows DLL handles.

## Checkpoint results

Local Linux validation passed on the final candidate:

- All 143 engine unit tests, including 28 prepared/prototype safety tests;
  13 core-contract checks and 5 ownership tests.
- 54 neighboring backup/restore, cancellation, waits, lifecycle, runtime-policy
  store and workflow-routing tests passed.
- 33 source/modularization/architecture/CI contract tests passed with explicit
  declaration repairs; historical baselines were not regenerated.
- 10 startup/session measurement and private-boundary contract tests, plus
  TypeScript checking with the actual-native verifier explicitly included.
- Seven actual-native prepared/reference workflow comparisons, the independent
  later request and duplicate checks, idle heartbeat/stop zero-init checks, and
  the eighth isolated child exercising successor-owner refusal/preservation.
- All 68 real native exports and the existing sync/async/error/callback,
  activation, telemetry, shutdown and listener rebind verification.
- Final Rust formatting, documentation links/public examples and the established
  Clippy policy, without new lint exemptions.

These checks overlap and do not constitute one combined test count. The fresh
debug addon used for the final native checks has SHA-256
`7191152d2d37892401845963d531d07584da1979d20cb0ae30d186f467056667`.
It is an uncommitted-worktree correctness candidate, not an optimized release
artifact or proof that the checked-out HEAD already contains these edits.

## Findings retained rather than hidden

1. The first native harness attempted manual admission before activation. That
   existing ingress waits for its activation receiver and blocked the harness.
   The corrected harness verifies an empty store before activation instead;
   existing HTTP readiness tests verify rejection of suspended HTTP traffic.
2. The first actual trace found StateService's hidden first-binding full engine
   open. It is now an explicit owned startup obligation, not an excluded request
   span or an inflated budget.
3. The existing trigger-registration gate accepts Model v7–v17 and rejects the
   modern lock/saga cohort. This optimization does not widen that compatibility
   gate. Modern lock/correction tests therefore exercise the prepared engine
   boundary directly, not native trigger activation. Fixing/supporting that
   activation gap requires a separate compatibility change.
4. The default-stack native for-each probe exposed stack pressure during nested
   iteration/event decoding. An isolated diagnostic-only larger-stack probe
   completed. Debugger frames located the native admitted/policy continuation
   chain and nested iteration/event decoding at the stack boundary. Production
   thread-stack limits are not raised: admitted, policy, resumed, loop,
   iteration and invocation futures receive explicit boxing boundaries. The
   final native gate removes inherited `RUST_MIN_STACK` overrides and passes
   with ordinary stacks. Failed probes are not performance samples.

## Deliberately unchanged and unrun

No dependency, SQLite journal/synchronous setting, schema/model/event version,
N-API export or performance budget changes are introduced. Existing startup
migrations, index repair, operator reconciliation and durable ambiguity remain.
Original refactor snapshots stay frozen; the explicit
[startup separation repair manifest](refactoring/runtime-startup-separation-repairs.json)
records changed ingress/native declarations separately from historical repairs.

Standalone commands/inspection, backup/restore, independent/unowned embedding,
worker journals, notification transport/provider lifecycle internals,
programmatic-lock service opening and the separate child-routing configuration
remain outside the zero-initialization claim. Child policy/configuration
inheritance is not certified by this work.

Windows/macOS behavior is wired into the existing combined-experience CI matrix;
local Linux success is not a platform pass. SRS4 still requires a fresh optimized
candidate, matched baseline/candidate cohorts (including startup cost), all
failures/outliers and memory/CPU evidence, installed-package journeys and final
cross-platform acceptance. No percentage speedup is claimed here.

## Reproduce

Run serially from the repository root, using available dependency caches:

```bash
bun run --cwd woml-cli test:startup-request-separation-contracts
cargo test --config profile.dev.debug=0 --config profile.dev.incremental=false -j 1 --locked --manifest-path core/Cargo.toml -p woml-engine --lib durable::prepared -- --test-threads=1
bun woml-cli/scripts/verify-startup-separation-native.ts path/to/fresh/library
bun woml-cli/scripts/verify-rust-core-native.ts path/to/fresh/library
bun run --cwd woml-cli test:rust-core-modularization-contracts
```

The combined CI command uses that job's freshly built/staged addon:
`bun run --cwd woml-cli test:startup-request-separation`.
These are correctness/structural checks, not an accepted timing capture. Never
run compilation or unrelated test workloads during SRS4 timing measurements.
