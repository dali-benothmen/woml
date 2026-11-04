# Invocation setup: release-readiness handoff

ISR6 assessment on 2026-11-04, branch `perf/runtime-performance`, checkpoint
`aa8a6ea46a327bf8cf00f333fe51920507ccca4e`.

**Status: blocked, not release accepted.** The startup-locking prerequisite
has not been corrected. A fresh disposable diagnostic reproduces the previously
reported operation-sequence hazard, and the production guard still contains
that sequence. Affected fresh native/installed-runtime journeys were not run.
No production change, optimized native build, package installation, publication
or new performance claim was made at this checkpoint.

The ISR2 candidate also remains rejected on its own performance evidence.
Correcting the locking issue would not make that candidate beneficial or close
the other shared-branch performance/resource findings.

## Prerequisite check

The existing [descriptor diagnostic](../woml-cli/scripts/investigate-sqlite-file-locks.ts)
was run once against a newly created disposable database, not workflow state:

```bash
bun woml-cli/scripts/investigate-sqlite-file-locks.ts
```

Report: `/tmp/woml-sqlite-file-locks-HYbrix/report.json`.

| Parent has not committed or rolled back its immediate transaction | Independent writer |
| --- | --- |
| Before the probe | Blocked |
| After pathname metadata inspection | Blocked |
| After raw database open, descriptor metadata inspection and close | Writer acquired |

The report records `unsafeDescriptorCloseReproduced: true` and
`currentGuardContainsTemporaryPinOpen: true`.

The [production guard](../core/woml-engine/src/durable/prepared.rs) SHA-256
remains `789876de99b19b8962adc13fe6c451a2a529b03ceab1679d4d7f78467d0dd6a6`.
The diagnostic SHA-256 is
`d58d09f99b95a3a6cf5c1ea86922fcbfbdf473d2dee19b9fb8c5f73e3ae856be`.
The guard identity path still opens and drops a temporary pin; revocation also
takes/drops the lifetime pin. Both paths need audit, not only the temporary
probe.

This is a Linux POSIX operation-sequence reproduction using Bun SQLite,
**not direct invocation of the private Rust guard**, attribution of every
previous I/O error, or a release gate. Its successful exit means the diagnostic
reproduced the hazard, not that storage is healthy. Even a negative diagnostic
would not independently certify the entire production pin lifecycle.
See the [original blocker](validated-state-reuse-verification.md#prepared-runtime-locking-blocker)
and [later reconfirmation](incremental-scheduling-release-evidence.md#blocking-prerequisite-reconfirmed).

## Evidence status

| ISR6 requirement | Status at this checkpoint |
| --- | --- |
| Independently verified startup-locking correction | **Blocked:** no correction landed; unsafe sequence remains |
| Beneficial invocation optimization to integrate | **Rejected:** primary batch 2.62% slower; repeated-module trigger unresolved |
| Selected script isolation and Rust transport/authority | Prior ISR4 Linux checks passed; not exhaustive or native-addon certification |
| Scoped CPU/allocation, physical peaks and heap plateau | Incomplete; unavailable measurements remain null |
| Fresh final optimized addon and actual exported interface | Not built/verified here; old artifacts do not certify final source |
| Clean installed repeated execution, live inspection and cancellation | Not run here, pending prerequisite |
| Installed host replacement, shutdown, restart and backup/restore tail | Not run here; earlier failed inspection/tail remain unresolved |
| Actual Linux/macOS/Windows installed and safety matrix | Not certified; local evidence and CI wiring do not substitute |
| Final startup/output/package/protocol/telemetry/isolation budgets | Unchanged, but not rerun/certified against fresh final artifacts |
| Honest blocked/deferred handoff | This record; no whole-branch acceptance |

The [retained ISR4/ISR5 review](invocation-setup-verification-and-review.md)
records 72 Bun and 18 Rust integration tests per artifact, plus 29 shared Rust
checks. Those results are historical evidence at that verification checkpoint,
not a fresh execution of the full release gate here.

At this checkpoint the original matched capture was audited offline again:
eight artifact identities and 42 raw per-process evidence files were verified,
and its original summary was reproduced. `primaryTargetMet`,
`integrationAccepted` and `releaseAccepted` remain false. No timing sample was
rerun, replaced or trimmed to obtain release acceptance.

## Required next correction

Handle startup locking as a separately reviewed correctness change before
resuming affected installed/native acceptance:

1. Audit acquisition, revalidation, failed validation, revocation, release and
   final pin teardown, including other active SQLite connections in the same
   process. Replacing one temporary probe with `stat` alone is insufficient.
2. Preserve pathname/native-VFS identity, replacement/ABA detection, schema,
   owner/lease and precommit validation, plus Windows sharing restrictions.
   Do not remove protections or increase timeouts to hide the failure.
3. Add a direct production-boundary regression proving an independent writer
   stays excluded for the entire active transaction through those lifecycle
   transitions. Keep fault injection in independent processes where needed;
   in-process Bun SQLite descriptors can themselves perturb native locks.
4. Independently validate the correction, then explicitly build fresh optimized
   artifacts and verify their identity/interface and clean installed package.
5. Rerun the previously failed installed inspection and its unexecuted tail,
   followed by actual platform and unchanged-budget gates. Retain old failures.

This is a resume checklist, not an implementation or an authorization to call
the current branch release-ready. Earlier SRS/VSR/ISQ controls and ISR4 resource/
policy gaps must remain independently visible even after the locking fix.

## Safe checks

The ISR diagnostic/type checks and architecture/source/CI contracts pass locally
at this checkpoint. Documentation/public-example and diff checks are recorded
with this handoff. These do not activate a runtime or certify installed state.
The production guard, public contracts, historical fixtures and budgets are
unchanged. The developer book is not updated to describe an accepted invocation
optimization, because none was accepted.
