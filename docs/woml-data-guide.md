# Choosing Where Workflow Data Lives

WOML provides four different tools because temporary acceleration, small
workflow memory, files, and queryable business records have different
reliability needs. Choose based on what the data means, not only its current
size.

| Question | Use | Why |
| --- | --- | --- |
| Can the workflow still be correct if this disappears? | `services.cache` | Expiring, evictable optimization data. |
| Is this a small JSON fact owned by one workflow across runs? | `services.state` | Durable, versioned, retry-safe workflow memory. |
| Is this a file, document, export, image, or larger payload? | `services.storage` | Durable object bytes with checksums and portable references. |
| Must records be filtered, joined, indexed, reported on, or shared by applications? | `services.db()` | Author-owned SQLite/PostgreSQL application database. |

## Practical examples

- Cache a customer API response for 15 minutes: **cache**.
- Remember an agent conversation's turn count: **state**.
- Count processed jobs correctly across concurrent runs: **state** with
  `increment`, or a database when other applications also own the counter.
- Save a generated PDF or a 20 MiB export: **storage**.
- Store customers, invoices, inventory, or analytics rows: **database**.

Do not put a correctness-critical counter in cache. Do not put large documents
or an unbounded conversation transcript in state. Do not use storage when the
workflow needs to query fields across thousands of records. Do not introduce a
database merely to remember one small workflow-owned cursor.

## Durability and ownership

| Property | Cache | State | Storage | Database |
| --- | --- | --- | --- | --- |
| May expire or evict | Yes | No | No | No |
| Workflow scoped automatically | Yes | Yes | No; object keys are explicit | No; schema/connection are explicit |
| Small JSON values | Yes | Yes | Supported, but usually unnecessary | Yes |
| Large values/files | No | No | Yes | Database-dependent |
| Query/filter/join | No | No | No | Yes |
| Optimistic version checks | No correctness guarantee | `ifVersion` | `ifVersion` | SQL transaction/application design |
| Named retry-safe mutation | No | Yes | Operation-specific | Operation/provider-specific |
| Normal run inspection exposes values | No | No | No object bodies | No rows |

State and cache use workflow ID plus the selected WOML state location for
scope. Storage and database are explicit application resources. None of these
services is injected into `context`; scripts receive only the result they
explicitly request and return.

## When state has outgrown State v1

Move to storage or a database when any of these becomes true:

- one value approaches the 256 KiB limit;
- the workflow approaches 10,000 keys or 64 MiB;
- another workflow or application must own/query the same records;
- data needs indexing, filtering, transactions across several records, or
  independent retention policy; or
- operators need administrative export/search tooling.

There is no automatic migration or fallback between these services. Make the
move explicitly so the new ownership and reliability contract is reviewable.

## Security summary

Run events and ordinary CLI inspection contain bounded metadata, not state
values, cache values, storage bodies, database rows, or credentials. The local
files still contain application data and must be protected and backed up. See
[Durable User State Operations](woml-durable-state.md) and
[Local Data Security](woml-data-security.md).
