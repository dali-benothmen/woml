# Operating WOML Production Triggers

Production Triggers turn a compiled WOML definition into a long-lived
automation. The supported trigger types are webhook, Slack, schedule, interval,
and named event. A foreground manual trigger waits for keyboard input; it does
not create a startup run. One-shot manual journeys use `woml test`.

The exact trigger headers, activation instructions, manual selection, and
status output are documented in
[WOML Terminal Experience](woml-terminal-experience.md).

## Start an automation

Activate one workflow file, several explicit files/directories, or every direct
`.woml` file in a directory:

```bash
woml run workflows/ \
  --host 127.0.0.1 \
  --port 3000 \
  --state .woml/state.sqlite
```

```bash
woml run workflows/orders.woml workflows/risk.woml
```

Multiple inputs are validated and activated as one runtime unit. Repeated file
paths are deduplicated, directory loading is non-recursive, and duplicate
workflow IDs fail before any workflow starts.

`woml run` validates all definitions and required secrets before it becomes
ready. It then keeps webhook and event endpoints, Slack Socket Mode connections,
and Rust-owned schedulers active until SIGINT or SIGTERM. Completing a workflow
run does not stop the automation.

Use `woml test workflow.woml` only for an intentional one-shot manual run.

## Supported trigger behavior

| Trigger | Source | Durable duplicate identity | `context.payload` |
|---|---|---|---|
| Webhook | HTTP POST | `Idempotency-Key`, or a fresh identity | Validated request body |
| Slack | App mention or direct message | Workspace and Slack event identity | Normalized conversation fields |
| Schedule | Five-field WOML cron and IANA timezone | Planned UTC instant | `{ scheduledAt, triggeredAt }` |
| Interval | Fixed-rate duration from `1s` through `30d` | Durable anchor and sequence | `{ scheduledAt, triggeredAt }` |
| Event | Authenticated named publication | Publisher event ID per subscriber | Validated event data |

All five enter the same Rust-owned occurrence boundary. Acceptance atomically
creates an immutable occurrence, a run bound to its compiled definition, and
the first run event. A repeated identity with the same payload returns the
original run; a changed payload conflicts.

## Secrets

Declare symbolic credentials only in reviewed trigger attributes:

```xml
<webhook
  id="order"
  path="/orders"
  auth="bearer"
  secret="{{secrets.ORDER_WEBHOOK_TOKEN}}"
/>

<slack
  id="agentMessage"
  events="app-mention,direct-message"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>

<event
  id="orderCreated"
  name="order.created"
  secret="{{secrets.EVENT_CONTROL_TOKEN}}"
/>
```

Configure each referenced name from the project directory:

```bash
woml secrets set ORDER_WEBHOOK_TOKEN
woml secrets set SLACK_BOT_TOKEN
woml secrets set SLACK_APP_TOKEN
woml secrets set EVENT_CONTROL_TOKEN
```

At activation, WOML resolves only names referenced by loaded definitions.
Secret values remain in memory, are reduced to credential digests where
possible, and never enter compiled definitions, workflow context, run events,
terminal output, or SQLite.

## HTTP ingress

Webhook and event triggers share the configured HTTP listener. Keep the default
loopback bind behind a trusted TLS reverse proxy for internet-facing use.

Webhook acceptance returns HTTP 202 with a durable `runId`; it does not wait for
workflow completion. Named event publication returns HTTP 200 with one delivery
result per exact-name subscriber, while accepted runs continue asynchronously.
Both transports require a top-level JSON object and enforce a 1 MiB body limit.

The CLI prints safe copy-pasteable `curl` examples at startup. For named events,
an application may call the printed endpoint directly or use the optional CLI
publisher client:

```bash
woml emit order.created \
  --id order-42-created \
  --data @order-created.json \
  --server http://127.0.0.1:3000 \
  --token-secret EVENT_CONTROL_TOKEN
```

`woml emit` names the stored secret because it does not load a workflow file.
Applications use the same HTTP contract with their own secret injection.

## Slack ingress

Slack triggers use Socket Mode. The app requires an app-level token with
`connections:write`, a bot token with the scopes listed in
`woml-cli/slack/manifest.json`, and enabled event subscriptions. Invite the bot
to filtered public or private channels.

WOML acknowledges an event envelope only after Rust durably accepts it. Slack
redelivery therefore resolves to the same run. Bot/self messages, edits,
deletes, unsupported subtypes, and unmatched channels are ignored without
entering workflow context.

## Schedule and interval operation

Rust owns time decisions and durable cursors. Schedule uses the frozen numeric
five-field cron dialect and canonical IANA timezones. Interval uses an anchored
fixed-rate grid, so slow workflow execution does not shift future planned
instants.

`on-missed="skip"` advances beyond elapsed occurrences. `run-once` creates at
most one recovery occurrence for the latest missed instant. Neither policy
creates an unbounded catch-up storm.

## State, restart, and definition changes

SQLite is the execution authority. Keep the database and its WAL/SHM files on
persistent local storage, restrict filesystem access, and back them up as one
consistent unit. Do not place the database on a shared network filesystem.

Every run remains bound to the exact compiled definition with which it began.
Changing a `.woml` file creates a new definition for future occurrences; it
does not rebind old runs or old duplicate identities. Historical Model v7 event
definitions without the later publisher-secret field remain readable for
recovery, but cannot open a new unauthenticated publisher route.

On restart:

- a committed but undispatched occurrence continues its existing run;
- completed steps and occurrences are not repeated;
- a safely scheduled retry resumes at its recorded time;
- an attempt that started without a terminal outcome fails closed as
  `interrupted`; and
- waiting Human Approval runs remain waiting.

Inspect a run without changing it:

```bash
woml get run_... --state .woml/state.sqlite
```

## Shutdown and single-node boundary

SIGINT and SIGTERM stop new admission, close provider connections and the HTTP
listener, and join the Rust runtime. Already committed state remains available
for restart recovery.

Production Triggers T13 is a durable single-node profile. Workflow Calls v1 may
intentionally use multiple local processes against one SQLite file when each
workflow ID has exactly one owner; that narrow call-routing profile does not
turn trigger schedulers into distributed schedulers. Do not run multiple owners
for one trigger workflow or claim distributed exactly-once scheduling.
Multi-node leases, queues, retention, and hosted administration belong to the
Production Runtime roadmap milestone.

## Troubleshooting

| Symptom | Action |
|---|---|
| `WOML_SECRET_NOT_FOUND` | Set the exact symbolic name shown in the authored trigger using `woml secrets set NAME`. |
| `WOML_WEBHOOK_ROUTE_CONFLICT` | Give every loaded webhook an exclusive path. `/_woml` is reserved. |
| `WOML_TRIGGER_UNAUTHORIZED` | Verify the caller uses the token referenced by that webhook or event trigger. |
| `WOML_TRIGGER_SCHEMA_INVALID` | Compare the JSON object with the trigger's inline Draft 2020-12 schema. |
| `WOML_TRIGGER_IDEMPOTENCY_CONFLICT` | Do not reuse one webhook key or event ID for changed data. |
| `WOML_TRIGGER_UNAVAILABLE` | Check state-file permissions, free disk space, SQLite ownership, and whether another process holds the database. |
| Slack connects but messages do nothing | Enable event subscriptions, verify app mention/DM subscriptions, channel filters, scopes, and bot membership. |
| Schedule or interval did not catch up repeatedly | This is intentional: `skip` creates none and `run-once` creates at most one missed occurrence. |
| A run failed as `interrupted` after a crash | WOML cannot prove the effect did not happen and intentionally refuses to replay it. |
| Port binding fails | Stop the other listener or choose a different `--port`; keep one owner per state database. |

Diagnostics never include raw request bodies rejected before admission or
resolved credentials. Preserve stderr, the run ID, and the state database when
investigating a failure.

## Runtime Policy backpressure

When a triggered workflow contains `<config>`, trigger admission enters the
same Rust-owned durable queue used by manual runs and Workflow Calls. Webhooks
return HTTP 503 plus `Retry-After` at the 10,000-run safety ceiling; Slack is
left unacknowledged; schedule/interval cursors do not advance; and named-event
subscriber delivery remains retryable. See
[WOML Runtime Policies](woml-runtime-policies.md) for the complete policy and
operator contract.

## Deployment checklist

1. Validate every workflow and secret in the deployment environment.
2. Use an explicit persistent `--state` path with restricted permissions.
3. Keep HTTP on loopback behind TLS ingress unless public binding is deliberate.
4. Use authenticated webhooks and events for untrusted networks.
5. Give Slack only the required scopes and enable the required subscriptions.
6. Configure a supervisor to restart `woml run` and send SIGTERM on deployment.
7. Back up SQLite consistently and test recovery using a copy.
8. Use source idempotency keys and `attempt.idempotencyKey` with external
   services that support deduplication.
9. Restrict outbound HTTP with an egress policy when workflow or trigger data
   can influence destinations; the local profile is not an SSRF sandbox.
10. Monitor readiness, rejection, run-terminal, scheduler, provider, and
    managed-service messages.
11. Run `bun run test:sc6` from `woml-cli` before publishing a build.
12. When using Workflow Calls across local processes, give every process the
    exact same persistent state path, keep one owner per workflow ID, and apply
    `docs/woml-workflow-calls-production.md`.
