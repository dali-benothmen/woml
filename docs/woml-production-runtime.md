# WOML Production Runtime and Operations

Production Runtime PRO0 through PRO4 provide the reviewed contracts,
deployment preflight, atomic direct-source activation, durable ownership,
restart recovery, graceful shutdown, and foreground/background operation. Runtime hosting uses
the direct `.woml` experience—there is no build command or deployment-package
extension.

## Current commands

Check one workflow as an authoring operation:

```bash
woml check workflows/orders.woml
```

Check several workflows together:

```bash
woml check workflows/orders.woml workflows/risk.woml
woml check workflows/
```

Perform the strict production-environment preflight:

```bash
woml check workflows/ --config woml.runtime.json
```

The last command validates the complete workflow set and optional operational
configuration, checks writable storage and disk headroom, validates listener
addresses, and confirms every referenced symbolic secret is available. It does
not open ports, connect Slack, start time triggers, create the SQLite database,
or create a run.

Use `--json` for CI:

```bash
woml check workflows/ --config woml.runtime.json --json
```

## Runtime configuration

The file is optional. Local users can continue with `woml run workflows/` and
safe defaults. A production configuration begins with:

```json
{
  "schemaVersion": 1,
  "deploymentName": "my-automation",
  "statePath": "./data/state.sqlite",
  "public": { "host": "127.0.0.1", "port": 3000 },
  "admin": { "host": "127.0.0.1", "port": 3001 },
  "logging": {
    "format": "text",
    "level": "info",
    "directory": "./logs"
  },
  "workers": 4,
  "shutdownTimeoutMs": 30000,
  "observability": { "health": true, "metrics": true }
}
```

Relative paths are relative to this configuration file. See the complete
[example](../examples/production/woml.runtime.json) and the
[Runtime Configuration v1 schema](schemas/runtime-configuration.v1.schema.json).

Supported precedence is:

1. an explicit CLI option;
2. a reviewed `WOML_RUNTIME_*` environment variable;
3. the configuration file; and
4. the safe default.

The supported environment names are documented in the
[Production Runtime v1 protocol](protocols/production-runtime-operations-v1.md).
These variables configure the host; they are not injected into scripts.

## Production secrets

Workflow syntax does not change:

```xml
secret="{{secrets.ORDER_WEBHOOK_TOKEN}}"
```

With `--config`, `woml check` collects required names from every selected
workflow and reports all missing names together. It never prints values.

The local OS credential store remains the authoring default. Production can
select one reviewed read-only source:

```bash
WOML_SECRETS_PROVIDER=env woml run workflows/
WOML_SECRETS_PROVIDER=files \
WOML_SECRETS_DIRECTORY=/run/secrets \
woml run workflows/
```

Environment values use the exact `WOML_SECRET_<NAME>` mapping. Mounted values
use one file named exactly `<NAME>` below an absolute
`WOML_SECRETS_DIRECTORY`. The directory must be real (not a symlink), owned by
the runtime user or root, and not group/world writable. Each secret must be a
regular non-link file owned by the runtime user or root with no group/world
permission. Values are limited to 2048 UTF-8 bytes.

`WOML_SECRETS_PROVIDER=production` combines mounted files (when configured),
environment, then the OS store in that precedence order. It resolves only
symbolic names declared by the activated workflows. Duplicate sources may hold
the same value during migration; different values fail activation with
`WOML_SECRET_SOURCE_CONFLICT`. Values are never added to definitions,
descriptors, events, logs, or inspection.

## Atomic activation

`woml run` treats every supplied file and directory as one deployment. WOML
orders and compiles that complete set, hashes every `.woml`, `.js`, and `.ts`
source, pins the exact compiled definitions and module artifacts in Rust, and
starts providers behind a closed readiness gate.

A bound webhook or event listener returns HTTP 503 with
`WOML_RUNTIME_NOT_READY` until every required provider is ready. Schedules,
intervals, startup manual triggers, Slack ingress, recovery dispatch, and
Workflow Calls are closed by the same gate. WOML rechecks source bytes and
directory membership immediately before opening admission. If a source changes
or Slack/provider startup fails, the partial runtime is closed and no new
trigger occurrence is admitted.

The runtime prints a short form of its internal activation identity after
readiness. That identity is derived from exact compiled definitions and module
artifacts; it is not a new artifact users need to build or manage.

## Foreground and background operation

Foreground remains the default for local development, systemd, Docker, and
Kubernetes:

```bash
woml run workflows/ --config woml.runtime.json
```

For a directly managed PC or VPS, detach explicitly:

```bash
woml run workflows/ --background
# short form: woml run workflows/ -d
```

The command returns only after the detached runtime is genuinely ready. It
prints the exact runtime ID, owner-only descriptor path, project-local log
path, and stop command. Stop that exact runtime gracefully with:

```bash
woml stop
# or: woml stop --state ./data/state.sqlite
```

Rust owns a renewable Store v14 lease for the state boundary. A second process
cannot activate the same deployment while that lease is live. After a crash,
a replacement waits for lease expiry, audits the store, recovers durable work
behind a closed admission gate, and only then reports ready.

SIGINT, SIGTERM, and `woml stop` all use the same ordered drain. A second signal
forces exit; ambiguous external effects still fail closed during recovery.

## Local administration security

The public trigger listener and the operations listener are separate. Runtime
Admin v1 binds only to loopback and accepts `list_runs`, `get_run`,
`cancel_run`, and `stop` with the current per-instance capability. The
owner-only runtime descriptor is atomically replaced when that capability
rotates; old, expired, stopped, and replaced-runtime credentials are rejected.
Event tokens, webhook credentials, approval tokens, and provider secrets never
authorize administration.

Live `woml list`, `woml get`, and `woml cancel` authenticate to the active
runtime first. Their displayed data remains the bounded, redacted local SQLite
projection. When no runtime descriptor exists, `list` and `get` remain safe
offline inspection commands; `cancel` can still record a durable request for a
replacement runtime to settle.

Admin requests are capped at 16 KiB, 16 concurrent operations, and 120
operations per minute per runtime. Responses are the small frozen Admin HTTP
v1 acknowledgement. The script-host protocol separately bounds frames,
context, results, and authored timeouts. Runtime worker configuration is
bounded to 1–256. For hostile or untrusted code, deploy WOML with operating
system/container memory, CPU, process, file-descriptor, filesystem, and network
limits: the v1 runtime is not a multi-tenant sandbox and JavaScript memory
isolation is not an application-level promise.

## Current phase boundary

PRO0 freezes the production contracts, PRO1 implements configuration and
non-activating preflight, PRO2 implements atomic activation, and PRO3
implements Store v14 ownership, recovery, background operation, exact stop,
and graceful shutdown. PRO4 implements production secret sources, authenticated
live run control, rotating capabilities, request bounds, and isolation
guidance.

These planned commands are not executable until their phases:

```bash
woml top                          # PRO6
woml backup                       # PRO7
woml prune --before 30d --dry-run # PRO8
```

WOML must reject an unavailable feature rather than accepting and ignoring it.
Foreground and background `woml run` now share the same Rust production host.
