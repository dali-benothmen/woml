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
| Workflow/step lifecycle hooks | [`lifecycleWorkflow.woml`](lifecycleWorkflow.woml) | `woml run examples/lifecycleWorkflow.woml` |

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
| Internal event fan-out | [`internalEvents/`](internalEvents/) |

Database, network, and provider examples may create local data or call external
systems. Read the linked service guide in the [documentation map](../docs/README.md)
before executing them.

## 6. Operate a complete deployment

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
