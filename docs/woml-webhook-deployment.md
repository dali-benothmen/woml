# Deploying a WOML Webhook Runtime

Webhook is one of WOML's supported Production Triggers. This guide focuses on
its HTTP boundary; the complete mixed-trigger operations guide is
[Operating WOML Production Triggers](woml-production-triggers.md). `woml run`
is the long-lived process: a supervisor starts it once, keeps it alive, sends
SIGTERM during deployment, and restarts it after exit.

Inbound webhook policy and outbound workflow HTTP are separate trust
boundaries. If a webhook payload can influence an outbound destination, apply
the allowlist and network-layer egress controls in
[WOML Outbound HTTP](woml-http-services.md); webhook authentication alone does
not prevent SSRF.

```bash
woml run workflows/ \
  --host 127.0.0.1 \
  --port 3000 \
  --state /var/lib/woml/state.sqlite
```

## Network boundary

The built-in listener serves HTTP. Put it behind a trusted TLS reverse proxy or
private ingress for internet-facing deployments. Keep the default loopback
binding when the proxy runs on the same machine. Binding a public interface is
an explicit operator decision.

Use `auth="bearer"` for protected routes and store the referenced value with
`woml secrets set NAME` or the deployment secret provider. `auth="none"` is
appropriate for local development or when a trusted ingress performs and
strips authentication; WOML prints a warning for every unauthenticated route.
Raw bearer values are resolved in memory, reduced to fixed-width digests during
registration, and never written to workflow context, events, progress, or the
state database.

## Delivery behavior

An accepted request receives HTTP 202 and a durable run ID. It does not wait for
the workflow result. Callers may omit `Idempotency-Key` to create a new
occurrence every time, or provide a stable key when retrying one logical
delivery. Reusing a key with the same payload returns the original run; reusing
it with a changed payload returns HTTP 409.

The payload must be a JSON object and is limited to 1 MiB. Define an inline
`<schema>` so invalid input is rejected before a run is created. Error responses
and terminal progress contain bounded diagnostics, never the rejected body.

## State and operations

The SQLite file is the durable execution authority. Store it on persistent
local storage, restrict its filesystem permissions, back it up consistently
with its WAL files, and never share one database over a network filesystem.
WOML waits briefly for ordinary SQLite writer contention; if durability is
unavailable, webhook admission returns HTTP 503 instead of acknowledging work
that was not committed.

Readiness, accepted occurrences, run IDs, failures, and results are emitted by
the process. Retrieve a run later with:

```bash
woml get run_... --state /var/lib/woml/state.sqlite
```

SIGINT and SIGTERM stop new admission and shut down the listener gracefully.
An occurrence committed before a process crash is recovered from the same run
identity at startup. A started effect with no terminal record remains
ambiguous and fails closed; WOML never guesses that it is safe to replay.

Use `woml list --state /var/lib/woml/state.sqlite` for bounded local discovery,
`woml get <runId> --json` for redacted lifecycle/cancellation state, and
`woml cancel <runId>` to request durable cancellation. These are local
state-file controls, not authenticated remote administration endpoints. Lock
down the state directory and see
[Lifecycle and Local Run Control](woml-lifecycle-and-run-control.md) before
placing cancellation behind an operator tool.

## Release gate

Run `bun run test:t13` from `woml-cli` before publishing. It rebuilds the native
package, runs frontend, Rust, and CLI suites, exercises the webhook example,
checks concurrency and failure boundaries, and scans public/durable artifacts
for configured WOML secrets.

When the deployment uses lifecycle hooks or local run control, also run
`bun run test:lec8`. That gate adds cancellation/recovery races, notification
separation, clean installation, schema compatibility, package auditing, and
the lifecycle/run-control performance budgets.

For workflows with `<config>`, queue admission may return HTTP 503 with
`Retry-After: 1` and `WOML_POLICY_QUEUE_FULL`; callers should retry using the
same idempotency key and payload. Run `bun run test:rp7` before publishing that
runtime and follow [WOML Runtime Policies](woml-runtime-policies.md).
