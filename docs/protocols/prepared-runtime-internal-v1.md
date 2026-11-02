# Prepared-runtime internal design v1

Status: SRS2 production integration and SRS3 safety checkpoint. This is a private
Rust implementation contract, not a serialized protocol, N-API token, new
public export, or certification of all platforms. Matched optimized performance,
clean-package acceptance, and platform execution remain SRS4 work.

## Purpose and boundary

Readiness proves that one owned runtime completed startup for one exact physical
database. It does not authorize a workflow, service call, attempt, or external
effect. Existing policy, emergency denies, durable operation identities, scheduler
claims, lock fencing, cancellation, and correction remain mandatory.

Production implementation lives in
[durable/prepared.rs](../../core/woml-engine/src/durable/prepared.rs) and
[runtime/preparation.rs](../../core/woml-engine/src/runtime/preparation.rs).
The SRS1 test-only prototype remains as a historical design regression; the
production tests exercise the real factory and transactions independently.

## Startup receipt and publication

The native coordinator preserves this order:

1. Full open, integrity audit, and exact runtime-owner acquisition.
2. Retain the acquisition connection through trigger preparation.
3. Full trigger-store opening, required recovery, activation validation, and
   immutable definition/module registration, with admission still closed.
4. Perform the existing first StateService binding once during owned startup.
   It previously invoked full engine initialization in the first request.
   This moves cost; it does not remove state validation or mutation checks.
5. Mint private preparation using both the fully initialized preparation store
   and the actual acquisition connection.
6. Bind listeners, return the prepared server, then explicitly activate admission.

The factory rejects a marker-only `open_ready` connection as a startup receipt.
A matching copied owner row is insufficient: both live SQLite connections must
refer to the pinned physical database. Failed preparation publishes no usable
native runtime; cleanup uses the original acquisition connection rather than
reopening or creating a database to release an owner.

Existing full initialization can repair indexes before native owner acquisition.
This checkpoint preserves that startup behavior. It does not claim that all
historical repair was already behind ownership, or move recovery into requests.

## Physical identity and compatibility

The context retains the canonical path, exact owner tuple, SQLite schema cookie,
shared lifecycle state, and an OS file pin:

- Unix: device/inode identity, checked against the live path, plus
  `SQLITE_FCNTL_HAS_MOVED` on the actual SQLite connection. This detects ABA
  swaps that a before/after pathname comparison alone cannot detect.
- Windows: volume/file identity and SQLite's borrowed Win32 handle via
  `SQLITE_FCNTL_WIN32_GET_HANDLE`. The pin permits reads/writes, not deletion.
  SQLite's borrowed handle is never closed by WOML.
- Unsupported identity controls reject; they never imply success.

Prepared connections use READ_WRITE without CREATE, preserve the five-second
busy timeout and foreign keys, and verify physical identity, store version,
schema cookie, owner tuple and lease. They never migrate, rebuild indexes, or
run global interrupted-run recovery. Missing or invalid preparation does not
silently fall back to repair. Ordinary cross-connection data commits remain
visible; `data_version` and event counts are not database identities.

The SQLite interfaces are the
[file-control API](https://www.sqlite.org/c3ref/file_control.html) and
[file-control opcodes](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html).
Only the bundled native VFS is targeted. Custom/network filesystems and arbitrary
in-place/offline manipulation of a live SQLite database are not certified.

## Transaction guard

`DurableEventStore` retains its own connection and projection cache. A private
connection adapter preserves existing transaction bodies and wraps their
transaction creation and commit:

1. Acquire the existing SQLite writer lock.
2. Recheck preparation against that actual connection.
3. Execute the existing validations, event/index updates and durable fences.
4. Recheck preparation before SQL commit.
5. Commit, then publish the existing candidate projection/cache.

A failed check rolls SQL back; a tentative projection is not published. Direct
autocommit `execute` writes on a prepared store also receive a guarded immediate
transaction. Unprepared connections preserve their original behavior.

The adapter is not a general-purpose SQL sandbox. Its raw connection dereference
exists for existing read helpers and transactional SQL helpers. New runtime
mutations must continue using the owned transaction/execute adapter; do not
introduce raw prepared-statement writes or independent commits to bypass it.
Serialized startup/schema literals and historical transaction baselines remain
unchanged.

An SQL guard cannot make an external effect atomic with shutdown or a filesystem
rename. Already-authorized effects still follow their own durable settlement and
ambiguity rules; no exactly-once external-effect guarantee is introduced.

## Owner, expiry and renewal

Deployment, activation and runtime instance must match the current durable owner
row, not an options snapshot. The in-memory lifetime is not a new durable fencing
token and does not replace per-run claims.

New requests reject an expired owner. Only the narrow Rust owner-heartbeat
method may retain the existing ability to renew the same exact row after expiry
when no successor has replaced it. Expiry alone does not permanently revoke the
context; transfer, incompatible identity/schema and explicit stop do.

Heartbeat uses a fresh narrow connection and the guarded exact-owner update.
Failure retains the native OwnershipLost notification and bounded shutdown.
Old renewal/release cannot alter or delete the successor's owner.

## Drain and stop

Admission closes before draining. Bound execution options retain one real,
durably admitted run identity. The transaction adapter publishes newly committed
trigger-admission identities only after successful SQL commit. A blocked ingress
response may return after draining starts; its exact already-committed dispatch
can still bind. An unrelated historical run cannot obtain that permit.

Unbound admission transactions reject drain at opening and transaction checks.
Already-bound work may settle under existing run/attempt/operation/claim rules
until the existing shutdown deadline. Runtime-owner heartbeat remains alive
through draining; it is aborted after server stop.

Shutdown retains listener/task/host cleanup, unregisters only the old routing
identity, releases only the exact owner inside a writer transaction, and revokes
all clones. Revocation drops the OS pin even when a dormant timeout task still
holds an options clone. A retained clone cannot reopen the store or repair it to
regain readiness.

## Scope and propagation

Applicable clones share readiness/revocation, never mutable projections or
cached authorization decisions. Replacing the capability registry or preparing a
remote worker clears coordinator preparation. Transplanting already-prepared
options into another server activation rejects before opening that store.
One-shot execute/resume entrypoints reject prepared options rather than invoking
global recovery with a live runtime's lifetime.

The native signal clone and heartbeat are taken from the successfully prepared
server, not the pre-startup options. Selected ingress, admitted execution,
authority attachment, wait/signal/timeout continuations, correction discovery,
scheduler-claim maintenance, activation and stop use the same private lifetime.

Explicit exclusions remain: independent/unowned Rust embedding, standalone
commands and inspection, backup/restore, worker journals, notification transport
internals, programmatic-lock service's separate opening, and the separate
child-routing execution configuration. Do not infer complete child configuration
inheritance or an all-features zero-open claim from this checkpoint.

## Evidence and remaining acceptance

Production unit tests cover publication receipts, actual-connection identity,
owner transfer after opening, expiry/renewal, guarded autocommit, pre-commit
rollback/cache authority, SQL failure, drain and late admission, current schema,
aliases, visibility, unsupported VFS, and platform-specific replacement.

The actual-native verifier compares one-shot and owned histories/results for
selected deterministic workflows. It checks startup still initializes, then
counts full-init spans across owned execution, idle owner heartbeats and stop in
an isolated process, without native inspection polling. It injects a successor
owner and verifies that no new run is admitted and no successor row is removed.

Lifetime-owned structural counters separately record the successful preparation
store's startup receipt and scoped prepared opening/recovery activity. These are
not a process-global or thread-local async scope, and do not claim attribution
for excluded APIs. No high-cardinality OpenTelemetry fields are added.

See [the inventory](../runtime-startup-separation.md) for converted callers and
[the verification record](../runtime-startup-separation-verification.md) for exact
results, failed diagnostic attempts, exclusions and unrun release gates.
