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

Human, plain, and JSON run presentations additionally neutralize terminal
controls and redact credential-shaped values, capability/approval URLs, and
idempotency material. This is a display boundary, not permission to return
secrets as business results. See
[WOML Terminal Experience](woml-terminal-experience.md#security-and-bounds).

When a runtime is active, `woml list`, `woml get`, and `woml cancel` first use
its owner-only descriptor and rotating loopback administration capability.
They never accept a public trigger or provider credential. `list` and `get`
also retain safe offline inspection when the runtime is stopped. Do not expose
the state file, descriptor, or loopback operations port to another user or
machine.

## What WOML guarantees

- State operation events use digests instead of raw keys and values.
- Cache values, storage bodies, database rows, credentials, and state values do
  not appear in ordinary run inspection.
- Startup fails closed when Store v14 integrity (including State v1) contradicts its schema,
  digests, canonical results, versions, or quotas.
- WOML does not silently copy the `secrets` object into workflow state.
- Reusable definitions receive only secret props explicitly bound with an exact
  `{{secrets.NAME}}` reference. Those values are omitted from Definition
  Package v9, Event v13, Inspection v5, provider receipts, and normal logs.
- Communication providers receive only their explicitly referenced symbolic
  credentials. Normalized inbound text and routing identities can enter
  workflow input because execution requires them, but raw envelopes, headers,
  signatures, access tokens, and provider profiles never do. Accepted content
  follows the same backup, retention, and prune policy as other run input.

Provider-specific credential rotation, diagnostics, callback verification, and
recovery are documented in
[Communication Provider Diagnostics and Operations](communication-provider-operations.md).

WOML cannot determine whether an arbitrary value authored by JavaScript is
sensitive. If a script stores a secret in state, that secret is present in the
unencrypted local SQLite file. Transparent encryption, remote authorization,
and external durable-state backends remain Production Runtime work.

## Process isolation

WOML bounds protocol frames, script context/results, operation timeouts,
runtime worker configuration, and local admin traffic. It does not claim that
Bun JavaScript is a hostile multi-tenant sandbox. Run untrusted workloads in a
dedicated OS account or container with memory, CPU, process, open-file,
read-only-filesystem, and egress limits. Keep the state, descriptor, mounted
secrets, logs, backups, and crash dumps outside other tenants' reach.
