# WOML Retention and Storage Maintenance

PRO8 gives a long-running WOML deployment a safe way to remove old execution
history. Rust remains the only authority that decides whether a run is safe to
delete. The Bun CLI only validates operator input and displays the versioned
plan or result.

## Manual retention

Always preview an operation first:

```bash
woml prune --before 30d --dry-run
```

The preview reports the eligible run count and a deterministic estimate of the
logical history bytes that would be removed. It does not expose workflow
payloads and does not mutate the database.

Run the reviewed operation with:

```bash
woml prune --before 30d
```

Use `--state <path>` for a non-default durable store and `--json` for automation.
Durations accept whole hours, days, or weeks, such as `12h`, `30d`, or `8w`.

`deletedBytes` is the logical size of deleted history, not an immediate promise
that the SQLite file becomes smaller. Normal pruning performs a bounded passive
WAL checkpoint. When temporary extra disk space and a stronger SQLite lock are
acceptable, explicit compaction is available:

```bash
woml prune --before 30d --compact
```

Compaction cannot be combined with dry run and should be reserved for a planned
maintenance window.

## Automatic retention

Automatic retention is opt-in through Runtime Configuration v1:

```json
{
  "schemaVersion": 1,
  "retention": {
    "enabled": true,
    "succeededAfterDays": 30,
    "failedAfterDays": 90,
    "cancelledAfterDays": 30,
    "maintenanceHourUtc": 3
  }
}
```

Start the deployment normally with `woml run ... --config <path>`. The CLI
prints the next UTC maintenance time. The scheduler calls the same Rust planner
and executor as `woml prune`; it does not run a second cleanup implementation.
Execution is off the Bun event loop, and the Rust executor commits at most 250
independent runs per transaction before yielding so trigger admission is not
starved.

## What can be removed

Only terminal `succeeded`, `failed`, or `cancelled` run history older than the
status-specific cutoff can be eligible. A workflow-call parent/child component
is removed together or retained together.

WOML always protects:

- queued, running, waiting, retrying, cancelling, and lifecycle-finalizing runs;
- unresolved approvals and their notification capabilities;
- active workflow calls and their parent/child histories;
- schedule, interval, webhook, Slack, and event deduplication records still in
  the frozen 30-day safety window;
- compiled definitions and module artifacts;
- `services.state` entries and state mutation settlement;
- `services.storage`, database, cache, and secret-provider data.

Dependency groups larger than the v1 batch limit remain protected. WOML will
not split such a group merely to meet a deletion target.

## Safety and recovery

Retention acquires the Store v14 maintenance lease, so backup, restore,
retention, checkpoint, and compaction cannot race each other. Eligibility is
rechecked inside every immediate deletion transaction. A crash can leave prior
batches committed, but cannot leave half of the current batch committed; the
remaining history is reconsidered on the next run.

The engine restores immutable-history deletion guards before each transaction
commits, performs an integrity check after maintenance, records Retention Result
v1 durably, and releases the lease on both success and failure. Disk-full,
corrupt-store, unsafe-path, and maintenance-conflict cases fail closed with a
stable WOML error code.

## Operations and observability

Automatic retention appears as the `retention` component in detailed health,
the operations stream, and `woml inspect`. Completed and failed passes increment
`woml_retention_total`; completion/failure logs contain counts and stable codes,
never workflow payloads. `woml_store_size_bytes` includes the database, WAL, and
shared-memory companion files, making WAL growth visible to existing monitoring.

The last Retention Result v1 is stored as a safe maintenance audit record. It
contains policy identity, completion time, deleted count, logical bytes, and the
guarantee `stateEntriesDeleted: 0`.
