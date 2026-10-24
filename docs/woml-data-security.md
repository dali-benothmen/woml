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
- Startup fails closed when Store v16 integrity (including durable waits, locks, and State v1) contradicts its schema,
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

`services.process.run()` adds an operator-controlled boundary for approved
native tools; it does not turn arbitrary native code into a safe sandbox.
Process Policy v1 starts commands with an empty environment, permits only
reviewed inherited or explicit names, resolves working directories beneath a
canonical project root, pins executable identity, bounds output, time, and
concurrency, and terminates the supervised process tree on timeout or
cancellation.

Known secret values are redacted from captured process output, but a program
can transform or write a secret in ways WOML cannot recognize. Do not pass
secrets as command-line arguments, because privileged host inspection may
expose them. Prefer owner-readable credential files or narrowly allowed
environment names, discard unneeded output, and never return
credential-bearing output as durable step context. See
[Running Local Tools Safely](woml-process.md).

With `stream: true`, the same known-secret redaction runs incrementally across
chunk boundaries before operational persistence. Script logs also redact known
WOML secrets and materialized artifact paths, and terminal renderers escape
control sequences. This does not recognize unknown, transformed, encoded, or
application-sensitive data. Treat `.woml/output.sqlite` and any transcript
artifact as sensitive operator data, grant rollover explicitly, and keep their
directories owner-controlled. See
[Streaming Output and Progress](woml-streaming-output-and-progress.md).

## Lock keys and lease safety

Lock keys are operator-visible coordination identifiers. Never include secrets,
bearer credentials, raw customer payloads, or personal data in them. Rust keeps
lease IDs, input digests, renewal state, and runtime ownership out of public
context, events, logs, metrics, and inspection.

Fencing tokens are ordering evidence, not credentials. Managed effects verify
current ownership where supported, but an arbitrary external target must store
and reject older tokens itself. Backup restore preserves fencing high-water and
clears live owners; activating both the source and restored databases is an
unsupported split-brain deployment. See the
[Locks threat model](woml-lock-threat-model.md).
## Remote execution trust boundary

Remote Execution Targets do not make a worker part of the workflow authority.
The coordinator remains authoritative for admission, attempts, retries,
policies, waits, locks, correction, and settlement. Workers receive one
integrity-bound compiled attempt through mutually authenticated transport.

Target configuration, certificate references, and worker membership remain
outside WOML source. Dispatches contain symbolic secret names only; values are
resolved by the worker's local provider after both coordinator and worker
policy allow the operation. Large values cross through scoped durable-artifact
grants rather than the JSON dispatch envelope.

If WOML can prove that a worker started an effect but cannot prove its terminal
outcome, the dispatch becomes ambiguous. It is never retried silently. An
operator must reconcile a trusted terminal journal record, record failure, or
explicitly authorize a new attempt with possible-duplicate risk.

The frozen protocol, bounds, threat model, and failure catalog are documented
in [Remote Execution Targets v1](protocols/remote-execution-targets-v1.md).
