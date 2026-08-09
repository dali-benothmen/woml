# WOML Database v1

Status: frozen and first executed by SC7 on 2026-08-10. SQLite is the SC7
backend; PostgreSQL joins this same JavaScript contract in SC8.

## JavaScript surface

```js
const db = services.db({
  driver: "sqlite",
  connection: "./data/customers.sqlite"
});

const customers = await db.query({
  text: "SELECT id, name FROM customers WHERE active = ?",
  values: [true]
});
```

`services.db(config)` returns an invocation-local, deeply read-only proxy. It
does not expose a connection object, pool, transaction handle, close method, or
internal state. The frozen methods are `query`, `execute`, `read`, `insert`,
`update`, `delete`, and `transaction`.

Every method optionally accepts `{ name: "stable-name" }` as its second
argument. A step may make one automatic effectful call to each method; repeated
writes require stable names. Reads may be multiplexed.

## Operations

- `query({ text, values? })` accepts one read-only parameterized statement and
  returns `{ rows, rowCount }`.
- `execute({ text, values? })` accepts one parameterized non-row statement and
  returns `{ rowsAffected, lastInsertId }`.
- `read({ table, columns?, where?, orderBy?, limit?, offset? })` builds a
  parameterized equality query from validated identifiers.
- `insert({ table, values })` builds one parameterized insert.
- `update({ table, values, where })` and `delete({ table, where })` require a
  non-empty equality filter. V1 deliberately has no implicit update-all or
  delete-all helper.
- `transaction({ operations })` executes 1–100 query/execute/insert/update/delete
  operations atomically. It returns `{ results }` in authored order. An error or
  cancellation rolls back the complete batch.

V1 never holds a transaction open across an arbitrary JavaScript callback.

## Values and limits

Parameters are JSON `null`, booleans, finite strings/numbers, or
`{ bytesBase64 }`. JavaScript integers must be safe integers. SQLite BLOB
results use `{ bytesBase64 }`. SQLite integers outside JavaScript's safe range
are rejected as `invalid_result` rather than silently rounded. SQLite has no
portable boolean column type, so queried booleans may appear as `0` or `1`.

Inputs use Capability Call v1's 1 MiB limit and results use its 4 MiB limit.
Database v1 additionally limits a result to 10,000 rows, 256 columns, SQL text
to 256 KiB, a transaction to 100 operations, and identifiers to 128 ASCII
letters/digits/underscores beginning with a letter or underscore.

## Effects, failures, and recovery

`query` and `read` are read effects. The SQLite handler verifies that raw
`query` statements are read-only. Every other method is an unsafe write. SQLite
constraint, busy, cancellation, malformed result, unavailable database, and
invalid input failures have distinct safe WOML codes; raw SQLite messages, SQL,
parameters, rows, and connection paths never enter durable events or terminal
diagnostics.

Rust records the generic managed operation start before dispatch and its
terminal event before returning to Bun. Recovery fails an unclosed write
operation as ambiguous and does not replay it. The transaction batch is atomic
inside SQLite, but a process loss after commit and before terminal-event append
still cannot prove to WOML that the external write did or did not commit.

## SQLite ownership and isolation

The connection is a user-owned filesystem path resolved from the activated
CLI process working directory. `:memory:`, SQLite URI filenames, empty paths,
directories, and paths resolving to WOML's selected state database are
rejected. Rust maintains one supervised writer per canonical user path and
opens isolated readers so Promise-based reads can be multiplexed. Every handle
turns on foreign keys, uses a bounded busy timeout, and applies WAL mode for
ordinary file databases.

The user database is application data. It is not WOML context, the event log,
cache, or the future durable user-state API. Conversely, the internal WOML
state database can never be opened through `services.db`.
