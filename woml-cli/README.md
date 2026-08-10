# WOML CLI

The WOML CLI compiles `.woml` workflows, activates their triggers through the
Rust core, and runs embedded JavaScript in isolated Bun Workers.

Every source now uses `<woml>` as its document root, with optional `<imports>`
before the one `<workflow>` definition. Direct `<workflow>` roots are rejected
with a migration hint.

## Check a workflow or local module graph

Validate and inspect a source without activating triggers or executing code:

```bash
woml check path/to/workflow.woml
woml check path/to/workflow.woml --json
```

For imported modules, the JSON form is the deterministic Definition Package
v3 containing Model v9, exact ESM bundles, canonical source maps, and generated
declarations. It is marked `runtimeReady: true`; `woml run` and `woml test`
register those immutable bundles with Rust and expose their named functions at
`services.<alias>` inside isolated Bun Workers.

```bash
woml test examples/moduleWorkflow.woml
```

Generate self-contained editor types for built-in services and local module
aliases:

```bash
woml types examples/moduleWorkflow.woml
```

This writes `woml-env.d.ts` beside the workflow. See
[Authoring Local WOML Modules](../docs/woml-modules.md) for editor setup,
explicit context/secret arguments, diagnostics, and mocked Bun unit tests.

Module artifacts are copied into durable state under their immutable definition
hash. Explicit recovery therefore does not read the current WOML or module
files, and it still works after either file is moved or deleted:

```bash
woml run missing-or-moved.woml --state .woml/state.sqlite --resume run_...
```

At activation the CLI prints module aliases only. Failures may show one safe
project-relative module location; artifact bytes, full stacks, absolute paths,
digests, and secret values are never printed.

## Activate an automation

`woml run` executes a selected manual trigger once at startup, activates every
production trigger, and stays alive until Ctrl+C:

```bash
woml run examples/retryWorkflow.woml
```

To choose the durable state database explicitly:

```bash
woml run examples/retryWorkflow.woml --state .woml/state.sqlite
```

To activate the webhook example:

```bash
woml run examples/webhookWorkflow.woml --host 127.0.0.1 --port 3000
```

At startup WOML prints a copy-pasteable `curl` example for every registered
webhook. Its sample JSON includes the required fields from the webhook schema.

Then, from another terminal:

```bash
curl --request POST http://127.0.0.1:3000/webhooks/orders \
  --header 'Content-Type: application/json' \
  --data '{"orderId":"order-42"}'
```

Readiness, accepted occurrences, run IDs, failures, and terminal statuses are
printed as an ongoing stream to stderr. A successful run also prints its final
workflow JSON, for example `Run run_... result: {"message":"Received order
order-42"}`. Finishing one run does not deactivate the workflow.

The generated example omits `Idempotency-Key`, so every call creates a fresh
occurrence. Add that header when a caller needs safe delivery retries; repeating
the same key and payload returns the original run instead of executing twice.

Webhook, Slack, schedule, interval, and named event are supported Production
Triggers. Activation, deployment, recovery, security, and troubleshooting are
covered in [Operating WOML Production Triggers](../docs/woml-production-triggers.md).

The release examples are:

```bash
woml run examples/webhookWorkflow.woml
woml run examples/slackTriggerWorkflow.woml
woml run examples/scheduleWorkflow.woml
woml run examples/intervalWorkflow.woml
woml run examples/events
```

## Execute once

Use `woml test` when you intentionally want one manual execution that prints
its JSON result and exits:

```bash
woml test woml/tests/fixtures/hello.woml
```

Inspect an asynchronously triggered durable run with:

```bash
woml runs get run_... --state .woml/state.sqlite
```

## Call an HTTP API

Use native Fetch when you need the complete Bun `Request`/`Response` API. Use
managed HTTP when you want Rust-owned pooling, timeouts, cancellation, status
policy, response parsing, limits, and durable operation events:

```js
const response = await services.http.request({
  url: "https://api.example.com/orders",
  method: "POST",
  json: { orderId: "order-42" },
  idempotency: {
    header: "Idempotency-Key",
    value: attempt.idempotencyKey
  }
}, { name: "create-order" });
```

See [WOML Outbound HTTP](../docs/woml-http-services.md) for the complete API,
failure behavior, SSRF boundary, deployment checklist, and benchmark.

## Retry a step

Retry is configured on `<step>`; there is no retry tag:

```xml
<step
  id="fetchData"
  retry="3"
  retry-backoff="exponential"
  retry-delay="1s"
  retry-max-delay="30s">
  <script>
    return await fetchData(attempt.idempotencyKey);
  </script>
</step>
```

`retry="3"` means at most three total attempts. Only a definitive
`script_threw` failure retries in the first profile. Timeouts, invalid results,
size-limit failures, host or Worker crashes, cancellation, and interrupted
attempts fail closed.

Every attempt receives:

- `attempt.number`: the current one-based attempt number.
- `attempt.maxAttempts`: the configured maximum.
- `attempt.idempotencyKey`: one stable key shared by all attempts of this step
  in this run.

Pass that key to external services that support idempotency. WOML cannot make an
arbitrary JavaScript side effect exactly once by itself.

## Resume a safely scheduled retry

After the engine durably schedules a retry, the CLI prints a recovery command.
If the CLI stops while waiting, run that exact command:

```bash
woml run "examples/retryWorkflow.woml" \
  --state ".woml/state.sqlite" \
  --resume "run_..."
```

Completed attempts and steps are not replayed. An attempt that started but has
no recorded outcome is ambiguous and fails as `interrupted` instead of running
again.

## Local development

```bash
cd woml-cli
bun install
bun run build
bun run test:sc6
```

The SC6 gate builds the distributable CLI/native core and verifies the
frontend, Rust engine, Bun host, CLI, contracts, compatibility, clean-package
execution, linting, type checking, and secret-leak protections.

See [the WOML language reference](../docs/woml-v0.1.md), [retry contract](../docs/protocols/retry-idempotency-v1.md), and [recovery guide](../docs/woml-recovery.md).
