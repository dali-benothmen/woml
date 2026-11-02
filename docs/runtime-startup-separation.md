# Runtime startup separation: inventory and measurement contract

Status: SRS0/SRS1 groundwork, not an enabled optimization. Production opening,
dispatch, recovery, and lease behavior are unchanged. The Rust readiness
prototype is compiled only by tests.

## Opener inventory

The classifications below describe ownership, not permissions inferred from a
path or from an existing schema marker. Source locations are module links so
line movement does not turn the inventory into stale call-site coordinates.

| Caller family | Current opening | Classification and next step |
| --- | --- | --- |
| [Native runtime startup](../core/woml-native/src/bridge.rs) | Full open, audit, owner acquisition | Startup-required. Must successfully acquire exact owner before publishing any prepared lifetime. |
| [Trigger preparation](../core/woml-engine/src/webhook/preparation.rs) | Full open, definitions/modules/policy activation, conditional recovery | Startup-required. Keep admission closed. Preserve live-route versus interrupted-run recovery distinctions. |
| [Admitted execution](../core/woml-engine/src/runtime.rs) | Full execution-store open after authority attachment | Primary owned-request candidate; one-shot callers of the same family are not automatically prepared. |
| [Capability attachment](../core/woml-engine/src/runtime/composition.rs) | Full authority-store open; managed resource binding | Primary candidate only with exact owned context. Authority and execution may retain separate connections; no shared global projection. |
| [Native owner heartbeat](../core/woml-native/src/bridge.rs) | Full opener before `renew_runtime_owner` | Primary candidate for narrow renewal, preserving failure notification and existing lease semantics. |
| [Notification continuation](../core/woml-engine/src/webhook/dispatch.rs) | Full opens after admitted execution suspends for delivery | Candidate when continuation belongs to the same live owner/store. Delivery/recovery stays authoritative. |
| [Admission and routing](../core/woml-engine/src/webhook/admission.rs) | Several `open_ready` paths already exist | Audit/propagate owned preparation; do not count these as new full-open savings. Include [routing](../core/woml-engine/src/webhook/routing.rs). |
| [Schedules](../core/woml-engine/src/webhook/schedules.rs), [intervals](../core/woml-engine/src/webhook/intervals.rs), [waits](../core/woml-engine/src/webhook/waits.rs) | Several ready opens, followed by execution helpers that can fully open again | Ready eligibility/claim transactions stay. Classify downstream continuations separately. |
| [Runtime wait/signal/timeout APIs](../core/woml-engine/src/runtime.rs) | Several full opens | Convert only inventoried owned callers, never globally replace public one-shot/administrative behavior. |
| [Reusable provider lifecycle](../core/woml-engine/src/runtime/lifecycle/reusable.rs) | Authority attachment plus full lifecycle-store open | Same-owner continuation candidate after identity and cancellation/correction audit. |
| [Workflow calls](../core/woml-engine/src/workflow_calls.rs) | Nested dispatch and child execution cross several APIs | Same-store ownership may propagate readiness, not parent authority. Cross-runtime calls must use the destination owner. Complete child configuration inheritance is not certified here. |
| Native stop, activation failure, ownership loss | Full opens during release/cleanup | Lifecycle control, not a request optimization by default. Never clear the successor's owner or repair the store to release a stale owner. |
| One-shot execution, CLI inspection, backup/restore, independent Rust embedding | Full opening unless an existing API already established readiness | Keep the existing path. Filename reuse and Arc cloning do not establish startup or ownership. |
| Remote worker journal/capability preparation | Separate worker state/authority | Explicitly exclude coordinator readiness; store identity and ownership domain differ. |

SRS2 must update this inventory per converted caller. No claim that all runtime
opens vanish is permitted. Lower-level `WomlWebhookServer` embedding does not
currently imply the N-API coordinator's owner acquisition; it remains unprepared
unless it goes through the reviewed owner-bound constructor.

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
| Expired scheduler claims | [Policy transactions](../core/woml-engine/src/durable/transactions/policy.rs) delete expired rows in claim/resume and inspect expiry when counting active capacity | Removal of initialization's extra DELETE does not remove these transactional expiry checks. SRS2 must preserve bounded cleanup through these existing paths and explicitly identify any remaining idle-store housekeeping requirement. |
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

### Scoped counter contract for SRS2

The existing profiler records full/ready opens, but its whole-process counts do
not by themselves prove which task caused a call. SRS2's deterministic tests
must carry an explicit per-prepared-lifetime observer through the connection
boundary, with separate startup, dispatch, continuation, heartbeat, maintenance,
and unprepared scopes. Define counters for full-open attempts/successes,
correction-index rebuilds, general recovery scans, and prepared-open attempts.
Use lifetime-owned counters, not a process-global atomic or thread-local scope
that can move or interleave across async tasks. Snapshot only after that test's
work drains. No new high-cardinality OpenTelemetry labels are authorized.

Assert zero startup-only work on each converted steady-state scope, while
separately asserting that required startup/recovery work did occur. The current
test-only model checks that narrow opening can coexist with a writer; it does
not pretend those future production attribution counters are already wired.

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
performance. SRS2 integration and SRS3/SRS4 hardening remain required.
