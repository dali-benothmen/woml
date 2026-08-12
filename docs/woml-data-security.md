# WOML Local Data Security

WOML keeps credentials and application data out of ordinary events, progress,
run inspection, and logs, but local persistence is not an encryption boundary.
The selected state directory can contain workflow history, durable user state,
cache data, module artifacts, and storage objects. Author-owned SQLite
databases may live elsewhere.

## Operator responsibilities

- Run WOML under a dedicated least-privilege operating-system account.
- Restrict the state directory to that account. State v1 additionally hardens
  its SQLite file to `0600` on Unix.
- Use encrypted disks and encrypted backups when values are sensitive.
- Never place the SQLite state database on a network filesystem.
- Back up the complete SQLite database coherently; back up `objects-v1`
  alongside it when using `services.storage`.
- Keep database URLs and provider credentials in `woml secrets`, never in
  WOML source or `services.state`.
- Treat copied databases, WAL files, crash dumps, and support bundles as
  sensitive application data.

`woml list` and `woml get` are local operator controls, not remotely
authenticated administration APIs. Do not expose direct state-file access or
wrap these commands in a network service without a reviewed authorization and
audit design.

## What WOML guarantees

- State operation events use digests instead of raw keys and values.
- Cache values, storage bodies, database rows, credentials, and state values do
  not appear in ordinary run inspection.
- Startup fails closed when Store v14 integrity (including State v1) contradicts its schema,
  digests, canonical results, versions, or quotas.
- WOML does not silently copy the `secrets` object into workflow state.

WOML cannot determine whether an arbitrary value authored by JavaScript is
sensitive. If a script stores a secret in state, that secret is present in the
unencrypted local SQLite file. Transparent encryption, remote authorization,
and external durable-state backends remain Production Runtime work.
