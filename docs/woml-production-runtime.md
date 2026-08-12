# WOML Production Runtime and Operations

Production Runtime PRO0 through PRO2 provide the reviewed contracts,
deployment preflight, and atomic direct-source activation. Runtime hosting uses
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

## Current phase boundary

PRO0 freezes the production contracts, PRO1 implements configuration and
non-activating preflight, and PRO2 implements atomic activation. Durable
deployment ownership and detached operation begin in PRO3.

These planned commands are not executable until their phases:

```bash
woml run workflows/ --background  # PRO3
woml stop                         # PRO3
woml top                          # PRO6
woml backup                       # PRO7
woml prune --before 30d --dry-run # PRO8
```

WOML must reject an unavailable feature rather than accepting and ignoring it.
The existing foreground `woml run workflows/` is now the atomically activated
production-trigger host while PRO3 adds durable ownership and background mode.
