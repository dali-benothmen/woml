# Archiving Legacy Cronflow SDK Data

Use this procedure before moving a deployment from the Cronflow JavaScript SDK
to WOML. It preserves legacy history for inspection and rollback; it does not
convert old runs into WOML runs.

Known legacy defaults include `.cronflow/data.db` and `cronflow.db`, but an
application may pass a custom path. Confirm the real path from the deployment
configuration instead of assuming a default.

## 1. Inventory the deployment

Record the following in a text manifest:

- the installed `cronflow` version and package-manager lockfile;
- the application commit or source archive;
- every workflow ID and trigger endpoint;
- the exact legacy database path;
- the names and providers of required secrets, but **not secret values**; and
- the runtime, operating-system, and native platform package versions.

Keep the manifest beside the archive, not inside the database.

## 2. Close admission and settle runs

Stop accepting webhook/provider traffic and disable schedules before stopping
the process. Let active work finish where practical. The legacy SDK's
`cancelRun()` implementation is incomplete, so do not use it as proof that a
run was safely cancelled.

After stopping all Cronflow writers, inspect unresolved runs with SQLite:

```bash
sqlite3 /absolute/path/to/data.db \
  "SELECT id, workflow_id, status, started_at FROM workflow_runs WHERE lower(status) IN ('pending','running','paused');"
```

Record unresolved IDs in the manifest. They remain legacy history and are not
resumed by WOML.

## 3. Create a consistent database backup

Prefer SQLite's backup command:

```bash
mkdir -p cronflow-archive
sqlite3 /absolute/path/to/data.db \
  ".backup 'cronflow-archive/legacy-state.sqlite'"
```

Do not copy a live database file while Cronflow is writing to it. If `sqlite3`
is unavailable, stop every writer first and copy the database together with any
matching `-wal` and `-shm` files as one set.

Copy the source snapshot, lockfile, and inventory manifest into the archive.
Do not copy plaintext secrets.

## 4. Verify and protect the archive

Check that SQLite can read it and record a checksum:

```bash
sqlite3 cronflow-archive/legacy-state.sqlite "PRAGMA integrity_check;"
sha256sum cronflow-archive/legacy-state.sqlite
chmod -R a-w cronflow-archive
```

`PRAGMA integrity_check` must return `ok`. Store a second encrypted copy on a
different device or backup system and apply the organization's retention and
access policy.

## 5. Start WOML separately

WOML uses a different event-sourced data model. Start it with a new WOML state
file, normally `.woml/state.sqlite`. Never rename the legacy database to that
path and never configure both runtimes to write to the same file.

## 6. Restore-test before deleting infrastructure

On an isolated machine or directory, restore a writable copy of the archive,
install the recorded final `0.11.x` dependency set from the lockfile, and prove
that the historical tables can be inspected. Do not attach restored legacy
triggers to production endpoints during this check.

Rollback means stopping WOML admission, restoring the legacy application and a
writable copy of the archived database, and transferring ingress ownership
back deliberately. Never run the archive itself as the writable production
database and never let both runtimes own the same ingress simultaneously.
