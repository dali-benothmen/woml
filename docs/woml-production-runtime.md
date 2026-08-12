# WOML Production Runtime and Operations

Production Runtime PRO0 and PRO1 provide the reviewed contracts and the first
deployment preflight. Runtime hosting still uses the existing direct `.woml`
experience—there is no build command or deployment-package extension.

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

## Current phase boundary

PRO0 freezes atomic activation, background control, ownership, administration,
observability, backup, restore, and retention contracts. PRO1 implements only
configuration and non-activating preflight.

These planned commands are not executable until their phases:

```bash
woml run workflows/ --background  # PRO3
woml stop                         # PRO3
woml top                          # PRO6
woml backup                       # PRO7
woml cleanup --older-than 30d     # PRO8; final spelling reviewed there
```

WOML must reject an unavailable feature rather than accepting and ignoring it.
The existing foreground `woml run workflows/` remains the production-trigger
host while PRO2 and PRO3 add atomic activation and durable ownership.
