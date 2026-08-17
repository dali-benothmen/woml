# WOML Documentation

Choose the shortest path for what you are doing. You do not need the protocol
or schema documents to write and run a workflow.

## Start here

1. [Getting started](getting-started.md) — install WOML and run the first
   keyboard-triggered workflow.
2. [Examples](../examples/README.md) — progress from hello to production
   deployment.
3. [Language reference](language-reference.md) — exact v1.0 elements,
   attributes, bindings, references, and validation rules.
4. [CLI reference](cli-reference.md) — every public command and option.

## Build workflows

| Topic | Guide |
| --- | --- |
| Triggers and ingress | [Production triggers](woml-production-triggers.md) |
| Terminal output and manual triggers | [Terminal experience](woml-terminal-experience.md) |
| Lifecycle and cancellation | [Lifecycle and run control](woml-lifecycle-and-run-control.md) |
| Approvals and notifications | [Notifications](woml-notifications.md) |
| Services overview | [Services](woml-services.md) |
| HTTP | [Outbound HTTP](woml-http-services.md) |
| SQL databases | [Database](woml-database.md) |
| Durable objects | [Storage](woml-storage.md) |
| Temporary acceleration | [Cache](woml-cache.md) |
| Durable workflow memory | [Durable state](woml-durable-state.md) |
| Internal workflow events | [Events service](woml-events-service.md) |
| Calling/starting workflows | [Workflow calls](woml-workflow-calls.md) |
| JavaScript/TypeScript modules | [Modules](woml-modules.md) |
| Reusable steps/providers | [Reusable definitions](woml-reusable-definitions.md) |
| Runtime policies | [Runtime policies](woml-runtime-policies.md) |
| Communication adapters | [Communication providers](woml-communication-providers.md) |

## Operate WOML

| Topic | Guide |
| --- | --- |
| Runtime configuration and ownership | [Production runtime](woml-production-runtime.md) |
| VPS, Docker, proxy, and Kubernetes | [Production deployment](woml-production-deployment.md) |
| Logs, metrics, and terminal inspection | [Observability](woml-observability.md) |
| Recovery behavior | [Recovery](woml-recovery.md) |
| Backup and restore | [Backup and restore](woml-backup-and-restore.md) |
| Retention and pruning | [Retention and maintenance](woml-retention-and-maintenance.md) |
| Secrets and local data | [Data security](woml-data-security.md) |
| Choosing storage/state/cache/database | [Data guide](woml-data-guide.md) |

Provider-specific setup is available for [Telegram](telegram.md),
[Discord](discord.md), and [WhatsApp](whatsapp.md).

## Contribute and maintain

- [Architecture](architecture.md)
- [Contributing](../CONTRIBUTING.md)
- [Security policy](../SECURITY.md)
- [Support](../SUPPORT.md)
- [WOML 1.0.0 release notes](release/v1.0.0-release-notes.md)
- [Release runbook](woml-release.md)
- [Provider extension direction](provider-extension-architecture.md)

Versioned documents under [`protocols/`](protocols/) and machine-readable
contracts under [`schemas/`](schemas/) are implementation and compatibility
artifacts. They are intentionally retained across product releases even when a
newer protocol version becomes active.
