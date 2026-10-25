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
woml check <workflow.woml|directory>... [--config <path>] \
  [--policy <name|path>] [--policy-trust <path>] \
  [--require-policy-signature] [--process-policy <path>] [--json]
```

Parses, validates, and compiles one deployment without activating triggers or
executing scripts. Direct `.woml` files in a directory are loaded
non-recursively. It also checks cross-workflow targets, local imports, referenced
secret names, and optional production configuration.

### `woml run`

```bash
woml run <workflow.woml|directory>... \
  [--config <path>] [--policy <name|path>] \
  [--emergency-deny <name|path>] [--policy-trust <path>] \
  [--require-policy-signature] \
  [--process-policy <path>|--allow-process <aliases>] \
  [--artifact-policy <path>] [--reviewer-directory <path>] \
  [--host <address>] [--port <port>] \
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

`--allow-process bun,git` grants only those resolved executable aliases to a
reviewed foreground run. It is rejected with `--background`. Use
`--process-policy <path>` for long-lived or background Process v1 authority.
`--reviewer-directory <path>` supplies operator-owned canonical reviewer
identities and roles for Rich Plan Approvals; missing authority fails closed
before the workflow reaches a protected effect.

`--policy` selects an operator-owned Infrastructure Policy by project-local
JSON path or by name from `.woml/policies/<name>.json`. `--policy-trust`
selects Ed25519 public-key trust; `--require-policy-signature` rejects unsigned
or untrusted bundles. Either trust option without `--policy` fails closed.
`--emergency-deny` accepts only a separately validated deny-only overlay and
also requires a base policy.

### `woml test`

```bash
woml test <workflow.woml> [--process-policy <path>|--allow-process <aliases>] \
  [--policy <name|path>] [--emergency-deny <name|path>] \
  [--policy-trust <path>] [--require-policy-signature] \
  [--artifact-policy <path>] [--reviewer-directory <path>] \
  [--state <path>] \
  [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>]
```

Runs one manual occurrence and exits. This is intended for automated tests and
CI, not as the normal automation host.

## Infrastructure Policies

```bash
woml policy check <name|path> \
  [--trust <path>] [--require-signature] [--json]
woml policy explain <workflow.woml> --policy <name|path> \
  [--step <stepId>] [--trust <path>] [--require-signature] [--json]
```

`policy check` validates strict JSON, limits, canonical identity, optional
signature trust, duplicate coverage, and unreachable allows. `policy explain`
performs conservative source analysis and reports `allowed`, `denied`,
`requirements_missing`, or `unknown_until_runtime`. It is not a runtime
authorization decision: dynamic arguments, targets, effect classification, and
durable evidence are decided by the Rust authority immediately before dispatch.

Named bundles resolve only from `.woml/policies/`. Explicit paths, names with
spaces or Unicode, and platform-native separators are accepted when the
canonical file remains under the project root. Traversal, symlink escape,
unexpected extensions, duplicate keys, and future contracts are rejected.

See [Infrastructure Policies](infrastructure-policies-v1.md) and the complete
[`infrastructurePolicies`](../examples/infrastructurePolicies/) example.

## Process permissions

Workflows using `services.process.run()` compile normally but execution is
disabled until an operator supplies one of these explicit grants:

```bash
# Short-lived local review; executable paths come from the operator PATH.
woml test checks.woml --allow-process bun,git

# Reviewed foreground/background policy with canonical paths and limits.
woml run workflows/ --process-policy ./process-policy.json --background
```

The policy also controls working directories, inherited and explicit
environment names, maximum timeout, captured output, and concurrency. See
[Running Local Tools Safely](woml-process.md) before enabling it in production.

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
woml recover <runId> --retry <actionId> [--state <path>] [--json]
woml recover <runId> --resolve <actionId> --note <text> [--state <path>] [--json]
woml stop [--state <path>] [--json]
```

- `inspect` is the live, colored terminal view of runtime and run health.
- `list` filters recent durable runs.
- `get` shows one redacted run, its steps, attempts, waits, control flow,
  bounded artifact metadata, and Rich Approval review/quorum/audit state.
- `cancel` records a durable cancellation request and propagates it through
  supervised work.
- `recover` records one state-checked operator decision for an ambiguous
  compensation or cleanup action. Retry means the operator proved the previous
  effect did not occur. Resolve means the effect was completed or repaired
  externally; the note is reduced to a digest and no fake success event is
  created.
- `stop` gracefully stops the background runtime that owns the selected store.

Run and runtime inspection deliberately omit payloads, secrets, credentials,
operation keys, provider tokens, and raw identity assertions. Rich Approval
Inspection v11 is an operator surface and may show bounded canonical principal,
provider-account, role, and decision history; that protected audit data never
enters ordinary workflow script context.

### Inspect and export durable artifacts

```bash
woml artifacts inspect <artifactId> [--state <path>] [--json]
woml artifacts export <artifactId> --output <path> \
  --artifact-policy <path> [--state <path>] [--json]
```

`inspect` returns name, size, digest prefix, status, retention, and provenance;
it cannot read the body. `export` is the explicit, audited operator boundary
for verified bytes. It requires the active artifact policy and refuses to
overwrite an existing destination. Neither command creates a public URL.

### Follow logs

```bash
woml logs <run-id|workflow-id> \
  [--follow|--no-follow] [--tail <count>|--after <cursor>] \
  [--source <stdout,stderr,script,runtime>] \
  [--phase <all|forward|corrective>] \
  [--state <path>] [--config <path>] [--json] \
  [--color=auto|always|never]
```

Replays bounded operational records and follows new process output, script
logs, and structured progress. `--after` resumes from one deployment cursor;
`--tail` selects recent records; source and phase filters do not widen access
to another run. The compatible `woml <subject> --logs` shorthand remains
available. Press Ctrl+C to detach; the automation continues running. See
[Streaming Output and Progress](woml-streaming-output-and-progress.md).

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

## Deliver an external signal

```bash
woml signal <signalName> --run <runId> --wait <waitId> \
  --id <idempotencyKey> --correlation <value> --payload <jsonFile> \
  [--state <path>]
```

Delivers a bounded JSON payload through the owner-authenticated local runtime
control channel. It is useful for operators and local scripts. Remote CI,
scanners, and applications should call the authenticated HTTP signal route
printed by `woml run`. Both paths use the same durable admission and
idempotency authority. See [Durable Waits](woml-durable-waits.md).

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
woml backup <backup-directory> [--state <path>] [--artifact-policy <path>] [--json]
woml restore <backup-directory> [--state <path>] [--artifact-policy <path>] [--replace] [--json]
woml prune --before <duration> [--state <path>] [--artifact-policy <path>] [--dry-run] [--compact] [--json]
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
# Project and infrastructure commands

The shortest supported project journey is `woml init`, `woml check`, and
`woml test`. Project-aware `run`, `test`, `check`, and `types` commands accept
`--project <path>` and otherwise inspect only the current directory for
`woml.project.json`.

Remote operations use `woml target list|check|inspect|drain`,
`woml worker check|start`, and `woml dispatch inspect|reconcile`. An ambiguous
dispatch must be investigated before reconciliation; it is never an automatic
retry signal. See the [operator runbook](combined-developer-experience-operator-runbook.md).
