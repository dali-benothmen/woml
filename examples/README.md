# WOML Examples

These examples are ordered from the first local workflow to production
operation. Every `.woml` file in this directory is validation-tested with
`woml check`; integrations still require their documented credentials or local
infrastructure before execution.

Run paths below from the repository root. After installing WOML, the same
commands work in your own project with copied files.

## 1. Start with the language

| Learn | Example | Run |
| --- | --- | --- |
| Manual trigger and sequential context | [`helloWorkflow.woml`](helloWorkflow.woml) | `woml run examples/helloWorkflow.woml` |
| Keyboard-triggered terminal output | [`terminalExperience/five-step-manual.woml`](terminalExperience/five-step-manual.woml) | `woml run examples/terminalExperience/five-step-manual.woml` |
| Conditional value routing | [`switchWorkflow.woml`](switchWorkflow.woml) | `woml run examples/switchWorkflow.woml` |
| Concurrent multi-step routes and joins | [`forkDistributionWorkflow.woml`](forkDistributionWorkflow.woml) | `woml run examples/forkDistributionWorkflow.woml` |
| Durable retry attempts | [`retryWorkflow.woml`](retryWorkflow.woml) | `woml run examples/retryWorkflow.woml` |
| Durable per-item processing | [`forEachWorkflow.woml`](forEachWorkflow.woml) | `woml run examples/forEachWorkflow.woml` |
| Workflow/step lifecycle hooks | [`lifecycleWorkflow.woml`](lifecycleWorkflow.woml) | `woml run examples/lifecycleWorkflow.woml` |
| Durable timers and external callbacks | [`durableWaits/`](durableWaits/) | `woml run examples/durableWaits/timer.woml` |
| Durable locks, leases, and fencing | [`locksAndLeases/`](locksAndLeases/) | `woml run examples/locksAndLeases/deployment.woml` |
| Compensation and mandatory cleanup | [`compensationAndCleanup/`](compensationAndCleanup/) | `woml run examples/compensationAndCleanup/previewEnvironment.woml` |

Use `woml test <file>` instead when you intentionally want one manual run that
prints the result and exits.

## 2. Add production triggers

| Trigger | Example | Notes |
| --- | --- | --- |
| Webhook | [`webhookWorkflow.woml`](webhookWorkflow.woml) | Prints its URL and generated `curl` request at startup. |
| Schedule | [`scheduleWorkflow.woml`](scheduleWorkflow.woml) | Uses a five-field cron expression and IANA timezone. |
| Interval | [`intervalWorkflow.woml`](intervalWorkflow.woml) | Uses a fixed durable interval. |
| Authenticated event | [`eventWorkflow.woml`](eventWorkflow.woml) | Configure `EVENT_CONTROL_TOKEN` before activation. |
| Slack | [`slackTriggerWorkflow.woml`](slackTriggerWorkflow.woml) | Configure Slack bot/app tokens and enable events. |

Activate any of them with `woml run <file>`. The process stays alive until
Ctrl+C because a workflow host must remain available for future occurrences.

## 3. Human approval

- [`slackApprovalWorkflow.woml`](slackApprovalWorkflow.woml)
- [`telegramApprovalWorkflow.woml`](telegramApprovalWorkflow.woml)
- [`discordApprovalWorkflow.woml`](discordApprovalWorkflow.woml)
- [`multiProviderApprovalWorkflow.woml`](multiProviderApprovalWorkflow.woml)

All destinations for one approval share one durable decision: approving or
rejecting through any delivery settles the same waiting step. Provider setup is
documented in [Communication providers](../docs/woml-communication-providers.md).

For immutable infrastructure plans, role-based quorum, durable reviewer
identity, and auditable evidence, use the
[`richApprovals/`](richApprovals/) Terraform and AI-remediation examples.
For the complete operator-policy boundary—reviewed artifacts, quorum, a fenced
deployment lease, pre-effect authorization, verification, and compensation—use
[`infrastructurePolicies/`](infrastructurePolicies/).

## 4. Reuse code

| Feature | Example |
| --- | --- |
| Local TypeScript module | [`moduleWorkflow.woml`](moduleWorkflow.woml) and [`modules/spreadsheet.ts`](modules/spreadsheet.ts) |
| Reusable custom step and console provider | [`reusableConsoleDemo/`](reusableConsoleDemo/) |
| Synchronous child result | [`workflowCallManual/`](workflowCallManual/) |
| Background child start | [`workflowStartManual/`](workflowStartManual/) |

Load parent and child workflow files together:

```bash
woml run \
  examples/workflowCallManual/workflow1.woml \
  examples/workflowCallManual/workflow2.woml
```

`services.workflows.call()` waits for the child result.
`services.workflows.start()` returns a durable child run ID while the parent
continues.

## 5. Use built-in services

| Service | Example |
| --- | --- |
| Native Fetch and managed HTTP | [`httpComparisonWorkflow.woml`](httpComparisonWorkflow.woml) |
| SQLite | [`sqliteWorkflow.woml`](sqliteWorkflow.woml) |
| PostgreSQL | [`postgresWorkflow.woml`](postgresWorkflow.woml) |
| Durable object storage | [`storageWorkflow.woml`](storageWorkflow.woml) |
| Expiring cache | [`cacheWorkflow.woml`](cacheWorkflow.woml) |
| Durable user state | [`durableStateWorkflow.woml`](durableStateWorkflow.woml) |
| Immutable files, plans, reports, backups, and diagnostics | [`durableArtifacts/`](durableArtifacts/) |
| Internal event fan-out | [`internalEvents/`](internalEvents/) |
| Supervised local and infrastructure tools | [`process/`](process/) |
| Live output, progress, replay, and governed transcript rollover | [`streamingOutput/`](streamingOutput/) |

Database, network, and provider examples may create local data or call external
systems. Read the linked service guide in the [documentation map](../docs/README.md)
before executing them.

Process examples are disabled until the operator explicitly grants an alias:

```bash
woml test examples/process/project-checks.woml --allow-process bun
woml test examples/process/kubernetes-rollout-check.woml --allow-process kubectl
```

Use [`process/process-policy.example.json`](process/process-policy.example.json)
as a production-policy template after replacing its executable paths. Read the
[Process guide](../docs/woml-process.md) before enabling background execution.

## 6. Operate a complete deployment

[`infrastructureAgent/`](infrastructureAgent/) is the capstone project: one
readable workflow composes secure processes, artifacts, waits, locks,
compensation, rich approval, infrastructure policy, streaming output,
OpenTelemetry, and remote placement. Its default adapters are deterministic,
offline, and harmless.

```bash
woml check --project examples/infrastructureAgent/woml.project.json
```

To connect WOML to an OpenTelemetry Collector and inspect the same run in
Jaeger, Prometheus, collector logs, and `woml inspect`, use the copy-ready
[`openTelemetry/`](openTelemetry/) stack and workflow family.

[`production/complete/`](production/complete/) combines workflows, a local
module, runtime configuration, workflow calls, webhook/event ingress, schedule,
runtime policy, and operator instructions. Deployment assets under
[`production/deployment/`](production/deployment/) cover systemd, Docker,
reverse proxy, Kubernetes, and alerting examples.

Start with a safe preflight:

```bash
woml check examples/production/complete/workflows/ \
  --config examples/production/complete/woml.runtime.json
```

Then follow the [production deployment guide](../docs/woml-production-deployment.md).

## Secrets and local state

Examples contain symbolic references such as
`{{secrets.TELEGRAM_BOT_TOKEN}}`, never real credentials. Configure values with
`woml secrets set <NAME>` and do not commit `.woml/`, databases, generated
backups, or local secret files.
# Try examples as a project

`woml check ./examples --json` validates the complete example collection. For a
new automation, run `woml init`, copy the closest example beneath the generated
`workflows/` directory, and run `woml check` again. Examples never contain live
credentials; resolve symbolic secrets through the local operator environment.

For a complete local-to-remote path, use the
[infrastructure-agent tutorial](../docs/infrastructure-agent-tutorial.md).
