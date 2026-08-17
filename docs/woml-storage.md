# WOML Durable Object Storage

`services.storage` is a Rust-owned local object store for
files and larger durable values that should not be copied into every workflow
event or context projection.

```js
const object = await services.storage.put({
  key: "reports/daily.json",
  value: { customers: 42 }
});

const saved = await services.storage.get({
  key: object.key,
  responseType: "json",
  ifVersion: object.version
});
```

## Operations

- `put({ key, value | text | bytesBase64, contentType?, overwrite?, ifVersion? })`
  atomically saves one object and returns its reference.
- `get({ key, responseType?, ifVersion? })` verifies the checksum and returns
  `{ object, data }`. The default response type is `json`; `text` and `bytes`
  are also available.
- `head({ key })` returns the reference or `null` without returning the body.
- `list({ prefix?, limit?, cursor? })` returns key-ordered references. The
  default limit is 100 and the maximum is 1,000.
- `delete({ key, ifVersion? })` removes one object. Deleting an already absent
  key without a condition safely returns `{ deleted: false, object: null }`.

An ordinary put is create-only. Use `overwrite: true` when unconditional
replacement is intended, or `ifVersion` for an optimistic conditional update.
Those two options cannot be combined. Repeating the same create-only write with
identical content converges on the same reference.

## Object references and durable data

Each successful write returns a portable JSON reference:

```json
{
  "contract": "woml.storage-object",
  "contractVersion": 1,
  "key": "reports/daily.json",
  "version": "v1:<sha256>",
  "checksum": { "algorithm": "sha256", "value": "<sha256>" },
  "size": 1234,
  "contentType": "application/json"
}
```

The object directory is `objects-v1` beside the selected WOML state database.
Back up the state database and this directory together. Keys are logical UTF-8
names, not filesystem paths; absolute paths, traversal segments, backslashes,
empty segments, NULs, and control characters are rejected.

Operation events may contain the logical key and safe reference metadata, but
never the object body, storage filesystem path, or temporary filename. If a
script explicitly returns loaded data, that bounded return value follows the
normal step-output and context rules.

## Large HTTP downloads

Managed HTTP can stream a response directly from Rust into storage:

```js
const response = await services.http.request({
  url: "https://files.example.com/export.csv",
  responseType: "storage",
  storage: {
    key: "imports/export.csv",
    overwrite: true
  }
});

return response.data; // the object reference, not the response body
```

This path does not copy the response body through the Bun Worker or workflow
context. `storage.contentType` may override the response Content-Type; without
either, WOML uses `application/octet-stream`. The local object maximum is 64
MiB. Ordinary service inputs and returned bodies remain subject to the smaller
Capability Call limits, so use direct-to-storage for larger downloads.

## Integrity, recovery, and failures

Local writes use a private temporary container, checksum it while writing,
sync it, acquire a process-safe lock, and atomically publish it. Partial uploads
are invisible and removed when their operation ends. Every `get` verifies the
stored SHA-256 checksum; corruption fails closed.

Stable failure codes distinguish invalid input, missing objects, conditional
conflicts, unsafe paths, corruption, size limits, cancellation, and unavailable
storage. Interrupted managed operations follow the existing fail-closed WOML
recovery rule. Object bodies and body-derived text do not enter diagnostics.

## Storage or durable state?

Use `services.state` for small workflow-owned JSON facts that need versions,
compare-and-set, counters, or retry-safe named mutations. Use storage for files,
larger documents, portable object references, and data that should not pass
through every script invocation. See
[Choosing Where Workflow Data Lives](woml-data-guide.md).

## Run the example

From the project root after building the CLI:

```bash
woml test examples/storageWorkflow.woml
```

The workflow stores the object under the `objects-v1` directory next to the
chosen state database and prints a small JSON summary.
