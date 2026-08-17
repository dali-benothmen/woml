# WOML Cache v1

Status: frozen for SC10 on 2026-08-10.

Cache v1 is the workflow-scoped, expiring JSON cache behind
`services.cache`. It is an optimization surface, not durable workflow state.

## Public operations

- `get(key)` returns `{ hit: true, value, expiresAt }` or `{ hit: false }`.
  A miss is a normal successful result, including when an entry has expired.
- `set(key, value, options?)` replaces the entry and returns
  `{ stored: true, expiresAt }`.
- `delete(key, options?)` returns `{ deleted }`; deleting a missing key is a
  normal idempotent success.
- `has(key)` returns `{ present }` and never exposes the value.
- `increment(key, amount?, options?)` atomically increments a JSON safe
  integer. A missing entry starts at zero. An existing non-integer or overflow
  fails without modifying the entry.
- `setIfAbsent(key, value, options?)` atomically creates an entry only when no
  unexpired entry exists. It returns the winning `{ stored, value, expiresAt }`.

The JavaScript method `setIfAbsent` is normalized to the protocol operation
name `set_if_absent`, because Capability Call v1 operation names are lowercase.

Options accept `ttl` and, for writes, the stable operation `name`. The facade
defaults `ttl` to `5m` and accepts one positive whole duration using `ms`, `s`,
`m`, `h`, or `d`, from 1 ms through 30 days. The Rust wire contract receives
the normalized `ttlMs` integer.

## Namespace and lifecycle

The Rust durable authority derives the namespace from the immutable run
binding. The namespace is the workflow ID, so separate workflow IDs cannot see
each other's entries while a new definition/version of the same workflow can
reuse warm cache entries. Workflow identity is not added to `context.run` and
does not cross the frozen Capability Call v1 protocol.

The local backend is `cache-v1.sqlite` beside the default WOML state database.
A custom state filename receives a state-derived cache filename, so separate
state locations do not share capacity or entries even when their databases are
in the same directory. Unexpired entries survive a CLI restart. Expiry is lazy
and exact: an entry is expired when `now >= expiresAt`. A test clock makes the
boundary deterministic.

## Bounds and eviction

Keys are non-empty UTF-8 strings of at most 256 bytes. Canonical JSON values
are limited to 256 KiB. One local cache is bounded to 10,000 entries and 64
MiB of canonical values. Writes first remove expired entries, then evict the
least-recently-used entries until both bounds hold. A durable access sequence,
with workflow scope and key as deterministic tie-breakers, defines eviction.

Every operation is one SQLite transaction. `increment` and `setIfAbsent` are
therefore atomic across concurrent runs and runtime processes sharing the state
location.

## Safety and diagnostics

Raw keys and values never enter operation events. Events contain only a
SHA-256 key digest and bounded booleans/counts. Cache misses are not errors.
Invalid input, size limits, integer mismatch/overflow, unavailable storage,
corrupt storage, cancellation, and missing internal workflow scope have stable
safe failure codes. Eviction is expected best-effort cache behavior and is not
reported as workflow failure.
