# Managed HTTP v1 — SC9 Storage Activation

Status: frozen on 2026-08-10 by SC9.

SC9 activates the reserved direct-to-storage response mode in Managed HTTP v1:

```json
{
  "responseType": "storage",
  "storage": {
    "key": "imports/export.csv",
    "contentType": "text/csv",
    "overwrite": true
  }
}
```

`storage.key` is required. `contentType`, `overwrite`, and `ifVersion` follow
Storage v1; `overwrite` and `ifVersion` are mutually exclusive. The `storage`
field is forbidden for every other response type, and storage mode is invalid
without it.

Rust validates the HTTP request and storage target before dispatch. After an
accepted response, Rust streams decoded response bytes into an unpublished
Storage v1 upload, computes its checksum/version, and atomically publishes the
object only after the full response succeeds. The Managed HTTP result keeps its
existing envelope and returns the Storage v1 object reference in `data`.
The combined operation is classified as an idempotent write even when its HTTP
method is `GET`, because publishing the local object changes durable storage.

The response body does not cross the Script Host boundary, enter workflow
context, or appear in operation events. HTTP safe result metadata remains the
status; storage paths and bodies remain private. Cancellation, response failure,
storage conflict, corruption, or size failure leaves no visible partial object.

This activation does not create Managed HTTP v2, change JSON/text/bytes
behavior, or activate an external object-store adapter.
