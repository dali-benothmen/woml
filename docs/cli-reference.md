# WOML CLI Reference

The `woml` command validates, activates, observes, and maintains durable WOML
automations. Human-readable colored output is the default; commands that expose
`--json` provide stable machine-readable output.

```bash
woml --help
woml --version
woml -v
```

Relative paths are resolved from the current directory. The default durable
store is `.woml/state.sqlite`.

## Author and activate

### `woml check`

```bash
woml check <workflow.woml|directory>... [--config <path>] [--json]
```

Parses, validates, and compiles one deployment without activating triggers or
executing scripts. Direct `.woml` files in a directory are loaded
non-recursively. It also checks cross-workflow targets, local imports, referenced
secret names, and optional production configuration.

### `woml run`

```bash
woml run <workflow.woml|directory>... \
  [--config <path>] [--host <address>] [--port <port>] \
  [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>] [--json] [--verbose] \
  [--color=auto|always|never] [--background|-d]
```

Activates one or more workflows as one runtime unit. Manual workflows wait for
keyboard input; network, provider, and time triggers remain active. Multiple
files are useful for `services.workflows.call()` and `.start()` because every
target is registered in the same durable runtime.

`--background` detaches after readiness and prints the runtime ID, log path,
and stop command. `--resume` continues a recoverable existing run against its
stored immutable definition. Use `--state` consistently when selecting a
non-default store.

### `woml test`

```bash
woml test <workflow.woml> [--state <path>] \
  [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>]
```

Runs one manual occurrence and exits. This is intended for automated tests and
CI, not as the normal automation host.

### `woml types`

```bash
woml types <workflow.woml|directory> [--output <path>]
```

Refreshes editor declarations for built-in services and local module aliases.
Normal `check` and `run` already refresh the default `woml-env.d.ts`; use this
advanced command only for a custom path or an explicit refresh.

## Observe and control runs

```bash
woml inspect [--state <path>] [--no-color]
woml list [--workflow <workflowId>] [--status <status>] \
  [--limit <1-200>] [--state <path>] [--json]
woml get <runId> [--state <path>] [--json]
woml cancel <runId> [--state <path>] [--json]
woml stop [--state <path>] [--json]
```

- `inspect` is the live, colored terminal view of runtime and run health.
- `list` filters recent durable runs.
- `get` shows one redacted run, its steps, attempts, waits, and control flow.
- `cancel` records a durable cancellation request and propagates it through
  supervised work.
- `stop` gracefully stops the background runtime that owns the selected store.

Run and runtime inspection deliberately omit payloads, secrets, credentials,
and operation keys.

### Follow logs

```bash
woml <run-id|workflow-id> --logs \
  [--state <path>] [--config <path>] [--json] \
  [--color=auto|always|never]
```

Shows matching historical output and follows new records. Press Ctrl+C to stop
following; the background automation continues running.

## Secrets

```bash
woml secrets set <NAME>
woml secrets list
woml secrets delete <NAME>
```

`set` prompts without echoing the value. Names use uppercase symbolic form such
as `PAYMENTS_API_TOKEN`. Workflow source refers to a configured value as
`{{secrets.PAYMENTS_API_TOKEN}}` in supported attributes or
`secrets.PAYMENTS_API_TOKEN` inside scripts. `list` prints names, never values.

## Publish an event

```bash
woml emit <eventName> --id <publisherEventId> --data @<jsonFile> \
  --server <url> --token-secret <NAME>
```

Publishes an authenticated event to a running WOML event endpoint. The token is
loaded from WOML secrets by symbolic name. Applications may call the same HTTP
endpoint directly; `emit` is a convenient operator client, not a requirement.

## Provider diagnostics

```bash
woml telegram doctor [--token-secret <NAME>] [--destination <chatId>] [--json]
woml discord doctor [--token-secret <NAME>] [--destination <channelId>] [--json]
woml whatsapp doctor [--access-token-secret <NAME>] [--app-secret <NAME>] \
  [--verify-token-secret <NAME>] [--phone-number-id <id>] \
  [--callback-url <https-url>] [--json]
```

Doctor commands validate credentials, permissions, and optional destinations
without creating a workflow run. They redact credentials from both human and
JSON output.

## Backup and retention

```bash
woml backup <backup-directory> [--state <path>] [--json]
woml restore <backup-directory> [--state <path>] [--replace] [--json]
woml prune --before <duration> [--state <path>] [--dry-run] [--compact] [--json]
```

- `backup` creates a coherent manifest and checksummed durable snapshot.
- `restore` verifies a backup before restoring it; `--replace` is required to
  replace an existing target store.
- `prune` removes terminal history older than the requested duration while
  preserving active/recoverable runs and referenced definitions. Start with
  `--dry-run`; `--compact` reclaims SQLite space after successful pruning.

See [Backup and restore](woml-backup-and-restore.md) and
[Retention and maintenance](woml-retention-and-maintenance.md) before operating
on production state.

## Exit behavior and diagnostics

Authoring errors carry a stable code, source location, message, and usually a
repair hint. Runtime and provider errors use stable codes without printing
secret values. A non-zero exit indicates that the requested command failed;
long-lived `run` and log-following commands normally end through Ctrl+C or an
explicit stop.
