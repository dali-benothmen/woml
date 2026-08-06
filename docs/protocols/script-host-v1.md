# WOML Script Host Protocol v1

Status: frozen for the Rust `hello.woml` integration slice

This document is the normative transport and lifecycle contract between the
Rust workflow core and the long-lived Bun script host. The accompanying JSON
Schema is normative for individual message shapes.

## 1. Scope

Protocol v1 executes the compiled handler `runtime.script` only. Rust owns run
state and scheduling. The Bun host owns JavaScript Worker creation and reports
one terminal outcome for each accepted invocation.

The protocol does not expose WOML syntax, XML, services, secrets,
`context.run`, `context.env`, typed references, retry policy, branch semantics,
approval, or engine controls.

## 2. Process Ownership

The Rust core starts one long-lived Bun child process, monitors it, and stops
it. The child reads protocol frames from stdin, writes protocol frames to
stdout, and writes optional diagnostics to stderr.

The host emits one `ready` message after initialization. Rust must validate its
protocol name and version before sending an `execute` message.

Closing stdin requests host shutdown. Rust closes stdin only after it has dealt
with all in-flight invocations. Unexpected EOF or process exit is a host crash.

## 3. Framing

Each message is encoded as UTF-8 JSON and preceded by exactly one ASCII header:

```text
Content-Length: <decimal UTF-8 byte length>\r\n
\r\n
<JSON bytes>
```

Rules:

1. `Content-Length` counts encoded UTF-8 bytes, not characters or UTF-16 code
   units.
2. Header names and punctuation are case-sensitive in v1.
3. The length is a base-10 non-negative integer with no sign.
4. A decoder must handle a header or body split across arbitrary reads.
5. A decoder must handle multiple complete frames in one read.
6. JSON whitespace outside the framed body is forbidden.
7. stdout contains frames only. Unframed stdout is a protocol violation.
8. A malformed or oversized frame may force the receiver to close the channel
   when a safe framed error cannot be produced.

The absolute frame-size limit is configurable and its default value is not
frozen by protocol v1.

## 4. Message Envelope

Every message contains:

- `protocol`: exactly `woml.script-host`.
- `protocolVersion`: exactly `1`.
- `messageType`: one of the v1 message types.

Execution messages also contain `invocationId`. It is unique among all
invocations active in one host instance and is the sole request/response
correlation key.

The schema identifier is:

```text
https://cronflow.dev/schemas/script-host-protocol/v1
```

## 5. Message Types

### 5.1 `ready`

Direction: Bun host to Rust.

The host sends `ready` once after it can accept execution requests. Its
`hostInstanceId` is unique for that process lifetime and is useful for crash
diagnostics; it is not workflow state.

### 5.2 `execute`

Direction: Rust to Bun host.

Required execution data:

- `invocationId`: correlation identity.
- `runId`: internal run identity; not a public `context.run` contract.
- `nodeId`: compiled DAG node identity.
- `attempt`: one-based attempt number.
- `handler`: exactly `runtime.script` in v1.
- `timeoutMs`: required positive execution deadline.
- `source`: JavaScript source body.
- `context`: strict JSON containing only `trigger` and `steps`.

The host validates the request before accepting it. An accepted request creates
one fresh Worker and eventually produces one `completed` response unless the
host connection is lost.

### 5.3 `completed`

Direction: Bun host to Rust.

A completed message contains either:

- `outcome.kind: success` and a strict-JSON `value`, or
- `outcome.kind: failure` and one host-reportable canonical attempt failure.

`durationMs` is the non-negative elapsed host time. It is diagnostic data and
does not determine event order.

## 6. Asynchronous Multiplexing

Protocol v1 is asynchronous and multiplexed from its first implementation.

Rust may send several `execute` messages without waiting for earlier terminal
responses. The host may run or queue them according to its configured
concurrency. Responses may arrive in any order and correlate only through
`invocationId`.

```text
Rust → execute inv_01
Rust → execute inv_02
Rust → execute inv_03

Bun  → completed inv_02
Bun  → completed inv_01
Bun  → completed inv_03
```

Neither side may infer DAG order, event order, or completion order from stream
position. One writer must serialize frames so bytes from concurrent responses
never interleave.

Multiplexing is transport capability only. It does not make WOML `<parallel>`
executable in the first Rust slice.

## 7. Isolation and Timeout

The host creates a new Bun Worker for every accepted invocation. Context is
cloned into the Worker and deeply frozen before user source runs. The Worker is
terminated after its terminal result.

When `timeoutMs` expires, the host terminates only that Worker and reports
`script_timed_out`. If Worker termination cannot be enforced or the entire host
becomes unhealthy, Rust terminates the host and treats all its in-flight
invocations as `host_crashed`.

Worker isolation prevents accidental JavaScript global-state reuse. It is not
an operating-system security sandbox for hostile code.

## 8. Canonical Failure Taxonomy

The shared schema identifier is:

```text
https://cronflow.dev/schemas/attempt-failure/v1
```

The canonical kinds are:

| Kind | Stable code | Producer |
|---|---|---|
| `script_threw` | `WOML_SCRIPT_THROWN` | Bun host |
| `script_timed_out` | `WOML_SCRIPT_TIMEOUT` | Bun host |
| `invalid_script_result` | `WOML_SCRIPT_NON_JSON_RESULT` | Bun host |
| `context_too_large` | `WOML_SCRIPT_CONTEXT_TOO_LARGE` | Rust or Bun host |
| `result_too_large` | `WOML_SCRIPT_RESULT_TOO_LARGE` | Bun host |
| `worker_crashed` | `WOML_SCRIPT_WORKER_CRASHED` | Bun host |
| `host_crashed` | `WOML_SCRIPT_HOST_CRASHED` | Rust supervisor |
| `interrupted` | `WOML_STEP_INTERRUPTED` | Rust recovery |

The script-host response permits the first six kinds. A dead host cannot report
its own crash. Rust synthesizes `host_crashed`. Recovery synthesizes
`interrupted` when a started attempt has no terminal event and the cause cannot
be proven.

Failures are never collapsed into one generic script error.

## 9. Size-Limit Failures

Protocol v1 freezes two separate mechanisms:

- `context_too_large`: invocation context exceeds the configured limit.
- `result_too_large`: script output exceeds the configured limit.

Failure details may include `actualBytes` and `limitBytes`. Their presence is
optional because a receiver may stop processing before it can safely determine
an exact value.

Rust measures serialized context before dispatch. Bun validates it again. Bun
validates and measures a result before including that result in a response.

The default context, result, and absolute frame limits remain configuration
decisions and are not frozen by this protocol version.

## 10. Host Crash and Replay

Rust detects host exit, EOF, or a broken pipe. Every invocation still in flight
for that host instance fails as `host_crashed`. Rust may start a new host with
bounded backoff for future work, but it must not automatically replay an
affected attempt.

On durable recovery, a `step_attempt_started` event without a matching terminal
attempt event fails as `interrupted`. It is not replayed automatically.

This is a fail-closed policy and makes no exactly-once side-effect guarantee.

## 11. Context and Results

The first slice sends the full available strict-JSON context with every
invocation. This is intentionally simple and may become expensive as prior
outputs grow. Selective context is not inferred from arbitrary JavaScript.

Non-JSON values such as `undefined`, functions, symbols, `BigInt`, non-finite
numbers, sparse arrays, non-plain objects, and circular structures are invalid
results and must not cross the boundary as successes.

Resolved secrets must never appear in source inputs, context, errors, results,
or protocol diagnostics.

## 12. Conformance

Both the Bun and Rust implementations must pass the same reviewed fixtures.
Conformance includes:

- Every valid message fixture validates against the protocol schema.
- Invalid versions, envelopes, outcomes, and failure code/kind pairs fail.
- UTF-8 byte length is correct for emoji and non-Latin text.
- CRLF inside source and context survives round trip.
- One frame may arrive in many chunks.
- Many frames may arrive in one chunk.
- Concurrent invocations may complete out of order without miscorrelation.

Any incompatible change to framing, correlation, message shape, or failure
shape requires a new protocol version.
