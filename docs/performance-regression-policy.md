# WOML Performance Regression Policy

Status: frozen with PERF8.

WOML separates deterministic regression gates from machine-sensitive workflow timings. CI must fail when an architecture guarantee or a stable component budget is broken. CI must not fail solely because a shared runner had a slow end-to-end sample.

## Hard gates

Hard gates protect:

- the canonical workflow's compiled graph shape;
- one supervised Bun script host across repeated runs;
- one fresh isolated worker for every script invocation;
- clean shared-host shutdown;
- the versioned measurement, span, and budget schemas;
- compiler and disabled-profiler microbenchmarks with deliberately generous headroom.

A hard failure means the pull request changed a protected contract or produced an algorithmic component regression. It must be fixed or accompanied by a reviewed budget change.

## Informational targets

The canonical two-step, eight-step sequential, and parallel workflows run against the release-shaped runtime in CI. Their cold activation, warm median, warm p95, and raw samples are written to a JSON artifact. Exceeding a target marks the report for review but does not fail the job.

Informational results are useful for trends and investigation. They are not product promises and should never be compared across machines without checking the recorded CPU, memory, platform, Bun version, commit, and fixture hash.

## Ownership

Budget owners are recorded in `docs/performance-regression-budgets.v1.json`:

- language frontend maintainers own compiler budgets;
- Rust runtime maintainers own durable execution and host lifecycle;
- CLI maintainers own presentation budgets;
- release maintainers own CI behavior and artifact retention.

The existing terminal and production budget files remain authoritative for their subsystems. PERF8 connects them to the versioned global policy rather than replacing them.

## Changing a budget

A pull request may change a budget only when it includes:

1. the reason the old budget is no longer correct;
2. before-and-after artifacts from the same machine;
3. median, p95, memory, and throughput impact where applicable;
4. confirmation that durability, isolation, security, and recovery are unchanged;
5. approval from the named owner.

Do not raise a limit merely to make a red CI job green. First reproduce the failure locally and determine whether it is a real regression, an intentional product cost, or runner noise.

## Commands

Run deterministic gates after building the release-shaped runtime:

```bash
bun run build
bun run test:performance-regression
```

Generate the advisory end-to-end report:

```bash
bun run report:performance-regression
```

The report is written to `woml-cli/.woml/performance/regression-v1.json`. For the complete controlled matrix, continue to use:

```bash
bun run benchmark:performance-baseline
```
