# WOML CLI Reference

Use these exact public commands. Relative paths resolve from the current directory; the default durable store is `.woml/state.sqlite`.

## General help and version

```bash
woml --help
woml --version
woml -v
```

## Validate without running

```bash
woml check <workflow.woml|directory>... [--config <path>] [--json]
```

Parses, validates, and compiles all selected workflow/dependency surfaces without executing scripts or activating triggers. Directory discovery is non-recursive for direct `.woml` files. It also refreshes `woml-env.d.ts`.

Use this after an agent creates or edits WOML. `--json` is appropriate for tooling.

## Activate workflows

```bash
woml run <workflow.woml|directory>... \
  [--config <path>] [--host <address>] [--port <port>] \
  [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>] [--json] [--verbose] \
  [--color=auto|always|never] [--background|-d]
```

`run` activates one or more workflows as a long-lived automation runtime. Manual triggers wait for keyboard input; webhook/provider/time/event triggers remain listening. Press Ctrl+C to stop a foreground runtime.

- Multiple files/directories form one runtime unit and enable direct workflow calls.
- `--trigger` selects one manual trigger when selection is ambiguous.
- `--resume` continues a recoverable stored run using its immutable definition.
- `--state` selects a durable store; use the same path consistently across cooperating processes and later operations.
- `--host`, `--port`, and `--approval-port` configure local HTTP listeners.
- `--background`/`-d` detaches after readiness and prints runtime/log/stop information.
- `--config` supplies production runtime configuration.
- `--verbose`, color, and JSON flags affect presentation, not workflow semantics.

## One-shot integration execution

```bash
woml test <workflow.woml> [--state <path>] \
  [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>]
```

Runs one manual occurrence and exits. It accepts exactly one workflow file with a manual trigger. Use it for integration tests/CI, not as the normal persistent automation host.

## Editor declarations

```bash
woml types <workflow.woml|directory> [--output <path>]
```

Explicitly refreshes declarations for built-in services and imported modules. Normal `check` and `run` already generate the default declaration file; use this only for an explicit refresh or custom output.

## Live inspector

```bash
woml inspect [--state <path>] [--no-color]
```

Opens the colored terminal operations view for the selected runtime/store. It shows safe workflow/run health without payloads or secrets.

## List, inspect, and cancel runs

```bash
woml list [--workflow <workflowId>] [--status <status>] [--limit <1-200>] [--state <path>] [--json]
woml get <runId> [--state <path>] [--json]
woml cancel <runId> [--state <path>] [--json]
```

- `list` filters recent durable runs.
- `get` displays one redacted run with attempts, waits, and control flow.
- `cancel` durably requests cancellation; it does not claim committed external effects were reversed.

The retired `woml runs` namespace is invalid; use `list`, `get`, or `cancel` directly.

## Stop a background runtime

```bash
woml stop [--state <path>] [--json]
```

Gracefully stops the background runtime owning the selected store.

## Follow logs

```bash
woml <run-id|workflow-id> --logs \
  [--state <path>] [--config <path>] [--json] \
  [--color=auto|always|never]
```

Prints matching historical output and follows new records. Ctrl+C exits log following without stopping the background runtime.

## Secrets

```bash
woml secrets set <NAME>
woml secrets list
woml secrets delete <NAME>
```

- `set` prompts securely without echoing the value.
- `list` prints symbolic names only.
- `delete` removes one configured value.

Names use uppercase symbolic form such as `PAYMENTS_API_TOKEN`. Run these commands in the project whose configured secret provider/location should own the values. Do not put values on the command line.

## Publish an event over HTTP

```bash
woml emit <eventName> --id <publisherEventId> --data @<jsonFile> \
  --server <url> --token-secret <NAME>
```

Loads the bearer token from WOML secrets and publishes a top-level JSON object to a running public event endpoint. `emit` is an operator convenience; applications may call the same endpoint directly. Internal workflow-to-workflow fan-out uses `services.events.emit()` instead.

## Provider diagnostics

```bash
woml telegram doctor [--token-secret <NAME>] [--destination <chatId>] [--json] [--color=auto|always|never]
woml discord doctor [--token-secret <NAME>] [--destination <channelId>] [--json] [--color=auto|always|never]
woml whatsapp doctor [--access-token-secret <NAME>] [--app-secret <NAME>] \
  [--verify-token-secret <NAME>] [--phone-number-id <id>] \
  [--callback-url <https-url>] [--json] [--color=auto|always|never]
```

Doctor commands inspect credentials, permissions, and optional destinations without creating a workflow run or sending a message. Slack currently has no public doctor subcommand; do not generate `woml slack doctor`.

## Backup and restore

```bash
woml backup <backup-directory> [--state <path>] [--json]
woml restore <backup-directory> [--state <path>] [--replace] [--json]
```

`backup` creates a coherent manifest and checksummed snapshot. `restore` verifies it first. `--replace` is required to replace an existing destination and is destructive; do not run it without explicit user intent and target verification.

## Retention

```bash
woml prune --before <duration> [--state <path>] [--dry-run] [--compact] [--json]
```

Removes terminal history older than the duration while preserving active/recoverable runs and referenced definitions. Always recommend `--dry-run` first. `--compact` reclaims SQLite space after successful pruning.

## Agent command policy

- Run `woml check` automatically after authoring when available.
- Do not start a long-lived runtime, publish an event, send provider traffic, cancel a run, delete a secret, restore over a store, or prune history unless the user requested that operation.
- When a command fails, preserve its stable error code and source location in the explanation.
- Never expose secret values in commands, output, examples, or summaries.
