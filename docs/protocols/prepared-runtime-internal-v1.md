# Prepared-runtime internal design v1

Status: SRS1 design with an executable **test-only prototype**, not a public
protocol, serialized token, production connection factory, or release-ready
permission boundary. No production opening API changes in this checkpoint.

## Decisions

1. Preparation proves store readiness for a particular owned lifetime, not
   authority to execute a workflow, service operation, or external effect.
2. Keep one-shot/unowned APIs unchanged. Initial optimization targets the native
   coordinator that already acquires a durable runtime owner. Independently
   embedded Rust servers and worker journals must not inherit it by filename.
3. Keep the context private inside Rust runtime options/state. The planned
   cross-crate entrypoint is a narrow method on the existing runtime-options
   facade that verifies startup and exact owner before returning options with
   private preparation attached. Exact implementation names remain SRS2 code
   review; no N-API token/flag, public context constructor, or new registry.
4. Retain separate scoped connections for execution and managed authority.
   No global SQLite connection, shared mutable projection, or new scheduler.
5. Prepared opening must not create a vanished database and must not invoke
   DDL, migration, index rebuilding, or general recovery. A missing context uses
   the old path; invalid preparation rejects rather than repairing itself.

## Startup and publication

Preserve startup compatibility work and the current native acquisition path:
full opening/audit → exact owner acquisition → trigger preparation and required
scoped recovery → prepared publication → admission opening. Failure must close
prepared listeners/resources, release only the exact owner if held, and never
publish a usable context. Do not move general recovery before owner acquisition.

Existing full opening performs transactional derived-index work before the
native owner is acquired. Keep that behavior at startup in SRS2; the change is
to stop repeating it in owned requests. The startup inventory records this
distinction rather than claiming that all repair is currently behind the owner.

SRS2 must mint context only at this successful startup boundary. The test-only
`after_startup` helper assumes its private test caller has fully opened/audited
the supplied `DurableEventStore`; it is deliberately not a production proof
that any arbitrary marker-only connection completed all startup work.

## Exact database identity

Path strings, database lengths/mtimes, equal event counts, version markers, and
copied owner rows do not distinguish a replacement database.

The selected prototype pins an open OS file for the original canonical state:

- Unix: retain device/inode identity while the pin prevents inode reuse. Compare
  the live pathname with that pin, and ask SQLite's **actual connection VFS**
  whether its file has moved. Verify that connection's database filename resolves
  to the expected path. Path checks alone cannot detect an ABA swap during open.
- Windows: retain a file handle shared for reading/writing but not deletion.
  Compare volume/file identity with SQLite's actual borrowed Win32 file handle.
  Do not close or transfer SQLite's handle, or assume Unix VFS controls work there.
- If the required file-control/identity proof is unsupported, reject it. Do not
  interpret an unimplemented file control as a successful identity check.

The relevant SQLite interfaces are documented in the
[file-control API](https://www.sqlite.org/c3ref/file_control.html) and
[file-control opcodes](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html).
The prototype uses HAS_MOVED on Unix and WIN32_GET_HANDLE on Windows, with the
bundled SQLite VFS. This is not certification of custom/network filesystems.

Same-path and ABA replacement regressions are executable on Unix. Windows tests
exercise its native handle binding; replacement while a delete-denying pin is
held follows Windows sharing behavior rather than assuming Unix rename succeeds.
Native Windows/macOS CI must pass before treating the adapter as validated there.

After opening READ_WRITE without CREATE, recheck physical identity, WOML store
version, and the captured SQLite schema cookie on that exact connection.
Ordinary data commits remain visible; SQLite `data_version` is not used as a
store identity or as a reason to invalidate after every legitimate write.

These checks are opening prerequisites, not a transaction fence. SRS2 must
recheck the actual connection/owner where work is admitted or mutated, retain
SQLite's locking guarantees, and roll back SQL work if a pre-commit identity
check fails. Arbitrary offline file manipulation while a runtime is active is
unsupported; detected movement fails closed. The design does not claim to make
external effects atomic with a filesystem rename.

## Owner and lease rules

Bind deployment, activation, and runtime instance to the exact durable owner
row. A local generation/Arc identity is only a lifetime discriminator, not a new
durable fencing token. Owner checks must read the current connection, not a
snapshot stored in options.

The current owner renewal transaction matches runtime instance and can renew
its own row after expiry if no successor replaced it. Do not silently change
that behavior as part of the optimization:

- Reject new request eligibility while the owner lease is expired.
- Allow only the trusted heartbeat path to perform the existing exact-owner
  renewal, including this existing expiry case.
- After successful renewal, requests may qualify again if identity/schema and
  ownership remain valid. Owner transfer or revocation cannot be resurrected by
  that renewal path.
- Owner mismatch/replacement revokes all clones. An old owner's release cannot
  clear a successor.

The prototype checks transfer both at opening and again on an already-open
connection. That demonstrates why checking once at construction is insufficient.
SRS2 must place durable checks inside the relevant admission/claim/mutation
transaction so another connection cannot transfer ownership between the check
and its guarded SQL mutation. Existing per-run claim/attempt/operation and lock
fences remain mandatory; a live runtime owner does not substitute for them.

## Lifetime and drain matrix

| State/event | New dispatch | Existing work | Heartbeat / next step |
| --- | --- | --- | --- |
| Prepared and live | Eligible after normal admission/authorization | Existing durable rules | Exact renewal |
| Draining | Closed | Only already-admitted identities may settle within the deadline | Maintain exact owner until drain completes |
| Expired but same owner | Refused | Existing claim/effect rules; no new forward dispatch | Existing exact renewal may succeed; otherwise stop |
| Owner transferred, file moved, incompatible schema, explicit revocation | Refused; shared clones invalid | Stop/cancel through existing authority; uncertain effects stay ambiguous | Never renew/release the successor or recover globally to regain access |
| Clean stop | Closed | Settle or cancel within existing shutdown contract | Revoke, release exact owner, drop handles |
| Offline restore/replacement | Old context unusable | No automatic replay of uncertain work | Fresh full startup and owner acquisition |

The prototype's `ExistingWorkPermit` tests only shared lifetime identity and
draining eligibility. It is not a business permission or proof of durable
admission. Production continuation permits must additionally bind the real
run/attempt/claim and retain compensation/cleanup rules. Request and heartbeat
roles must be separate trusted Rust paths, never a caller-selected JS boolean.

An in-memory state check cannot make an entire script atomic with stop or lease
loss. SRS2 must integrate the real admission barrier, active-run accounting,
transaction boundaries, host cancellation, and bounded shutdown. Already
authorized in-flight settlement follows existing semantics, not an invented
promise that every pending operation disappears instantaneously on revocation.

## Propagation and authority

Cloning the same owner/store's options may share readiness/revocation. It must
not capture stale policy decisions, emergency overlays, secrets, reviewers,
modules, service registries, resource limits, or mutable projections.

Same-owner child work may inherit readiness but still needs its exact child
authority and scheduling capacity. Clear/re-establish preparation when moving to
another store, activation, destination runtime, worker journal, or independently
constructed capability domain. Never reuse coordinator preparation for a worker.
This design does not certify the deferred child-configuration propagation audit.

## Executable checkpoint and remaining gates

`core/woml-engine/src/durable/prepared_contract.rs` is included only under
`#[cfg(test)]`. It verifies exact construction, missing-file behavior, foreign
lifetimes, shared revocation/draining, physical replacement including ABA,
owner transfer, expiry/renewal, cross-connection data visibility, incompatible
schema rejection, unsupported VFS rejection, and opening while a writer holds
an IMMEDIATE transaction without invoking startup repair.

The test fixture releases SQLite and pin handles before removing temporary
directories, including on Windows. There are no production opener substitutions,
new dependencies, event/store versions, native exports, or budget changes.

Before SRS2 completion, integrate the startup receipt and admission/transaction
guards described above, and test that the actual converted request/heartbeat
paths do no full initialization. Before SRS3/SRS4 completion, add race/failure
rehearsals at real native boundaries and complete clean-package, recovery,
performance, and platform validation. Passing this prototype alone proves none
of those later release gates.
