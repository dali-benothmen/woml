# WOML Storage v1

Status: frozen for SC9 on 2026-08-10.

Storage v1 is the portable contract behind `services.storage`. The first
backend is a Rust-owned local object directory beside the selected WOML state
database. External object-store adapters remain deferred.

## Operations

- `put({ key, value | text | bytesBase64, contentType?, overwrite?, ifVersion? })`
  atomically writes one object and returns its portable reference. A missing
  overwrite policy means create-only. `overwrite: true` replaces any version;
  `ifVersion` replaces only the named version. Those options are mutually
  exclusive.
- `get({ key, responseType?, ifVersion? })` verifies the stored checksum and
  returns `{ object, data }`. The JavaScript facade defaults `responseType` to
  `json`; bytes are returned as `{ bytesBase64 }`.
- `head({ key })` returns the object reference or `null` without reading the
  body into the script boundary.
- `list({ prefix?, limit?, cursor? })` returns key-ordered references and an
  opaque continuation cursor. Defaults are an empty prefix and limit 100.
- `delete({ key, ifVersion? })` returns `{ deleted, object }`. Deleting an
  absent key without a condition is an idempotent success with `deleted: false`.

## Portable object reference

Every reference contains only:

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

The content-derived version makes an identical repeated write converge on the
same reference. It is not a claim that external adapters must expose provider
ETags. Object bodies never enter operation events; only key, version, size,
content type, counts, and safe failure fields may be recorded.

## Local safety and limits

Keys are normalized UTF-8 logical names, not filesystem paths. They cannot be
absolute, contain empty, `.` or `..` segments, backslashes, NULs, or control
characters. The local backend maps each key to a SHA-256 filename in a private
flat directory, rejects symlinked store/object paths, uses a process-safe lock,
writes a complete temporary container, syncs it, and atomically renames it.

One object is limited to 64 MiB. Ordinary `put` and `get` additionally remain
inside Capability Call v1's input/result limits. Managed HTTP storage mode can
stream a larger response directly from Rust into the local store without
crossing Bun or workflow context.

Checksum or container corruption fails closed. Conditional conflicts, missing
objects, limits, cancellation, unavailable storage, and invalid keys have
distinct safe codes. Filesystem paths, temporary names, object bodies, and
body-derived error text are never public diagnostics.
