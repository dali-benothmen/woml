# WOML Production Runtime and Operations

Production Runtime v1 provides deployment preflight, atomic direct-source
activation, durable ownership,
restart recovery, graceful shutdown, foreground/background operation,
observability, the terminal inspector, verified backup/restore, retention,
deployment recipes, compatibility auditing, and release performance budgets. Runtime hosting uses
the direct `.woml` experience—there is no build command or deployment-package
extension.

Foreground run presentation, Enter-driven manual admission, background
`--logs`, color/plain/JSON behavior, and viewer recovery are documented in
[WOML Terminal Experience](woml-terminal-experience.md).

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

For copy-ready systemd, Docker, Nginx, single-pod Kubernetes, monitoring,
security, and upgrade procedures, see
[Deploying WOML Production Runtime v1](woml-production-deployment.md).

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

Telegram long polling, Discord Gateway connections, and WhatsApp signed
callbacks follow the same atomic readiness and shutdown boundary. Their setup,
runtime diagnostics, credential rotation, and recovery procedures are in
[WOML Communication Providers](woml-communication-providers.md) and
[Communication Provider Diagnostics and Operations](communication-provider-operations.md).

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

## Observability

WOML exposes minimal local liveness/readiness probes plus authenticated detail,
bounded snapshots, ordered SSE updates, JSON/text runtime records, and stable
Prometheus metrics on the loopback operations listener. Telemetry is derived
from durable recent-run state and live component state; it never becomes
workflow truth. Slow or broken telemetry clients are isolated from execution.

See [WOML Runtime Observability](woml-observability.md) for the endpoints,
redaction rules, stream resynchronization, limits, and monitoring guidance.

## Backup, restore, and upgrades

Create a coherent verified snapshot of the default durable state location:

```bash
woml backup ./backups/woml-2026-08-12
```

Restore is offline and refuses a live target. Replacing an existing database
requires `--replace` and retains the previous database as a reported rollback
copy:

```bash
woml stop
woml restore ./backups/woml-2026-08-12 --replace
woml run workflows/
```

Use `--state <path>` on both commands when the runtime does not use
`.woml/state.sqlite`. Backup Manifest v1, checksums, definition/module
verification, supported-store migration, state-path portability, filesystem
responsibilities, and separate secret-provider recovery are documented in
[WOML Backup, Restore, and Store Upgrades](woml-backup-and-restore.md).

Old terminal execution history can be previewed and removed without touching
active runs or durable user data:

```bash
woml prune --before 30d --dry-run
woml prune --before 30d
```

See [WOML Retention and Storage Maintenance](woml-retention-and-maintenance.md)
for automatic policy configuration, protected data, WAL checkpoints, and
explicit compaction.

## Supported release boundary

Production Runtime v1 includes non-activating preflight, atomic activation,
durable ownership, recovery, foreground/background operation, graceful
shutdown, production secret sources, authenticated run control, observability,
the live terminal inspector, coherent backup, guarded restore, store upgrades,
retention, and bounded SQLite maintenance. It is the supported continuous
single-machine release profile.

Current production operations include:

```bash
woml inspect
woml backup ./backups/woml-2026-08-12
woml restore ./backups/woml-2026-08-12
woml prune --before 30d --dry-run
woml prune --before 30d
```

The frozen performance budgets and latest local release-gate results
are published in
[`examples/production/performance-budgets.v1.json`](../examples/production/performance-budgets.v1.json).
On the reference development machine, the gate measured approximately 590 ms
startup, 307 ms recovery, 117 ms average concurrent admission, 12 ms snapshot,
18 ms metrics, 1 ms inspector rendering, 216 ms online backup, and 88 ms
retention. These numbers are regression budgets and measurements, not universal
latency promises for every host or workflow.

WOML rejects unavailable features rather than accepting and ignoring them.
Foreground and background `woml run` share the same Rust production host.
