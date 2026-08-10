# WOML Outbound HTTP

WOML supports two outbound HTTP paths inside `<script>`. Both execute ordinary
HTTP requests, but they make different product promises:

```js
const nativeResponse = await fetch(url);

const managedResponse = await services.http.request({
  url,
  method: "POST",
  headers: { authorization: `Bearer ${secrets.API_TOKEN}` },
  json: { customerId: "customer-42" },
  timeout: "10s",
  idempotency: {
    header: "Idempotency-Key",
    value: attempt.idempotencyKey
  }
}, { name: "create-customer" });
```

## Choosing the HTTP path

Native `fetch()` is Bun's Fetch implementation. It returns a native `Response`,
supports streams and the familiar web API, and does not throw for non-2xx
statuses. WOML records a redacted start and terminal observation, but Bun owns
the actual request and response body.

`services.http.request()` is the managed option. Rust owns connection pooling,
TLS validation, redirects, decompression, timeouts, cancellation, response
limits, status policy, and durable operation events. It returns exactly:

```js
{ status, ok, headers, data, url, redirected }
```

Managed HTTP defaults to `GET`, JSON response parsing, a 30-second timeout,
accepted statuses 200–299, and at most 10 followed redirects. Request bodies
use one of `json`, `text`, or `bytesBase64`; byte responses are returned as a
Base64 string. `ok` always means native HTTP 2xx, even when `acceptedStatus`
allows another range.

Multiple reads may run concurrently. One effectful call may use automatic
identity. If a step can make several writes, give every call a stable second
argument such as `{ name: "create-customer" }`. This name is part of the
logical operation identity and must not depend on loop order or an attempt
number.

For a large download, `responseType: "storage"` and a `storage: { key, ... }`
target stream the response directly from Rust into durable object storage. The
returned `data` is a portable object reference rather than the body, so the
body never crosses Bun or context. See `docs/woml-storage.md` for overwrite,
conditional version, content-type, integrity, and size behavior.

## Failure and retry behavior

A managed failure throws `WomlServiceError`. Its stable fields are `code`,
`service`, `operation`, `callId`, `retryable`, `ambiguous`, and safe `details`.
Non-accepted statuses, invalid JSON, timeout, cancellation, transport failure,
and result limits remain distinct.

Rust records `operation_started` before dispatch and a terminal operation event
before Bun receives the result. A recovered start without a terminal event is
ambiguous and fails closed. Read requests and writes carrying a reviewed
external idempotency key may be retryable when the failure classification makes
that safe. WOML never treats an arbitrary POST as exactly once.

## Durable and secret boundaries

Capability envelopes carry request data in memory, but durable operation
metadata contains only safe fields such as method, sanitized origin, status,
duration, byte count, and digest. Query values, request and response bodies,
credentials, authorization/cookie headers, and external idempotency values do
not enter operation metadata. A script can still deliberately return response
data as its step result; authored return values follow the normal bounded
context rules.

Use only literal `secrets.NAME` reads. The compiled model stores the symbolic
name, while the CLI resolves and injects its value for that invocation.

## SSRF and network policy

The current local profile intentionally permits any valid `http:` or `https:`
destination reachable from the WOML process, including loopback, private
addresses, and redirects to another network. This is useful for local APIs but
means an untrusted URL can become a server-side request-forgery path.

For local development:

- run only trusted workflow definitions and trigger payloads;
- do not concatenate untrusted values into a destination URL without an
  explicit allowlist;
- prefer a fixed origin and vary only validated path or query values; and
- use `redirect: "error"` when a request must never change destinations.

For hosted or multi-tenant deployment, application validation is not enough.
Run WOML behind an egress firewall or proxy that denies loopback, link-local,
private, metadata-service, and internal DNS destinations unless explicitly
allowed. Apply policy again after DNS resolution and on every redirect to
address DNS rebinding and redirect pivots. Restrict schemes to HTTP(S), bound
response sizes and timeouts, and isolate tenants at the process/network layer.

WOML does not yet ship a hosted network-policy engine. A future hosted profile
must be deny-by-default and version its allow/deny behavior before accepting
untrusted destination URLs. The local profile must not be described as an SSRF
sandbox.

## Deployment checklist

1. Use a persistent state path and a process supervisor.
2. Store credentials with `woml secrets set`, never in `.woml` source.
3. Restrict outbound network access to required origins.
4. Choose explicit timeouts, status ranges, and redirect policy.
5. Give every repeated write a stable operation name and provider-supported
   idempotency value.
6. Monitor safe service failure codes and ambiguous interruptions.
7. Run the SC6 release gate and the local benchmark on deployment-class
   hardware before publishing performance expectations.

## Benchmarking

From `woml-cli`, after `bun run build`:

```bash
bun run benchmark:http
```

The benchmark uses one local loopback server and measures inside one script
attempt, excluding CLI/process startup. It reports sequential latency and
concurrent throughput for both paths. Results are environment-specific: native
Fetch has less bookkeeping, while managed HTTP deliberately pays for the
Rust/Bun round trip and two durable event appends. SC6 makes no claim that the
managed path is universally faster; its value is supervision and durability.
