# WOML Services and Capabilities Contracts v1

Status: frozen by SC0 on 2026-08-09 and hardened for independent publication
by SC6 on 2026-08-10. Native Fetch observation shipped in SC4; Managed HTTP v1
became executable in SC5.

## Public boundary

WOML-owned capabilities use the reserved `services` binding. Script Bindings
v1 is exactly `context`, `attempt`, `services`, and `secrets`. `context.run`,
module imports, service tags, and engine-control APIs remain unavailable.

The frontend discovers only literal `secrets.NAME` reads. It records sorted
symbolic names in Model v8. The runtime resolves and injects only those values
into one script invocation. Values never enter a compiled model, event,
progress record, diagnostic, or fixture.

## Generic calls

Capability Call v1 is the only hot-path envelope for managed built-ins. Calls
and replies correlate by `{invocationId, callId}`. Multiple invocations and
multiple calls within one invocation may be active, and replies may arrive in
any order.

An operation has three identities:

- `callId`: unique correlation for this call attempt;
- `operationKey`: stable logical-effect identity used by WOML-owned handlers;
- optional `providerIdempotencyKey`: sent outside WOML only when a reviewed
  provider contract supports it.

Rust derives `operationKey` as SHA-256 over these exact UTF-8 bytes:

```text
woml.capability-operation\0v1\0{stepIdempotencyKey}\0{operationName}
```

The serialized key is `sha256:` followed by the lowercase hexadecimal digest.
The durable authority independently verifies the request's step identity
against the active attempt before it records or dispatches the operation.

Automatic operation identity is allowed for one effectful call to a given
capability operation in a step. A script that may perform multiple effectful
calls must give them stable logical names. Read-only calls may be multiplexed.
Managed HTTP freezes the author-facing naming shape as the optional second
argument `services.http.request(request, { name: "stable-name" })`. WOML
prefixes that name with `http.request.` before deriving the operation key.

Default v1 limits are 1 MiB input, 4 MiB result, 8 MiB protocol frame, 30
seconds, 32 calls in flight per invocation, and 256 calls in flight across one
host. Both peers enforce them. The failure enum already reserves input, result,
and frame size failures, so changing a configured number is not a protocol
version change.

`handler_crashed`, `worker_crashed`, and `host_crashed` remain distinct.
`interrupted` is a capability failure kind. `ambiguous` is also a boolean axis:
it tells the retry engine whether an external effect may have happened. A host
crash is therefore represented once, for example `kind: host_crashed` with
`ambiguous: true`; it is not duplicated as a second failure taxonomy.

If a managed error escapes user JavaScript, Attempt Failure v3 carries
`kind: service_failed` and the same Capability Call v1 failure as its `cause`.
Script Host v4 and Run Event v8 use that shape. Older attempt failures remain
unchanged, and the service classification is never flattened to
`script_threw`.

## Cancellation and recovery

Rust records `operation_started` before dispatching a managed effect, then a
terminal operation event before releasing the result to Bun. Cancelling an
invocation cancels its active calls where supported and returns a correlated
cancelled result. Cancellation does not claim rollback.

A recovered managed start without a terminal event is fail-closed and
ambiguous unless that capability's separately versioned recovery contract can
prove the outcome. WOML does not replay an arbitrary effect merely because its
script attempt was interrupted.

## Native Fetch

WOML captures Bun's native `fetch`, observes it, and returns Bun's native
`Response` unchanged. It does not rebuild Fetch, parse bodies automatically,
throw for non-2xx, or retry. The request cannot be dispatched until Rust
durably acknowledges its redacted start observation.

Only method, sanitized origin/path, status, timing, and bounded known byte
counts may be observed. Query strings, headers, credentials, and bodies never
cross the observation boundary. Fetch completion means response headers were
received; WOML does not consume or replace the response stream.

## Managed HTTP v1

`services.http.request()` is Rust-managed. The JavaScript facade supplies the
documented defaults and lowers to Managed HTTP v1. `json`, `text`, and
`bytesBase64` request bodies are mutually exclusive. Results are exactly:

```js
{ status, ok, headers, data, url, redirected }
```

The default method is `GET`; accepted status is 200-299; response type is
`json`; redirect mode is `follow` with at most 10 redirects; timeout is 30
seconds. `ok` retains the familiar 200-299 meaning even when an accepted-status
override permits another response. A non-accepted response rejects with a
`service_rejected` WOML service error. JSON parse errors are `invalid_result`.
TLS validation and compression use the Rust client's secure defaults.
Cancellation is best effort and its terminal classification remains explicit.

The optional external idempotency shape is
`idempotency: { header: "Idempotency-Key", value: "..." }`. WOML verifies that
the same value is present in Capability Call identity before Rust dispatches
the request; the idempotency header must not also be supplied in `headers`.
Byte request bodies and byte response data are standard Base64 strings.

Retry classification is conservative: reads and a write carrying a reviewed
external idempotency key can be retryable for failures known to occur before a
response; unsafe or ambiguous writes are not automatically replayed.

## Safe diagnostics

Service Progress v1 is ephemeral. Run Event v8 is durable. Both use bounded,
scalar safe metadata. They never carry inputs, outputs, connection strings,
authorization/cookie headers, SQL text/parameters, event payloads, queue
payloads, cache values, storage bodies, or arbitrary provider responses.

## Deferred decisions

SC0 does not resolve `context.run`, imported JavaScript/WOML modules, service
tags, durable user state, workflow cancellation APIs, database/storage/cache/
event/queue method contracts, or multi-tenant OS isolation. Those remain
separate, versioned milestones.
