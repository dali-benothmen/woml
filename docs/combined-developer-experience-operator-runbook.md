# Combined Developer Experience Operator Runbook

## Before activation

1. Run `woml check --project ./workflow.json --json`.
2. Review the project digest and every referenced policy, reviewer, target, and
   trust document.
3. Run `woml target check` for each remote target.
4. Run `woml worker check --config ./woml.worker.json --json` on each worker.
5. Confirm artifact storage, Operational Output Store retention, OpenTelemetry
   collector health, backup destination, and free disk space.
6. Back up the durable state before a risky rollout.

## During operation

Use `woml list`, `woml get`, and `woml logs` for runs. Use the target and
dispatch inspection commands for remote placement. Drain a target or worker
before maintenance; draining prevents new assignments but does not falsify the
state of work already accepted.

OpenTelemetry is advisory. Export failure must not change workflow settlement.
The durable event store remains authoritative; operational output and telemetry
are bounded projections.

## Ambiguous remote work

An **ambiguous** dispatch is a safety state, not a generic failure. It means an
external side effect may have happened and automatic retry could duplicate it.

1. Run `woml dispatch inspect <dispatch-id> --state <path> --json`.
2. Preserve worker journals, relevant artifacts, logs, and external-system
   evidence.
3. Verify the real external state with a second operator when the effect is
   destructive.
4. Reconcile with a unique request id and a reason. Use `record-failure` only
   when failure is proven. Use `authorize-possible-duplicate-retry` only when
   the operator accepts that risk.
5. Attach the inspection and reconciliation result to the incident record.

Reconciliation cannot manufacture a successful workflow result.

## Backup and recovery

```bash
woml backup ./backups/current --state ./.woml/state.sqlite --json
woml restore ./backups/current --state ./recovery/state.sqlite --json
woml list --state ./recovery/state.sqlite --json
```

Restore to a new state path for verification. Use `--replace` only during an
approved replacement window after stopping the owning runtime. Keep the retained
previous database until the recovered deployment passes health and run checks.

## Shutdown

Drain remote targets, wait for accepted attempts to settle, stop the runtime,
flush the OpenTelemetry exporter within its bounded shutdown window, and create
a final verified backup. Never delete a worker journal that contains an
unreconciled dispatch.
