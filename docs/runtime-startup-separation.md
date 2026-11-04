# Runtime startup separation: inventory and measurement contract

Status: bounded SRS2 production integration with SRS3 safety tests. Owned native
requests use guarded prepared opening; unowned/one-shot behavior remains.
The installed repeated-request/restart/restore regression passes locally after
the startup metadata repair. Matched performance acceptance and platform CI
remain pending; see [the release evidence](runtime-startup-separation-release.md).

## Current opener inventory

These are ownership classifications, not permission inferred from a filename or
schema marker. Converted callers still perform durable authorization and claims.

| Caller family | Owned path now | Unowned path / boundary |
| --- | --- | --- |
| [Native startup](../core/woml-native/src/bridge.rs) | Full open, audit, exact owner acquisition; retain acquisition connection through publication | Unchanged startup obligation; initial index repair still precedes owner acquisition. |
| [Trigger preparation](../core/woml-engine/src/webhook/preparation.rs) | Full opening, scoped recovery, definition/module/policy validation, first StateService binding, then private publication | Admission closed throughout. Marker-only ready connection is not a receipt. |
| [Admitted execution](../core/woml-engine/src/runtime.rs) | Guarded prepared connection bound to one durable run | No context: previous full opener. One-shot APIs do not inherit readiness. |
| [Capability attachment](../core/woml-engine/src/runtime/composition.rs) | Separate guarded authority connection | No shared projection/cache; policy, emergency denies and exact operation authority remain. |
| [Native owner heartbeat](../core/woml-native/src/bridge.rs) | Narrow exact-owner renewal, same timing and failure notification | Existing same-row post-expiry renewal retained; never renew a successor. |
| [Admission](../core/woml-engine/src/webhook/admission.rs), events, WhatsApp and [routing](../core/woml-engine/src/webhook/routing.rs) | Guarded prepared opening and transaction checks | Previously ready opens: no claimed full-open saving. Existing unowned ready path remains. |
| [Schedules](../core/woml-engine/src/webhook/schedules.rs), [intervals](../core/woml-engine/src/webhook/intervals.rs), [waits](../core/woml-engine/src/webhook/waits.rs) | Guarded owned discovery/admission | Existing eligibility/claim transactions remain. Exact newly committed dispatch identities may survive drain. |
| [Runtime wait/signal APIs](../core/woml-engine/src/runtime.rs), timeouts, scheduler-claim maintenance and correction discovery | Selected owned continuations use preparation | Without preparation retain full opening; standalone administrative APIs unchanged. |
| [Notification wrapper](../core/woml-engine/src/webhook/dispatch.rs) | Its store reads are prepared | Notification transport/journey internals and reusable-provider lifecycle remain deliberately unconverted. |
| [Workflow calls](../core/woml-engine/src/workflow_calls.rs) | Separate child-routing configuration/openers remain unconverted | Parent readiness is not destination authority. Complete child configuration inheritance is not certified. |
| [Programmatic lock service](../core/woml-engine/src/locks.rs) | Separate service opener remains unconverted | Distinct from markup locks using the prepared engine store. |
| Operator artifact metadata | Existing metadata DDL/validation prepared atomically before owned cookie publication | Prevents a first live `woml get` from changing the published schema. No backend, backend identity, network activity or capability grant. |
| Artifact backend binding | Separate managed backend opening remains unconverted | Metadata preparation is not backend activation or a zero-init claim for artifact-enabled attempt attachment. |
| Native activation, drain and stop | Guarded routing cleanup and exact owner release; no create/repair fallback | Failed preparation releases via original acquisition connection. Heartbeat remains alive during drain. |
| Standalone execution/inspection, backup/restore, independent Rust embedding | Original opening contracts | Never infer coordinator readiness from path or Arc reuse. |
| Worker journal/capability preparation | Separate worker ownership domain | Coordinator preparation cleared. |

The first StateService binding already validated/opened engine state on the
first request. It now runs once behind owned startup before publication; this
moves cost and preserves validation. Other resource attachment stays as before.
Existing operator artifact metadata is also prepared before publication because
standalone run inspection can otherwise lazily change the schema of a live
runtime. This does not enable an artifact backend. Output, providers,
worker/listener startup and telemetry are not eagerly enabled.

Lower-level `WomlWebhookServer` embedding stays unprepared unless its explicit
owned constructor receives the verified acquisition connection. Transplanting
prepared options into another activation rejects. See
[the verification record](runtime-startup-separation-verification.md) for
evidence and exclusions; not all runtime full opens have disappeared.

## Initialization obligations

See [opening](../core/woml-engine/src/durable/opening.rs),
[bootstrap](../core/woml-engine/src/durable/bootstrap.rs), and
[integrity](../core/woml-engine/src/durable/integrity.rs).

| Obligation | Evidence / authoritative writer | Disposition |
| --- | --- | --- |
| Atomic empty-store publication | Existing bootstrap transaction; rejects non-empty foreign/partial layouts | Preserve startup path and historical migrations. |
| Compatibility and integrity | Version/layout validators and startup audit | Complete before prepared publication; prepared connections still check current version/layout cookie, not full replay. |
| Correction indexes | [Correction index writer](../core/woml-engine/src/durable/indexes/correction.rs) and [event commits](../core/woml-engine/src/durable/events/commits.rs) update within event transactions | Preserve startup repair and ordinary transactional writes. Operator `externally_resolved` rows are mixed authority, not a disposable cache. |
| Policy/run-summary indexes | Event commits and policy/admission transactions maintain per-run rows; initialization conditionally rebuilds | Verify all converted mutation paths retain those writes. No lazy request rebuild or publishing tentative projections. |
| Expired scheduler claims | [Policy transactions](../core/woml-engine/src/durable/transactions/policy.rs) delete expired rows in claim/resume and inspect expiry when counting active capacity | Preserved. Idle heartbeats no longer incidentally perform startup's extra DELETE. Existing eligibility transactions, retention and fresh startup retain cleanup; expired rows cannot confer capacity/authority. No new global idle recovery task is added. |
| Interrupted runs | [Startup preparation](../core/woml-engine/src/webhook/preparation.rs) branches on live routes and scopes child recovery | Do not run general recovery during a request or against another live run. |
| Owner/routing renewal | [Ownership transactions](../core/woml-engine/src/durable/transactions/ownership.rs) | Keep exact durable predicates. Preparation is not a new fencing token. |
| Authorization/effects | Capability authority, policy evidence/emergency denies, run/attempt/claim/operation identities | Never cache away. An eligible prepared connection does not authorize a managed effect. |

Current full initialization repairs derived indexes before N-API owner
acquisition. This plan retains that existing serialized startup behavior; it
does not relocate index repair into a new owner-less request. Semantic recovery
and preparation publication remain behind successful ownership. Any proposed
change to the pre-claim startup ordering needs its own compatibility argument.

## Frozen SRS0 measurement contract

`scripts/startup-separation-baseline.ts` never builds native code or installs
packages. Supply the explicitly rebuilt optimized library; it checks clean
production source at capture, copies and hashes the library, fingerprints the
compiler/host/harness, and writes an immutable manifest to a fresh ignored
directory under `woml-cli/.woml/performance/`. Source/binary provenance is explicit
but is not an independently signed build attestation.

The fixed cohort is **13 scenarios × 3 independent child processes × 3 measured
requests = 117 requests**, plus **360 untimed seeding executions**:

- Sequential-2, storage-4, sequential-32 in fresh-store one-shot, reused-store
  one-shot, and activated modes: nine scenarios.
- Activated sequential-2 with 20 and 100 real finalized one-step histories.
- Activated sequential-2 with two concurrent later admissions (the first is
  isolated). This probe alone authors concurrency=2, and records its source hash.
- Activated sequential-2 with a 4.5-second idle window for owner heartbeats.

The existing three-minute workflow timeout and approved service boundaries stay.
The runner has a five-minute per-process watchdog including seeding, captures
stdout/stderr on failures, and terminates only its owned child process tree.
It checks resource headroom between processes; no native/test builds should run
during the accepted timing capture.

Seeding and compilation are outside request timings. Retained-history probes
are not fresh-process cold-activation measurements: seeding has already loaded
the addon. Preparation, activation, first request, later requests, seeding,
idle, and shutdown are separate fields. Histories and authoritative outcomes
are verified read-only after shutdown, without inspector polling in a timed run.

Own-process CPU includes native threads but excludes Bun hosts/workers; CPU
intervals overlap in the concurrent probe and must not be summed. RSS is sampled
after each request in that process only, not a process-tree peak or a leak test.
Broader memory and platform certification remain SRS4 work.

The concurrent probe can expand the existing host pool for its second
simultaneous request. Its later-request latency therefore includes that host
startup where needed; it is not a fully prewarmed concurrent-throughput test.

Latency uses each process's median of its two later requests, then summarizes
the three process medians. First requests are separate. These are not robust
tail-percentile estimates. Reverse scenario order in alternate batches; never
retry a slow process to select a better result.

Acceptance is frozen in `startupAcceptance` before candidate measurement:

1. Structural: zero full initialization/index rebuild/general recovery on the
   converted steady-state paths; exact attribution must be scoped by runtime,
   not inferred from overlapping global spans.
2. Target: at least 10% improvement in activated sequential-2 with 100 retained
   runs, measured as the median of process medians. This is a goal, not a claim.
3. Investigate >5% regressions in the one-shot controls, preparation/first
   request, or shutdown; overlapping/noisy ranges are inconclusive. Existing
   product budgets are not raised or replaced by these investigation criteria.
4. Investigate >16 MiB increase in sampled own-process RSS. Do not substitute
   this limited diagnostic for later host-tree/long-lived memory tests.

If more trials are needed, retain the initial evidence and predeclare a matched
expanded cohort. Do not tune cohort sizes, pick best batches, or discard outliers
after seeing a candidate. Traces run separately and inclusive span totals are
never added into an exclusive wall-time budget.

### Scoped structural evidence

The production connection adapter carries lifetime-owned counters for successful
startup receipt work and scoped prepared opening/recovery activity. They are not
process-global counters or thread-local async scopes. The startup receipt counts
that preparation store, not unrelated connections. Prepared global recovery
records the attempted call and rejects before scanning live runs.

Tests check zero full initialization/rebuild/general recovery in selected scopes
and real startup work in the receipt. The isolated native verifier separately
counts actual full-init spans before/after owned requests, idle heartbeats and
stop, without native inspection polling. Excluded APIs are not assigned zero
counts by assumption; no new telemetry labels are introduced.

## Reproduction

### Matched optimized comparison (release evidence)

The baseline command below captures a baseline; it does **not** compare two
libraries. For a matched comparison, retain that entire baseline directory and
use the separate explicit build and comparison commands from `woml-cli`:

```bash
bun run build:startup-request-separation-candidate
bun run compare:startup-request-separation --baseline .woml/performance/startup-baseline-EXAMPLE --candidate-receipt .woml/performance/startup-build-EXAMPLE/receipt.json
```

Replace both example paths with the actual printed paths. The build step normally
needs committed, clean production source and cached Cargo dependencies. For an
uncommitted implementation, explicitly add `--worktree`: the receipt retains
the production patch and its digest alongside pinned HEAD and the copied addon.
Untracked production files still reject. The patch must remain identical through
build and measurement. The helper serially builds release mode and records
source/artifact identity.
The timing command never builds or installs anything. Finish **all** builds and
test suites before running it. It checks available memory/disk and rejects
concurrent Cargo/rustc work on Unix; on Windows, ensuring no other build/test
workloads run is an explicit operator prerequisite. Resource snapshots are
retained between child processes, not inside timed requests.

The comparison reruns **both** immutable libraries: 13 scenarios × 3 matched
process pairs, 78 processes, 234 measured requests and 720 untimed seed runs.
Each adjacent pair reverses library order according to a fixed schedule; scenario
order also reverses in alternate batches. Each scenario has a 2:1 order split
because there are three pairs; across all scenarios the split is 20:19. This is
not two giant sequential captures compared after the fact. Compilation, seeding,
callback completion boundaries and sampled CPU/RSS match the original child.

Compiler, script-host and measurement-helper fingerprints must match the frozen
baseline. A fixture/host/compiler change fails comparison rather than silently
starting a new experiment. The capture orchestrator itself is allowed to differ:
it does not change the child execution boundary. Artifact hashes and production
source are checked before and after capture. Local receipts document the explicit
build; they are not independently signed provenance.

The output retains the complete schedule, manifests, per-process resource
snapshots, copied binaries, databases, raw stdout/stderr, samples and summary.
An interrupted or failed cohort retains `failure.json` and cannot pass as a
complete comparison. No sample is retried, trimmed or replaced. An exit code of
**2** means measurement completed but performance acceptance needs review
(overlap, missed target or control/memory investigation); it is not a passing
release gate. Correctness/structural and installed-package checks remain separate.

Comparison summaries expose units explicitly. The reused `medianMs` summary
field for the `rssBytes` metric contains **bytes**, not milliseconds. That metric
uses the largest post-request RSS sample in each process; it still does not
measure process-tree peaks or long-lived leaks. Paired CPU differences cannot be
added across simultaneous admissions.

### Operator lifetime

Use `woml run` to keep the coordinator active for incoming work. Preparation
belongs to that one owned lifetime; it is not persisted permission to skip future
startup. After shutdown/redeployment/restore, the next owner prepares again.
One-shot `woml test`, standalone inspection and backup/restore intentionally keep
their existing opening costs. Do not expect these commands to acquire the same
steady-state speedup.

There is no public "trust this prepared database" flag. Every selected request
still checks the real store, live owner, run identity, policy and transactional
authority. Replacement, incompatible schema or ownership loss rejects the old
context instead of falling back to repair. Stop the owner before restoring or
replacing state; follow the existing restore and recovery procedures. See
[the internal contract](protocols/prepared-runtime-internal-v1.md) for drain and
invalidation rules.

### Baseline and separate trace capture

From the repository root, on a clean production tree with cached dependencies:

```bash
cargo build --offline -j 1 --locked --manifest-path core/woml-native/Cargo.toml --lib --release
bun woml-cli/scripts/verify-rust-core-native.ts dist/target/release/libwoml_core.so
```

Use the actual platform library name on macOS/Windows. Then, from `woml-cli`:

```bash
bun scripts/startup-separation-baseline.ts --native ../dist/target/release/libwoml_core.so --phase baseline
bun scripts/startup-separation-baseline.ts --native ../dist/target/release/libwoml_core.so --phase trace --scenarios sequential-2-activated,storage-4-activated,idle-heartbeat
```

The baseline capture intentionally requires clean production source. Perform it
before enabling SRS2 changes. Later comparison tooling must use the immutable
baseline library/manifest and the exact matched harness; do not disable source
checks to use a random staged addon. Raw timing and traces are ignored runtime
artifacts; the committed summary and harness make the experiment reproducible
without depending on untracked investigation notes.

## Limits of this checkpoint

SRS1's test-only readiness model is described in
[the prepared-runtime design](protocols/prepared-runtime-internal-v1.md).
No production caller is faster yet. Cross-platform CI tests the prototype;
local Linux measurements do not certify Windows/macOS or installed-package
performance. SRS4 matched optimized comparison and release acceptance remain required.
