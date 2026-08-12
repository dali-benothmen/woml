# WOML Runtime Observability

PRO5 provides the safe local observability layer used by the future
`woml top`. Workflow events and the SQLite store remain execution truth;
telemetry is a bounded projection and can be discarded or re-created.

## What operators can observe

Runtime Operations Snapshot v1 combines the durable recent-run projection with
live runtime health. It reports:

- runtime lifecycle, readiness, and uptime;
- loaded workflow identities and trigger types;
- bounded recent run status and duration;
- queue, waiting, failure, retry, approval, and Workflow Call indicators;
- store, trigger, provider, and worker health; and
- bounded alerts containing stable codes and safe messages.

Rust also supplies compact payload-free durable totals for run states, trigger
occurrences, scheduled retries, unresolved approvals, and active Workflow
Calls. These survive runtime replacement; only live provider/Worker counters
restart with their owning process.

Snapshots never include workflow payloads, context, step results, durable state
keys or values, URLs, provider messages, secret values, or admin credentials.
Run and workflow IDs remain visible because they are explicit operator
correlation identifiers.

Runtime Operations Stream v1 normalizes existing runtime progress without
creating or changing workflow events. Sequence numbers increase within one
runtime instance. The server retains 1024 updates. A client that asks for a
sequence older than the retained window receives
`WOML_OBSERVABILITY_STREAM_GAP`, closes the stream, fetches a fresh snapshot,
and reconnects from that snapshot's sequence. At most eight stream clients are
retained. A slow client is disconnected instead of applying backpressure to
workflow execution.

## Health and authentication

The Runtime Admin listener remains loopback-only. Minimal probes reveal no
runtime identity or component detail:

- `GET /livez` returns only liveness `ok/unready`;
- `GET /readyz` returns only readiness `ok/unready`.

These two endpoints are intentionally unauthenticated for a local supervisor.
Detailed surfaces require the current rotating capability from the owner-only
runtime descriptor:

- `GET /v1/health` — detailed component health;
- `GET /v1/snapshot` — Runtime Operations Snapshot v1;
- `GET /v1/stream?after=<sequence>` — Runtime Operations Stream v1 over SSE;
- `GET /v1/metrics` — Runtime Metrics v1 JSON records;
- `GET /metrics` — Prometheus text exposition.

Do not expose this listener through the public webhook/event ingress or a
reverse proxy. A monitoring client must reread the descriptor after credential
rotation or runtime replacement and reconnect. Public trigger, approval,
Slack, webhook, and event credentials cannot access these endpoints.

`observability.health` and `observability.metrics` in Runtime Configuration v1
can disable their respective endpoints. Disabled endpoints return 404 rather
than pretending to be enabled.

## Logs and metrics

Runtime Log Record v1 supports text and newline-delimited JSON. JSON records
contain a timestamp, stable level/code, bounded safe message, runtime and
deployment identity, and only reviewed correlation fields. Set:

```json
{
  "schemaVersion": 1,
  "logging": { "format": "json", "level": "info" }
}
```

The Metrics v1 allowlist prevents arbitrary labels. Run IDs, node IDs, URLs,
state keys, arbitrary error messages, payload fields, and secret names or
values are never labels. The Prometheus endpoint uses only those frozen metric
names and labels, including readiness, uptime, workflow/run counts, recent run
duration, trigger/retry/approval/Workflow Call indicators, Worker restarts, and
SQLite store bytes.

Snapshots are capped at 1000 runs, 10000 workflows, 1000 components, and 200
alerts by schema; the current local run projection contributes at most its 200
most recently updated runs. Admin responses are capped at 2 MiB. Authenticated
operations are capped at 16 concurrent requests and 120 operations per minute.

## Failure isolation

Observability cannot decide a workflow business outcome. A failed snapshot,
filesystem-size read, disconnected stream, full client buffer, formatting
failure, or disabled endpoint returns a bounded telemetry failure or drops that
client. Rust workflow execution, trigger admission, durable state, and business
events continue independently.

For hostile code, the process-isolation guidance in
[WOML Local Data Security](woml-data-security.md) still applies. Observability
does not turn Bun Workers into a multi-tenant security sandbox.

## Release gate

Run from `woml-cli`:

```bash
bun run test:pro5
```

The gate composes PRO4 and validates golden snapshots/logs/metrics, schemas,
the packaged runtime, readiness changes, authentication, response and rate
bounds, stream ordering/gaps/backpressure, disabled endpoints, broken-client
isolation, redaction, type checking, and the observability overhead budget.
