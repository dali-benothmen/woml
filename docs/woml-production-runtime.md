# WOML Production Runtime and Operations

Production Runtime PRO0 through PRO3 provide the reviewed contracts,
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

## Secrets

Workflow syntax does not change:

```xml
secret="{{secrets.ORDER_WEBHOOK_TOKEN}}"
```

With `--config`, `woml check` collects required names from every selected
workflow and reports all missing names together. It never prints values. The
existing local or environment-backed WOML secret provider remains the source.

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

## Current phase boundary

PRO0 freezes the production contracts, PRO1 implements configuration and
non-activating preflight, PRO2 implements atomic activation, and PRO3
implements Store v14 ownership, recovery, background operation, exact stop,
and graceful shutdown.

These planned commands are not executable until their phases:

```bash
woml top                          # PRO6
woml backup                       # PRO7
woml prune --before 30d --dry-run # PRO8
```

WOML must reject an unavailable feature rather than accepting and ignoring it.
Foreground and background `woml run` now share the same Rust production host.
