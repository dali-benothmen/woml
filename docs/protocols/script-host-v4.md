# WOML Script Host Protocol v4

Status: frozen by SC0 on 2026-08-09. Protocol v3 remains immutable.

Protocol v4 is a full-duplex, long-lived Rust/Bun boundary. One Bun host serves
multiple isolated invocations. Rust may send several `execute` messages without
waiting. A script may issue several nested capability calls. Completed scripts,
capability results, and Fetch acknowledgements may arrive out of order and are
matched only by their explicit IDs.

## Frames

Every message is UTF-8 JSON framed as:

```text
Content-Length: <number of UTF-8 bytes>\r\n
\r\n
<exact JSON bytes>
```

The maximum v1 frame is 8 MiB. Byte length is never JavaScript string length.
Literal CRLF inside a JSON string is escaped and cannot terminate the header.
The reviewed fixture includes multibyte text and a literal CRLF value.

## Message directions

Rust to Bun:

- `execute`
- `cancel`
- `capability_result`
- `fetch_observation_ack`

Bun to Rust:

- `ready`
- `completed`
- `capability_call`
- `fetch_observation`

`execute.bindings` contains Script Bindings v1 metadata and only the resolved
secret values required by that node. This message is memory-only. It must never
be logged, persisted, or used as a fixture with nonempty secret values.

Every capability wrapper repeats `invocationId` and `callId`; they must equal
the IDs inside its Capability Call v1 payload. Every Fetch wrapper repeats
`invocationId` and `requestId`; they must equal the observation payload. A
mismatch is a protocol violation and fails the affected invocation closed.

Rust acknowledges a native-Fetch `started` observation only after its durable
`operation_started` append. A negative or missing acknowledgement prevents the
network dispatch. Terminal Fetch observations are also acknowledged so the
host knows whether tracking remained trustworthy.

An uncaught managed call rejection is reported as Attempt Failure v3
`service_failed`, retaining the capability, operation, call ID, retryability,
ambiguity, and original Capability Call v1 cause. It is not rewritten as an
ordinary JavaScript throw.

## Isolation and failure

Each invocation runs in an isolated worker context inside the long-lived Bun
host. Timeout/cancellation terminates that worker and its outstanding calls.
One worker failure is `worker_crashed`; losing the shared process is
`host_crashed`. An in-flight managed operation becomes ambiguous unless its
handler can prove a terminal outcome. Existing script failure shapes remain
those frozen in Attempt Failure v2.
