# WOML Durable User State v1 Contracts

Status: frozen by DS0 on 2026-08-12. DS1 exposes the typed authoring surface,
DS2 implements the transaction-tested Store v13 Rust authority, DS3 connects
real WOML scripts through the managed capability path, and DS4 hardens local
concurrency, recovery, integrity, security, and performance.

## Public service contract

`services.state` provides `get`, `has`, `set`, `delete`, `increment`, and
`setIfAbsent`. Values are JSON. Reads return an explicit missing shape rather
than `undefined`; mutations return the committed version/result.

Every mutation requires `{ name: "stable-name" }`. `set`, `delete`, and
`increment` additionally accept `ifVersion`. Version `0` requires absence, a
positive version requires an exact match, and omission is unconditional.

Frozen limits are 256 UTF-8 bytes per key, 256 KiB of canonical JSON per value,
10,000 live keys and 64 MiB of canonical values per workflow scope, and
JavaScript safe integers for versions/increments. Quota failure never evicts or
changes another entry.

## Scope and authority

Rust derives scope from the selected state location plus workflow ID. Runs and
definition versions of that workflow share state; another workflow cannot read
it. State survives restarts and run-history retention until explicitly deleted.

State is authoritative application data in separate Store v13 tables. It is
not part of folded `context`, is not reconstructed from run events, and never
falls back to Cache v1. Cache may expire/evict; state may not.

## Mutation identity and atomicity

Rust derives the logical mutation identity from run ID, step or lifecycle
subject, attempt idempotency key, and the author's stable operation name. The
same mutation and canonical input returns its first durable result. Reusing an
identity with another operation, key, or input fails with
`WOML_STATE_OPERATION_IDENTITY_CONFLICT`.

Store v13 commits the value/version change, mutation input/result record,
redacted audit metadata, and the immutable settlement proof in one SQLite
transaction. This makes increments retry-safe and compare-and-set atomic across
processes. The ordinary run event is appended immediately afterward. If the
host stops in that narrow gap, recovery validates the settlement proof and
appends the missing success event without replaying the state mutation; the
interrupted script attempt still fails closed.

The frozen physical authority uses three separate tables:

- `woml_state_entries` for scope/key, canonical JSON, byte count, version, and
  update instant;
- `woml_state_mutations` for immutable logical operation identity, canonical
  input digest, and the original result needed for reattachment; and
- `woml_state_quotas` for transactionally maintained live-key/value-byte totals.

Store v12-to-v13 migration creates and validates all three tables and their
required indexes inside one transaction, updates store metadata last, and
never rewrites definitions or Event v1-v11 histories. Failure rolls the entire
migration back. Unknown future versions and missing/corrupt required objects
fail closed.

## Redaction and security

Run events/progress/errors/inspection may contain only State Operation Metadata
v1: operation, digests, version/byte counts, duration, and safe outcome. Raw
keys and values are forbidden. The local SQLite database necessarily contains
application keys and values and is not transparently encrypted. Operators must
protect it like a business database; encryption and remote authorization are
Production Runtime work.

## Existing protocol proof

Capability Call v1 already carries capability `state`, a static operation,
versioned JSON input/result, internally derived identity, limits, and generic
safe failures. Script Host v7 already transports that generic call/result
without a new field. DS0 fixtures validate the exact State v1 request inside
the unchanged Capability Call v1 schema, so neither protocol is versioned.

## Frozen artifacts

- `durable-state.v1.schema.json`
- `durable-state-mutation-identity.v1.schema.json`
- `state-operation-metadata.v1.schema.json`
- `durable-state-store.v13.schema.json`
- reviewed operation, result, identity, Store v13, generic capability-call,
  conflict, quota, interruption, and redaction fixtures

## DS0 review decisions

1. Methods and return shapes are small, explicit, and JSON-only.
2. Scope is derived internally from state location plus workflow ID.
3. State and cache have different contracts, storage, lifetime, and failures.
4. Every mutation is author-named and reattachable across retries.
5. Identity/input disagreement fails closed.
6. Compare-and-set and version mutation are one Rust transaction.
7. Duplicate increments return their first stored result.
8. Quota failure preserves every previous entry and never evicts.
9. Raw keys and values are excluded from public operation metadata.
10. Mutation, reattachment result, audit, and the recovery settlement proof are
    atomic; the ordinary success event is recoverable from that proof.
11. Run-history retention does not own or delete workflow state.
12. Capability Call v1 and Script Host v7 carry State v1 unchanged.
13. Transparent encryption and remote authorization are explicitly deferred.
14. State is never injected into `context.run`, `context`, or `secrets`.
