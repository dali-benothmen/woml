# WOML Production Runtime and Operations Implementation Plan

Status: PRO0 through PRO7 completed on 2026-08-12. The Production Runtime v1
contracts, Runtime Configuration v1, whole-deployment preflight, atomic source
activation, durable ownership, recovery, background operation, exact stop,
graceful shutdown, security, observability, terminal inspection, and verified
backup/restore are implemented. Production Runtime v1 is deliberately a strong single-machine
deployment profile: one
long-running `woml run` host can activate several workflows, survive supervised
restarts, expose safe operational visibility, and protect and maintain its
durable data. Multi-machine scheduling and a hosted control plane remain a
later milestone.

## 1. Product Outcome

WOML moves from a capable local workflow engine to a runtime that a team can
operate continuously for real business automation.

The main production journey remains simple:

```bash
woml run workflows/
```

For direct background operation on a PC or VPS:

```bash
woml run workflows/ --background
```

Operators can open a live terminal inspector without stopping or owning the
runtime process:

```bash
woml inspect
```

After this milestone, a user can:

- activate several `.woml` files together as one validated runtime unit;
- keep webhook, Slack, event, schedule, and interval triggers active 24/7;
- restart or replace the runtime without losing accepted work;
- know whether the process is alive, ready, overloaded, or degraded;
- inspect workflows, runs, queues, retries, approvals, calls, and failures from
  an `htop`-style terminal inspector;
- emit structured logs and machine-readable metrics for external monitoring;
- inject production secrets without putting values in source or compiled
  definition records;
- back up and restore the complete durable authority coherently;
- remove expired run history without deleting durable user state or data still
  required by active runs; and
- upgrade the runtime and storage schema without silently changing historical
  workflow truth.

The first production result is not a WOML cloud platform. It is a dependable,
observable, secure single-node runtime suitable for a VM, bare-metal server,
Docker container, or single Kubernetes pod with persistent storage.

## 2. Product Principles

### 2.1 `woml run` remains the runtime

Production operation does not introduce a second execution product. The
long-lived host remains `woml run`. Process supervisors such as systemd,
Docker, or Kubernetes start it, restart it, and deliver termination signals.

Foreground is the default because it keeps container and service-manager
behavior predictable. `--background`/`-d` is an explicit convenience for a PC
or directly managed VPS; WOML never detaches without being asked.

### 2.2 `.woml` source is the deployment interface

Users deploy the same `.woml` files they develop and review. `woml run` resolves
the complete input set, parses, validates, compiles, hashes, and durably pins
the exact definitions and local modules before any trigger becomes ready.

Compilation and definition retention are internal runtime responsibilities.
They do not require a public build command or another file extension.

### 2.3 One durable authority, many views

Immutable definitions and run events remain workflow truth. SQLite remains the
first production profile's durable authority. Runtime summaries, metrics,
health, logs, and the terminal inspector are projections or operational
signals; none may become a second workflow state machine.

### 2.4 Readiness is stricter than liveness

Liveness means the host process can respond. Readiness means it has passed
integrity and compatibility checks, loaded every required definition and
secret, acquired deployment ownership, activated all required trigger
providers, and can durably admit work.

A degraded or unready process must not claim that automation is available.

### 2.5 Safe shutdown stops admission before execution ownership

On SIGINT or SIGTERM, WOML first becomes unready, then stops new trigger
admission, closes provider ingress, settles or safely interrupts owned work,
flushes durable operations, and finally releases runtime ownership.

When the shutdown deadline expires, WOML preserves the existing fail-closed
rule for ambiguous effects. It never invents a successful result to make the
process exit cleanly.

### 2.6 Observability must help humans and machines

`woml inspect` is the human live view. Structured logs, health endpoints, metrics,
and a versioned local operations protocol are the machine interfaces. The TUI
is not the only monitoring system and does not need to remain open for WOML to
detect or report failures.

### 2.7 Secure by default, explicit when exposed

Trigger ingress and runtime administration are separate network boundaries.
The administration endpoint binds to loopback by default, uses a short-lived
runtime credential, and never reuses webhook or event-trigger tokens.

Remote administration, when eventually supported, requires reviewed identity,
authorization, TLS, and audit contracts. Production Runtime v1 does not turn a
loopback admin endpoint into an unauthenticated remote API.

### 2.8 Retention must never erase live truth

Cleanup may remove eligible terminal run history only after all active-run,
workflow-call, approval, retry, lifecycle, definition, and artifact references
have been checked. It never removes `services.state` values, service storage,
secrets, or active definitions as a side effect.

### 2.9 Backups are useful only when restoration is proven

Copying a live SQLite file is not the product contract. WOML creates a coherent
backup through the database backup mechanism, records checksums and version
metadata, and provides a restore verification path. A release gate must execute
an actual backup-and-restore journey.

### 2.10 Version every expensive operational boundary

Deployment manifests, runtime configuration, instance ownership, health,
operations snapshots and streams, metrics names, retention plans, backup
manifests, and store migrations are reviewed schemas before runtime behavior is
implemented.

## 3. Scope

### Included

- A production profile for one machine and one persistent WOML state location.
- One `woml run` host loading one file, multiple files, or directories.
- Atomic validation, compilation, definition pinning, and all-or-nothing
  activation for every input passed to one `woml run` command.
- Foreground operation by default and explicit `--background`/`-d` operation.
- A simple `woml stop` command for graceful background-runtime shutdown.
- An optional versioned runtime configuration file with safe defaults and
  explicit CLI/environment precedence; it is never required for ordinary use.
- Runtime preflight for workflows, modules, native compatibility, secrets,
  ports, storage, permissions, and required providers.
- Durable runtime-instance identity, ownership, heartbeat, takeover, and
  graceful shutdown semantics.
- Safe restart recovery for triggers, queued runs, retries, approvals,
  workflow calls, lifecycle work, policies, and durable user state.
- Separate trigger and local administration listeners.
- Minimal liveness/readiness endpoints and detailed authenticated health.
- Structured text/JSON logs with stable codes and correlation identifiers.
- Bounded metrics for workflow, run, queue, trigger, worker, storage, and
  provider health.
- A versioned operations snapshot and live update stream.
- The interactive `woml inspect` terminal inspector.
- Local authenticated runtime control for inspection and cancellation.
- Development secrets plus production environment/mounted-file secret
  injection using the existing `{{secrets.NAME}}` syntax.
- Online coherent backup, offline verified restore, and startup integrity
  checks.
- Safe terminal-run retention, dry-run planning, scheduled cleanup, and SQLite
  maintenance.
- Transactional storage migrations and compatibility gates for all existing
  Models v1-v12, Events v1-v11, and Store v13 data.
- Docker and systemd reference deployments and a single-pod Kubernetes guide.
- Operational performance budgets and a clean-package production release gate.

### Not included

- Multiple machines concurrently owning the same workflow deployment.
- Leader election, remote worker pools, autoscaling, or distributed queues.
- Cross-machine Workflow Call routing.
- Distributed runtime-policy, cache, state, or trigger ownership.
- Multi-region failover or globally replicated event storage.
- A hosted WOML web dashboard or SaaS control plane.
- A public `woml build` deployment step or `.womlpack` file format.
- Portable/signed deployment archives; these remain part of the postponed
  Module System distribution and registry roadmap.
- Browser-based workflow editing.
- Public internet exposure of the local operations endpoint.
- Organization accounts, roles, teams, tenant billing, or audit exports.
- In-process multi-tenant security isolation.
- Automatic TLS certificate issuance or reverse-proxy management.
- Built-in adapters for Vault, AWS Secrets Manager, GCP Secret Manager, or
  Azure Key Vault in the first profile.
- Transparent encryption of every SQLite page.
- Hot source-file watching or live mutation of an activated deployment.
- Arbitrary deletion or browsing of raw `services.state` values.
- Remote database backends or network-filesystem SQLite.
- Package signing and public registry publication.

These remain explicit later roadmap items rather than implied capabilities of
the first production release.

## 4. Product and CLI Experience

### 4.1 Development activation

Existing source activation remains unchanged:

```bash
woml run workflows/
```

The runtime compiles and validates source before admitting work. It remains in
the foreground until SIGINT or SIGTERM.

### 4.2 Background activation

```bash
woml run workflows/ --background
```

The familiar short form is also supported:

```bash
woml run workflows/ -d
```

The detached host performs the same complete preflight and atomic activation as
foreground mode. The initiating command waits until the child reports either
`ready` or a bounded startup failure, then returns control to the terminal.

Successful output is concise and actionable:

```text
WOML runtime started in the background.

Status:    ready
PID:       18427
Workflows: 6
Inspect: woml inspect
Logs:      .woml/logs/runtime.log
Stop:      woml stop
```

Background mode survives closing the terminal. It does not promise restart
after a machine reboot or process crash. For that guarantee, systemd, Docker,
or Kubernetes runs the normal foreground command and owns restart policy.

### 4.3 Graceful background stop

```bash
woml stop
```

`woml stop` discovers the current project's owner-only runtime descriptor,
authenticates to that exact runtime instance, requests graceful shutdown, and
waits for the frozen shutdown result. It does not search for or kill processes
by name.

`woml stop` controls the runtime host. `woml cancel <runId>` controls one
workflow run. The two commands never share semantics.

### 4.4 Production configuration

The planned configuration flag is:

```bash
woml run workflows/ --config woml.runtime.json
```

The configuration file is optional. Ordinary use requires only `woml run` and
safe defaults. The exact Runtime Configuration v1 schema is frozen in PRO0. It
covers runtime operation, not workflow business behavior. It may contain:

- deployment name;
- public trigger host and port;
- local admin host and port;
- state database path;
- log format and level;
- metrics and health exposure;
- runtime worker and resource ceilings;
- graceful-shutdown deadline;
- retention policy and maintenance window; and
- backup destination policy.

It must not contain secret values, workflow steps, trigger payloads, or hidden
replacements for `<config>` workflow policy.

The precedence contract is:

1. explicit CLI option;
2. reviewed WOML runtime environment variable;
3. runtime configuration file;
4. safe built-in default.

The runtime prints the resolved non-sensitive configuration and its source. It
never prints secret values.

### 4.5 Validation without activation

`woml check` expands to accept the same file, multi-file, directory, and
runtime-configuration inputs as `woml run`:

```bash
woml check workflows/ --config woml.runtime.json
```

This performs production preflight without opening ports, connecting Slack,
starting schedules, claiming deployment ownership, or admitting runs.

### 4.6 Operational commands

The product surface remains compact:

```bash
woml run workflows/ --background
woml stop
woml inspect
woml list
woml get <runId>
woml cancel <runId>
woml backup <destination>
woml prune --before 30d --dry-run
```

`list`, `get`, and `cancel` keep their current simple names. When an active
runtime descriptor is discoverable, the commands use the authenticated local
operations boundary. Offline read-only inspection remains possible against a
state path. Any offline mutation must acquire exclusive maintenance ownership
or fail clearly.

## 5. Atomic Deployment Activation v1

One `woml run` invocation treats all of its input files and directories as one
deployment unit. Activation is all-or-nothing.

Before any trigger can accept work, WOML:

1. resolves and deterministically orders the complete input set;
2. reads one stable snapshot of every `.woml` and local module source;
3. parses, validates, compiles, and hashes that snapshot;
4. validates workflow IDs, routes, event names, Workflow Calls, modules,
   runtime policies, providers, and symbolic secret requirements together;
5. preflights the native engine, state location, listeners, permissions, and
   required secret values;
6. durably registers the exact compiled definitions and module artifacts;
7. acquires runtime/deployment ownership;
8. starts providers behind a closed global readiness gate; and
9. opens admission only after every required component is ready.

If any item fails, the runtime closes partially started components, releases
safe temporary ownership, admits no new occurrence, and reports one actionable
startup failure. A webhook listener that is bound but not globally ready
returns unavailable; Slack/provider deliveries remain unacknowledged so their
normal redelivery contract applies.

The internal Deployment Activation v1 record contains:

- a stable deployment ID;
- one activation ID derived from the exact compiled definition/module set;
- workflow IDs and definition hashes;
- internal module artifact identities;
- runtime and protocol compatibility requirements;
- symbolic required-secret names, never values; and
- the resolved non-sensitive operational configuration digest.

This record is an internal durability and observability contract, not a new
file that users build or deploy. Active and historical runs retain their exact
pinned definitions and module artifacts. Editing source after readiness does
not mutate the active deployment; v1 applies changes through a controlled
restart of the same `woml run` command.

## 6. Production Runtime Host v1

### 6.1 Runtime identity

Each active host has:

- a stable deployment ID and exact activation identity;
- a fresh runtime instance ID for that process lifetime;
- the exact runtime and native-engine versions;
- a startup timestamp and monotonically updated heartbeat;
- the owned workflow definition IDs and hashes; and
- an explicit lifecycle state.

Planned lifecycle states are:

```text
starting -> recovering -> ready -> draining -> stopped
                       \-> degraded
                       \-> failed
```

Lifecycle state is operational truth, not a workflow run event.

### 6.2 Ownership

Production Runtime v1 permits one live owner for a deployment identity in one
state location. Ownership is a leased Store contract, not a PID file alone.

Startup must reject:

- a second live owner for the same deployment;
- overlapping trigger ownership that would duplicate external occurrences;
- incompatible active workflow-policy definitions;
- a state location already owned by an incompatible runtime version; and
- ambiguous stale ownership that has not passed its takeover deadline.

A stale lease may be reclaimed only after durable recovery and integrity checks
pass. Process IDs are diagnostic metadata and are never sufficient recovery
proof because PIDs can be reused.

### 6.3 Startup order

The host starts in this order:

1. parse runtime configuration;
2. resolve one stable source snapshot and verify native compatibility;
3. resolve the state location and harden permissions;
4. open, migrate, and integrity-check the store;
5. acquire deployment ownership;
6. load pinned definitions and module artifacts;
7. resolve required symbolic secrets;
8. recover durable runs and scheduler/provider state;
9. start isolated script/provider hosts;
10. bind the local admin listener;
11. activate trigger providers and public ingress;
12. mark readiness true and publish a runtime snapshot.

Failure before step 12 leaves the runtime unready and releases any safe
partially acquired resources. No trigger is acknowledged before durable
admission is available.

### 6.4 Graceful shutdown

On the first termination signal, the host:

1. sets readiness false;
2. rejects or drains new ingress according to the frozen transport contract;
3. disconnects Slack and stops time-trigger scheduling;
4. stops claiming queued/retry/call work;
5. signals active cancellable Bun and capability work;
6. waits up to the configured shutdown deadline;
7. records truthful interruption or leaves durable resumable work;
8. flushes telemetry and performs a bounded WAL checkpoint;
9. releases its runtime lease; and
10. exits with a meaningful status code.

A second termination signal may request immediate termination, but it cannot
rewrite durable outcomes. Recovery applies the existing ambiguity rules.

### 6.5 Deployment replacement

V1 uses controlled replacement, not hot mutation:

1. validate the new `.woml` source set without activating it;
2. ask the supervisor to terminate the old runtime;
3. let it become unready and drain within its deadline;
4. run the same `woml run` command against the new source set and the same
   persistent state location; and
5. run recovery before the new host becomes ready.

New occurrences bind to new active definitions. Existing runs remain bound to
their original definition hashes and artifacts.

## 7. Secrets and Security Profile

### 7.1 Authoring remains unchanged

Workflow authors continue using symbolic references:

```xml
bot-token="{{secrets.SLACK_BOT_TOKEN}}"
```

Production Runtime does not introduce another secret syntax.

### 7.2 Supported v1 secret sources

The first production profile supports:

- the existing encrypted/permission-protected local WOML secret store for
  development and single-user servers;
- environment variables injected by a process supervisor or container
  platform; and
- mounted secret files with strict ownership and permissions.

The exact name mapping, precedence, reload, and conflict rules are frozen in
PRO0. In v1, rotating a secret may require a graceful runtime restart, but it
never requires editing the `.woml` source.

### 7.3 Secret preflight

Deployment manifests expose only required symbolic names. Startup resolves
every required name before readiness and reports all missing names together.
Resolved values remain memory-only and are never written to:

- compiled workflow models;
- compiled definition and module records;
- runtime descriptors;
- events or folded context;
- logs, metrics, traces, health, or TUI screens;
- SQLite run data; or
- crash reports and fixtures.

### 7.4 Separate administration boundary

The local operations server is distinct from the public webhook/event listener.
Its default is loopback only. On startup WOML creates an owner-only runtime
descriptor containing the endpoint and a fresh capability credential so
`woml inspect` and local commands can discover and authenticate automatically.

The descriptor:

- uses mode `0600` on Unix and equivalent owner-only permissions where
  supported;
- contains no workflow or provider secret;
- is scoped to one runtime instance;
- is invalid after shutdown or instance replacement; and
- cannot authorize trigger publication or human approval.

### 7.5 Process isolation boundary

WOML continues to run embedded scripts in isolated Bun workers with real
timeouts and bounded protocol messages. Production hardening adds configured
worker, memory, open-file, process, and result ceilings where the host platform
supports them.

WOML does not claim that a JavaScript isolate is a complete hostile-tenant
sandbox. Untrusted tenant code requires container/VM and OS-level isolation.

## 8. Observability Contracts

### 8.1 Runtime snapshot

Runtime Operations Snapshot v1 is a bounded, redacted point-in-time view that
can power `woml inspect`, `woml health`, and future tools. It includes:

- runtime identity, lifecycle, version, uptime, and readiness;
- loaded workflow IDs, names, definition versions/hashes, and trigger types;
- active, waiting, queued, retrying, succeeded, failed, and cancelled counts;
- queue ages, concurrency use, and rate-limit eligibility summaries;
- active run IDs, current safe node labels, durations, and parent/child links;
- waiting approval count and deadline without approval credentials;
- trigger/provider readiness and bounded error codes;
- Bun script/provider host health and restart counts;
- SQLite health, schema version, approximate size, and maintenance status;
- last backup and retention run summaries; and
- bounded recent operational alerts.

It excludes payloads, results, raw state keys/values, secrets, headers, tokens,
script source, and unbounded error text.

### 8.2 Live operations stream

Runtime Operations Stream v1 carries ordered operational changes after a
snapshot. Every message has a runtime instance ID and monotonically increasing
sequence number. A gap forces the client to fetch a new snapshot.

The stream may combine existing versioned trigger, execution, retry, lifecycle,
workflow-call, schedule, interval, notification, and policy progress into one
safe operations envelope. It does not alter or replace the underlying run-event
vocabulary.

Backpressure is bounded. A slow inspector loses incremental updates and
resynchronizes; it never consumes unbounded runtime memory or blocks workflow
execution.

### 8.3 Structured logs

Production logs support human text and newline-delimited JSON. Each structured
record contains a stable version, timestamp, level, code, runtime/deployment
identity, and relevant safe correlation IDs such as workflow ID, run ID,
trigger ID, node ID, or provider name.

Logs distinguish:

- workflow business failure;
- lifecycle warning;
- trigger rejection;
- provider degradation;
- storage/recovery failure;
- security/authentication rejection;
- operator action; and
- runtime host failure.

Raw payloads and secret-bearing values are never included by default. Log sinks
must not be allowed to block the execution authority indefinitely.

### 8.4 Metrics

Metrics v1 uses a bounded stable label set. Planned metrics include:

- runtime readiness and uptime;
- loaded workflows and trigger-provider readiness;
- runs admitted, started, terminal, failed, cancelled, and timed out;
- active, queued, waiting, and retrying runs;
- run and step duration histograms;
- trigger accepted, duplicate, rejected, and unauthorized totals;
- retries scheduled/exhausted;
- approvals waiting, approved, rejected, and expired;
- workflow calls admitted, waiting, and terminal;
- policy queue age, concurrency usage, and rate-limit waits;
- script/provider worker crashes and restarts;
- database busy/corruption/size/checkpoint observations; and
- backup, restore-verification, and retention outcomes.

Run IDs, customer IDs, payload values, URLs, arbitrary step IDs, state keys,
and error messages are forbidden as metric labels because they create sensitive
or unbounded cardinality.

Prometheus text exposure is the first machine integration. Metrics bind to the
local admin listener by default.

### 8.5 Health

The local operations server provides:

- liveness: the host event loop and admin server respond;
- readiness: every required production dependency is ready for durable
  admission; and
- authenticated detailed health: per-component safe status and stable codes.

Minimal liveness/readiness responses contain no workflow names, paths, secret
names, payloads, or internal error detail. Detailed health remains protected.

Readiness becomes false for conditions including:

- lost runtime ownership;
- store corruption or unavailable durable admission;
- incompatible store/model/protocol version;
- missing required definition artifact;
- required provider startup failure;
- failure to bind required ingress; or
- shutdown/draining.

Ordinary workflow failure does not make the runtime unready.

## 9. `woml inspect` Terminal Inspector

### 9.1 Product role

`woml inspect` is an interactive operations client, not an executor. Closing the
inspector does not stop workflows. It discovers the active runtime descriptor,
authenticates to the local operations server, fetches a snapshot, and follows
the bounded live stream.

If no active runtime is discoverable, it reports one actionable message and
suggests the exact `woml run` or connection option. It never silently opens and
mutates SQLite as a fake live runtime.

### 9.2 Default layout

```text
 WOML INSPECT  healthy  uptime 2d 04h  workflows 8  active 6  queued 16
────────────────────────────────────────────────────────────────────
 WORKFLOWS              ACTIVE  QUEUED  FAILED  LAST RUN   TRIGGERS
 order-processing            3      12       1  2s ago     webhook
 support-agent               1       0       0  14s ago    slack
 inventory-sync              2       4       3  8s ago     interval
────────────────────────────────────────────────────────────────────
 LIVE RUNS
 STATUS    WORKFLOW          RUN ID       CURRENT WORK       DURATION
 running   order-processing  run_b67...   chargeCustomer       1.8s
 waiting   order-processing  run_a31...   managerApproval       12m
 retrying  inventory-sync    run_f92...   updateInventory       4.2s
────────────────────────────────────────────────────────────────────
 SELECTED RUN: run_a31...
 receiveOrder ✓ -> calculate ✓ -> approval ◉ -> fulfill ○
 Waiting for approval; deadline in 23h 48m; Slack delivered
────────────────────────────────────────────────────────────────────
 ALERTS
 12:53:18 inventory-sync  retrying updateInventory (attempt 2/3)
 12:53:17 order-processing run succeeded in 643ms
────────────────────────────────────────────────────────────────────
 ↑↓ select  Enter inspect  l logs  c cancel  / search  ? help  q quit
```

### 9.3 Views

The first inspector contains:

- **Overview** — runtime health and workflow summary;
- **Runs** — active and recent runs with safe status and duration;
- **Triggers** — route/provider/schedule readiness and recent acceptance;
- **Approvals** — waiting approvals, provider delivery, and deadlines;
- **Queues** — queue depth/age, concurrency, rate-limit, and retry waits;
- **Failures** — bounded failures grouped by stable code; and
- **Runtime** — Bun/Rust host health, database, backup, retention, CPU, and
  memory observations available from the process.

### 9.4 Interaction

The first supported controls are:

- arrow keys and Tab to navigate;
- Enter to inspect a workflow or run;
- `/` to search and filter;
- `l` to show bounded correlated log records;
- `c` to request cancellation with explicit confirmation;
- `r` to force snapshot refresh;
- `?` to show help; and
- `q` to close the inspector only.

The TUI does not expose raw state, secrets, approval decision tokens, trigger
credentials, or full arbitrary payloads. Human approval remains provider/HTTP
capability based; observing an approval does not grant approval authority.

### 9.5 Terminal behavior

The inspector must:

- restore the terminal on normal exit, crash, and signal;
- handle resize and narrow screens;
- support color-disabled and accessible status rendering;
- avoid animation or refresh rates that waste CPU;
- remain usable over SSH;
- display a plain-text error when stdout is not a TTY; and
- cap retained rows and log lines.

An optional later convenience may open it from `woml run`, but
the independent `woml inspect` command is the contract because production runtimes
are commonly supervised without an attached terminal.

## 10. Backup, Restore, and Upgrade

### 10.1 Backup

```bash
woml backup /backups/woml-2026-08-12
```

The runtime performs or coordinates a coherent SQLite online backup rather
than copying database/WAL/SHM files independently. Backup Manifest v1 records:

- format and schema version;
- creation time;
- deployment/activation identity;
- database checksum and size;
- included store version;
- active-definition/artifact inventory;
- retention boundary; and
- verification result.

Secrets are excluded. Operators back up secret providers separately according
to provider policy.

### 10.2 Restore

Restore is an offline, explicit operation. It refuses to overwrite an active
runtime or an unconfirmed destination. It validates manifest, checksums,
supported versions, SQLite integrity, schema objects, definitions, events,
state quotas/mutation proofs, and required artifacts before activation.

The release gate restores into a fresh directory, activates the restored
deployment, recovers durable waits, and proves prior `services.state` values
remain present.

### 10.3 Upgrade

Runtime upgrades:

- inspect store, model, definition-package, and protocol versions before
  modification;
- create or require a verified backup before a destructive migration;
- perform schema migrations transactionally;
- update store metadata last;
- never rewrite immutable run events or definition content;
- reject unknown future versions;
- preserve old runs and pinned artifacts; and
- provide an explicit rollback boundary.

Downgrading across a store migration is rejected unless a reviewed reverse
migration or compatible backup restore exists.

## 11. Retention and Storage Maintenance

### 11.1 Retention ownership

Retention applies to eligible terminal execution history. It does not own:

- active, queued, waiting, retrying, or lifecycle-finalizing runs;
- definitions or module artifacts referenced by retained/active runs;
- unresolved approvals or notification delivery audit needed by them;
- active workflow-call parent/child relationships;
- schedule/interval/event deduplication still inside its frozen safety window;
- `services.state` entries or mutation records required for reattachment;
- `services.storage`, database, cache, or secrets; or
- runtime security/audit records still inside their own policy.

### 11.2 Retention policy

Runtime Configuration v1 may define separate terminal-history ages for:

- succeeded runs;
- failed runs;
- cancelled runs; and
- operational logs/audit summaries.

V1 does not delete solely because a database reaches an unreviewed size. A
future capacity policy may stop admission safely, but must not silently erase
history.

### 11.3 Planning and execution

```bash
woml prune --before 30d --dry-run
woml prune --before 30d
```

Dry run is the default recommendation and reports counts/bytes by category
without exposing payloads. Execution:

1. acquires a maintenance lease;
2. computes one versioned Retention Plan v1;
3. rechecks eligibility in the deletion transaction;
4. deletes one bounded batch;
5. records a safe maintenance audit result;
6. checkpoints WAL according to policy; and
7. yields between batches so admission is not starved.

Automatic retention uses the same planner and executor. A crash midway leaves
completed batches committed and remaining work eligible for the next pass.

### 11.4 SQLite maintenance

The runtime monitors database/WAL size and performs bounded checkpoints.
Compaction/VACUUM is explicit or scheduled in a maintenance window because it
may require extra disk space and stronger locking. Maintenance failure degrades
health and alerts operators; it never fabricates workflow failure.

## 12. Error Surface

All new CLI, runtime, activation, operations, backup, and retention errors retain
the WOML error shape: stable code, safe message, and relevant source location or
operational subject. Planned code families include:

### Configuration and deployment

```text
WOML_RUNTIME_CONFIG_INVALID
WOML_RUNTIME_CONFIG_UNSUPPORTED
WOML_DEPLOYMENT_INPUT_INVALID
WOML_DEPLOYMENT_ACTIVATION_CONFLICT
WOML_DEPLOYMENT_PREFLIGHT_FAILED
WOML_DEPLOYMENT_SOURCE_CHANGED
WOML_DEPLOYMENT_SECRET_MISSING
```

### Host and ownership

```text
WOML_RUNTIME_OWNER_ACTIVE
WOML_RUNTIME_OWNER_STALE_UNSAFE
WOML_RUNTIME_STARTUP_FAILED
WOML_RUNTIME_NOT_READY
WOML_RUNTIME_DRAINING
WOML_RUNTIME_SHUTDOWN_TIMED_OUT
WOML_RUNTIME_ADMIN_UNAVAILABLE
WOML_RUNTIME_ADMIN_UNAUTHORIZED
WOML_RUNTIME_DESCRIPTOR_INVALID
WOML_RUNTIME_BACKGROUND_START_FAILED
WOML_RUNTIME_NOT_RUNNING
WOML_RUNTIME_STOP_CONFLICT
```

### Observability

```text
WOML_OBSERVABILITY_STREAM_GAP
WOML_OBSERVABILITY_CLIENT_SLOW
WOML_METRICS_UNAVAILABLE
WOML_TUI_NOT_INTERACTIVE
WOML_TUI_TERMINAL_UNSUPPORTED
```

### Backup, restore, and retention

```text
WOML_BACKUP_FAILED
WOML_BACKUP_INVALID
WOML_BACKUP_VERIFICATION_FAILED
WOML_RESTORE_TARGET_ACTIVE
WOML_RESTORE_INCOMPATIBLE
WOML_RETENTION_PLAN_INVALID
WOML_RETENTION_TARGET_ACTIVE
WOML_RETENTION_MAINTENANCE_BUSY
WOML_STORAGE_MAINTENANCE_FAILED
```

Ordinary “no runs matched,” “nothing eligible to prune,” and “inspector client
resynchronized” outcomes are not errors.

## 13. Versioned Artifacts Required Before Runtime Code

PRO0 freezes and reviews:

1. Runtime Configuration v1 and Production Preflight v1 schemas and precedence
   fixtures.
2. Deployment Activation v1 input snapshot, identity, atomicity, and readiness
   schema.
3. Background Runtime Control v1 start/readiness/stop behavior.
4. Runtime Instance v1 identity, lifecycle, lease, and ownership schema.
5. Store v14 logical schema/migration contract for runtime ownership,
   maintenance, and backup metadata if the reviewed design requires new durable
   tables.
6. Runtime Descriptor v1 and local credential lifecycle.
7. Runtime Admin HTTP v1 request/response/authentication contract.
8. Runtime Operations Snapshot v1 schema.
9. Runtime Operations Stream v1 schema and resynchronization rules.
10. Runtime Log Record v1 schema and redaction rules.
11. Runtime Metrics v1 names, labels, and cardinality budgets.
12. Runtime Health v1 liveness/readiness/detail schema.
13. Backup Manifest v1 and restore-verification schema.
14. Retention Policy/Plan/Result v1 schemas.
15. Graceful shutdown and stale-owner recovery fixtures.
16. TUI screen-state, stream-gap, narrow-terminal, and redaction fixtures.
17. Compatibility fixtures for Models v1-v12, Events v1-v11, Store v13,
    Definition Packages v1-v7, Script Host v1-v7, and all published service
    protocols.

If PRO0 proves that an existing generic protocol carries a new operation
without changing its shape, that proof is frozen. Otherwise the protocol is
versioned before implementation rather than widened silently.

## 14. Implementation Phases

### Phase summary

| Phase | What changes | Result after the phase |
|---|---|---|
| PRO0 (complete) | Freeze activation, background control, ownership, admin, observability, backup, and retention contracts. | Every layer agrees on the production boundaries before runtime behavior changes. |
| PRO1 (complete) | Add runtime configuration and production preflight. | Users can validate a whole deployment and its environment without activating it. |
| PRO2 (complete) | Make direct `.woml` input activation atomic and durably pinned. | `woml run` either activates the complete source set safely or activates nothing. |
| PRO3 (complete) | Add durable ownership, recovery, background mode, `woml stop`, and graceful shutdown. | One foreground or detached runtime can own a deployment safely across restarts and replacements. |
| PRO4 (complete) | Harden production secrets, local administration, and process/resource boundaries. | Production activation fails safely and operational control is authenticated and separated from ingress. |
| PRO5 (complete) | Add health, structured logs, metrics, snapshots, and live operations streaming. | Humans and monitoring systems can understand runtime health without reading SQLite or raw logs. |
| PRO6 (complete) | Build the interactive `woml inspect` terminal inspector. | Users can observe and safely control active automations from an htop-style CLI view. |
| PRO7 (complete) | Add coherent backup, verified restore, and safe runtime/store upgrades. | A deployment can recover from disk loss or a failed upgrade using a tested procedure. |
| PRO8 (complete) | Add retention planning, cleanup, and bounded SQLite maintenance. | Run history can be managed without deleting active truth or durable user state. |
| PRO9 (complete) | Harden, benchmark, package, document, and publish Production Runtime v1. | WOML is supported for continuously operated single-machine production deployments. |

### PRO0 — Freeze production contracts and reviewed fixtures (completed)

Changes:

- Decide the exact single-machine deployment and ownership boundary.
- Freeze configuration fields, precedence, defaults, bounds, and secret-free
  requirements.
- Freeze stable source snapshots, activation identity, all-or-nothing provider
  readiness, internal definition pinning, and source/module size limits.
- Freeze `--background`/`-d`, startup handoff, runtime descriptor, log
  destination, and `woml stop` behavior.
- Freeze runtime instance identity, lifecycle, lease, takeover, readiness, and
  graceful-shutdown semantics.
- Freeze the separate local admin boundary, descriptor permissions,
  authentication, session expiry, and control operations.
- Freeze snapshots, stream ordering/gaps, logs, metrics, health, and redaction.
- Freeze backup consistency, restore verification, retention eligibility, and
  maintenance locking.
- Decide whether Store v14 is required and freeze its migration before any
  table is added.
- Add reviewed success, failure, crash, takeover, shutdown, backup, restore,
  retention, stream-gap, and TUI fixtures.
- Preserve every existing schema and historical fixture byte-for-byte.

Result:

The TypeScript CLI, Rust runtime, SQLite store, N-API bridge, TUI, deployment
tooling, and operator documentation target the same reviewed contracts.

Gate:

Every PRO0 schema validates its positive/negative fixtures, all compatibility
fixtures remain unchanged, cross-artifact references agree, and the PRO0 Review
Gate in Section 20 is answered.

Completion notes:

- Fifteen schemas freeze Runtime Configuration/Preflight v1, internal Deployment
  Activation v1, Background Runtime Control v1, Runtime Instance/Descriptor and
  Admin HTTP v1, Store v14 coordination, operations snapshot/stream, structured
  logs, metrics, health, backup, and retention.
- `docs/protocols/production-runtime-operations-v1.md` resolves direct `.woml`
  activation, configuration precedence, background readiness/stop targeting,
  startup/shutdown order, readiness, local administration, observability,
  backup, and retention without inventing a public build artifact.
- Reviewed fixtures pin the complete startup/shutdown sequences and prove that
  PID alone is not authority, terminal close is not machine-restart recovery,
  telemetry stays redacted, and retention deletes zero State v1 entries.
- Store v14 is frozen as a required future transactional migration for runtime
  owner, maintenance lease, and verified-backup coordination. PRO0 adds no
  database table or runtime behavior.
- `bun run test:pro0` is the focused contract/type gate.

### PRO1 — Runtime configuration and production preflight (completed)

Changes:

- Parse and validate Runtime Configuration v1 with source-aware diagnostics.
- Apply explicit CLI/environment/file/default precedence without reading
  arbitrary environment values into workflow context.
- Expand `woml check` to multi-file, directory, and runtime-config inputs.
- Preflight definitions, modules, native engine, state directory, ports,
  required secrets, provider configuration, disk headroom, and permissions.
- Report all missing secret names and independent configuration problems in one
  bounded review instead of failing one at a time.
- Print a resolved redacted configuration summary suitable for CI.
- Preserve source-mode `woml run` with no mandatory config file.

Result:

A user can prove that a deployment is structurally and environmentally ready
without opening ingress, connecting providers, or creating a run.

Gate:

Tests cover precedence, invalid fields, bounds, path normalization, missing
secrets, port conflicts, read-only/unwritable storage, clean environments,
redaction, current CLI compatibility, and deterministic preflight for the exact
inputs later passed to `run`.

Completion notes:

- Runtime Configuration v1 is optional and manually validated without adding a
  runtime JSON-schema dependency. Relative paths resolve from the config file;
  CLI overrides, reviewed `WOML_RUNTIME_*` variables, config fields, and safe
  defaults follow the frozen precedence.
- `woml check` now accepts explicit multiple files, directories, `--config`,
  and `--json`. Repeated files are deduplicated; duplicate workflow IDs and
  webhook routes fail across the complete deployment.
- Supplying `--config` enables strict environment preflight: writable state,
  log, and backup ancestry, 64 MiB minimum disk headroom, distinct validated
  listener addresses, and one aggregated check for every required symbolic
  secret.
- Preflight never opens a listener, connects Slack, starts a scheduler, writes
  the SQLite state file, acquires ownership, or creates a run.
- Historical single-workflow `woml check` and `woml check --json` output remain
  compatible when no runtime config is supplied.
- `bun run test:pro1` composes the PRO0 contract gate with configuration,
  multi-input, storage, secrets, redaction, route-conflict, every historical
  Model/Event fixture, and all-schema compatibility tests. It is included in
  the repository release gate.

### PRO2 — Atomic direct-source activation (completed)

Changes:

- Reuse the existing WOML frontend and deterministic module compilation inside
  the normal `woml run` path.
- Resolve all explicit files/directories deterministically and read one stable
  source snapshot for the complete activation attempt.
- Validate workflow IDs, trigger routes/names, Workflow Calls, modules,
  policies, providers, and symbolic secret requirements together.
- Compute one internal activation identity from exact compiled definitions and
  module artifacts without exposing another user file format.
- Durably pin definitions and artifacts before trigger admission opens.
- Start provider/listener components behind one closed global readiness gate.
- Roll back partial startup and admit no occurrence when any component fails.
- Detect source changes during activation rather than mixing bytes from two
  revisions.
- Prove `woml check` and `woml run` validate the same source snapshot and
  compiled models.

Result:

Users continue to run only `.woml` files. The complete input set either becomes
ready together or fails before accepting work, while recovery retains the exact
internally pinned definition used by every run.

Gate:

Tests cover deterministic input order, duplicate IDs/routes, modules,
workflow calls, every trigger type, missing secrets, source changes during
startup, partial provider failure, concurrent incoming traffic before
readiness, definition pinning, rollback, restart, path escape, size bounds, and
secret scans.

Completion notes:

- `woml run` resolves every explicit file and direct directory member into one
  absolute, deduplicated, lexically ordered deployment input set. `woml check`
  uses the same compiler path and therefore the same compiled definitions.
- Each WOML document and every local JavaScript/TypeScript module is hashed.
  The compiler rejects disagreement between inspection and executable-package
  passes, then rereads the complete snapshot before startup and again after
  provider readiness. Directory membership is also rechecked, so files cannot
  be added or removed unnoticed during activation.
- One `sha256:` activation identity is derived from the sorted workflow IDs,
  exact compiled-definition hashes, and sorted runtime-module artifact
  digests. It remains internal; users still deploy only `.woml` source.
- Rust preparation validates the complete registration set and durably pins
  compiled definitions and module artifacts before any ingress can open.
- The native runtime now has separate prepare and activate operations. A
  prepared HTTP listener returns `503 WOML_RUNTIME_NOT_READY`; schedules,
  intervals, startup manual runs, Slack ingress, event dispatch, recovery, and
  Workflow Call routing remain closed until activation.
- The CLI starts Slack and other required components while Rust is suspended,
  rechecks the source snapshot, and activates Rust only when every provider is
  ready. Any provider or activation failure closes partial components in the
  existing `finally` rollback path and creates no trigger occurrence.
- Existing engine callers keep immediate-start behavior unless they explicitly
  request suspended preparation, preserving the pre-PRO2 runtime API behavior.
- `bun run test:pro2` composes PRO0/PRO1 compatibility with stable-source,
  deterministic-identity, closed-ingress, durable-pinning, provider-rollback,
  typecheck, and focused Rust compatibility coverage. It is included in the
  repository release gate.

### PRO3 — Durable ownership, background mode, recovery, and shutdown (completed)

Changes:

- Add runtime instance identity and leased deployment ownership in Rust.
- Enforce one live production owner per deployment/state boundary.
- Implement the frozen startup state machine and readiness transition.
- Run Store v13-to-v14 migration transactionally if PRO0 requires it.
- Recover triggers, schedules, intervals, queues, retries, approvals, workflow
  calls, lifecycle actions, policies, and State v1 before readiness.
- Heartbeat ownership without making lease rows workflow truth.
- Reclaim stale ownership only after its safety deadline and recovery audit.
- Add `woml run ... --background` and `-d` through a detached child that owns
  the runtime; the initiating process waits for its authenticated readiness or
  startup-failure handoff.
- Create the owner-only Runtime Descriptor v1 and a separate loopback control
  endpoint needed for exact-instance shutdown.
- Route detached runtime output to a documented project-local log destination
  and print that path after readiness.
- Add `woml stop`, which targets the descriptor's exact runtime instance and
  requests graceful shutdown rather than killing a process by name.
- Clean stale descriptors only after ownership and process-identity validation.
- Implement ordered SIGINT/SIGTERM drain with a bounded deadline.
- Preserve fail-closed ambiguous-effect behavior at forced shutdown.
- Emit runtime lifecycle progress and meaningful process exit codes.

Result:

Users can run WOML in the foreground or background, stop a detached runtime
gracefully, and use a supervisor to restart or replace it without duplicate
trigger ownership, lost accepted work, or a false ready signal.

Gate:

Tests cover foreground/background parity, terminal detachment, readiness
handoff, child startup failure, `woml stop`, no-running-runtime behavior, clean
start, second owner, process crash, stale descriptor/lease, PID reuse, restart
during every durable wait, first/second termination signal, slow Bun work,
stuck provider, SQLite contention, failed migration, and no contradictory run
outcomes.

Completion notes:

- Store v14 is implemented as a transactional migration from Store v13. It
  adds the frozen runtime-owner, maintenance-lease, and verified-backup tables
  without changing immutable definitions, events, or State v1 records.
- Rust acquires one exact deployment/runtime lease per state boundary before
  preparing the trigger host, audits the database before stale takeover,
  heartbeats the lease, stops on ownership loss, and releases only its own
  lease after shutdown.
- Recovery and definition registration finish while the PRO2 admission gate is
  closed. The CLI publishes readiness only after Rust recovery, source
  stability checks, provider readiness, activation, admin binding, and the
  owner-only descriptor are complete.
- `woml run ... --background` and `-d` spawn the same runtime as a detached
  child. The parent waits for an authenticated owner-only ready/failure
  handoff, writes output to `.woml/logs/runtime.log` (or the configured log
  directory), and never reports success for a merely spawned process.
- Runtime Descriptor v1 is written with Unix mode `0600`. `woml stop`
  authenticates to its loopback admin endpoint and targets the exact runtime
  ID; PID is used only to prove an unreachable descriptor belongs to an absent
  process before stale cleanup.
- The first SIGINT/SIGTERM or authenticated stop closes admission and drains
  tracked work through the configured deadline. A second signal forces exit;
  interrupted effects retain the existing fail-closed recovery semantics.
- `bun run test:pro3` composes all earlier production gates with Store v14
  migration/ownership, background readiness/failure, second-owner rejection,
  descriptor permissions, exact stop, stale cleanup, typecheck, and packaged
  runtime coverage. It is included in the repository release gate.

### PRO4 — Production secrets and administration security (completed)

Changes:

- Implement reviewed environment and mounted-file secret providers alongside
  the local development store.
- Resolve only activation-declared symbolic names and enforce source
  precedence.
- Validate mounted-file ownership/mode and reject unsafe secret paths.
- Harden the separate loopback operations listener and Runtime Descriptor v1.
- Generate, rotate, expire, and erase the per-instance admin capability used by
  `woml stop`, `woml inspect`, and live run-management commands.
- Route live `list`, `get`, and `cancel` through authenticated admin operations
  while preserving safe offline inspection.
- Enforce request, response, connection, and operation-rate bounds.
- Add script/provider worker resource ceilings and explicit OS/container
  isolation documentation.
- Redact all new surfaces and scan crash/error paths.

Result:

Production credential values remain outside source and compiled definition
records, and local operational control cannot be reached through public trigger
credentials or an unauthenticated port.

Gate:

Tests cover missing/conflicting secret sources, unsafe files, rotation across
restart, descriptor theft after replacement, expired credentials, public/admin
listener confusion, request flooding, unauthorized cancellation, non-leakage,
and platform permission behavior.

Completion notes:

- `WOML_SECRETS_PROVIDER` now supports the local OS store, the reviewed
  environment mapping, strict mounted files, and an explicit production
  composition. Production composition resolves only requested symbolic names
  in mounted-file/environment/OS order and rejects different duplicate values
  instead of silently choosing one.
- Mounted-file secrets require an absolute real directory, exact valid secret
  filenames, regular non-link files, bounded UTF-8 values, runtime-user/root
  ownership, a non-writable directory for group/other, and Unix files with no
  group/other access. All production sources remain read-only through the CLI.
- Runtime Admin v1 now binds only to loopback and dispatches authenticated
  `list_runs`, `get_run`, `cancel_run`, and exact-instance `stop`. Live CLI
  commands pass through that authority while their existing bounded redacted
  SQLite projections remain the display contract and offline inspection path.
- The per-instance 256-bit admin capability expires after one hour, rotates at
  half-life, atomically replaces a published owner-only descriptor, rejects
  the stolen previous value immediately, and is erased from the live handle on
  shutdown.
- Admin traffic is bounded to 16 KiB requests, 16 simultaneous operations, and
  120 operations per minute. Requests have strict protocol shapes and use a
  timing-safe credential comparison; public trigger/provider credentials have
  no admin authority.
- Script protocol size/timeout limits and the bounded runtime worker setting
  are documented alongside the OS/container memory, CPU, process, file,
  filesystem, and egress controls required for hostile code. WOML v1 does not
  claim Bun is a multi-tenant sandbox.
- `bun run test:pro4` composes the complete PRO3 gate with mounted-file modes,
  source conflicts, symbolic-name resolution, capability rotation/expiry,
  public/admin confusion, live operation dispatch, flooding/size bounds,
  redaction, verification, and typecheck. It is included in the repository
  release gate.

### PRO5 — Observability foundation (completed)

Changes:

- Produce Runtime Operations Snapshot v1 from the durable store plus bounded
  live host health.
- Normalize existing progress protocols into Runtime Operations Stream v1
  without changing run events.
- Add monotonically sequenced streaming, bounded buffers, gap detection, and
  snapshot resynchronization.
- Add Runtime Log Record v1 with text and JSON formatting.
- Add stable bounded Metrics v1 and Prometheus exposition.
- Add liveness, readiness, and authenticated detailed health endpoints.
- Track workflow/run/queue/retry/approval/call/trigger/provider/worker/store
  health and durations.
- Audit labels, errors, logs, snapshots, and streams for secret, payload, state,
  and cardinality leakage.
- Keep telemetry failures separate from workflow business outcomes.

Result:

Operators and monitoring systems can answer whether WOML is healthy, what is
running or waiting, where queues are growing, and which stable failures require
attention.

Gate:

Golden log/metric/snapshot fixtures, stream-gap/backpressure tests,
readiness-transition tests, cardinality budgets, slow/broken telemetry client
tests, redaction scans, and disabled-observability overhead benchmarks pass.

Completion notes:

- Runtime Operations Snapshot v1 now combines Rust's bounded redacted recent
  run projection with live runtime lifecycle, exact loaded workflow hashes and
  trigger types, component state, duration, queue/wait/failure counts, and a
  bounded safe alert list. It does not change or duplicate workflow events.
- A compact Rust observation query derives all-run status and trigger totals,
  currently scheduled retries, unresolved approvals, and active Workflow Calls
  from Store v14 without returning payloads, results, state, or event bodies;
  those facts therefore survive process replacement.
- Existing trigger, schedule, interval, Workflow Call, policy, and runtime
  progress is normalized into Runtime Operations Stream v1. Each runtime has a
  monotonic sequence, a 1024-update ring, explicit gap/resnapshot behavior, an
  eight-client ceiling, and slow-reader disconnection without workflow
  backpressure.
- Runtime Log Record v1 is emitted in text or newline-delimited JSON with only
  reviewed identifiers and safe authored messages. JSON progress logging drops
  arbitrary payload/result/error detail instead of serializing legacy output.
- Runtime Metrics v1 and Prometheus exposition use only the frozen metric and
  label allowlists. Payload fields, context, state keys/values, URLs, messages,
  run IDs, node IDs, secrets, and capabilities cannot become labels.
- The loopback operations listener exposes unauthenticated minimal `/livez`
  and `/readyz`; detailed health, snapshots, streams, JSON metrics, and
  Prometheus metrics require the rotating PRO4 capability. Configuration can
  disable health or metrics explicitly.
- Observability responses are bounded to 2 MiB and share PRO4 concurrency/rate
  protection. A failed store/size read, stale sequence, slow stream, malformed
  query, or broken telemetry client returns a safe telemetry failure and never
  decides a workflow business outcome.
- `bun run test:pro5` composes PRO4 with frozen-schema golden fixtures,
  sequence/gap/backpressure cases, readiness/authentication and disabled-state
  transitions, cardinality/redaction checks, broken telemetry isolation, a
  packaged `.woml` runtime journey, overhead budgets, verification, and
  typecheck. It is included in the repository release gate.

### PRO6 — `woml inspect` ✅ Completed

Changes:

- Add the `woml inspect` command as a Bun/TypeScript terminal client of the local
  admin protocol.
- Implement Overview, Runs, Triggers, Approvals, Queues, Failures, and Runtime
  views.
- Add live snapshot/stream synchronization, reconnect, and stale-data markers.
- Render branch, parallel, retry, approval, lifecycle, and workflow-call
  progress as a compact execution path.
- Add search, filter, resize, no-color, help, and bounded log drill-down.
- Add confirmed cancellation using the existing durable run-control authority.
- Restore terminal state after exit, signal, disconnect, and rendering failure.
- Reject non-TTY output with a useful alternative such as `woml list --json`.
- Enforce CPU, memory, refresh-rate, and retained-row budgets.

Result:

Running `woml inspect` gives users an htop-style, color-coded live view of active automation and
safe local run control without stopping the runtime.

Gate:

Deterministic virtual-terminal tests cover every view, keyboard control,
confirmation, reconnect, stream gaps, narrow terminals, resize, no color,
Unicode, long names, thousands of runs, redaction, terminal restoration, and
inspector-client failure isolation.

Completed implementation:

- `woml inspect` is the final public command name; it clearly describes a
  terminal inspection tool without implying that a browser UI will open.
- The independent Bun/TypeScript client consumes the authenticated PRO5
  snapshot and SSE stream, reloads rotating descriptors, marks stale data,
  reconnects with bounded backoff, and resynchronizes after stream gaps.
- Overview, Runs, Triggers, Approvals, Queues, Failures, and Runtime views are
  available with Tab and numeric navigation, selection, details, filtering,
  recent live events, help, forced refresh, and confirmed durable cancellation.
- The htop-style palette is cyan for structure, green for healthy/succeeded,
  blue for running, yellow for waiting/retrying, magenta for queued, and red
  for failure/alerts. Every state remains text-labelled; `NO_COLOR` and
  `--no-color` are supported.
- Current-node and parent-run metadata are maintained as bounded discardable
  observability projections, allowing compact step/retry/lifecycle/call work
  to be shown without changing the frozen Snapshot v1 shape.
- Alternate-screen, cursor, raw-input, signal, disconnect, and render-failure
  cleanup restore the terminal. The inspector rejects non-TTY use and points
  automation users to `woml list --json`.
- `bun run test:pro6` composes PRO5 with deterministic virtual-terminal tests,
  the PRO6 verifier, and type checking; it is included in the release gate.

### PRO7 — Backup, restore, and upgrades ✅ Completed

Changes:

- Add coherent online backup coordination and `woml backup`.
- Freeze and write Backup Manifest v1 with checksums and compatibility data.
- Verify every backup after creation without exposing secret values.
- Add explicit offline restore validation and guarded target replacement.
- Restore exact definitions, events, policy data, approvals, state, and
  required artifacts.
- Require/verify backup policy before destructive store migrations.
- Add transactional migration and rollback tests from every supported store
  version through the production version.
- Document supervisor, persistent-volume, filesystem, and secret-provider
  recovery responsibilities.

Result:

An operator can lose the runtime directory, restore a verified backup into a
clean environment, and continue the same durable workflows and user state.

Gate:

Online-write backup, crash mid-backup, checksum corruption, partial archive,
missing artifact, active-target rejection, full restore, old-store upgrade,
future-version rejection, rollback, and recovery-of-waits tests pass.

Completed implementation:

- `woml backup <directory> [--state <path>] [--json]` now uses Rust's SQLite
  online-backup authority under the Store v14 maintenance lease. It publishes
  only after a temporary snapshot passes database, event-store, State v1,
  definition, and required module-artifact audits.
- The frozen Backup Manifest v1 records the database byte size and streaming
  SHA-256 digest, Store version, exact sorted definition inventory,
  deployment/activation identity, creation time, backup identity, and verified
  status. It contains no secret-provider values.
- `woml restore <directory> [--state <path>] [--replace] [--json]` verifies the
  manifest and bytes again, rejects symlinks/partial input/future versions/live
  targets, prepares a temporary Store v14 database, clears only ephemeral
  runtime leases/claims/routes, and atomically installs it.
- Existing target replacement is never implicit. `--replace` moves the prior
  database and any WAL/SHM companions to a reported `.pre-restore-*` rollback
  path before the final swap.
- State v1 now persists its original store-location identity before snapshot,
  so a verified restore at a different absolute path preserves the same
  workflow-scoped `services.state` values without weakening isolation between
  independently created databases.
- Store v13 restore migrates transactionally on the temporary copy to Store
  v14. Unknown future versions fail closed. The current v13-to-v14 migration is
  additive; any future destructive migration must require the durable
  last-verified-backup record before changing a live store.
- Backup and restore preserve immutable definitions, events, policy/retry and
  approval history, Workflow Calls, module artifacts, and durable user state.
  Recovery tests prove an unresolved human approval remains waiting.
- Secret providers, `.woml` source/configuration deployment, and logs remain
  separate operator-owned recovery assets. The new operations guide documents
  persistent storage, supervisor, off-host backup, encryption, and restore
  responsibilities.
- `bun run test:pro7` builds the packaged native/CLI boundary, runs online-WAL,
  missing-artifact, maintenance-conflict, old/future-store, approval-wait,
  corruption, partial-input, symlink, active-target, replacement/rollback, and
  moved-State-v1 journeys, verifies frozen artifacts, type-checks, and runs
  clippy. It is included in the release gate.

### PRO8 — Retention and storage maintenance ✅ Completed

Changes:

- Add Retention Policy, Plan, and Result v1.
- Add `woml prune --dry-run` and explicit bounded cleanup.
- Execute optional scheduled retention through the same Rust authority.
- Protect active runs, calls, approvals, retries, lifecycle work, definitions,
  module artifacts, trigger deduplication, and State v1 mutation settlement.
- Prove state/storage/database data have independent ownership.
- Batch cleanup transactionally with maintenance leasing and crash-safe resume.
- Add WAL-size monitoring, bounded checkpoints, and explicit compaction
  guidance/operation.
- Report maintenance status through health, metrics, logs, and `woml inspect`.

Result:

A long-running deployment can control historical growth without deleting
active execution truth or durable application data.

Gate:

Mixed-status, active-parent/child, approval, retry, lifecycle, old-definition,
deduplication-window, State v1, concurrent-admission, crash, disk-full,
dry-run/effect equality, batching, and maintenance-lock tests pass.

Completed implementation:

- `woml prune --before <duration> --dry-run` produces the frozen Retention Plan
  v1 without mutation; removing `--dry-run` executes the matching Rust-owned
  cleanup and reports Retention Result v1. `--json`, `--state`, and an explicit
  maintenance-window `--compact` operation are supported.
- Rust derives eligibility from Store v14 run summaries and dependency facts.
  Only old terminal history is eligible. Live policy work, approvals,
  notifications, workflow calls, recent trigger/event deduplication, and their
  connected run histories remain protected.
- Definitions and module artifacts remain immutable. State v1 and all
  service-owned storage/database/cache/secret data are outside retention
  ownership; every result fixes `stateEntriesDeleted` to zero.
- Cleanup uses the shared maintenance lease, rechecks a bounded candidate set
  inside each immediate transaction, commits no more than 250 independent runs
  per batch, restores immutable guards atomically, yields between batches, and
  records the latest safe audit result durably.
- Normal cleanup performs a passive WAL checkpoint. File compaction is never
  implicit; `--compact` invokes VACUUM only under the maintenance lease.
- Runtime Configuration v1 can opt into status-specific daily retention at a
  UTC maintenance hour. Scheduled execution uses the same Rust authority
  through a non-blocking native call, so Bun continues serving triggers.
- Retention status is exposed through health components, structured logs,
  operations events, `woml_retention_total`, store/WAL byte monitoring, and
  `woml inspect`.
- The PRO8 gate covers dry-run/effect equality, multi-batch history, active and
  recent history, workflow-call dependency groups, State/definition survival,
  maintenance contention, packaged CLI behavior, automatic scheduling,
  versioned schema validation, type checking, and Rust linting.

### PRO9 — Harden and publish Production Runtime v1 ✅ Completed

Changes:

- Add clean-consumer installation and clean-server deployment journeys.
- Publish Docker, systemd, reverse-proxy, single-pod Kubernetes, backup,
  restore, retention, monitoring, security, and upgrade guides.
- Add reference runtime configuration and alerting examples.
- Add one full example deployment containing webhook, Slack, schedule/event,
  approval, retry, module, workflow call, policy, lifecycle, and State v1 use.
- Benchmark startup, recovery, admission, runtime overhead, snapshot/stream,
  metrics, TUI, backup, and retention behavior.
- Test low disk, read-only filesystem, clock change, slow shutdown, provider
  outage, process crash, corrupt database, and large bounded history.
- Validate every schema and historical fixture together.
- Audit the installed CLI package, source activation boundary, and internal
  definition records; scan all public/durable outputs for active secrets and
  sensitive state.
- Add `test:pro9` and include it in the repository release gate.

Result:

Production Runtime and Operations v1 becomes a supported release profile for
continuous single-machine WOML automation.

Gate:

The clean release passes frontend, Rust, Bun, N-API, SQLite, all triggers,
retries, approvals, branches, parallel, services, modules, workflow calls,
lifecycle, cancellation, policies, state, migration, ownership, observability,
TUI, security, backup, restore, retention, packaging, benchmarks, and
historical compatibility.

Completed implementation:

- `woml-cli` is now a publishable Apache-2.0 package with a documented `woml`
  binary, bounded package contents, release metadata, and a clean archive
  installation journey. Source, tests, databases, workflow inputs, and local
  secret material are excluded from the installed package.
- The clean-server journey installs that archive into an empty project, checks
  a workflow directory, activates a real continuous webhook deployment,
  executes and observes a run, creates a verified online backup, stops the
  exact owner, restores to a fresh state path, and previews retention.
- Runtime startup is the sole SQLite schema and migration authority. Active
  trigger paths open the already-validated store without repeating DDL, so a
  short writer conflict remains bounded and recoverable instead of creating a
  lock-upgrade failure during webhook or event admission.
- One complete four-workflow deployment demonstrates authenticated webhook and
  event ingress, Slack ingress, schedule, approval, retry, a local TypeScript
  module, Workflow Calls, runtime policies, lifecycle notifications/hooks, and
  durable State v1. `woml check <directory>` now correctly uses deployment
  checking even when no runtime config is supplied.
- Copy-ready Docker, systemd, Nginx TLS reverse-proxy, single-pod Kubernetes,
  Runtime Configuration v1, and Prometheus alert examples are published with
  the single-owner/local-SQLite boundary made explicit.
- The production deployment guide covers installation, preflight, foreground
  supervision, secrets, resource isolation, monitoring, backup, restore,
  retention, upgrade/rollback, and failure behavior.
- Production performance budgets are versioned and enforced for CLI startup,
  runtime startup/recovery, admission, snapshots, metrics, operations stream,
  inspector rendering, backup, retention, and installed bytes. The reviewed
  benchmark completes comfortably inside every budget.
- The compatibility audit validates all 101 published schemas, parses 179 JSON
  fixtures, and validates 18 historical Model fixtures plus 175 historical
  Event fixtures under their original schema versions.
- Release scans cover the installed CLI, production examples, documentation,
  definitions, outputs, and durable behavior for active secret-like values and
  sensitive payload markers. Corrupt stores fail closed.
- `bun run test:pro9` is the independent Production Runtime v1 gate; it builds
  the packaged native boundary, runs the complete frontend and Rust engine
  suites, all PRO1-PRO9 operations journeys, benchmarks, compatibility and
  security verification, type checking, and clippy. It is included in the
  repository release gate and is also exposed as `test:production-release`.

## 15. Expected File Areas

| Area | Expected locations |
|---|---|
| Runtime configuration | `woml-cli/src/runtime-config/*`, CLI parsing and schemas |
| Atomic activation | `woml-cli/src/activation/*`, existing `woml` package/compiler APIs |
| Background control | focused CLI start/stop, descriptor, and runtime-control modules |
| Runtime host | `core/woml-engine/src/runtime.rs` plus focused host/ownership modules |
| Durable authority | `core/woml-engine/src/durable.rs`, Store migration and maintenance modules |
| Operations protocol | focused Rust admin/health/telemetry modules and N-API bridge |
| TUI | `woml-cli/src/top/*` with isolated rendering/state tests |
| Secrets | `woml-cli/src/secrets/*`, production provider adapters |
| Logs and metrics | focused Rust/TypeScript observability modules |
| Backup and retention | focused Rust store backup/maintenance modules and CLI clients |
| CLI | `woml-cli/src/cli.ts`, split into focused command modules as needed |
| Schemas and fixtures | `docs/schemas/*`, `docs/protocols/*`, reviewed fixtures |
| Examples and operations docs | `examples/production/*`, `docs/*` deployment/operator guides |
| Packaging and release | `woml-cli/package.json`, package audit and `verify-pro*.ts` scripts |

Large current files may be split to keep host, store, CLI, and observability
ownership understandable. Contracts, not the current file arrangement, decide
the final module boundaries.

## 16. Verification Matrix

| Area | Required proof |
|---|---|
| Configuration | Precedence is deterministic, bounded, redacted, and optional for local use. |
| Activation | All source inputs compile from one snapshot and become ready together or not at all. |
| Background | Detached startup reports true readiness and `woml stop` targets only the exact owner. |
| Ownership | One live production owner exists; stale takeover is safe and durable. |
| Startup | Readiness follows complete preflight, migration, recovery, and provider activation. |
| Shutdown | Admission stops first; active work drains or fails closed without false outcomes. |
| Recovery | Every trigger, wait, call, policy, lifecycle, and state path survives restart safely. |
| Secrets | Only named requirements are discovered; values never leak and unsafe sources fail. |
| Admin | Operations are separate, loopback by default, authenticated, bounded, and audited. |
| Logs | Stable structured records correlate safely without payload or secret leakage. |
| Metrics | Names and labels are bounded; telemetry cannot block execution. |
| Health | Liveness and readiness differ and detailed health remains protected. |
| Stream | Ordered bounded updates resynchronize after gaps and slow clients are isolated. |
| TUI | Every view is live, bounded, accessible, redacted, and restores the terminal. |
| Backup | Online backups are coherent, checksummed, verified, and secret-free. |
| Restore | A clean host resumes prior runs/state from a verified compatible backup. |
| Upgrade | Migrations are transactional; old events/definitions remain immutable. |
| Retention | Only eligible terminal history is removed; active truth/state remain intact. |
| Maintenance | Checkpoints/compaction are bounded, visible, and do not starve admission. |
| Performance | Disabled overhead is small and production operations meet frozen budgets. |
| Packaging | A clean machine can install, run `.woml`, detach, observe, stop, back up, and recover. |
| Compatibility | All published model/event/store/protocol fixtures still validate and run. |

## 17. Risks and Guardrails

### A CLI inspector can be mistaken for the observability system

The inspector consumes versioned snapshots and streams. Health, logs, and
metrics continue independently when no TUI is attached.

### Dashboard refresh can slow workflows

Snapshot queries are bounded and use rebuildable summaries. Streaming uses a
bounded buffer; slow clients resynchronize instead of applying backpressure to
workflow execution.

### Metrics can leak data or consume unbounded memory

Metric labels come from a frozen allowlist. Arbitrary run, payload, URL, error,
step, customer, and state values are forbidden.

### “Healthy” can hide unusable automation

Liveness and readiness are separate. Required provider, store, definition,
ownership, and ingress failures make readiness false with safe component codes.

### Internal compilation can accidentally persist secrets

Activation records contain only symbolic secret names and safe digests.
Compiled definitions, internal module artifacts, runtime descriptors, and the
installed package are scanned; resolved values remain memory-only.

### Runtime configuration can become a second workflow language

It controls hosting and operations only. Business retries, concurrency, rate
limits, timeout, triggers, steps, and lifecycle remain in `.woml`.

### A second runtime can duplicate triggers

Durable deployment and trigger ownership is acquired before provider activation.
A process-local lock or PID file is insufficient.

### Graceful shutdown can claim safety while effects are ambiguous

The frozen fail-closed rule remains. The shutdown deadline changes how long the
host waits, not the truth of an unrecorded external side effect.

### Admin endpoints can become a remote unauthenticated control plane

The default is a separate loopback listener with an owner-only ephemeral
descriptor. Public binding is not included in v1.

### Log collection can expose secrets

Structured records are produced from safe fields rather than redacting an
arbitrary formatted object afterward. Release scans include error and crash
paths.

### Backup can capture inconsistent SQLite files

WOML uses SQLite's coherent backup mechanism and verifies the result. It does
not independently copy the main database, WAL, and SHM while writers run.

### Restore can overwrite good production data

Restore is offline, checks active ownership, validates into a separate target,
and requires explicit replacement confirmation.

### Retention can break event-sourced recovery

Only terminal, dependency-free histories are eligible. Eligibility is rechecked
transactionally, required definition/artifact and deduplication facts are
retained, and State v1 has independent ownership.

### SQLite can be oversold as a distributed database

The v1 production profile uses persistent local storage on one machine. Network
filesystems, multiple machines, and cross-region failover are explicitly
unsupported.

### One process can be mistaken for high availability

A supervisor provides restart recovery, not zero-downtime node failure. True
high availability requires the later Distributed Runtime milestone.

### A JavaScript isolate can be mistaken for hostile-code containment

Worker contexts provide state isolation and timeout control. Untrusted tenants
require container or VM boundaries, resource limits, and a future hosted
security model.

## 18. Global Roadmap After Production Runtime and Operations

1. **Distributed Runtime and Hosted Control Plane** — multi-node ownership,
   leader election, remote workers, cross-machine Workflow Calls, high
   availability, distributed policy/state backends, authenticated remote
   administration, tenant identity, and a future web dashboard.
2. **Complete the postponed Module System phases** — MS5 locked third-party
   packages, remaining MS6 permissions/security, MS7 portable distribution,
   and MS8 hardening/publication.
3. **WOML Package Registry and Community Ecosystem** — signed publication,
   discovery, provenance, moderation, compatibility, and deprecation.
4. **Additional Infrastructure Adapters** — postponed `services.queue` plus its
   queue trigger/dead-letter contract, document databases, external object
   storage, distributed caches, and external durable-state backends according
   to demand.
5. **Additional Communication Providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when justified.
6. **Advanced Workflow Control** — keyed concurrency, priority/delayed queues,
   configurable overflow, explicit pause/resume, cancellation propagation,
   compensation/sagas, long approval-waiting synchronous child calls, state
   transactions, and reviewed remote control APIs.
7. **Advanced Operations Experience** — optional web dashboard, historical
   analytics, tracing integrations, audit export, managed secret-provider
   adapters, remote artifact stores, and automated capacity recommendations.
8. **Retire the JavaScript Chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

Completed roadmap milestones—Retries and Idempotency, Production Triggers,
Services and Capabilities, the essential Module System, Durable Workflow Calls,
Workflow Start, Lifecycle and Engine Controls, Runtime Policies, and Durable
User State—remain the baseline and are not repeated as future work.

## 19. Decisions Deliberately Deferred Beyond v1

The following remain open but do not block the single-machine production
profile:

- distributed leader election and lease consensus;
- remote worker protocol and workload placement;
- cross-machine Workflow Call discovery/routing;
- remote multi-user roles and authorization;
- hosted web dashboard architecture;
- remote SQL/event/state authority;
- package signing and registry trust roots;
- live deployment rollout without process replacement;
- production secret-manager vendor adapters;
- transparent database encryption;
- trace export protocol and vendor integrations; and
- multi-tenant hostile-code execution.

If implementation pressure appears to require one of these, the phase stops and
the boundary is reviewed rather than silently choosing a local default that
claims distributed semantics.

## 20. PRO0 Review Gate

No production-host behavior begins until PRO0 answers and tests:

1. How is one stable snapshot formed from every `.woml` and local module input?
2. Can the complete source set become ready together or fail without admission?
3. Is one internal activation identity derived from the exact compiled
   definitions and modules without a new public artifact?
4. Are exact definitions and artifacts durably pinned before a run can use
   them?
5. Is every secret represented only by symbolic name in source, activation
   records, and optional configuration?
6. What is the exact CLI/environment/config/default precedence?
7. How does `--background` hand true readiness or failure back to its initiating
   terminal?
8. Does `woml stop` target only the exact runtime instance from the protected
   descriptor?
9. Which checks make a host live, ready, degraded, draining, or failed?
10. What durable record prevents two owners from activating the same deployment?
11. When may a stale owner be reclaimed after a crash?
12. What exact shutdown order stops admission without corrupting active work?
13. How do current ambiguity rules apply when the shutdown deadline expires?
14. Is public trigger ingress completely separate from local administration?
15. How is the runtime descriptor protected, rotated, and invalidated?
16. Which operations are available locally and which remain deferred remotely?
17. Does one bounded snapshot explain every workflow/run/wait/queue/provider?
18. Can a stream gap always recover through a fresh snapshot?
19. Are logs and metrics safe by construction with bounded label cardinality?
20. Which failures make readiness false without confusing workflow failure?
21. Can `woml inspect` fail or disconnect without affecting execution?
22. Does backup capture one coherent durable authority while writes continue?
23. Can restore prove definitions, events, state, and artifacts agree before
    activation?
24. Which terminal histories are retention-eligible, and which references block
    deletion?
25. Does retention leave `services.state`, storage, databases, and secrets
    untouched?
26. Are migrations transactional and old event/definition bytes immutable?
27. Are Models v1-v12, Events v1-v11, Store v13, and all historical fixtures
    preserved?
28. Is the first release explicitly single-machine without implied HA or
    distributed scheduling?

Once these artifacts and answers are reviewed, PRO1 may begin.

## 21. Definition of Done

Production Runtime and Operations v1 is complete when a user can:

1. activate one file, several files, or a directory directly with `woml run`;
2. observe that all inputs validate, compile, pin, and become ready atomically;
3. start the runtime in the background with `--background`/`-d` and stop that
   exact instance gracefully with `woml stop`;
4. validate the entire deployment and production environment without starting
   it;
5. activate it with `woml run` under systemd, Docker, or a single Kubernetes
   pod;
6. observe truthful liveness/readiness and safely scraped metrics;
7. restart or replace the process without losing accepted runs, waits, queues,
   policies, calls, or durable state;
8. use `woml inspect` to see workflows, active runs, queues, retries, approvals,
   failures, triggers, workers, and storage health;
9. inspect and cancel a run through authenticated local operations;
10. inject and rotate production credentials without editing `.woml` source;
11. back up a live deployment and restore it successfully into a clean target;
12. prune eligible historical runs without touching active truth or
    `services.state`;
13. upgrade from every supported store version transactionally; and
14. pass the PRO9 clean-package, crash, recovery, security, observability, TUI,
    backup, retention, migration, performance, and compatibility gate.

The milestone is not complete merely because WOML runs inside Docker, exposes a
`/health` route, or draws a terminal table. Atomic source activation, durable
definition pinning and ownership, truthful readiness, safe shutdown/recovery,
authenticated operations, bounded observability, proven restore, safe
retention, and compatibility must agree on the same reviewed production
contract.
