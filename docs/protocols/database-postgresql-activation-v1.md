# Database v1 PostgreSQL Driver Activation

Status: frozen on 2026-08-10 by SC8.

This artifact activates the previously reserved `postgres` driver in Database
v1. It does not create Database v2 and does not change the JavaScript methods,
request fields, operation names, result envelopes, effect classifications, or
durable operation-event shape frozen by SC7.

The reviewed Database v1 driver set is now:

```json
{ "driver": "sqlite" }
{ "driver": "postgres" }
```

`connection` remains an opaque string at the JavaScript/Rust boundary. It is a
filesystem path for SQLite and a PostgreSQL URL or libpq-style connection
string for PostgreSQL. It is never safe event metadata.

Raw SQL retains the selected database's native placeholder syntax: SQLite uses
`?`; PostgreSQL uses `$1`, `$2`, and so on. The CRUD helpers remain the portable
surface because they generate the correct prepared parameters for each driver.

PostgreSQL mutation results use `lastInsertId: null`. Database v1 does not
expose generated values from a PostgreSQL `RETURNING` clause because
`execute` rejects row-returning statements and `query` is strictly read-only.
A future reviewed contract must add a returning-write operation rather than
weakening `query`'s effect classification.

No document/NoSQL behavior is activated by this artifact. A document database
gets a separately reviewed method contract instead of imitating SQL.
