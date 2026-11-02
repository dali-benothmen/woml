# Runtime startup separation: inventory and measurement contract

Status: bounded SRS2 production integration with SRS3 safety tests. Owned native
requests use guarded prepared opening; unowned/one-shot behavior remains.
SRS4 matched optimized performance, installed packages and platform acceptance
are not certified by this checkpoint.

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
| Native activation, drain and stop | Guarded routing cleanup and exact owner release; no create/repair fallback | Failed preparation releases via original acquisition connection. Heartbeat remains alive during drain. |
| Standalone execution/inspection, backup/restore, independent Rust embedding | Original opening contracts | Never infer coordinator readiness from path or Arc reuse. |
| Worker journal/capability preparation | Separate worker ownership domain | Coordinator preparation cleared. |

The first StateService binding already validated/opened engine state on the
first request. It now runs once behind owned startup before publication; this
moves cost and preserves validation. Other resource attachment stays as before.
Output, providers, worker/listener startup and telemetry are not eagerly enabled.

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
