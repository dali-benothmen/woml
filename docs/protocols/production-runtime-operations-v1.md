# WOML Production Runtime and Operations v1 Contracts

Status: frozen by PRO0 on 2026-08-12. PRO1 through PRO4 implement configuration,
preflight, atomic activation, background hosting, durable ownership, production
secret sources, authenticated local administration, and the PRO5 observability
foundation. PRO6 implements the terminal inspector and PRO7 implements coherent
backup, guarded restore, and supported-store recovery. Retention remains gated
to PRO8.

## Product boundary

The public deployment input remains `.woml` source:

```bash
woml run workflows/
woml run workflows/ --background
woml stop
woml inspect
woml backup ./backups/woml-2026-08-12
woml restore ./backups/woml-2026-08-12
```

There is no public `woml build` command and no `.womlpack` format in Production
Runtime v1. WOML compiles, hashes, and pins definitions internally. Portable
distribution belongs to the postponed Module System roadmap.

The first production profile is one machine, one local persistent SQLite state
location, and one active owner for a deployment. A supervisor supplies restart
policy. This is not high availability and does not permit two machines to share
SQLite or own the same trigger deployment.

## Runtime Configuration v1

The configuration file is optional JSON with `schemaVersion: 1`. It contains
operational hosting policy only: deployment name, state path, public/admin
listeners, logging, workers, shutdown deadline, observability, retention, and
backup directory. It cannot contain workflow steps, trigger payloads, secret
values, or replacements for workflow `<config>`.

Relative paths resolve from the configuration file directory. Without a file,
paths resolve from the working directory.

Precedence is frozen per supported field:

1. explicit CLI option;
2. reviewed `WOML_RUNTIME_*` environment variable;
3. runtime configuration file;
4. safe built-in default.

PRO1 recognizes only:

- `WOML_RUNTIME_DEPLOYMENT`;
- `WOML_RUNTIME_STATE`;
- `WOML_RUNTIME_HOST` and `WOML_RUNTIME_PORT`;
- `WOML_RUNTIME_ADMIN_HOST` and `WOML_RUNTIME_ADMIN_PORT`;
- `WOML_RUNTIME_LOG_FORMAT`, `WOML_RUNTIME_LOG_LEVEL`, and
  `WOML_RUNTIME_LOG_DIRECTORY`;
- `WOML_RUNTIME_WORKERS`; and
- `WOML_RUNTIME_SHUTDOWN_TIMEOUT_MS`.

Arbitrary environment variables are never injected into scripts or workflow
context. Secret variables remain owned by the existing secret provider and
`{{secrets.NAME}}` surface.

The public listener defaults to `127.0.0.1:3000`; the local admin listener
defaults to `127.0.0.1:3001`. They cannot use the same host and port. Workers
default to 4 and the shutdown deadline defaults to 30 seconds.

## PRO1 production preflight

Normal authoring remains compatible:

```bash
woml check workflow.woml
```

It parses, validates, compiles, and refreshes editor declarations as before.
Supplying several inputs performs one cross-workflow check. Supplying
`--config` additionally performs production-environment preflight:

```bash
woml check workflow-a.woml workflow-b.woml --config woml.runtime.json
woml check workflows/ --config woml.runtime.json --json
```

The preflight:

- deterministically discovers direct `.woml` files in every input directory;
- deduplicates repeated absolute file paths;
- rejects duplicate workflow IDs and webhook routes across the deployment;
- compiles modules and exact workflow definitions without executing code;
- validates the runtime configuration and field precedence;
- checks state/log/backup locations through their nearest existing writable
  ancestor without creating a run;
- requires at least 64 MiB available on the state filesystem;
- aggregates all symbolic secret references and reports every missing name;
- prints only non-sensitive resolved configuration and secret-provider name;
  and
- never binds a listener, connects Slack, starts a scheduler, acquires runtime
  ownership, or admits a trigger occurrence.

The PRO1 JSON result uses `woml.production-preflight/v1`. The historical single
workflow `woml check --json` package output remains unchanged when `--config`
is absent.

Checking whether another process currently owns a TCP port would require
binding it and introduce a time-of-check/time-of-use race. PRO1 validates port
bounds and public/admin conflicts. PRO3 owns actual atomic bind/readiness.

## Atomic Deployment Activation v1

One future `woml run` invocation treats its complete source input as one unit.
It reads a stable source/module snapshot, validates and compiles the complete
set, calculates an internal activation identity, pins exact definitions and
module artifacts, acquires ownership, recovers durable work, and starts all
required providers behind a closed readiness gate.

Any failure before readiness closes partial resources and admits no new
occurrence. Editing files after readiness does not mutate the active
deployment. V1 applies changes through a controlled restart.

The activation record contains symbolic secret names, never values. It is an
internal durability/projection record, not a user-created artifact.

## Background Runtime Control v1

`--background` and `-d` start an exact detached child runtime. The initiating
process waits for an authenticated `ready` or `startup_failed` handoff. It never
prints success merely because a process was spawned.

On success it prints the ready status, PID, workflow count, inspector command,
log path, and `woml stop`. Closing the initiating terminal does not stop the
runtime. Machine restart and crash restart remain supervisor responsibilities.

`woml stop` discovers an owner-only descriptor, authenticates to the local
admin listener, targets the exact runtime instance ID, requests graceful
shutdown, and waits for settlement. It never kills a process by name and never
treats a PID as authority. `woml cancel` remains control of one workflow run.

Background execution starts in PRO3; PRO1 does not accept and ignore these
flags.

## Runtime ownership and Store v14

Store v14 is required because runtime ownership, maintenance leasing, and last
verified-backup metadata must coordinate across processes and restart. These
records are rebuildable/operational coordination, not workflow truth.

The migration from Store v13 is transactional, creates and validates required
objects, and updates version metadata last. It never rewrites immutable
definitions, Event v1-v11 histories, policies, approvals, service operations,
or State v1 data. Store v14 implementation starts in PRO3.

One owner lease binds deployment ID, activation ID, runtime instance ID,
heartbeat, and expiry. PID is diagnostic only. A stale lease can be reclaimed
only after its expiry and a complete startup recovery/integrity audit.

## Startup and readiness

The frozen startup order is:

1. configuration validated;
2. stable source snapshot compiled;
3. store opened, migrated, and audited;
4. deployment ownership acquired;
5. exact definitions/artifacts registered;
6. required symbolic secrets resolved;
7. durable recovery completed;
8. script/provider workers started;
9. local admin listener bound;
10. required trigger providers and ingress started; and
11. readiness opened.

Liveness means the admin process can answer. Readiness additionally requires
durable admission, ownership, definitions, secrets, recovery, providers, and
ingress. Ordinary workflow failure does not make the host unready. Store,
ownership, required-provider, compatibility, or ingress failure does.

## Graceful shutdown

The first termination/stop request performs:

1. close readiness;
2. close new ingress;
3. disconnect providers;
4. stop new scheduler claims;
5. signal active cancellable work;
6. wait through the configured deadline;
7. settle truthful durable state using existing ambiguity rules;
8. flush bounded telemetry/checkpoint work;
9. release ownership; and
10. exit.

The deadline never turns an ambiguous external effect into success. A second
signal may force process termination, but recovery remains fail-closed.

## Local administration security

Public webhook/event ingress and administration are separate listeners and
credential families. Admin defaults to loopback. A per-instance descriptor is
owner-only (`0600` on Unix), contains the admin URL and an ephemeral capability,
and expires when that runtime instance stops or is replaced.

The capability cannot publish events, call webhooks, resolve approvals, or act
as a workflow/provider secret. PRO4 rotates it halfway through a one-hour
lifetime and atomically replaces a published descriptor. The old value is
invalid immediately; shutdown erases the in-memory value and removes the exact
instance descriptor. Admin v1 accepts at most 16 KiB per request, 16 concurrent
operations, and 120 operations per minute. Only loopback binding is accepted.

The reviewed production secret sources are the local OS credential store,
`WOML_SECRET_<NAME>` environment values, and strict mounted files beneath an
absolute `WOML_SECRETS_DIRECTORY`. Production precedence is mounted file,
environment, then OS store. Only activation-declared names are resolved, and
different values for one name across configured sources fail closed. Mounted
directories and files cannot be symlinks; Unix ownership is the runtime user or
root, directories cannot be group/world writable, and files cannot grant any
group/world access.

## Observability

Operations Snapshot v1 is a bounded, redacted projection for workflows, active
and recent runs, components, and alerts. Operations Stream v1 is an ordered
per-instance sequence. A sequence gap requires a new snapshot; slow consumers
are disconnected/resynchronized rather than blocking execution.

Structured Log v1 permits only safe correlation identifiers and bounded
messages. Metrics v1 freezes the metric/label allowlist; arbitrary run IDs,
node IDs, URLs, state keys, payload fields, and error messages are forbidden
labels. Health v1 separates public minimal liveness/readiness from authenticated
detailed health.

`woml inspect` consumes these contracts and never becomes a second runtime truth.
Closing or crashing the TUI cannot stop or slow workflow execution.

PRO5 implements the snapshot and stream on the authenticated loopback listener.
The stream retains 1024 monotonically sequenced updates per runtime instance,
allows eight clients, emits `WOML_OBSERVABILITY_STREAM_GAP` when snapshot
resynchronization is required, and disconnects slow readers. Snapshot/admin
responses are bounded to 2 MiB. Minimal `/livez` and `/readyz` responses carry
only the frozen public health shape; detailed health, snapshot, stream, JSON
metrics, and Prometheus exposition require the rotating admin capability.

Runtime Log v1 uses only its frozen correlation fields. Runtime Metrics v1 uses
only its frozen names and label allowlist; payloads, context, state, results,
URLs, arbitrary messages, run IDs, and node IDs cannot become labels.
Telemetry failures return safe operational errors and never decide workflow
business outcomes.

## Backup and retention

Backup Manifest v1 describes a coherent SQLite online backup plus exact
definition/artifact inventory. It never includes secret values. Restore is an
offline verified operation and rejects active targets.

`woml backup <directory> [--state <path>]` publishes exactly `manifest.json`
and `state.sqlite` through a temporary-directory/atomic-rename boundary. Rust
owns the SQLite online snapshot, maintenance lease, integrity audit, definition
and module-artifact validation, and verified-backup record. The Bun CLI owns
strict paths, manifest encoding, streaming SHA-256 verification, and the
operator-facing result.

`woml restore <directory> [--state <path>] [--replace]` is offline. It verifies
the immutable backup before copying, prepares and migrates a temporary target,
clears only ephemeral runtime ownership/claim/route state, audits the result,
and atomically installs it. Existing targets require `--replace`; their prior
database and WAL/SHM companions move to a reported rollback path. Active
descriptor processes or live Store v14 owner leases reject restoration.

Store v13 and v14 are accepted. Supported old stores migrate transactionally
on the temporary restore copy; future versions fail closed. State v1 persists
its original location identity in store metadata before snapshot, so restoring
the verified database to a different absolute path preserves workflow state
scope. Secret-provider contents, source deployments, runtime configuration,
and logs are separate operational assets and are not copied by this contract.

Retention v1 removes only dependency-free terminal history. It never owns
active/queued/waiting/retrying runs, unresolved approvals, active Workflow
Calls, required definitions/artifacts, protected trigger deduplication, State
v1, storage, databases, cache, or secrets. A retention result freezes
`stateEntriesDeleted: 0`.

## Frozen artifacts

- `runtime-configuration.v1.schema.json`
- `production-preflight.v1.schema.json`
- `deployment-activation.v1.schema.json`
- `background-runtime-control.v1.schema.json`
- `runtime-instance.v1.schema.json`
- `production-runtime-store.v14.schema.json`
- `runtime-descriptor.v1.schema.json`
- `runtime-admin-http.v1.schema.json`
- `runtime-operations-snapshot.v1.schema.json`
- `runtime-operations-stream.v1.schema.json`
- `runtime-log-record.v1.schema.json`
- `runtime-metrics.v1.schema.json`
- `runtime-health.v1.schema.json`
- `backup-manifest.v1.schema.json`
- `retention.v1.schema.json`
- reviewed PRO0 contracts and semantics fixtures

## Deliberately deferred

PRO0 does not resolve multi-node consensus, remote workers, cross-machine
Workflow Calls, remote administration, hosted dashboards, third-party secret
managers, transparent SQLite encryption, portable deployment archives, package
signing, or multi-tenant hostile-code execution.
