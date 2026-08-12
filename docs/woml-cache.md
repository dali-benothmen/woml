# WOML Workflow Cache

SC10 adds a Rust-owned local cache through `services.cache`. Use it to avoid
repeating an expensive API request, database lookup, or calculation when a
temporarily stale or missing value is safe.

```js
const cached = await services.cache.get("customer:42");
if (cached.hit) return cached.value;

const customer = await loadCustomer();
await services.cache.set("customer:42", customer, { ttl: "15m" });
return customer;
```

Cache is never durable business state. WOML may return a miss because an entry
expired, was evicted, or the runtime moved to another state location. Workflow
correctness must not depend on a cache entry existing.

## Operations and results

- `get(key)` returns `{ hit: true, value, expiresAt }` or `{ hit: false }`.
- `set(key, value, options?)` replaces the value and returns
  `{ stored: true, expiresAt }`.
- `delete(key, options?)` returns `{ deleted }`. A missing key returns false,
  not an error.
- `has(key)` returns `{ present }` without returning the value.
- `increment(key, amount?, options?)` atomically adds a safe integer. A miss
  starts at zero; omitting `amount` adds one. An existing non-integer fails.
- `setIfAbsent(key, value, options?)` atomically creates the entry or returns
  the value already stored as `{ stored: false, value, expiresAt }`.

Write options accept `ttl` and a stable `name`. TTL defaults to `5m`; use an
integer number of milliseconds or a whole duration such as `500ms`, `10s`,
`15m`, `2h`, or `30d`. The valid range is 1 ms through 30 days. If an existing
counter is incremented, its original expiry is preserved. `setIfAbsent` uses
the requested TTL only when it wins the creation race.

## Workflow scope and definition updates

Entries are scoped by workflow ID and state location:

- runs of the same workflow share entries;
- a new definition or version with the same workflow ID keeps the warm cache;
- a different workflow ID cannot read the entry; and
- a different WOML state location has a different local cache.

Rust derives that scope from the durable run binding. It is not supplied by
the script, is not exposed as `context.run`, and is not added to the frozen
Capability Call v1 protocol.

## Expiry, eviction, and restart

The default local backend is `cache-v1.sqlite` beside the selected WOML state
database. A custom state filename receives a state-derived cache filename, so
two state databases in the same directory still have independent capacity and
entries. Unexpired entries survive `woml run` process restarts. Expiry is
exact: an entry is a miss as soon as the current time reaches its expiry.

The local store holds at most 10,000 entries and 64 MiB of canonical JSON.
Each value is at most 256 KiB and each key at most 256 UTF-8 bytes. WOML removes
expired entries first and then evicts least-recently-used entries. `get`, `has`,
and a losing `setIfAbsent` count as access. Eviction is normal cache behavior.

## Atomicity, events, and failures

Every cache operation is one SQLite transaction. `increment` and
`setIfAbsent` remain atomic when multiple runs or runtime processes share the
same state location.

Managed-operation events record the operation and a SHA-256 key digest, never
the raw cache key or value. A script can still explicitly return a bounded
value as its step result; that follows ordinary context persistence rules.

Misses and deleting absent entries are successful results. Stable failures are
reserved for malformed inputs, values over the limit, integer mismatch or
overflow, cancellation, corrupt storage, unavailable storage, and a missing
internal workflow scope.

Atomicity does not promise exactly-once JavaScript execution. If a step
increments a counter and later fails, a step retry can increment it again.
Cache must not be used for financial counters or any other correctness-critical
state. Use a named `services.state.increment` for a small workflow-owned
counter, or a database operation with an idempotency design when records are
shared or queryable.

## Cache or durable state?

Use cache only when a miss, expiry, or eviction changes performance rather than
correctness. Use `services.state` when a future run must remember a small JSON
fact and the write must safely reattach after retry. State never expires or
evicts automatically and supports durable versions plus `ifVersion`.

The complete cache/state/storage/database comparison is in
[Choosing Where Workflow Data Lives](woml-data-guide.md).

## Run the example

Build/link the CLI, then run this twice from the project root:

```bash
woml test examples/cacheWorkflow.woml
woml test examples/cacheWorkflow.woml
```

The first run initializes the customer and counter. The second reuses the
customer, increments the counter atomically, and reports `loadedFromCache` as
true. Removing `.woml/cache-v1.sqlite` intentionally clears this local cache;
do that only while no WOML runtime is using the state location.
