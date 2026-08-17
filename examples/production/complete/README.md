# Complete production deployment

This source set demonstrates the complete Production Runtime v1 surface in one
deployment: authenticated webhook and event ingress, Slack ingress and approval,
schedule, retry, local module, Workflow Call, runtime policy, lifecycle hooks,
and durable State v1.

Configure `ORDER_WEBHOOK_TOKEN`, `EVENT_CONTROL_TOKEN`, `SLACK_BOT_TOKEN`, and
`SLACK_APP_TOKEN` through a supported WOML secret provider. Create and invite
the Slack app to `#woml-operations` and `#woml-approvals`, then run:

```bash
woml check workflows/ --config woml.runtime.json
woml run workflows/ --config woml.runtime.json
```

The risk workflow is call-only; it is loaded with the other files and cannot be
triggered directly. The example is intentionally infrastructure-neutral. Use
the deployment recipes in `docs/woml-production-deployment.md` for systemd,
Docker, reverse proxy, or single-pod Kubernetes.
