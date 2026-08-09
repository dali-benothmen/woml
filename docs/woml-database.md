# WOML Database Service

SC7 provides a zero-setup, Rust-managed SQLite database inside `<script>`:

```js
const db = services.db({
  driver: "sqlite",
  connection: "./.woml/customers.sqlite"
});

const customers = await db.query({
  text: "SELECT id, name FROM customers WHERE active = ?",
  values: [true]
});
```

`services.db()` returns a read-only database handle with `query`, `execute`,
`read`, `insert`, `update`, `delete`, and `transaction`. Bun provides the
JavaScript facade, while Rust owns the connection, SQL execution, limits,
cancellation, result conversion, operation events, and recovery policy.

## Configuration and ownership

Database v1 accepts exactly `driver` and `connection`. SC7 supports only
`driver: "sqlite"`; PostgreSQL is the next database milestone. `connection`
must be a filesystem path. In-memory databases, SQLite URI strings,
directories, and the SQLite file used for WOML runtime state are rejected.

The workflow database is application data owned by the workflow author. It is
separate from `.woml/state.sqlite`, which remains private engine storage. WOML
does not create a missing parent directory, so the path's directory must exist.
A write operation can create the SQLite file; a read opens an existing file
read-only and never creates storage as a hidden side effect.

## Query, execute, and CRUD

Use `query` for one read-only SQL statement and `execute` for one mutation or
schema statement:

```js
const selected = await db.query({
  text: "SELECT name FROM customers WHERE id = ?",
  values: [customerId]
});

const changed = await db.execute({
  text: "UPDATE customers SET active = ? WHERE id = ?",
  values: [false, customerId]
}, { name: "deactivate-customer" });
```

Never interpolate untrusted values into SQL text. Positional `?` parameters
accept `null`, booleans, JavaScript-safe numbers, strings, and binary values as
`{ bytesBase64: "..." }`.

The portable CRUD helpers quote strict ASCII identifiers and parameterize every
value:

```js
await db.insert({
  table: "customers",
  values: { name: "Ada", active: true }
}, { name: "create-customer" });

await db.read({
  table: "customers",
  columns: ["id", "name"],
  where: { active: true },
  orderBy: [{ column: "id", direction: "asc" }],
  limit: 100
});

await db.update({
  table: "customers",
  values: { active: false },
  where: { id: 42 }
}, { name: "deactivate-customer" });

await db.delete({
  table: "customers",
  where: { id: 42 }
}, { name: "delete-customer" });
```

`update` and `delete` require a non-empty `where` object. This prevents an
accidental whole-table write through the convenience API; an intentional bulk
operation must be explicit SQL passed to `execute`.

Queries and reads return `{ rows, rowCount }`. Mutations return
`{ rowsAffected, lastInsertId }`, where `lastInsertId` is `null` when the
statement is not an insert.

## Atomic transactions

Database v1 uses an explicit batch rather than a JavaScript callback:

```js
const batch = await db.transaction({
  operations: [
    { operation: "insert", table: "orders", values: order },
    {
      operation: "execute",
      text: "UPDATE inventory SET quantity = quantity - ? WHERE sku = ?",
      values: [order.quantity, order.sku]
    },
    {
      operation: "query",
      text: "SELECT quantity FROM inventory WHERE sku = ?",
      values: [order.sku]
    }
  ]
}, { name: "create-order" });
```

Rust runs 1–100 operations on one connection and commits only when every
operation succeeds. Any failure rolls back the full batch. Nested transactions
and CRUD `read` inside a transaction are not part of Database v1; use a
parameterized `query` in the batch.

## Identity, failures, and recovery

Reads may be repeated and multiplexed. An effectful method may be called once
with automatic identity. When the same effectful method can run more than once
inside one step, give each call a stable second argument such as
`{ name: "write-audit-row" }`. Names must describe the logical operation and
must not depend on loop order or attempt number.

Failures throw `WomlServiceError` with stable fields including `code`,
`service`, `operation`, `retryable`, and `ambiguous`. Constraints, busy/locked
connections, invalid input, cancellation, unsafe results, and size limits stay
distinct. An interrupted write is marked ambiguous and is not replayed
automatically. Transaction atomicity prevents a normal failed batch from
committing a partial result, but it does not turn an externally interrupted
commit into proof of exactly-once execution.

## Limits and durable data

Database v1 bounds SQL text to 256 KiB, results to 10,000 rows and 256 columns,
transactions to 100 operations, capability input to 1 MiB, and capability
result data to 4 MiB. Integers outside JavaScript's safe range and invalid UTF-8
text are rejected; blobs are returned as `bytesBase64`.

Durable operation events contain only the driver, operation identity, timing,
safe row counts, and safe failure data. Connection paths, SQL, parameters,
returned rows, and credentials are not operation metadata. A script may still
deliberately return selected database data as its step result, which follows
the normal bounded workflow-context rules.

## Run the example

From the project root after building the CLI:

```bash
woml run examples/sqliteWorkflow.woml
```

The first run creates `.woml/customers.sqlite`; each later run increments Ada's
visit count. The workflow remains portable to the PostgreSQL milestone only for
the parts explicitly promised by Database v1—SQLite-specific raw SQL remains
SQLite-specific.
