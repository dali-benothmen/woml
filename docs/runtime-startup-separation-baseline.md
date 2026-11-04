# Startup separation: SRS0/SRS1 checkpoint

Recorded 2026-11-02. This is baseline and design evidence, **not a performance
improvement claim**. Production request paths have not been switched.

## Provenance

- Shared implementation branch: `perf/runtime-performance`.
- Baseline source: `6743b8dc854553e47031879175bf263824990b9d` (merged inspector
  and CI fixes). Production source was clean when each capture began.
- Explicit serial build: `cargo build --offline -j 1 --locked --manifest-path
  core/woml-native/Cargo.toml --lib --release`; optimized build completed before
  accepted capture, and the real 68-export native lifecycle verifier passed.
- Native SHA-256: `3cd5160f67653407d5ecb554f91fd2aacabec667c98eb8368441b7071c1c77ad`.
- Environment: Linux x64, Bun 1.3.14, Intel Core i7-9750H @ 2.60 GHz.
- Accepted untraced capture: `woml-cli/.woml/performance/startup-baseline-9SzCoE/`.
- Separate trace capture: `woml-cli/.woml/performance/startup-trace-nrUJ40/`.

The captures retain manifests, source/fixture/harness fingerprints, copied native
artifacts, raw child stdout/stderr, actual SQLite stores, complete markers, and
per-process measurements. Large artifacts remain ignored; this summary and the
committed harness do not require the older untracked investigation report.

An earlier whole capture, `startup-baseline-XhPXj6`, overlapped a local Rust test
build near its end. It is retained as diagnostic evidence, not used in the
baseline below. The complete predeclared cohort was repeated after compilation
finished. No individual slow sample was discarded or replaced. In particular,
the slow reused-store sequential-32 process in the accepted cohort remains.

## Fixed cohort and boundaries

The untraced capture completed **39 independent child processes, 117 measured
requests, and 360 untimed finalized seeding executions**. All histories/results
were verified read-only after runtime shutdown. No builds ran during the accepted
timing capture. Three independent processes were used per scenario, with three
requests in each process. The first request is separate; “later” below is the
median of each process's two later requests, summarized across processes.

Values are milliseconds, rounded here only for display. Raw unrounded values
remain in the capture. This small cohort is not a robust p95 estimate.

| Scenario | First-request median | Later process-median | Later process-median range |
| --- | ---: | ---: | ---: |
| Sequential-2 / fresh one-shot | 735.0 | 636.6 | 581.2–642.8 |
| Sequential-2 / reused-store one-shot | 707.0 | 591.8 | 537.9–598.8 |
| Sequential-2 / activated | 623.6 | 79.8 | 71.8–83.8 |
| Storage-4 / fresh one-shot | 1083.2 | 955.9 | 941.2–967.1 |
| Storage-4 / reused-store one-shot | 1053.7 | 947.1 | 923.3–973.6 |
| Storage-4 / activated | 954.9 | 244.5 | 199.4–246.3 |
| Sequential-32 / fresh one-shot | 1771.6 | 1702.9 | 1676.5–1728.0 |
| Sequential-32 / reused-store one-shot | 1822.9 | 1626.1 | 1604.1–4150.8 |
| Sequential-32 / activated | 1534.9 | 838.6 | 825.3–888.1 |
| Activated / 20 retained seed histories | 608.9 | 153.7 | 146.8–160.3 |
| Activated / 100 retained seed histories | 657.7 | 175.1 | 166.2–189.4 |
| Activated / two concurrent later admissions | 601.9 | 366.7 | 225.8–389.2 |
| Activated / 4.5-second idle before first request | 594.9 | 135.8 | 90.0–163.4 |

Activated preparation medians were 122.8 ms for sequential-2, 61.1 ms for
storage-4, and 125.7 ms for sequential-32; shutdown medians were 182.7, 28.4,
and 28.4 ms respectively. These are separate boundaries, not parts to subtract
from an individual request's duration. Activation and per-process seeding/idle
timings, CPU and own-process RSS snapshots are preserved in raw measurements.

Do not compare different modes as though only database opening changed: activated
execution already reuses a host, and one-shot completion differs from the
admission-to-terminal-callback boundary. Retained probes have already loaded the
addon during seeding. Concurrent CPU intervals overlap and cannot be summed.
The concurrent probe may start an additional host when two admissions first
overlap; its result is not a fully prewarmed concurrent-throughput measurement.

## Diagnostic traces, not timing baseline

Three separate traced processes completed nine requests with the same library:

| Scenario | Full-open spans in whole process | Full-open spans within each later dispatch | Host spawns |
| --- | ---: | --- | ---: |
| Sequential-2 activated | 10 | 2, 2 | 1 |
| Storage-4 activated | 10 | 2, 2 | 1 |
| Idle-heartbeat probe | 12 | 2, 2 | 1 |

Whole-process counts include startup/shutdown and may include background work.
Containment within a dispatch window is not exclusive causal attribution. The
idle probe additionally recorded two full opens after startup completed and
before any first dispatch began, matching the source's owner heartbeat path.
No request ran in that interval. Inclusive span durations are not added into a
wall-time breakdown, and traced latencies are not pooled with untraced samples.

This confirms that repeated full initialization is still present. It does not
establish a universal speedup available from removing it, nor that every full
open can safely be removed. The already-existing one-host reuse is a control,
not an improvement introduced by this work.

## SRS1 decisions and verification scope

See the [caller/maintenance inventory](runtime-startup-separation.md) and
[prepared-runtime design](protocols/prepared-runtime-internal-v1.md).

The prototype is compiled only under `cfg(test)`; it is not attached to runtime
options, N-API, production admission, or a request connection factory. It
exercises actual SQLite/file handles and existing owner transactions rather
than authorizing effects through a fake ready flag. It covers readiness,
revocation/draining, exact owner and store identity, Unix replacement/ABA,
unsupported VFS, schema changes, cross-connection commits, and expiry/renewal.

Linux local verification and test counts are recorded in the implementation
checkpoint: 22 durable unit tests (including 11 prototype tests), 13 core contract
path tests, and all 5 ownership tests passed; 9 profiling/measurement tests and
33 modularization/source/CI tests passed. TypeScript checking, Rust formatting,
the established Clippy policy, and public-documentation/example validation also
passed. CI runs the prototype and portable contracts serially on Ubuntu,
macOS, and Windows. Local results do not certify those other platforms, custom
VFS/network filesystems, installed packages, long-lived host-tree memory, or
transaction/host integration that belongs to SRS2–SRS4.

The 10% improvement target, 5% non-regression investigation criterion, and
16 MiB own-process RSS investigation criterion were frozen before candidate
measurement in `startupAcceptance`. Existing release budgets remain unchanged.
High variation in some control cases is visible above; future comparisons must
report inconclusive results honestly and predeclare additional matched evidence
if needed, not select favorable batches.
