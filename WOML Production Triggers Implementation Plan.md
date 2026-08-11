# WOML Production Triggers Implementation Plan

Status: T0 through T13 completed on 2026-08-08. Production Triggers is a
complete publishable single-node milestone. Manual, webhook, Slack, cron
schedule, fixed-rate interval, and named event triggers are active. Rust owns atomic
occurrence admission, HTTP validation, durable run creation, background DAG
execution, the injected clock, SQLite schedule and interval cursors, bounded
misfire recovery, and process-crash recovery. `woml run` stays alive, while
`woml test` owns one-shot execution and `woml get` inspects durable
results. There is no separate public `woml serve` mode.

Named event syntax, Model v7 lowering, Event Publication v1, the authenticated
publisher endpoint, deterministic runtime fan-out, and `woml emit` are active.

## 1. Product Outcome

This milestone turns WOML workflows from commands that a person starts manually
into workflows that can start from real production activity.

After it is complete, an author can declare:

- an HTTP webhook that starts a run when a JSON request arrives;
- a Slack message trigger for app mentions and direct messages;
- a cron schedule evaluated in a named timezone;
- a fixed interval that survives process restarts without drifting; and
- a named application event that may fan out to multiple workflows.

The features are delivered in that order. Webhook becomes independently
publishable first. Slack follows while its existing provider transport is still
a small, well-understood reuse target. Schedule, interval, and event then build
on the same durable occurrence boundary. We do not hold a useful webhook or
Slack release until all five trigger types are complete.

The first acceptance journey is:

```bash
woml run examples/webhookWorkflow.woml --host 127.0.0.1 --port 3000
```

followed by:

```bash
curl --request POST http://127.0.0.1:3000/webhooks/orders \
  --header 'Authorization: Bearer <token>' \
  --header 'Content-Type: application/json' \
  --header 'Idempotency-Key: order-delivery-123' \
  --data '{"orderId":"order-42"}'
```

The request receives a durable run identity immediately:

```json
{
  "runId": "run_...",
  "status": "accepted",
  "duplicate": false
}
```

The workflow executes through the same Rust DAG engine, event log, retry,
branch, parallel, approval, and notification runtime already used by
`woml run`.

The Slack acceptance journey starts the same long-lived runner with
`examples/slackTriggerWorkflow.woml`, then a workspace member mentions `@WOML`
or sends the bot a direct message. The CLI reports the accepted trigger/run ID,
and `woml get` exposes its terminal state. This milestone makes Slack the
agent's inbound conversation channel. The outbound conversational reply is a
separate `slack.send` capability on the next Services and Capabilities roadmap;
keeping that effect explicit avoids hiding message sends inside trigger logic.

## 2. `woml run` Is the Long-Lived Automation Runtime

WOML is workflow automation, so its primary command must activate a workflow,
not merely execute one occurrence and exit. A triggered run is one execution of
the DAG; the WOML runtime is the long-lived process that remains ready for the
next webhook, Slack message, schedule, interval, event, or manual occurrence.

`woml run` therefore:

1. compiles and validates the supplied WOML definitions;
2. asks Rust to register their trigger contracts;
3. opens the configured HTTP/provider listeners and durable scheduler;
4. accepts occurrences and creates runs atomically; and
5. stays alive until it receives a shutdown signal.

There is no separate public `woml serve` command. That distinction would expose
an infrastructure concept without giving the workflow author useful product
value.

For compatibility with the current manual workflow journey, a selected
`<manual>` trigger fires once when `woml run` activates the definition. The
process remains alive after that occurrence finishes. A separate `woml test`
command performs one selected manual occurrence and exits; it is for local
experimentation, CI, and debugging, not production activation.

`--watch` is not the mechanism that keeps automation alive. When added later,
it means only “reload changed WOML source while the already-long-lived runtime
continues operating.” In a future hosted WOML product, the platform owns this
same activation lifecycle and the terminal command may be invisible.

## 3. Baseline Entering This Milestone

At the start of this milestone, the executable WOML frontend requires exactly
one `<manual>` trigger. `woml run` compiles it, asks the Rust core to execute one
durable run, and exits. T4 deliberately replaces that command lifecycle with
the long-lived activation behavior defined above.

The language catalog already describes webhook, schedule, interval, and event
tags, but the frontend rejects them and the Rust executable profile accepts
only `trigger.manual`. Slack currently exists only as an approval notification
provider, not as a workflow trigger. The older Cronflow files under
`core/src/triggers.rs`, `core/src/webhook_server.rs`, and the JavaScript SDK are
migration evidence; they are not the WOML production-trigger implementation and
must not create a second execution path.

The new trigger runtime must converge on the current WOML architecture:

```text
.woml
  -> TypeScript parse / validate / lower
  -> versioned compiled Model v7
  -> Rust native listeners or a normalized provider adapter
  -> versioned Rust Trigger Ingress v1
  -> Rust occurrence authority
  -> atomic occurrence + run_started persistence
  -> existing Rust DAG execution loop
  -> isolated Bun Workers for <script>
```

Generic HTTP and time-based trigger ownership stays in Rust. Slack's existing
Bun transport may retain the Socket Mode and Slack Web API integration, but it
must normalize inbound events and hand them to Rust through Trigger Ingress v1.
It may not create runs, deduplicate occurrences, or execute the DAG itself.

## 4. What “Done” Means

Production Triggers are complete when:

1. One workflow can declare one or more supported triggers and every trigger
   starts the same compiled DAG.
2. Accepted external or time-based occurrences are durable before execution.
3. Exactly one run is created for one deduplicated trigger occurrence.
4. Trigger identity is recorded in the event history without changing the
   public `context.payload` payload shape.
5. Webhook schema validation, authentication, size limits, and helpful HTTP
   errors happen before a run is created.
6. Slack reconnects and duplicate Socket Mode deliveries create one durable run
   per Slack event, and bot/self messages cannot create feedback loops.
7. Schedule and interval cursors survive restart and never create the same
   planned occurrence twice.
8. Event publication fans out deterministically and isolates one subscriber's
   failure from other subscribers.
9. Rust owns occurrence admission, deduplication, scheduling, run creation, and
   recovery. Provider adapters do not decide whether a trigger starts a run.
10. `woml run` exposes source-aware startup errors, clear readiness output,
    secret-safe progress for many occurrences, and graceful shutdown.
11. Older manual-only Model/Event v1–v6 definitions and run semantics remain
    compatible; their former one-shot CLI journey moves to `woml test`.
12. Each trigger type passes its clean-package acceptance journey before being
    advertised as executable.

## 5. Scope

### Included

- Multiple trigger definitions in one workflow.
- Existing `<manual>` behavior and explicit manual-trigger selection when a
  workflow declares more than one manual trigger.
- `<webhook>` with a static path, POST JSON body, optional JSON Schema, explicit
  authentication mode, and standard request idempotency.
- `<slack>` with app-mention/direct-message event selection, channel filters,
  symbolic credentials, reconnect handling, and Slack event deduplication.
- `<schedule>` with a frozen five-field cron dialect and IANA timezone.
- `<interval>` with durable fixed-rate semantics.
- `<event>` with a named event, JSON payload, optional JSON Schema, and durable
  fan-out through a versioned engine ingress API.
- A single-node durable trigger-occurrence authority in Rust.
- Long-lived `woml run`, one-shot `woml test`, read-only run inspection, and a
  local/application event publish command.
- Versioned model, event, ingress, occurrence, and HTTP response contracts.
- Restart recovery, deduplication, diagnostics, security, packaging, and docs.

### Not included

- Multi-node leader election, distributed scheduler leases, or active-active
  trigger ownership. These belong to Production Runtime.
- Built-in TLS certificate management. The first profile binds localhost by
  default and expects a production reverse proxy or platform ingress for TLS.
- Dynamic route parameters, wildcards, query-to-JSON coercion, multipart bodies,
  or arbitrary request headers in workflow context.
- Synchronous webhooks that hold the HTTP request open until workflow
  completion.
- Provider-specific signature schemes such as Stripe, GitHub, or Shopify. They
  can later compile to the same ingress contract.
- Discord, WhatsApp, Telegram, and other communication-provider triggers. Slack
  is the first provider trigger and establishes the adapter contract they reuse.
- A general outbound Slack message/reply operation. A Slack trigger is inbound;
  the conversational response operation belongs to Services and Capabilities.
  The trigger payload deliberately includes safe channel/thread identifiers so
  that future operation can reply without changing this trigger contract.
- Kafka, SQS, NATS, Redis, or other broker adapters. The first event profile
  freezes the engine boundary those adapters will call.
- Hot reload of workflow definitions. Restarting `woml run` activates a new
  immutable definition; existing runs stay bound to the old hash. A future
  `woml run --watch` may automate that reload without changing the meaning of
  plain `woml run`.
- Workflow-level concurrency, rate limiting, queues, cancellation, and general
  lifecycle hooks. Those remain in Lifecycle and Engine Controls.
- Exposing request credentials, headers, internal occurrence records, or
  `context.run` to scripts.

## 6. Proposed Authoring Contract

These shapes are proposed for review and become frozen in T0. The frontend must
continue rejecting a trigger type until the phase that makes that type
executable; documented syntax alone is not runtime support.

### 6.1 Manual

```xml
<manual id="manualRun" />
```

Manual behavior remains backward compatible. If a definition contains multiple
manual triggers, the author selects one explicitly:

```bash
woml run workflow.woml --trigger manualRun
```

Omitting `--trigger` remains valid when exactly one manual trigger exists. That
manual occurrence fires once during activation; after it finishes, `woml run`
stays ready for every other configured trigger and for clean shutdown. For a
single execution that exits, the author uses:

```bash
woml test workflow.woml --trigger manualRun
```

### 6.2 Webhook

```xml
<webhook
  id="newOrder"
  path="/webhooks/orders"
  method="POST"
  auth="bearer"
  secret="{{secrets.ORDER_WEBHOOK_TOKEN}}">
  <schema>
    {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "type": "object",
      "required": ["orderId"],
      "properties": {
        "orderId": { "type": "string" }
      },
      "additionalProperties": false
    }
  </schema>
</webhook>
```

| Attribute | Required | First executable profile |
|---|---:|---|
| `id` | Yes | Stable trigger identity. |
| `path` | Yes | Exact absolute path beginning with `/`; no parameters or wildcards. |
| `method` | No | Defaults to `POST`; v1 executes POST only. Other designed methods remain rejected. |
| `auth` | Yes | `bearer` or explicit `none`. Requiring a value prevents accidentally publishing an unprotected endpoint. |
| `secret` | For bearer | Exact `{{secrets.NAME}}` reference; forbidden with `auth="none"`. |

Webhook v1 accepts an `application/json` top-level object no larger than 1 MiB.
An empty body, array, scalar, malformed JSON, or unsupported content type is
rejected before a run exists. Headers, query parameters, cookies, and bearer
credentials do not enter `context.payload`.

`auth="bearer"` compares the presented token in constant time. Symbolic secret
names may appear in the compiled registration contract, but resolved values are
never placed in the model, event log, SQLite occurrence records, context,
diagnostics, or access logs. Missing secrets fail startup before the port binds.

`auth="none"` is allowed for deliberate public endpoints, local testing, or a
trusted reverse proxy. `woml run` prints a visible warning for each such route.

### 6.3 Slack

```xml
<slack
  id="agentMessage"
  events="app-mention,direct-message"
  channels="woml-testing,agent-support"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>
```

| Attribute | Required | First executable profile |
|---|---:|---|
| `id` | Yes | Stable trigger identity. |
| `events` | Yes | Comma-separated subset of `app-mention` and `direct-message`. |
| `channels` | No | Comma-separated channel names accepted for app mentions; omission accepts every visible channel. |
| `bot-token` | Yes | Exact symbolic bot-token secret reference. |
| `app-token` | Yes | Exact symbolic Socket Mode app-token secret reference. |

The first profile deliberately does not listen to every channel message. It
accepts only a new human app mention or direct message, which avoids turning an
ordinary busy channel into an accidental workflow queue. Bot messages, the
WOML bot's own messages, edits, deletes, and unsupported Slack subtypes are
ignored and cannot form reply loops.

The normalized trigger payload is stable and intentionally smaller than
Slack's raw event envelope:

```json
{
  "type": "app-mention",
  "text": "@WOML summarize order 42",
  "userId": "U123",
  "channelId": "C123",
  "messageTs": "1710000000.000100",
  "threadTs": "1710000000.000100",
  "teamId": "T123"
}
```

For a threaded message, `threadTs` preserves the existing thread; otherwise it
equals `messageTs`. These routing identifiers are safe workflow input and allow
a future `slack.send` capability to answer in the same conversation. OAuth
tokens, Socket envelopes, retry counters, authorization headers, and unreviewed
Slack fields never enter context.

The Slack app manifest must add the event subscriptions and minimum bot scopes
required by the selected trigger features, including `app_mentions:read` for
mentions and `im:history` for direct messages. Existing channel lookup and
Socket Mode scopes remain reused. The CLI preflight explains when the app needs
to be reinstalled after its scopes change.

### 6.4 Reusing the Existing Slack Provider

The approval provider already has valuable production behavior in
`woml-cli/src/notification-provider/real-slack.ts`: symbolic credential
resolution, channel lookup/cache, one Socket Mode connection per workspace,
reconnect handling, Slack Web API calls, error classification, and
secret-safe diagnostics.

We should extract those transport primitives into a shared Slack transport
module used by both notification delivery and trigger ingestion. We must not
make the approval provider protocol pretend to be a trigger protocol. The two
features share transport and credentials but retain separate versioned message
contracts:

```text
shared Slack transport
  -> approval adapter -> Notification Provider Protocol -> Rust approval core
  -> trigger adapter  -> Slack Trigger Protocol v1 -> Trigger Ingress v1
                         -> Rust occurrence core
```

Under `woml run`, one long-lived Slack transport host owns each workspace
connection and routes separate approval-adapter and trigger-adapter messages.
This is what makes connection reuse real rather than merely sharing utility
functions in two processes.

Slack Socket Mode envelopes are acknowledged only after Rust durably accepts or
recognizes the occurrence, while staying within Slack's acknowledgement
deadline. If Rust is unavailable, the adapter does not acknowledge and permits
Slack's redelivery behavior. Slack's outer `event_id`, combined with workspace
and trigger identity, is the durable deduplication source.

### 6.5 Schedule

```xml
<schedule
  id="dailyReport"
  cron="0 9 * * *"
  timezone="Europe/Berlin"
  on-missed="skip"
/>
```

| Attribute | Required | First executable profile |
|---|---:|---|
| `id` | Yes | Stable trigger identity. |
| `cron` | Yes | POSIX-style five-field minute/hour/day-of-month/month/day-of-week expression. |
| `timezone` | No | IANA timezone; defaults to `UTC`. |
| `on-missed` | No | `skip` or `run-once`; defaults to `skip`. |

The first profile has minute precision and no seconds field or nonstandard
macros. A nonexistent daylight-saving wall time is skipped. If a wall time
occurs twice, its two distinct UTC instants are distinct occurrences.

On restart, `skip` advances to the next future occurrence without creating a
run for elapsed times. `run-once` creates at most one recovery occurrence for
all elapsed times, using the latest missed scheduled instant. It never creates
an unbounded catch-up storm.

### 6.6 Interval

```xml
<interval
  id="refreshCache"
  every="5m"
  on-missed="skip"
/>
```

| Attribute | Required | First executable profile |
|---|---:|---|
| `id` | Yes | Stable trigger identity. |
| `every` | Yes | Positive whole duration from `1s` through `30d`. |
| `on-missed` | No | `skip` or `run-once`; defaults to `skip`. |

An interval is fixed-rate, not “wait this long after the previous run
finishes.” The first registration stores a durable UTC anchor. Every planned
time is `anchor + sequence × every`, so restart and long-running workflows do
not silently move the grid. Missed-run behavior matches schedule.

### 6.7 Event

```xml
<event
  id="orderCreated"
  name="order.created"
  secret="{{secrets.EVENT_CONTROL_TOKEN}}"
>
  <schema>
    {
      "type": "object",
      "required": ["orderId"],
      "properties": {
        "orderId": { "type": "string" }
      }
    }
  </schema>
</event>
```

| Attribute | Required | First executable profile |
|---|---:|---|
| `id` | Yes | Stable trigger identity. |
| `name` | Yes | Dot-separated event name such as `order.created`. |
| `secret` | No | Optional public HTTP publisher credential, written as `{{secrets.NAME}}`. SC11 allows omission for internal-only events. |

`<event>` may contain at most one inline Draft 2020-12 `<schema>`. One published
event carries a required publisher event ID and a top-level JSON object. Every
matching trigger receives the same immutable payload and creates its own run.
One workflow's validation or execution failure does not roll back another
workflow's accepted occurrence.

The first built-in public publisher uses a versioned authenticated engine API
and a CLI client:

```bash
woml secrets set EVENT_CONTROL_TOKEN

woml emit order.created \
  --id event_123 \
  --data @order-created.json \
  --server http://127.0.0.1:3000 \
  --token-secret EVENT_CONTROL_TOKEN
```

`woml run` reads each present symbolic reference and resolves only that named
value from the existing WOML secret store. An event without `secret` opens no
public endpoint and remains available to `services.events.emit()`. No secret value enters
the source or compiled model. Future broker adapters call the same Rust ingress
operation and must not bypass occurrence deduplication.

## 7. `context.payload` Contract

Production triggers do not introduce `context.run` and do not wrap existing
payloads in a new envelope.

| Trigger | `context.payload` value |
|---|---|
| Manual | The manual JSON input object. |
| Webhook | The validated JSON request body object. |
| Slack | The normalized message object containing type, text, user, channel, message/thread, and workspace IDs. |
| Schedule | `{ scheduledAt, triggeredAt }` using RFC 3339 UTC timestamps. |
| Interval | `{ scheduledAt, triggeredAt }` using RFC 3339 UTC timestamps. |
| Event | The validated published event payload object. |

Trigger ID, handler name, occurrence ID, authentication data, HTTP metadata,
and scheduler cursor are engine metadata. They are recorded where required for
durability but are not exposed through `context.payload`.

## 8. Versioned Runtime Contracts

T0 freezes these artifacts before implementation:

1. **Compiled Workflow Model v7** — keeps the DAG unchanged and activates a
   strict tagged union for `trigger.manual`, `trigger.webhook`, `trigger.slack`,
   `trigger.schedule`, `trigger.interval`, and `trigger.event` configs.
2. **Run Event v7** — extends `run_started` with `triggerId`, `triggerHandler`,
   and `triggerOccurrenceId` while retaining the direct `trigger` payload used
   to fold `context.payload`.
3. **Trigger Occurrence v1** — the durable record that binds one accepted
   occurrence to one run and supports deduplication before execution starts.
4. **Trigger Ingress v1** — the language-neutral call from listeners,
   schedulers, CLI publishing, and future adapters into the Rust authority.
5. **Webhook HTTP v1** — success and error response envelopes, status mapping,
   size-limit failures, authentication failures, and duplicate behavior.
6. **Slack Trigger Protocol v1** — normalized inbound Slack occurrence,
   acknowledgement result, connection lifecycle, size limit, and safe failure
   shapes between the provider adapter and Rust ingress.
7. **Trigger Progress v1** — secret-safe readiness, occurrence acceptance,
   duplicate, rejection-summary, run-start, and run-terminal messages for the
   CLI.
8. **Event Publication v1** — authenticated publisher intent, deterministic
   subscriber ordering, per-subscriber accepted/rejected results, duplicate and
   conflict behavior, and bounded fan-out.
9. **Event Publisher HTTP v1** — reserved route, required headers, public
   response envelope, request-level errors, and transport status mapping.

Model v1–v6 and Event v1–v6 remain immutable. A Model v7 run uses Event v7 for
its complete history; one run never mixes versions.

## 9. Durable Occurrence and Deduplication Model

Every accepted occurrence crosses one Rust transaction boundary:

```text
validate admission
  -> insert or resolve trigger occurrence
  -> create immutable run bound to definition hash
  -> append run_started v7
  -> commit
  -> dispatch existing DAG
```

There is no state where an occurrence is acknowledged but its run identity was
not committed. If the process crashes after commit and before dispatch, normal
event-fold recovery continues the run.

### 9.1 Source identities

| Trigger | Deduplication source |
|---|---|
| Manual | New generated occurrence ID for every explicit invocation. |
| Webhook | `Idempotency-Key` header when supplied; otherwise a new generated occurrence. |
| Slack | Slack workspace/team ID + outer Events API `event_id` + workflow ID + trigger ID. |
| Schedule | Workflow ID + trigger ID + planned UTC instant. |
| Interval | Workflow ID + trigger ID + durable anchor sequence. |
| Event | Publisher event ID + workflow ID + matching trigger ID. |

Raw external idempotency keys are hashed before persistence. Repeating one key
with the same payload returns the original `runId` and `duplicate: true` without
starting another run. Repeating the key with a different payload returns
`409 WOML_TRIGGER_IDEMPOTENCY_CONFLICT`.

The frozen v1 derivation is:

```text
sourceIdentityHash = "sha256:" + hex(SHA-256(UTF-8(sourceIdentity)))
payloadHash        = "sha256:" + hex(SHA-256(UTF-8(RFC8785(payload))))
unique key         = (workflowId, triggerId, sourceIdentityHash)
```

Rust creates opaque `occ_...` and `run_...` IDs only for a new row. A duplicate
returns the stored IDs. Manual invocations and webhook requests without an
`Idempotency-Key` receive a fresh cryptographically random source identity
before admission.

The occurrence identity does not include the current definition hash. A
delivery retried after a definition update still resolves to the original run
instead of silently executing the external event against a second definition.

Trigger deduplication prevents duplicate runs. Step-level
`attempt.idempotencyKey` remains a separate identity that protects retryable
effects inside one run.

## 10. Webhook HTTP Behavior

Webhook execution is asynchronous. The listener does not hold a caller's socket
open while scripts, approvals, retries, or notifications execute.

| Situation | HTTP result |
|---|---|
| New durable occurrence | `202 Accepted` with `runId`, `status: accepted`, `duplicate: false`. |
| Same key and same payload | `202 Accepted` with original `runId`, `duplicate: true`. |
| Same key and different payload | `409 WOML_TRIGGER_IDEMPOTENCY_CONFLICT`. |
| Malformed JSON or non-object body | `400 WOML_TRIGGER_PAYLOAD_INVALID`. |
| JSON Schema failure | `400 WOML_TRIGGER_SCHEMA_INVALID` with safe JSON Pointer issues. |
| Missing/invalid bearer token | `401 WOML_TRIGGER_UNAUTHORIZED`. |
| Wrong method | `405 WOML_TRIGGER_METHOD_NOT_ALLOWED`. |
| Body over the frozen limit | `413 WOML_TRIGGER_PAYLOAD_TOO_LARGE`. |
| Unknown route | `404 WOML_TRIGGER_NOT_FOUND`. |
| Durable authority unavailable | `503 WOML_TRIGGER_UNAVAILABLE`; caller may safely retry with the same key. |

Rejected requests create no workflow run and append no run event. Bounded
transport/security logs may record their code, route, time, and request ID, but
never credentials or raw payloads.

## 11. Slack Trigger Behavior

The Slack adapter owns provider-specific transport only. For each configured
workspace it:

1. resolves the existing symbolic bot and app credentials at startup;
2. reuses one shared Socket Mode connection for notification interactions and
   inbound trigger events;
3. validates Slack's envelope and supported event shape;
4. drops bot/self/edited/deleted events before workflow admission;
5. applies the compiled event-type and channel filters;
6. normalizes the safe message payload;
7. submits the occurrence to Rust through Trigger Ingress v1; and
8. acknowledges Slack after Rust commits or identifies the duplicate.

Multiple Slack trigger tags using the same credential pair reuse one connection
and one channel cache. A single Slack event may intentionally match multiple
workflow triggers; each trigger receives its own occurrence identity and run,
while a redelivery to the same trigger resolves to the original run.

If the Socket connection drops, the adapter reconnects with the existing
bounded policy. If an event is received but Rust cannot make a durable admission
decision before the Slack deadline, the adapter leaves it unacknowledged so
Slack may redeliver it. A provider acknowledgement is never treated as a
workflow result.

Approval buttons and messages sent by the existing notification provider
continue to use their notification contracts. Because Slack-trigger ingestion
ignores bot/self messages, approval delivery and future outbound agent replies
cannot recursively trigger the same agent workflow.

## 12. Scheduler Behavior

Rust owns the clock, durable cursor, occurrence claim, and run creation. The Bun
CLI may show progress but cannot decide that a schedule is due.

For each loaded definition, the scheduler:

1. validates cron/timezone or interval duration before readiness;
2. restores or creates the durable cursor;
3. computes the next planned UTC instant deterministically;
4. sleeps only as an optimization, never as authoritative state;
5. atomically claims the occurrence and creates its run when due; and
6. advances the durable cursor before waiting again.

Tests use an injected deterministic clock. Production code must not depend on
wall-clock sleeps for correctness. Clock movement, daylight-saving transitions,
restart, and two triggers becoming due together must not create duplicate or
lost occurrence identities.

The first single-node profile permits runs from different occurrences to
overlap. Durable workflow-level admission, concurrency, rate limits, and queues
are added later under Lifecycle and Engine Controls; the trigger system must
feed that future admission boundary rather than implement a conflicting one.

## 13. CLI Product Surface

### 13.1 Activate a workflow

```text
woml run <workflow.woml|directory> [--host <address>] [--port <port>]
  [--state <path>]
  [--trigger <manualTriggerId>] [--resume <runId>]
  [--approval-port <port>]
```

- Default host: `127.0.0.1`.
- Default state: `.woml/state.sqlite`.
- Event workflows automatically resolve every secret explicitly referenced by
  their `<event secret="{{secrets.NAME}}">` attributes.
- A directory loads its direct `*.woml` files in lexical order.
- Route, workflow-ID, and trigger conflicts fail startup before the port binds.
- Definitions are a static registration snapshot for the process lifetime.
- A selected manual trigger fires once at startup; external and time-based
  triggers remain active afterward.
- Finishing, failing, or pausing one workflow occurrence does not stop the
  runtime or prevent the next occurrence.
- Readiness lists registered webhook routes, Slack workspaces/event filters,
  schedules, intervals, and events without printing resolved secrets.
- Progress output is an ongoing stream keyed by occurrence ID and run ID; one
  terminal result is not presented as the terminal result of the process.
- SIGINT/SIGTERM stops new admission and leaves committed runs recoverable.
  It never pretends an interrupted active script is safe to replay.

Plain `woml run` does not reload changed source. A future `--watch` option may
activate new immutable definitions after validation while already-started runs
remain bound to their original definition hash.

### 13.2 Execute once for testing

```text
woml test <workflow.woml> [--trigger <manualTriggerId>]
  [--state <path>] [--approval-port <port>]
```

This command invokes one manual occurrence, prints its terminal result, and
exits. It preserves a fast playground and CI journey without weakening the
automation meaning of `woml run`. It does not activate webhook, Slack,
schedule, interval, or event listeners.

### 13.3 Inspect a run

```text
woml get <runId> [--state <path>] [--json]
```

This read-only command prints the folded status and, when terminal, the result
or safe failure. It gives webhook and event callers a way to inspect an
asynchronously accepted run without defining a hosted management API in this
milestone.

### 13.4 Emit an event

```text
woml emit <eventName> --id <publisherEventId> --data @<jsonFile>
  --server <url> --token-secret <NAME>
```

The CLI resolves the named token from the existing WOML secret store and does
not accept its value as a command-line argument. Unlike `woml run`, this
publisher command does not load a workflow file, so the symbolic name is
provided explicitly with `--token-secret`.

## 14. Error Surface

All compile and registration failures keep the existing WOML error shape: code,
source file, line, column, and message. Runtime HTTP errors use the versioned
JSON response contract.

The initial diagnostic catalog includes:

| Code | Meaning |
|---|---|
| `WOML_TRIGGER_UNSUPPORTED` | The current executable phase does not support this trigger. |
| `WOML_TRIGGER_ID_DUPLICATE` | A workflow repeats a trigger ID. |
| `WOML_WEBHOOK_PATH_INVALID` | Path is not an exact safe absolute route. |
| `WOML_WEBHOOK_ROUTE_CONFLICT` | Two loaded definitions claim the same method and path. |
| `WOML_WEBHOOK_METHOD_UNSUPPORTED` | Method is designed but not executable in v1. |
| `WOML_WEBHOOK_AUTH_INVALID` | Authentication attributes are missing or contradictory. |
| `WOML_WEBHOOK_SECRET_MISSING` | A symbolic secret cannot be resolved at registration. |
| `WOML_SLACK_TRIGGER_EVENT_INVALID` | Authored event filters or an incoming Slack event are unsupported or malformed. |
| `WOML_SLACK_TRIGGER_EVENT_DUPLICATE` | The `events` list repeats an event name. |
| `WOML_SLACK_TRIGGER_CHANNEL_INVALID` | A channel filter is not a lowercase name or Slack conversation ID. |
| `WOML_SLACK_TRIGGER_CHANNEL_DUPLICATE` | The channel filter repeats a destination. |
| `WOML_SLACK_TRIGGER_SCOPE_MISSING` | The installed Slack app lacks a required trigger scope or subscription. |
| `WOML_SLACK_TRIGGER_UNAVAILABLE` | Socket Mode cannot establish or restore the configured workspace connection. |
| `WOML_TRIGGER_SCHEMA_INVALID` | Inline schema or runtime payload validation failed. |
| `WOML_SCHEDULE_CRON_INVALID` | Cron expression does not match the frozen dialect. |
| `WOML_SCHEDULE_TIMEZONE_INVALID` | Timezone is not a known IANA identifier. |
| `WOML_TRIGGER_MISFIRE_INVALID` | `on-missed` is not a supported policy. |
| `WOML_INTERVAL_INVALID` | Interval is malformed or outside its bounds. |
| `WOML_EVENT_NAME_INVALID` | Event name does not follow the frozen grammar. |
| `WOML_TRIGGER_HISTORY_INVALID` | Occurrence/run/event history is contradictory or corrupt. |

Diagnostics must never echo bearer tokens, control tokens, raw idempotency keys,
or rejected request bodies.

## 15. Implementation Phases

### Phase summary

| Phase | What changes | Product result |
|---|---|---|
| T0 | Freeze shared trigger, occurrence, event, ingress, HTTP, progress, and fixture contracts. | Every layer targets one reviewed production-trigger architecture. |
| T1 | Teach the TypeScript frontend to validate and lower multiple triggers and executable webhook syntax. | A webhook WOML file compiles to an exact Model v7 DAG definition. |
| T2 | Build Rust's atomic trigger-occurrence and run-start authority. | Any approved ingress can create or deduplicate one durable run safely. |
| T3 | Build the Rust webhook listener, authentication, JSON/schema validation, and HTTP responses. | A real POST request safely creates one WOML run. |
| T4 | Turn `woml run` into the long-lived runtime; add `woml test`, run inspection, readiness, and webhook diagnostics. | One command activates an automation that keeps accepting and reporting workflow runs. |
| T5 | Harden, package, and publish webhook. | Webhook is independently production-ready in the single-node profile. |
| T6 | Compile Slack trigger syntax and extract the reusable Slack transport. | A Slack trigger shares the proven provider connection without sharing the wrong protocol. |
| T7 | Execute, harden, package, and publish Slack triggers. | Mentions and direct messages start one durable WOML run per Slack event. |
| T8 | Activate schedule validation/lowering and freeze cron/timezone/misfire behavior. | Schedule markup becomes a deterministic Model v7 trigger. |
| T9 | Add the durable Rust scheduler and publish schedule. | Cron workflows fire at the correct instants and survive restart. |
| T10 | Activate and execute durable fixed-rate interval triggers. | Interval workflows keep a stable time grid across delays and restart. |
| T11 | Activate named-event syntax, schema validation, and publisher contracts. | Event workflows and publishers share one exact versioned contract. |
| T12 | Build durable event publish/fan-out plus `woml emit`. | One application event safely starts every matching workflow once. |
| T13 | Harden all trigger composition, compatibility, packaging, security, and docs. | Production Triggers becomes a complete publishable WOML milestone. |

### T0 — Freeze contracts and reviewed fixtures

Status: completed.

Changes:

- Approve the syntax and semantics proposed in this document.
- Freeze Model v7, Event v7, Trigger Occurrence v1, Trigger Ingress v1,
  Webhook HTTP v1, Slack Trigger Protocol v1, and Trigger Progress v1 schemas.
- Freeze occurrence identity derivation, payload hashing, duplicate behavior,
  context projection, and the atomic persistence boundary.
- Produce one source and exact compiled fixture for manual+webhook, Slack,
  schedule, interval, and event.
- Produce accepted, duplicate, conflict, restart, and corrupt-history fixtures.

Result:

The frontend, core, CLI, and future adapters cannot silently invent different
trigger meanings.

Gate:

Every JSON fixture validates, semantic assertions pin the hard-to-reverse
fields, and older model/event fixtures remain byte-for-byte unchanged.

### T1 — Compile multiple triggers and webhook

Status: completed.

Changes:

- Replace the exactly-one-manual frontend rule with the reviewed trigger list.
- Validate workflow-wide trigger IDs, webhook structure, route, POST method,
  explicit auth, symbolic secret sink, and inline JSON Schema.
- Lower webhook configuration into Model v7 without resolved credentials.
- Preserve source locations for registration and HTTP schema diagnostics.
- Keep Slack, schedule, interval, and event rejected until their activation
  phases.

Result:

A real webhook WOML document becomes a deterministic, engine-ready definition.

Gate:

The reviewed source deep-equals its compiled fixture; malformed routes, auth,
schemas, attributes, placement, and duplicate IDs point to the exact source.

### T2 — Build the Rust occurrence authority

Status: completed.

Changes:

- Validate Model v7 trigger unions in Rust.
- Add immutable occurrence storage and uniqueness constraints to SQLite.
- Extend `run_started` through Event v7 and fold the same public trigger payload.
- Implement the one-transaction occurrence/run/event boundary.
- Implement same-payload duplicate resolution and changed-payload conflict.
- Recover a committed run whose execution was never dispatched.

Result:

Every listener, provider adapter, and scheduler has one safe API for starting a
workflow.

Gate:

Crash tests cover every boundary before, during, and after the transaction;
concurrent identical submissions produce one run and one `run_started` event.

### T3 — Execute webhooks through Rust

Status: completed.

Changes:

- Bind the HTTP listener in the WOML Rust runtime.
- Enforce route/method, content type, byte limit, bearer auth, JSON shape, and
  Draft 2020-12 validation before occurrence admission.
- Implement the frozen success/error response mapping.
- Execute accepted occurrences through the existing durable DAG runtime.
- Keep request transport metadata and credentials outside workflow context.

Result:

Calling the webhook starts the same workflow engine used by `woml run`.

Gate:

End-to-end HTTP tests prove success, schema failure, authentication failure,
oversize rejection, duplicate replay, conflict, and recovery after acceptance.

### T4 — Ship the webhook CLI journey

Status: completed.

Changes:

- Change `woml run` from one-shot execution into long-lived workflow activation
  with file/directory loading, host, port, state, and manual-trigger options.
- Preserve the former one-shot manual journey as `woml test`.
- Preflight every definition, route, and secret before listening.
- Add secret-safe readiness and continuous occurrence/run progress output.
- Print a schema-informed, copy-pasteable `curl` example for every registered
  webhook at startup.
- Print the final workflow JSON automatically when an asynchronous run
  succeeds, while retaining `woml get` for later inspection.
- Add `woml get` for asynchronous status inspection.
- Add graceful signal handling without unsafe active-attempt replay.
- Keep source reload out of plain `woml run`; reserve `--watch` for a later
  reviewed hot-reload contract.

Result:

A user can activate, call repeatedly, observe, stop, restart, and inspect a
webhook workflow using the packaged CLI. Completing one run does not deactivate
the workflow.

Gate:

- From a clean package, `woml run` reaches readiness and remains alive after a
  manual occurrence or webhook-triggered run reaches a terminal state.
- Two webhook requests accepted by the same process create and report two
  independent runs.
- Readiness includes a usable `curl` command, and successful terminal output
  includes the folded workflow result.
- SIGINT/SIGTERM performs the documented graceful shutdown, and restarting
  `woml run` returns the original run for a repeated idempotency key.
- `woml test` executes one manual occurrence, prints its result, and exits.

### T5 — Harden and publish webhook

Status: completed.

Changes:

- Test concurrent requests, slow clients, malformed framing, route conflicts,
  port conflicts, large payloads, database contention, and host crashes.
- Add constant-time auth tests and artifact/state/log secret scans.
- Test webhook composition with retry, branch, parallel, approval, and existing
  Slack notification delivery.
- Update language, architecture, CLI, security, deployment, and migration docs.
- Add the webhook example to the release smoke suite.

Result:

Webhook becomes the first supported Production Trigger.

Gate:

The webhook-specific gate and the complete WOML release gate pass without
changing manual workflows.

Implementation notes:

- The hardening gate covers concurrent admission, SQLite contention, slow
  clients, malformed framing, streamed and declared oversize bodies, route and
  port conflicts, duplicate delivery, restart recovery, and script-host
  failure.
- A discovered host-startup gap was closed: the attempt and terminal failure
  are now durable instead of leaving the run indefinitely `running`.
- Bearer credentials are reduced to fixed-width SHA-256 digests during route
  registration and candidate digests are compared in constant time.
- State, progress, and packaged-artifact scans reject raw credentials and raw
  idempotency identities.
- Webhook-originated runs are proven to compose with retry, branch, parallel,
  durable approval waiting, and the existing Slack notification journey.
- `bun run test:t5` is the webhook release gate, and `test:release` now points
  to it.

### T6 — Compile Slack triggers and extract shared transport

Status: completed.

Changes:

- Activate `<slack>` trigger placement, attributes, event filters, channel
  filters, and symbolic credential sinks in the frontend and Model v7.
- Freeze Slack Trigger Protocol v1 and the normalized `context.payload` payload.
- Extract credential resolution, channel lookup/cache, Web API behavior, Socket
  Mode connection/reconnect, error classification, and diagnostics from the
  existing approval-specific module into a shared Slack transport.
- Keep the existing Notification Provider Protocol and approval behavior
  unchanged while moving them onto the shared transport.
- Update the Slack app manifest with reviewed event subscriptions and minimum
  scopes, and add actionable reinstall diagnostics.

Result:

WOML understands Slack trigger definitions and both Slack features reuse one
transport foundation without conflating trigger and notification semantics.

Gate:

The reviewed Slack source deep-equals Model v7, transport compatibility tests
prove approval behavior is unchanged, and no new Socket connection is opened
for matching credentials.

Implementation notes:

- `<slack>` now validates event and optional channel lists with source-located
  errors, requires exact symbolic bot/app-token references, and deep-equals the
  reviewed Model v7 fixture.
- Notification destinations keep their existing space-separated `#channel`
  syntax; trigger filters use the separately frozen comma-separated channel
  syntax, so the two placements cannot silently borrow each other's attrs.
- Credential resolution, bot identity and channel caches, Web API behavior,
  Socket Mode lifecycle/reconnect, error classification, and safe scope
  diagnostics now live in the shared transport foundation.
- Socket envelopes are routed without global auto-acknowledgement. Approval
  interactions acknowledge in the approval adapter; event envelopes remain
  available for T7's durable-admission acknowledgement rule.
- Matching resolved app credentials reuse one Socket connection even when two
  symbolic references point to that credential. Compatibility journeys prove
  approval delivery, actions, updates, retry diagnostics, and durable behavior
  are unchanged.
- The reviewed app manifest now subscribes to `app_mention` and `message.im`
  and includes `app_mentions:read` and `im:history`. Scope failures continue to
  tell the user to update permissions and reinstall the app.
- `bun run test:t6` remains the standalone T6 contract/transport gate. Slack
  event decoding, Rust admission, and actual run creation were deliberately
  deferred to—and are now completed by—T7.

### T7 — Execute and publish Slack triggers

Status: completed.

Changes:

- Decode and validate `app_mention` and `message.im` Socket Mode events.
- Ignore bot/self messages and unsupported Slack message subtypes.
- Normalize the safe payload and submit it through Rust Trigger Ingress v1.
- Deduplicate Slack redelivery using workspace plus Slack `event_id` and trigger
  identity.
- Coordinate durable Rust admission with Slack envelope acknowledgement.
- Add Slack readiness, reconnect, missing-scope, channel-filter, and occurrence
  diagnostics to `woml run`.
- Test coexistence with approval buttons and notification message updates.
- Add the Slack trigger example to the clean-package release smoke suite.

Result:

A Slack mention or direct message starts one durable WOML workflow, and the
message/thread routing information is ready for a future outbound Slack
capability.

Gate:

Real/fake Slack journeys prove mention, DM, channel filtering, reconnect,
redelivery deduplication, no bot loop, restart recovery, approval compatibility,
packaging, and secret safety.

Implementation notes:

- The long-lived Rust trigger runtime now exposes the frozen asynchronous
  Trigger Ingress v1 admission boundary to Bun. Slack-only workflows use the
  same durable occurrence, event log, execution loop, recovery, and run
  inspection authority as webhooks.
- Bun decodes `app_mention` and `message.im`, rejects malformed or oversized
  input, ignores bot/self/unsupported subtype traffic, applies mention channel
  filters, and emits only the reviewed seven-field `context.payload` payload.
- A Slack envelope is acknowledged only after Rust commits a new or duplicate
  occurrence. Workspace, Slack event ID, workflow, and trigger form the stable
  source identity; duplicate delivery returns the original run without a
  second dispatch.
- `woml run` reports Slack connection readiness, reconnect attempts, matched
  occurrences, duplicate recognition, filter misses, safe failures, and final
  run results. Slack credentials remain symbolic in every Rust registration.
- Approval actions and trigger events have separate listeners and routing on
  the shared transport. Tests prove that either can be handled without
  consuming or acknowledging the other's message.
- `examples/slackTriggerWorkflow.woml` is the runnable one-channel product
  example and compiles through the reviewed Model v7 contract. The separate
  two-channel fixture preserves broader contract coverage. `bun run test:t7`
  remains the historical T7 gate.

### T8 — Compile schedules and freeze time semantics

Status: completed.

Changes:

- Activate `<schedule>` in the frontend and Model v7 profile.
- Validate the five-field cron dialect, IANA timezone, and `on-missed` policy.
- Freeze UTC occurrence identity and daylight-saving behavior with fixtures.
- Produce source-aware cron and timezone diagnostics.

Result:

Schedule markup has one portable meaning before a clock starts driving it.

Gate:

Deterministic fixture tables cover ordinary dates, month/year boundaries,
leap day, nonexistent times, repeated times, and invalid expressions.

Implementation notes:

- `<schedule>` is now an active frontend element with exact `id`, `cron`,
  optional `timezone`, and optional `on-missed` attributes. Omitted values
  lower deterministically to `UTC` and `skip`.
- WOML Cron v1 freezes five numeric fields, single-space separation, lists,
  inclusive non-wrapping ranges, steps, field bounds, Sunday as `0` or `7`,
  and POSIX day-of-month/day-of-week OR behavior. Seconds, names, macros, and
  Quartz tokens fail with source-located diagnostics.
- Canonical IANA timezone identifiers are required. The versioned schedule
  semantics artifact pins ordinary dates, year/month boundaries, leap day,
  non-hour offsets, nonexistent DST times, and both UTC instants of a repeated
  wall time.
- Misfire behavior, occurrence identity, and the exact two-field
  `context.payload` contract were frozen before Rust began driving the clock.
- `examples/scheduleWorkflow.woml` is the reviewed Model v7 product fixture.
  `bun run test:t8` remains the historical T8 gate.

### T9 — Execute and publish durable schedules

Status: completed.

Changes:

- Add durable schedule cursors and an injected Rust clock.
- Claim due occurrences atomically through the T2 authority.
- Implement `skip` and bounded `run-once` recovery.
- Register schedules in `woml run` and report their next due instants.
- Recover process crashes at every cursor/occurrence/run boundary.

Result:

Cron workflows fire once per planned occurrence and safely continue after
restart.

Gate:

Fake-clock, restart, DST, simultaneous-due, and clean-package tests pass; no
wall-clock timing assertion is used as the correctness proof.

Implementation notes:

- Durable store schema v5 adds one mutable cursor per workflow/trigger while
  keeping definitions, occurrences, runs, and events immutable.
- A due occurrence, run binding, `run_started` event, and cursor advance commit
  in one immediate SQLite transaction through the T2 admission authority.
- Rust implements WOML Cron v1 against the same frozen occurrence table as the
  TypeScript frontend, including POSIX day matching, leap dates, non-hour
  offsets, and both DST gap/repeat rules.
- The scheduler clock is injected. Fake-clock tests prove normal firing,
  simultaneous due schedules, `skip`, bounded `run-once`, and restart behavior
  without sleeping until real cron instants.
- Schedule-only and Slack-only runtimes do not bind an unnecessary HTTP port.
  Webhook definitions still bind the configured listener.
- Schedule Progress v1 is a new strict diagnostics contract; Trigger Progress
  v1 remains unchanged and takes over after occurrence admission.
- `context.payload` for a schedule is exactly `{ scheduledAt, triggeredAt }` in
  RFC 3339 UTC. Occurrence identity is stable from workflow ID, trigger ID, and
  planned UTC instant.
- `woml run examples/scheduleWorkflow.woml` remains active and reports its next
  due instant. `bun run test:t9` remains the historical T9 gate.

### T10 — Execute and publish durable intervals

Status: completed.

Changes:

- Activate `<interval>` validation and lowering.
- Store the first registration anchor and monotonically increasing sequence.
- Compute fixed-rate occurrences without drift.
- Reuse schedule misfire, occurrence, recovery, progress, and long-lived runtime
  machinery.
- Test intervals shorter and longer than workflow execution time.

Result:

Interval workflows remain predictable across slow runs and process restarts.

Gate:

Deterministic-clock tests prove the exact grid, misfire behavior, simultaneous
intervals, and absence of duplicate sequences.

Implementation notes:

- The frontend accepts whole durations from `1s` through `30d`, lowers them to
  exact `everyMs`, and defaults `on-missed` to `skip`.
- Durable store schema v6 adds an interval cursor containing the definition,
  duration, policy, first-registration UTC anchor, next sequence, and next
  planned instant.
- Rust computes every occurrence as `anchor + sequence × every`; workflow
  execution duration never moves the grid and does not block later interval
  admissions.
- Cursor advancement and occurrence/run creation commit atomically. Source
  identity includes workflow, trigger, anchor, and sequence, while
  `context.payload` remains exactly `{ scheduledAt, triggeredAt }`.
- Restart recovery is bounded: `skip` moves directly to the next future grid
  point and `run-once` admits only the latest missed occurrence.
- Interval Progress v1 reports the durable anchor, next sequence, next planned
  instant, recovery reason, and safe runtime errors without widening Trigger
  Progress v1.
- `woml run examples/intervalWorkflow.woml` stays active, needs no HTTP socket,
  prints its next due instant, and fires every five seconds. `bun run test:t10`
  is the T10 release gate.

### T11 — Freeze and compile named events

Status: completed.

Changes:

- Activate `<event>` with the frozen name grammar, required symbolic publisher
  secret, and optional inline schema.
- Freeze authenticated publisher request/response and fan-out result contracts.
- Freeze publisher event ID, duplicate/conflict, partial subscriber rejection,
  and payload-size behavior.
- Extend the reviewed secret sinks so an event publisher credential compiles as
  a symbolic reference and is resolved only at runtime.

Result:

Workflow authors and application publishers share one precise event contract.

Gate:

Compiled, ingress, fan-out, duplicate, schema-failure, and secret-safety
fixtures pass conformance tests.

Implementation notes:

- `<event id="..." name="..." secret="{{secrets.NAME}}">` validates and
  lowers to the reviewed `trigger.event` Model v7 shape, with a symbolic secret
  reference and optional inline Draft 2020-12 schema.
- Event names use the frozen lowercase segmented grammar and are limited to
  256 characters. Schema diagnostics point to the responsible WOML source.
- Event Publication v1 and Event Publisher HTTP v1 pin the logical publisher,
  reserved `POST /_woml/events/{eventName}` transport, required `Event-ID`,
  bearer control authentication, 1 MiB payload limit, and 1,000-subscriber
  first-profile bound.
- Fan-out order is deterministic. Per-subscriber outcomes independently report
  accepted, duplicate, schema-invalid, idempotency-conflict, unavailable, or
  invalid-history results. Overall status is `accepted`, `partial`, or
  `rejected` and a completed fan-out returns HTTP 200.
- Each subscriber uses a collision-safe hash of event ID, workflow ID, and
  trigger ID as its Trigger Ingress source identity. A retry after a crash can
  complete remaining subscribers without duplicating accepted runs.
- There is no conventional or hard-coded event secret name. `woml run` resolves
  the name authored on `<event>`; `woml emit` requires `--token-secret <NAME>`
  because it does not read a workflow. Secret values never enter WOML, Model
  v7, publisher messages, durable state, fixtures, or diagnostics.
- Historical Model v7 event definitions persisted before the publisher-secret
  field existed remain valid for folding and interrupted-run recovery. The
  current WOML frontend and active event ingress still require the symbolic
  secret, so backward-readable storage does not create an unauthenticated
  publisher route.
- T11 originally gave an explicit T12 diagnostic instead of opening an inactive
  listener. T12 replaces that staging boundary with the authenticated runtime.
  `examples/eventWorkflow.woml` remains the reviewed T11 contract source, and
  `bun run test:t11` remains its compiler/contract gate.

### T12 — Execute event publication and fan-out

Status: completed.

Changes:

- Add the authenticated event ingress endpoint to the long-lived Rust runtime.
- Add `woml emit` using symbolic secrets from the existing secret store.
- Match loaded event triggers deterministically by exact event name.
- Validate and admit every subscriber independently.
- Return all accepted run IDs and safe per-subscriber rejections.
- Recover a crash during fan-out without duplicating accepted subscriber runs.

Result:

Applications can publish one named event and safely start every subscribed WOML
workflow.

Gate:

Multi-workflow, duplicate, partial validation, concurrent publication, restart,
and clean-package CLI journeys pass.

Implementation notes:

- The long-lived Rust trigger host now serves authenticated
  `POST /_woml/events/{eventName}` routes beside workflow-owned webhooks.
- `woml run <directory>` automatically resolves the secret references authored
  on its event triggers, loads direct `.woml` files in lexical order, prints
  each unique event URL once, and provides a safe curl example without
  resolving credentials into the output. Subscribers to the same event name
  must resolve to the same token; unrelated event names may use different ones.
- One publication validates every exact-name subscriber independently and
  returns deterministic `accepted`, `partial`, or `rejected` delivery results.
  Accepted runs execute asynchronously through the existing durable Rust DAG.
- Event IDs are combined with workflow and trigger identity before durable
  admission. Replays return the original run, changed payloads conflict, and a
  retry after a mid-fan-out crash admits only subscribers that were still
  missing.
- `woml emit` reads a JSON object from `@<file>`, resolves its bearer token from
  the existing symbolic secret store, and publishes through the same HTTP
  contract used by any external application.
- `examples/events/` contains two workflows subscribed to `order.created` plus
  a sample payload. The T12 gate proves HTTP and CLI publication, two-workflow
  fan-out, partial schema rejection, authentication, restart deduplication,
  mid-fan-out recovery, and secret safety.

### T13 — Complete Production Triggers

Status: completed.

Changes:

- Run cross-trigger composition and compatibility tests.
- Test many routes, Slack workspaces/events, schedules, intervals, and event
  subscribers together.
- Verify definition updates never rebind existing runs or old occurrence IDs.
- Test process/Worker crashes, SQLite contention, payload limits, and shutdown.
- Complete security, operations, migration, and troubleshooting documentation.
- Run frontend, Rust, Bun host, CLI, typecheck, Clippy, schema, packaging, and
  secret-leak gates.

Result:

Webhook, Slack, schedule, interval, and event are supported Production Triggers,
while manual definitions and execution semantics remain compatible through the
long-lived `woml run` and one-shot `woml test` journeys.

Gate:

All five reviewed examples work from a clean package, every safe restart
boundary recovers, ambiguous script work still fails closed, and the complete
release gate passes.

Implementation notes:

- The T13 coexistence journey activates webhook, Slack, schedule, interval,
  and named event in one long-lived runtime. It executes real webhook and event
  occurrences while the Slack connection and both Rust schedulers remain
  registered, then proves secret-safe output and graceful shutdown.
- Historical Model v7 event definitions without the later authored publisher
  secret remain readable during recovery. Current source and active ingress
  still require a resolved symbolic secret, preserving both immutable history
  and authenticated publication.
- `docs/woml-production-triggers.md` is the unified deployment, security,
  state, recovery, migration, and troubleshooting guide. The webhook-specific
  guide remains as focused HTTP material.
- `bun run test:t13` is the completed milestone gate, and `test:release` now
  points to it. The transitive gate rebuilds the package and runs all frontend,
  Rust, isolated CLI, schema, packaging, crash/recovery, contention, and secret
  safety checks.

## 16. Expected File Areas

| Area | Expected location |
|---|---|
| Language parsing/validation/lowering | `woml/src/compiler.ts`, `model.ts`, frontend tests and fixtures |
| Versioned contracts | `docs/schemas/compiled-workflow-model.v7.schema.json`, `run-event.v7.schema.json`, new trigger protocol schemas |
| Rust model/event/folding | `core/woml-engine/src/model.rs`, `event.rs`, `projection.rs`, `engine.rs` |
| Occurrence persistence/recovery | `core/woml-engine/src/durable.rs` plus a focused trigger module if needed |
| HTTP listener and scheduler | New WOML-owned Rust modules under `core/woml-engine`; not the legacy Cronflow execution path |
| Shared Slack transport | Refactored modules under `woml-cli/src/notification-provider/` or a new `woml-cli/src/slack/` boundary shared by trigger and notification adapters |
| Native/CLI boundary | `core/src/woml_bridge.rs`, `woml-cli/src/rust-executor.ts` or focused trigger clients |
| CLI commands/diagnostics | `woml-cli/src/cli.ts` split into command modules as the surface grows |
| Product fixtures | `examples/webhookWorkflow.woml`, `slackTriggerWorkflow.woml`, `scheduleWorkflow.woml`, `intervalWorkflow.woml`, `eventWorkflow.woml`, and the two-subscriber `examples/events/` journey |
| Public docs | `docs/woml-v0.1.md`, `docs/architecture.md`, CLI and trigger operations guides |

Exact modules may differ after code inspection. Layer ownership may not: the
TypeScript frontend understands WOML, Rust owns durable trigger decisions, and
Bun Workers execute JavaScript only after Rust starts a run.

## 17. Verification Matrix

| Area | Required proof |
|---|---|
| Source | Every tag/attribute has exact placement, grammar, defaults, and source diagnostics. |
| Model | Model v7 trigger configs are strict, portable, and contain no resolved secret. |
| Events | Every v7 run records immutable trigger and occurrence identity and folds the exact public payload. |
| Atomicity | Accepted occurrence, run, and `run_started` either all commit or none commit. |
| Deduplication | Same identity+payload returns one run; changed payload conflicts. |
| Webhook | Auth, JSON, schema, size, route, method, status, and restart behavior match HTTP v1. |
| Slack | Mention/DM filters, channel filters, reconnect, acknowledgement, redelivery, and bot-loop prevention match Slack Trigger v1. |
| Slack reuse | Approval delivery/actions remain compatible and matching credentials share one connection/cache. |
| Schedule | Cron/timezone/DST/misfire decisions match deterministic fixtures. |
| Interval | Durable anchor and sequence produce a drift-free grid. |
| Event | Exact-name fan-out admits each subscriber once and isolates subscriber rejection. |
| Composition | Every trigger can enter retry, branch, parallel, approval, and notification workflows. |
| Recovery | Committed runs continue; no occurrence or completed work replays. |
| Compatibility | Manual-only Model/Event v1–v6 behavior and fixtures remain unchanged. |
| Security | Tokens, raw keys, headers, rejected payloads, and capabilities never leak into artifacts or logs. |
| Packaging | Every published trigger journey works from the clean package. |

## 18. Risks and Guardrails

### “Production” does not mean distributed yet

This milestone provides durable, secure single-node trigger behavior. It must
not claim multi-node exactly-once scheduling before lease and ownership
contracts exist. The occurrence API is deliberately shaped so a future
distributed owner can use it without changing WOML syntax.

### HTTP acknowledgement is not workflow success

`202 Accepted` means the occurrence and run identity are durable. The workflow
may later fail, wait for approval, or exhaust retries. Documentation and clients
must not interpret acceptance as the terminal result.

### Trigger deduplication and step idempotency are different

Occurrence identity prevents the same source occurrence from creating multiple
runs. It cannot deduplicate arbitrary side effects inside the run; scripts must
continue using `attempt.idempotencyKey` with capable external services.

### Time semantics are a public API

Cron dialect, timezone, DST, interval anchor, and misfire policy are frozen and
tested before scheduler implementation. We do not delegate accidental semantics
to whichever scheduling library is convenient.

### Public endpoints require deliberate security

Authentication mode is explicit, localhost is the default bind, request bodies
are bounded, and resolved secrets never cross the model/event boundary. Provider
signature verification is added as reviewed provider-specific syntax, not an
unvalidated script hook.

### Reuse transport, not domain contracts

Slack notification delivery and Slack trigger ingestion need the same Socket
Mode/Web API machinery, but they represent different durable decisions. Sharing
the transport reduces duplicated connections and provider bugs; merging their
protocols would couple approval semantics to workflow-start semantics. The
adapters remain separate and Rust receives both through their reviewed
boundaries.

### Inbound Slack does not by itself send an agent reply

The trigger starts a workflow and preserves safe conversation routing IDs. A
general outbound `slack.send` operation is intentionally added under Services
and Capabilities, where effects, retries, permissions, and idempotency can be
specified correctly. We must not hide an automatic reply side effect inside a
trigger.

## 19. Global Roadmap After Production Triggers

1. **Retries and idempotency** — completed in RI7.
2. **Production triggers** — completed in T13: webhook, Slack, schedule,
   interval, and event. Discord, WhatsApp, and Telegram later reuse the
   provider-trigger boundary.
3. **Services and capabilities** — HTTP, database, messaging, and other useful
   registered operations that automatically receive the relevant effect key;
   this includes outbound Slack messaging for conversational agents.
4. **WOML module system** — import reusable `.woml` files and embed their
   exported workflows/components with a React-like composition experience.
   Exact module syntax, inputs, outputs, namespacing, packaging, and versioning
   remain deliberately undesigned until that milestone.
5. **Lifecycle and engine controls** — cancellation, hooks, workflow-level
   concurrency, rate limiting, queues, timeouts, and durable user state.
6. **Production runtime** — hosting, deployment, multi-node ownership,
   observability, retention, and operational administration.
7. **Additional communication providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when product demand
   justifies them.
8. **Retire the JavaScript chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

After T13, the next planning milestone is Services and Capabilities, beginning
with a safe built-in HTTP operation.

## 20. T0 Review Gate (Completed; CLI Naming Clarified After T3)

T0 reviewed and froze these expensive contract choices before frontend
implementation began:

- `woml run` is the long-lived local/deployable automation runtime, and there is
  no separate public `woml serve` command;
- `woml test` is the explicit one-shot manual execution journey;
- `--watch` is reserved for future source reload and is not required to keep a
  workflow active;
- webhook v1 is asynchronous POST JSON and returns a durable run ID;
- webhook auth is explicit `bearer` or `none`;
- the standard `Idempotency-Key` header provides optional request deduplication;
- Slack v1 accepts app mentions and direct messages, filters bot/self messages,
  and deduplicates with Slack's workspace/event identity;
- Slack trigger and approval notification adapters share transport machinery but
  keep separate versioned protocols;
- one accepted occurrence atomically creates exactly one run and Event v7
  records its trigger identity;
- `context.payload` remains the direct payload rather than a new envelope;
- schedule uses a five-field cron dialect and IANA timezone;
- schedule/interval missed policy is bounded to `skip` or `run-once`;
- interval is fixed-rate from a durable anchor;
- event delivery uses exact names, required publisher IDs, and independent
  durable fan-out;
- single-node ownership is publishable now while distributed ownership remains
  explicitly deferred; and
- Model v7, Event v7, Occurrence v1, Ingress v1, HTTP v1, Slack Trigger v1, and
  Progress v1 are frozen before executable code.

The corresponding schemas, protocol document, reviewed fixtures, and
conformance tests form the versioned gate for later phases. T1 compiles the
reviewed webhook source to Model v7, T2 supplies the Rust-owned atomic
occurrence/run/`run_started` boundary, and T3 connects real HTTP requests to
that authority and the existing durable DAG runtime.

The long-lived `woml run` decision is a product-surface correction made after
T3. It does not alter Model v7, Event v7, Occurrence v1, Ingress v1, Webhook
HTTP v1, or the Rust execution boundary. T4 implements the corrected lifecycle
and CLI names without reopening those frozen runtime contracts.
