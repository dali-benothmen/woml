# WOML Script-Host Protocol v3

Status: frozen and implemented in RI3; this is the production Rust/Bun
protocol version.

The normative schema is
`docs/schemas/script-host-protocol.v3.schema.json`. Protocols v1 and v2 remain
immutable for stored older-model runs.

Protocol v3 preserves the long-lived Bun host, one isolated Worker per
invocation, Content-Length-framed UTF-8 JSON, asynchronous multiplexing,
out-of-order completion, `invocationId` correlation, and per-invocation
cancellation.

The execute message replaces the numeric v2 `attempt` field with immutable
attempt metadata:

```json
{
  "attempt": {
    "number": 2,
    "maxAttempts": 3,
    "idempotencyKey": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  }
}
```

The Bun Worker exposes this object as the read-only JavaScript binding
`attempt`. The existing `context` stays limited to `context.trigger` and
`context.steps`; attempt metadata is not copied into it.

Rust creates a fresh invocation ID for every attempt while preserving the
stable idempotency key. Bun reports outcomes only. Rust owns retry eligibility,
backoff, scheduling, and durable recovery.

Protocol frames count UTF-8 bytes, not JavaScript characters. Conformance tests
must include multibyte source/context content and literal CRLF text.
