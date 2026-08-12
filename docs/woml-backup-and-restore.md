# WOML Backup, Restore, and Store Upgrades

PRO7 adds a tested recovery path for WOML's durable SQLite authority. It keeps
the ordinary product experience small: there is no archive format to build and
no backup service that must remain running.

## Online backup

Create a backup while `woml run` is active or stopped:

```bash
woml backup ./backups/orders-2026-08-12
```

If the deployment uses a non-default state location:

```bash
woml backup ./backups/orders-2026-08-12 \
  --state ./data/state.sqlite
```

The destination must not already exist. WOML acquires the durable maintenance
lease, asks SQLite for one coherent online snapshot, validates the copied
database, verifies every compiled definition and required module artifact,
writes SHA-256 size/checksum metadata, and publishes the directory atomically.
Concurrent workflow writes may continue; the backup represents one consistent
point in time.

Each backup contains exactly:

```text
orders-2026-08-12/
├── manifest.json
└── state.sqlite
```

`manifest.json` conforms to
[Backup Manifest v1](schemas/backup-manifest.v1.schema.json). `--json` prints
that same manifest for automation.

An interrupted backup remains an unpublished temporary directory and is not a
valid recovery point. A completed backup is accepted only after its checksum,
SQLite integrity, store version, definition inventory, compiled definitions,
module artifacts, and durable State v1 authority pass verification.

## Offline restore

Stop the target runtime, then restore into a clean state location:

```bash
woml restore ./backups/orders-2026-08-12 \
  --state ./data/state.sqlite
```

Restore verifies the manifest and database again before changing the target.
It rejects symlinks, incomplete directories, altered bytes, unsupported future
store versions, missing artifacts, and a target owned by a live runtime.

If the target database already exists, explicit confirmation is required:

```bash
woml restore ./backups/orders-2026-08-12 \
  --state ./data/state.sqlite \
  --replace
```

WOML first prepares and audits a temporary database. Only then does it move the
old database to the reported `.pre-restore-<timestamp>` rollback path and swap
the restored database into place. Keep that rollback copy until the restored
deployment has been validated.

Restart the same deployed WOML source after restore:

```bash
woml run workflows/ --config woml.runtime.json
```

The database restores immutable definitions, module artifacts, run bindings,
events, retry/lifecycle/policy state, unresolved approvals, Workflow Calls,
trigger coordination, and `services.state`. Runtime ownership, scheduler
claims, and route leases are deliberately cleared so the new process can own
recovery safely. A persisted State v1 location identity preserves workflow
state even when the restored database is placed at a different path.

The backup is not a source deployment bundle. Keep `.woml` files, local module
source, and runtime configuration in version control or your normal deployment
system. Pinned definitions and bundled module artifacts inside SQLite remain
available for already-admitted runs.

## Store upgrades and rollback

Restore accepts Store v13 and v14. A supported older backup is migrated on the
temporary restore copy, audited as Store v14, and swapped into place only after
success. Unknown future versions are rejected rather than guessed.

Current v13-to-v14 migration is additive and transactional. A future migration
that is classified as destructive must require a recorded verified backup
before changing the live store. Store metadata is updated last; immutable
definitions and run events are never rewritten. Downgrades across an
incompatible store version use the retained pre-restore database or a verified
backup, not an implicit reverse migration.

## Operational responsibilities

- Put the state database and backup destinations on reliable local persistent
  storage. Do not run SQLite on an unsupported network filesystem.
- A process supervisor restarts `woml run`; WOML backup does not replace
  systemd, Docker, Kubernetes, or VM supervision.
- Copy completed backup directories off the machine according to your recovery
  objectives. Test restore regularly on a clean host.
- Preserve owner-only permissions and encrypt backup storage where required.
  The SQLite file contains workflow history, payloads, results, and durable user
  data.
- Secret providers are separate. WOML never copies OS-store, environment, or
  mounted-file secret values into the backup manifest. Back up or reprovision
  those providers using their own secure process before starting the restored
  workflow.
- Logs are operational output and are not part of the durable backup. Archive
  them separately if your audit policy requires it.

PRO8 adds bounded retention and storage maintenance. It does not change the
PRO7 rule that only a verified coherent snapshot is a supported recovery point.
Retention is not a backup: it intentionally removes eligible history, while a
verified coherent snapshot remains the supported recovery point.
