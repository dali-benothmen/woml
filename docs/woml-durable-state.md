# Durable User State Operations

`services.state` is WOML's small, permanent memory for one workflow ID. Rust
owns it in the same SQLite file selected by `woml run --state`; future runs and
new definitions of that workflow see the same values. It is not run context,
cache, object storage, or a general database.

For a product-level comparison with cache, storage, and database, see
[Choosing Where Workflow Data Lives](woml-data-guide.md).

## Reliability boundary

- Every mutation requires a stable `name`. A retry with the same logical
  identity returns the first committed result rather than applying it twice.
- SQLite serializes writes across threads and local processes. WOML waits up to
  five seconds for a busy database using SQLite's bounded increasing-delay busy
  handler, then returns retryable `WOML_STATE_STORE_UNAVAILABLE`.
- Versions, `ifVersion`, increments, quota accounting, the immutable mutation
  result, and its settlement proof commit in one immediate transaction.
- The normal operation-success event is appended immediately afterward. If the
  host stops in that narrow gap, recovery validates the immutable mutation
  proof and records the missing success event without executing the mutation
  again. The interrupted script attempt still fails closed because unrelated
  script effects may remain ambiguous.
- Startup runs SQLite `quick_check`, foreign-key checks, schema checks, digest
  and canonical-JSON verification, quota reconciliation, and live-version
  reconciliation. A contradiction fails with
  `WOML_STATE_STORE_CORRUPT`; WOML does not guess or silently repair it.

`services.state` is deliberately absent from `woml get`, run summaries,
progress, events, and ordinary logs. Those surfaces expose only operation name,
digests, version/byte counts, duration, and safe outcome. State tables have no
run-ownership foreign keys, so future run-history retention cannot cascade into
workflow state.

## Concurrency guidance

Use `increment` for counters and `setIfAbsent` for one-winner initialization.
For read/modify/write logic, read the current version and pass it as
`ifVersion`; if another run wins first, handle `WOML_STATE_CONFLICT` by reading
again and making a new named logical decision. Do not reuse one mutation name
for different input.

Cancellation and workflow deadlines prevent work that has not entered its
transaction. Once a mutation transaction commits, WOML reports or recovers that
committed outcome; cancellation does not roll it back.

## Local security

On Unix, WOML hardens the SQLite file to owner read/write permissions (`0600`)
when State v1 opens it. Operators must also protect the parent directory,
backups, filesystem snapshots, and the account running WOML.

The local database is **not transparently encrypted** and necessarily contains
the authored keys and JSON values. Do not store secrets or sensitive business
data there unless the host's disk and backup encryption meet your requirements.
Remote authorization and encrypted external state backends remain Production
Runtime work.

## Backup and recovery

The state tables are authoritative and cannot be rebuilt from run events.
Back up the complete SQLite database, not selected WOML tables.

For the simplest consistent backup, stop every WOML process using the file and
copy the database. For a live system, use SQLite's backup API or a reviewed
SQLite snapshot procedure; copying only the main file while WAL writes are
active can omit committed data.

If startup reports corruption:

1. Stop every process using the database.
2. Preserve the database and any `-wal`/`-shm` files for diagnosis.
3. Restore the last known-good complete backup to a separate path.
4. Run WOML against that restored path and let the startup integrity audit
   verify it before replacing anything.

Do not drop tables, edit digests or quotas, or delete mutation rows to make the
check pass. Those records are part of retry safety.

## DS4 verification

From `woml-cli/`:

```bash
bun run test:ds4
bun run benchmark:state
```

The hardening gate covers independent processes contending on one key,
same-identity deduplication, distinct named mutations, bounded lock waiting,
recovery after a committed mutation, corrupt digests/quotas/results, Unix file
permissions, run-inspection redaction, run/state ownership separation, and
state operation latency/database-size budgets.

DS5 publishes the feature through `bun run test:ds5`, which adds clean package
installation, packaged native execution across process restarts, both public
examples, every-schema compilation, historical model/event fixtures, package
auditing, and secret/redaction scans.
