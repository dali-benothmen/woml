# WOML Database v1

Status: frozen and executed through SC8 on 2026-08-10. SQLite and PostgreSQL
share this JavaScript contract. The PostgreSQL driver activation is pinned in
`database-v1-sc8-activation.md`.

## JavaScript surface

```js
const db = services.db({
  driver: "postgres",
  connection: secrets.POSTGRES_URL
});

const customers = await db.query({
  text: "SELECT id, name FROM customers WHERE active = $1",
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
PostgreSQL supports Boolean results directly. Its reviewed result types are
Boolean, safe integers, finite floating-point numbers, text, binary, UUID,
date, timestamp, timestamp-with-time-zone, and JSON/JSONB serialized as text.
Authors cast other PostgreSQL types to one of those types explicitly.

Inputs use Capability Call v1's 1 MiB limit and results use its 4 MiB limit.
Database v1 additionally limits a result to 10,000 rows, 256 columns, SQL text
to 256 KiB, a transaction to 100 operations, and identifiers to 128 ASCII
letters/digits/underscores beginning with a letter or underscore.

## Effects, failures, and recovery

`query` and `read` are read effects. SQLite verifies prepared-statement
read-only status; PostgreSQL executes raw `query` inside a read-only
transaction. Every other method is an unsafe write. Constraint, contention,
cancellation, malformed result, unavailable database, connection-loss, and
invalid-input failures have distinct safe WOML codes. Provider messages, SQL,
parameters, rows, connection paths, URLs, and credentials never enter durable
operation metadata or terminal diagnostics.

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

## PostgreSQL ownership and pooling

`driver: "postgres"` accepts a PostgreSQL URL or libpq-style connection
string. Authors normally pass `secrets.POSTGRES_URL`; the resolved value stays
in invocation memory and Rust's connection configuration. TLS uses the system
of Web PKI roots bundled by the runtime when SSL is enabled. Local development
may opt into `sslmode=disable`; production credentials should require TLS.

Rust keeps a pool per hashed connection configuration, permits at most 16
active connections per pool, and permits at most 64 pools in one process.
Idle connections are reused, closed connections are discarded, and calls wait
for a bounded pool permit instead of opening an unbounded number of sockets.
Cancellation drops the affected connection so PostgreSQL stops the active
operation; later calls acquire a healthy connection. A cancelled or lost read
is retryable where the failure proves no unsafe write. A write whose terminal
outcome cannot be proven is marked ambiguous and fails closed.

PostgreSQL raw SQL uses `$1`, `$2`, and so on. Prepared parameter types are
inferred by PostgreSQL, so ambiguous expressions should include a cast such as
`$1::INT4`. CRUD helpers generate the correct placeholders automatically.
PostgreSQL mutation results always report `lastInsertId: null`; Database v1 has
no portable `RETURNING` operation.
