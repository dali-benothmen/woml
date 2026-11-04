# Startup/request separation: release evidence and remaining acceptance

Recorded 2026-11-03 (Europe/Berlin), on `perf/runtime-performance`.

**Status: not yet accepted as a performance release.** The initial complete
matched cohort did not demonstrate the frozen improvement target. Installed
testing also exposed a startup-order integration gap; that gap is repaired and
verified separately. Measurements of the earlier library are not measurements
of the repaired library. Platform CI and a new matched capture remain required.

## Artifacts and provenance

All native builds were optimized Cargo release builds, serial and separate from
timing. The compiler, script host, measurement child and fixture fingerprints
matched the frozen baseline. Linux ELF comments on both initial libraries record
Rust 1.88.0 and GCC 11.4.0. Bun was 1.3.14, Linux x64, on an i7-9750H.

| Role | Source / receipt | Native SHA-256 |
| --- | --- | --- |
| Frozen baseline | `6743b8dc854553e47031879175bf263824990b9d`; `startup-baseline-9SzCoE` | `3cd5160f67653407d5ecb554f91fd2aacabec667c98eb8368441b7071c1c77ad` |
| Initial measured candidate | Clean `903eff7cf7bc73c708812077d03c652de9ca5217`; `startup-build-fsD9OB` | `c93d8271d7a29d9467418803852a90452ab0000ddf5d81f8df55c8e42fb1c2dd` |
| Repaired candidate | Same HEAD plus retained production patch; `startup-build-0A4TXx` | `79c093944060da06a000cd674f8dff5e2d1285e6d843e6d0db0816330594cd07` |

Receipts and captures live under ignored `woml-cli/.woml/performance/`.
The repaired candidate's raw production-patch digest is
`f0019b274d1f2dc965c54a80a7fa27ab9a198b8bde566f672cd808deb68ad22e`.
The explicit `--worktree` build mode records that patch rather than presenting
uncommitted production source as a clean commit. Original baselines stay intact.
These local receipts are not independently signed build attestations.

## Initial matched cohort: all results, not selected winners

Capture: `startup-comparison-oq5YFA`. It completed **78 independent processes,
234 measured requests and 720 untimed real seed executions**. No native/test
builds ran during this capture. Three adjacent baseline/candidate process pairs
were run per scenario, alternating library order and reversing scenario order
in the middle batch. No slow or failed sample was retried or dropped.

First requests are separate. “Later” is the median of each process's two later
requests, then the median across three process batches. Values are milliseconds;
percentages below use unrounded values. A negative later change means faster.
These are small-cohort observations, not robust tail or universal speed claims.

| Scenario | Baseline first | Candidate first | Baseline later | Candidate later | Later change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sequential-2, fresh one-shot | 730.0 | 722.7 | 668.1 | 631.5 | −5.5% |
| Sequential-2, reused one-shot | 701.4 | 707.4 | 584.0 | 614.8 | +5.3% |
| Sequential-2, activated | 652.3 | 578.3 | 75.5 | 67.3 | −10.9% |
| Storage-4, fresh one-shot | 1018.3 | 1038.6 | 946.8 | 1077.9 | +13.9% |
| Storage-4, reused one-shot | 1128.2 | 1006.8 | 919.9 | 926.8 | +0.8% |
| Storage-4, activated | 913.5 | 1031.6 | 240.1 | 243.2 | +1.3% |
| Sequential-32, fresh one-shot | 1837.9 | 1849.3 | 1820.8 | 1762.1 | −3.2% |
| Sequential-32, reused one-shot | 1830.1 | 1754.9 | 1624.3 | 1718.6 | +5.8% |
| Sequential-32, activated | 1578.4 | 1589.9 | 911.1 | 903.5 | −0.8% |
| Activated, 20 retained histories | 558.0 | 621.4 | 161.7 | 178.9 | +10.6% |
| Activated, 100 retained histories | 574.7 | 557.7 | 187.1 | 232.4 | +24.2% |
| Activated, two concurrent later requests | 630.9 | 549.8 | 371.3 | 322.7 | −13.1% |
| Activated, 4.5-second idle window | 725.8 | 663.4 | 175.6 | 65.7 | −62.6% |

### Why this is not a passing performance gate

The predeclared primary target was at least 10% improvement with 100 retained
histories. Its observed median was instead **24.2% slower**. Baseline process
medians ranged 172.8–196.3 ms; candidate medians ranged 156.6–308.4 ms. The ranges
overlap, so the automated classification is `inconclusive`, not a proven speedup
or a proof that the implementation has no regression. The command correctly
returned exit code 2 after retaining a complete result.

The three primary pairs were 196.3→156.6, 172.8→308.4 and 187.1→232.4 ms.
Raw requests show a pronounced slow third request in multiple processes:

| Batch | Baseline later requests | Candidate later requests |
| --- | --- | --- |
| 0 | 103.5, 289.0 | 85.9, 227.3 |
| 1 | 82.8, 262.7 | 295.2, 321.7 |
| 2 | 87.7, 286.4 | 58.3, 406.4 |

Own-process CPU for that cohort fell from 58.9 to 46.6 ms (−20.8%), but CPU
excludes host processes and does not explain wall-time waiting. Admission,
claim/maintenance scheduling, SQLite waits, callback delivery and interactive
machine load need investigation before assigning a cause. Background desktop
workloads were observed; this is a possible confounder, not evidence that every
slow sample can be dismissed as noise. No such sample was excluded.

The automated 5% investigation checks also flagged:

- reused sequential-2 and sequential-32, and fresh storage-4 one-shot controls;
- storage-4 and retained-20 first requests;
- storage-4, retained-20 and retained-100 preparation;
- sequential-32, retained-20 and retained-100 activation;
- retained-20 and idle-window shutdown.

All flagged ranges overlap except idle-window shutdown: its median increased
40.0→145.7 ms, with baseline range 35.7–61.4 and candidate range 121.2–161.9 ms.
That non-overlapping slowdown is an unresolved release concern, not an accepted
trade-off. Preparation with 100 retained histories increased 76.0→97.0 ms;
moving validation to startup must remain visible in this comparison.

No scenario crossed the 16 MiB sampled own-process RSS investigation threshold
in this initial cohort. Median per-process maximum sampled RSS differences were
between −1.15 and +1.23 MiB. This is neither peak/tree memory measurement nor a
long-lived leak certificate, and it does not certify the later repaired artifact.

## Installed-package finding and repair

The first installed journey intentionally ran `woml get` while a request was
active. Even without an enabled artifact backend, that command calls artifact
inspection, whose existing implementation lazily creates artifact metadata
tables. This changed SQLite's schema cookie after preparation had published it.
The strict prepared guard then invalidated the owner and the run stopped during
its start hook. Merely avoiding live inspection would have hidden a regression.

Owned startup now prepares and validates the **existing** artifact metadata in
one immediate transaction before publishing readiness. It uses the original
DDL/version and a non-creating engine-file connection. It does not select or
open a local/S3 backend, create a backend store identity, clean materializations,
activate networking, or grant artifact authority. Schema-change invalidation,
owner checks, transactional authorization and public versions remain unchanged.

The new prepared regression verifies live artifact inspection, an unchanged
schema cookie, a retained inspection audit, successful owner renewal, absence
of a backend identity/directory and fail-closed future metadata versions. The
actual-native verifier also inspects between independent admitted requests.
Standalone inspection's intentional full opening is accounted for separately,
not mislabeled as an owned request initialization.

The final installed test uses only the installed CLI and packaged native/host
files for execution. Generic read-only SQLite checks verify authoritative result
bodies because the public `get --json` contract reports lifecycle/status rather
than result bodies. It exercises live inspection, start/complete hook output,
four managed StateService increments across repeated requests, duplicate
delivery, shutdown/rebind, restart, backup and restore. CLI-native/schema
contracts are not changed to satisfy the test.

## Verification and release decision

Local Linux results for the repaired candidate:

- All **144 engine unit tests**, including 29 prepared/prototype checks, passed.
- Actual optimized native verification passed seven prepared/reference workflow
  comparisons plus the isolated owner-transfer case. Live artifact inspection
  between independent admissions preserves the published schema cookie.
- All **68 native exports** and synchronous/asynchronous errors, callbacks,
  activation, telemetry, shutdown and rebind matched the preserved baseline.
- The new installed repeated-request journey passed **81 assertions**. The four
  existing combined/artifact/telemetry/remote package tests passed **116
  assertions**, using the freshly staged optimized native artifact.
- All 54 OpenTelemetry/observability/inspector/runtime contract tests and the
  unchanged telemetry budgets/verifier passed.
- The unchanged combined-experience budgets/verifier passed: project resolution
  p95 31.1 ms, preflight p95 314.8 ms, cold one-shot activation 580.2 ms, warm
  one-shot activation p95 579.5 ms, remote preparation p95 155.1 ms, streamed
  output overhead 53.0%, main JS 1,150,183 bytes and installed JS 2,497,413 bytes.
  These one-shot/packaging checks are **not** proof of prepared steady-state
  performance acceptance.

All **19 portable startup/session/comparison tests**, TypeScript checking and
**33 source/modularization/CI contract tests** passed. The established Clippy
policy, Rust formatting and documentation/link/public-example checks passed
without new allowances or changes to frozen baselines. These selections overlap
with earlier checkpoints; they are not one disjoint combined test count.

During native fault injection, an in-process Bun SQLite write encountered
`SQLITE_IOERR_DELETE_NOENT`. Bun and the addon have independent SQLite
implementations; sharing a process while closing file descriptors can interfere
with native POSIX SQLite lock ownership. The injection now runs in a separate
bounded verifier child, preserving real OS database locking instead of retrying
the mutation. The production ownership guard and successor-preservation
assertions are unchanged. This diagnostic is not an accepted timing sample.

Representative commands, serially from `woml-cli`, after the explicit release
build/staging step:

```bash
bun run test:startup-request-separation-contracts
bun scripts/verify-startup-separation-native.ts ../dist/target/release/libwoml_core.so
bun scripts/verify-rust-core-native.ts ../dist/target/release/libwoml_core.so .woml/performance/startup-baseline-9SzCoE/baseline.node
bun run test:startup-request-separation-package
bun run test:combined-developer-experience-package
bun run test:opentelemetry
bun scripts/benchmark-combined-developer-experience.ts
bun scripts/verify-combined-developer-experience-release.ts
```

Use platform-appropriate library paths. The preserved comparison binary is
Linux-only; it is not a cross-platform artifact. Build/test commands must finish
before any matched timing capture starts. Correctness tests do not require that
local historical binary; the second argument to the generic native verifier is
an optional local before/after comparison.

Zero-init evidence remains scoped: lifetime counters and isolated actual-native
traces check selected owned dispatch, authority, continuation, heartbeat and
shutdown paths. The design still uses separate connections, not a new connection
pool or shared projection. Span counts are not a complete physical connection
census. Standalone commands, independent embedding, notification internals,
programmatic-lock opening, artifact backend binding and separate child-routing
configuration are outside that claim. See the
[caller inventory](runtime-startup-separation.md).

The existing Linux/macOS/Windows combined CI matrix now runs the installed
regression after its fresh release build, in addition to prepared safety and
existing package, lazy activation, export and budget checks. Local Linux results
do not mean those platform jobs have run or passed. No thresholds, package
limits, fixture bytes, dependencies or public contract versions are raised.
The release-source validation job also runs the prepared safety and installed
journey after its explicit local native build; this is not a replacement for
the matching-platform native release jobs or matched timing acceptance.

Remaining acceptance work:

1. Investigate the retained-store request-delay pattern and idle-window shutdown
   regression using separate focused traces. Do not conflate lower CPU with
   faster observed completion.
2. Predeclare and run new matched evidence for the **repaired** artifact on a
   quiet machine, retaining the initial cohort. If evidence remains inconclusive
   or slower, simplify/defer the optimization rather than relax the target.
3. Pass the fresh-build platform CI matrix and retain failure diagnostics.

Until these are satisfied, keep the final phase unchecked and do not present
this work as an accepted general runtime speedup. Reproduction commands and
measurement boundaries are in the [measurement guide](runtime-startup-separation.md).
