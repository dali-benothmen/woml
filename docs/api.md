# WOML v1 Complete User API Reference

> This is the consolidated author-facing reference for the WOML website. It
> documents the syntax and behavior implemented by the current v1 compiler,
> CLI, Rust engine, Bun script host, and built-in services. Unsupported and
> deferred behavior is listed explicitly near the end.

## 1. Installation and requirements

WOML is distributed as the global `woml-cli` npm package. It installs the
`woml` executable and selects the matching native Rust engine automatically.

Requirements:

- Bun 1.3.14 or later for authored JavaScript and CLI runtime support.
- A supported Linux, macOS, or Windows target for the prebuilt native engine.
- A writable local directory for `.woml/` state during normal local use.

Install with one package manager:

```bash
npm install --global woml-cli
# or
bun add --global woml-cli
# or
pnpm add --global woml-cli
```

Verify the installation:

```bash
woml --version
woml -v
woml --help
```

The public package is installed globally. A workflow project does not need to
add WOML as an application dependency.

## 2. Minimal runnable workflow

```xml
<woml>
  <workflow id="hello" name="Hello WOML" version="1.0.0">
    <triggers>
      <manual id="start" />
    </triggers>

    <steps>
      <step id="prepare" name="Prepare greeting">
        <script>
          return { name: context.payload.name ?? "World" };
        </script>
      </step>

      <step id="greet" name="Build message">
        <script>
          return { message: `Hello ${context.steps.prepare.name}` };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

```bash
woml check hello.woml
woml run hello.woml
```

Press Enter to create a manual run. The process remains active for later runs;
press Ctrl+C to stop it.

## 3. Language rules

### 3.1 WOML is XML-like, not XML

WOML uses familiar markup but preserves the body of `<script>` as raw
JavaScript. Authors do not use CDATA and do not escape JavaScript operators:

```xml
<script>
  if (score < 0.8 && enabled) {
    return { accepted: true };
  }
  return { accepted: false };
</script>
```

Rules:

- Do not add an XML declaration.
- Element and attribute names are lowercase and case-sensitive.
- Multiword names use kebab-case.
- Attribute values must be quoted.
- Duplicate, unknown, or misplaced attributes and elements are errors.
- Comments use `<!-- ... -->` outside raw script/schema content.
- The first literal `</script>` ends a script body, even if it appears inside a
  JavaScript string or comment. Construct that text in pieces if needed.
- The first literal `</schema>` ends an inline schema body.

### 3.2 Identifiers

| Identity | Pattern | Example |
| --- | --- | --- |
| Workflow ID | `[a-z][a-z0-9]*(?:-[a-z0-9]+)*` | `process-order` |
| Trigger, step, parallel, choice, switch, fork, and approval ID | `[a-z][A-Za-z0-9]*` | `calculateTotal` |
| Fork branch ID | JavaScript-safe ID, local to its fork | `instagram` |
| Module alias | `[a-z][A-Za-z0-9]*` for JS/TS modules; kebab-style custom tag names for reusable WOML definitions | `pricing`, `calculate-discount` |

Trigger IDs are unique within the workflow. Executable and structural IDs
share one workflow-wide namespace because they can participate in durable
identity and `context.steps` output.

### 3.3 Durations and rates

Durations use a positive number and one unit:

```text
10ms  30s  5m  2h  7d
```

Bare numbers are invalid. A runtime-policy rate is a positive count, a slash,
and a duration:

```text
100/1m
```

### 3.4 JSON values

Step results, trigger payloads, workflow-call payloads, context, cache values,
and state values must be JSON-compatible: null, booleans, finite safe numbers,
strings, arrays, and objects. `undefined`, functions, symbols, BigInt, circular
values, clients, and class instances are not valid durable outputs.

## 4. Document profiles

Every file has exactly one `<woml>` root and one of three profiles.

### 4.1 Runnable workflow

```xml
<woml>
  <imports>...</imports>
  <workflow>...</workflow>
</woml>
```

Only a document containing `<workflow>` can be activated by `woml run`.

### 4.2 Reusable step

```xml
<woml>
  <imports>...</imports>
  <props>...</props>
  <step>...</step>
  <lifecycle>...</lifecycle>
</woml>
```

This file defines a custom step tag and is imported by a runnable workflow. It
does not own triggers or runs.

### 4.3 Reusable notification provider

```xml
<woml>
  <imports>...</imports>
  <props>...</props>
  <provider kind="notification">...</provider>
  <lifecycle>...</lifecycle>
</woml>
```

This file defines transport for a custom notification tag. Rust still owns
delivery identity, approval authority, retries, and settlement.

## 5. Complete tag index

| Tag | Valid parent | Purpose |
| --- | --- | --- |
| `<woml>` | Document root | Classifies one workflow or reusable definition. |
| `<imports>` | `<woml>` | Contains local module imports. |
| `<module>` | `<imports>` | Imports `.js`, `.ts`, or reusable `.woml` source. |
| `<props>` / `<prop>` | Reusable document | Declares a reusable definition's public props. |
| `<workflow>` | `<woml>` | Defines one executable workflow. |
| `<config>` | `<workflow>` | Defines runtime concurrency, rate, timeout, and queue policy. |
| `<lifecycle>` | Workflow or reusable definition | Contains observational lifecycle hooks. |
| `<triggers>` | `<workflow>` | Contains one or more workflow triggers. |
| `<manual>` | `<triggers>` | Creates runs from keyboard input. |
| `<webhook>` | `<triggers>` | Creates runs from HTTP requests. |
| `<schedule>` | `<triggers>` | Creates runs from a cron schedule. |
| `<interval>` | `<triggers>` | Creates runs on a fixed time grid. |
| `<event>` | `<triggers>` | Subscribes to a named event. |
| `<slack>` | Triggers or `<notify>` | Receives Slack messages or delivers notifications. |
| `<telegram>` | Triggers or `<notify>` | Receives Telegram messages or delivers notifications. |
| `<discord>` | Triggers or `<notify>` | Receives Discord messages or delivers notifications. |
| `<whatsapp>` | Triggers or `<notify>` | Receives signed WhatsApp callbacks or delivers templates. |
| `<schema>` | Webhook or event | Contains inline JSON Schema Draft 2020-12. |
| `<steps>` | `<workflow>` | Contains the ordered business flow. |
| `<step>` | A flow container or reusable root | Defines one executable operation or reusable step body. |
| `<script>` | Step, lifecycle hook, or provider | Runs raw asynchronous JavaScript. |
| `<parallel>` | Flow container | Runs direct child steps concurrently and joins them. |
| `<choose>` | Flow container | Routes on strict boolean conditions. |
| `<when>` / `<otherwise>` | `<choose>` | Define ordered conditional routes. |
| `<result>` | Choice, switch, or relevant route arm | Publishes a stable route result. |
| `<switch>` | Flow container | Routes an exact string to one case. |
| `<case>` / `<default>` | `<switch>` | Define exact-string routes. |
| `<fork>` | Flow container | Starts concurrent multi-step branches. |
| `<branch>` | `<fork>` | Defines one sequential fork-owned route. |
| `<approval>` | Flow container | Waits durably for approve/reject or timeout. |
| `<notify>` | Approval or lifecycle hook | Delivers actionable or informational messages. |
| `<when-approved>` / `<when-rejected>` | `<approval>` | Define approval continuations. |
| `<provider>` | Reusable root | Defines a custom notification transport. |

## 6. Workflow structure

### 6.1 `<woml>`

`<woml>` is required and accepts no attributes. It contains optional
`<imports>` followed by exactly one runnable workflow or reusable definition.

### 6.2 `<workflow>`

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Stable lowercase kebab-case workflow identity. |
| `name` | No | Human-readable name used by the terminal. |
| `description` | No | Human-readable summary used by tools and output. |
| `version` | No | Author-owned workflow version; it does not select the WOML grammar. |

It contains exactly one `<steps>` plus optional singleton `<config>`,
`<lifecycle>`, and `<triggers>`. Those four children may appear in any source
order; WOML recognizes them by name. Omit `<triggers>` for a call-only workflow.
An empty `<triggers />` is invalid.

`tags`, workflow-level `timeout`, and workflow-level `concurrency` attributes
are not accepted. Runtime policy belongs in `<config>`.

### 6.3 `<config>`

`<config>` is optional, self-closing, may occur once, and must contain at least
one attribute.

| Attribute | Type | Meaning |
| --- | --- | --- |
| `concurrency` | Positive integer, maximum 1,000,000 | Maximum actively executing runs for this workflow ID across processes sharing the state store. |
| `rate-limit` | `count/duration` | Strict rolling-window limit on first execution starts. |
| `timeout` | `1ms` through `365d` | Total run deadline after first execution starts; durable waits and lifecycle time count. |
| `queue` | Lowercase dot/underscore/kebab name, max 128 characters | Durable FIFO scheduling lane. It is not a messaging queue. |

```xml
<config concurrency="4" rate-limit="100/1m" timeout="10m" queue="orders" />
```

Queue time is excluded from the workflow deadline. Retry delays, approval
waits, synchronous child waits, and lifecycle finalization after execution
starts are included. Waiting releases concurrency capacity and reacquires it
when work becomes ready.

## 7. Triggers

Every trigger starts the same root `<steps>`. Its normalized input becomes
`context.payload`. Multiple triggers may exist, but WOML v1 does not route each
trigger to a different entry node.

### 7.1 `<manual>`

```xml
<manual id="start" />
```

Only `id` is accepted. `woml run` stays active and creates a run when the
operator presses Enter. A keyboard run currently receives `{}` as its payload.
Select a particular manual trigger with `--trigger <id>` when necessary.

### 7.2 `<webhook>`

```xml
<webhook
  id="newOrder"
  path="/webhooks/orders"
  method="POST"
  auth="bearer"
  secret="{{secrets.ORDER_WEBHOOK_TOKEN}}"
>
  <schema>
    {
      "type": "object",
      "required": ["orderId"],
      "properties": { "orderId": { "type": "string" } },
      "additionalProperties": false
    }
  </schema>
</webhook>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Trigger identity. |
| `path` | Yes | Static absolute route. Parameters, wildcards, repeated slashes, and `/_woml` are forbidden. |
| `method` | No | Defaults to `POST`; executable v1 ingress uses POST. |
| `auth` | Yes | `bearer` or explicitly insecure `none`. |
| `secret` | With bearer | Exact `{{secrets.NAME}}`; forbidden with `auth="none"`. |

`<schema>` is optional and may occur once. It contains JSON Schema Draft
2020-12. Invalid source schema prevents activation. An invalid request returns
HTTP 400 with `WOML_TRIGGER_SCHEMA_INVALID` and creates no run. Accepted
requests return HTTP 202 with a durable run ID; execution continues
asynchronously. The body limit is 1 MiB and the CLI prints a copyable curl
request at startup.

### 7.3 `<schedule>`

```xml
<schedule id="dailyReport" cron="0 8 * * *" timezone="Europe/Berlin" on-missed="run-once" />
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Trigger identity. |
| `cron` | Yes | Five numeric fields: minute, hour, day-of-month, month, day-of-week. |
| `timezone` | No | Canonical IANA timezone; defaults to `UTC`. |
| `on-missed` | No | `skip` or `run-once`; defaults to `skip`. |

WOML Cron v1 supports wildcards, lists, inclusive ranges, and `/step`. It does
not support seconds, names, macros, wrapping ranges, or Quartz-only tokens.
Restricted day-of-month and day-of-week use POSIX OR semantics. DST nonexistent
times are skipped; repeated wall times map to both UTC instants. Payload:
`{ scheduledAt, triggeredAt }` using RFC 3339 UTC timestamps.

### 7.4 `<interval>`

```xml
<interval id="refreshCache" every="5m" on-missed="skip" />
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Trigger identity. |
| `every` | Yes | Fixed interval from `1s` through `30d`. |
| `on-missed` | No | `skip` or `run-once`; defaults to `skip`. |

Intervals use an anchored fixed-rate grid, so a slow run does not shift future
planned instants. Payload: `{ scheduledAt, triggeredAt }`.

### 7.5 `<event>`

```xml
<event
  id="orderCreated"
  name="order.created"
  secret="{{secrets.EVENT_CONTROL_TOKEN}}"
>
  <schema>{ "type": "object", "required": ["orderId"] }</schema>
</event>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Trigger identity. |
| `name` | Yes | Named event, max 256 characters, with at least two lowercase segments separated by `.`, `_`, or `-`. |
| `secret` | No | Enables authenticated public HTTP publication when present. Omit for internal-only events. |

The optional schema follows the webhook schema rules. With a secret, the
runtime exposes `POST /_woml/events/{eventName}` and requires a bearer token plus
an `Event-ID`. One publication fans out to every exact-name subscriber. Each
subscriber validates and admits independently, so the result may be accepted,
partial, or rejected. Without a secret, no public route is opened, but
`services.events.emit()` can still publish internally.

### 7.6 `<slack>` trigger

```xml
<slack
  id="agentMessage"
  events="app-mention,direct-message"
  channels="woml-testing,agent-support"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Trigger identity. |
| `events` | Yes | `app-mention`, `direct-message`, or both as a comma-separated list. |
| `channels` | No | Comma-separated lowercase names or Slack conversation IDs; filters mentions. |
| `bot-token` | Yes | Exact bot-token secret reference. |
| `app-token` | Yes | Exact Socket Mode app-token secret reference. |

WOML shares one Socket Mode connection per credential pair, durably admits the
event before acknowledgement, and ignores bot/self messages, edits, deletes,
unsupported subtypes, and unmatched channels.

### 7.7 `<telegram>` trigger

```xml
<telegram id="agentMessage" events="message" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
```

Telegram v1 accepts only `events="message"`. The bot token is required. WOML
uses shared long polling and ignores bot-authored messages.

### 7.8 `<discord>` trigger

```xml
<discord
  id="agentMessage"
  events="app-mention,direct-message"
  channels="200000000000000001,200000000000000002"
  bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
/>
```

`events` accepts app mentions and/or direct messages. `channels` is an optional
comma-separated allowlist of 17–20 digit Discord channel IDs. Channel names and
slash commands are not supported. WOML uses a shared resumable Gateway
connection.

### 7.9 `<whatsapp>` trigger

```xml
<whatsapp
  id="customerMessage"
  events="message"
  phone-number-id="123456789012345"
  verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
  app-secret="{{secrets.WHATSAPP_APP_SECRET}}"
/>
```

WhatsApp v1 accepts only `events="message"`. `phone-number-id` is Meta's
numeric Phone Number ID, not the display number. `verify-token` and
`app-secret` are required secret references. WOML exposes
`/callbacks/whatsapp`, verifies `X-Hub-Signature-256` over raw bytes, and does
not create runs for delivery-status callbacks.

### 7.10 Communication trigger payload

Slack, Telegram, Discord, and WhatsApp normalize provider messages under
`context.payload`:

| Field | Meaning |
| --- | --- |
| `provider` | `slack`, `telegram`, `discord`, or `whatsapp`. |
| `event` | `message`, `app-mention`, or `direct-message`. |
| `text` | Bounded text when available. |
| `senderId`, `senderName` | Stable sender ID and optional safe display name. |
| `conversationId`, `conversationType` | Conversation identity and `direct`, `group`, or `channel` type. |
| `messageId` | Stable provider message identity. |
| `replyToMessageId`, `threadId` | Optional reply/thread identities. |
| `occurredAt` | Normalized RFC 3339 timestamp. |
| `providerData` | Small provider-specific scalar metadata. |

Raw envelopes, signatures, headers, tokens, profiles, and credentials do not
enter workflow context.

## 8. Steps and script execution

### 8.1 `<steps>`

`<steps>` is required and accepts no attributes. It contains one or more flow
items. Document order means sequential dependency. A successful earlier item
makes its guaranteed outputs visible to later items.

Valid direct flow items are `<step>`, `<parallel>`, `<choose>`, `<switch>`,
`<fork>`, and `<approval>`. Empty root steps are invalid.

### 8.2 `<step>`

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Stable DAG node and `context.steps.<id>` output identity. |
| `name` | No | Display name. |
| `description` | No | Display explanation. |
| `retry` | No | Total attempts, `1`–`10`; defaults to one. |
| `retry-backoff` | No | `fixed` or `exponential`; requires retry greater than one. |
| `retry-delay` | No | Delay/initial delay from `1ms` through `24h`; defaults to `1s`. |
| `retry-max-delay` | No | Exponential cap through `24h`; invalid for fixed backoff. |

A fundamental step contains exactly one `<script>`. Timeout belongs to the
workflow `<config>` in v1; `<script>` accepts no attributes.

Retry uses multiplier two without jitter for exponential backoff. Only a
definitive `script_threw` failure is retried automatically. Timeout, invalid
JSON, size failure, process crash, interruption, and cancellation fail closed.

### 8.3 `<script>`

A script body is the body of an asynchronous JavaScript function. It may use
`await`, conditions, loops, exceptions, Fetch, modules, and services without a
function wrapper:

```xml
<script>
  const response = await services.http.request({
    url: "https://api.example.com/orders",
    method: "POST",
    json: context.payload
  }, { name: "create-order" });

  return response.data;
</script>
```

A step contributes data only through its returned JSON value. Context
mutation, locals, and worker globals do not persist. The successful return is
recorded before downstream scheduling and becomes `context.steps.<stepId>`.
The final value-producing node on the main route is the workflow's public JSON
result. Structural choices/switches can make that output path-stable with
`<result>`. Failed or cancelled runs never print a partial main-route value as
a successful workflow result.

### 8.4 Script bindings

| Binding | Availability | Purpose |
| --- | --- | --- |
| `context` | Business and workflow-lifecycle scripts | Read-only trigger payload and durable step outputs. |
| `attempt` | Business and workflow-lifecycle scripts | Attempt number, maximum attempts, and stable idempotency key. |
| `services` | Scripts and local modules | Built-in and imported managed capabilities. |
| `secrets` | Scripts with statically proven literal secret reads | Resolved values for exact `secrets.NAME` access. |
| `fetch` | Scripts and local modules | Bun-native Fetch API. |
| `console` | Scripts | Redacting terminal logging. |
| `lifecycle` | Lifecycle scripts only | Read-only lifecycle event data. |
| `props` | Reusable step/provider scripts | Declared invocation props. |
| `notification` | Reusable notification providers | Bounded message, actions, and delivery idempotency data. |

`context.run` and `context.env` do not exist. `context.trigger` is a deprecated
runtime compatibility alias for older compiled definitions; new source uses
`context.payload`.

### 8.5 `context`

Public paths:

```js
context.payload
context.steps.<stepId>
context.steps.<choiceOrSwitchOrApprovalId>
```

`context.payload` always means workflow input, whether the run came from a
webhook, provider, timer, event, manual trigger, workflow call, or workflow
start. `context.steps` contains only outputs visible at the current DAG
position. The object is a derived read-only projection, not authoritative
mutable state.

### 8.6 `attempt`

```js
attempt.number          // one-based current attempt
attempt.maxAttempts     // compiled total-attempt limit
attempt.idempotencyKey  // stable across attempts of this logical step/run
```

Use the idempotency key only with an external API that explicitly supports an
idempotency field or header.

### 8.7 `secrets`

Scripts may use only literal, statically discoverable reads:

```js
secrets.PAYMENTS_API_TOKEN
```

Dynamic enumeration and `secrets[name]` are not supported. Compiled models
store names, not values. Attribute syntax uses `{{secrets.NAME}}`, not
JavaScript syntax.

## 9. Control flow

### 9.1 `<parallel>`

```xml
<parallel
  id="checks"
  name="Run checks"
  description="Check stock and risk concurrently."
  concurrency="2"
  on-error="wait-all"
>
  <step id="stockCheck"><script>return checkStock();</script></step>
  <step id="riskCheck"><script>return checkRisk();</script></step>
</parallel>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Stable parallel-group identity; not a context output. |
| `name`, `description` | No | Display metadata. |
| `concurrency` | No | Maximum active children; defaults to child count and cannot exceed it. |
| `on-error` | No | `fail-fast` or `wait-all`; defaults to `fail-fast`. |

Rules:

- It contains one or more direct `<step>` children only.
- Every child sees the same pre-fork context.
- Siblings cannot reference each other's output.
- Each child publishes its own `context.steps.<id>` value.
- The group has no aggregate output.
- `fail-fast` stops unstarted children and requests cancellation of active
  children; `wait-all` lets all children settle. Both fail the group if any
  child fails.
- Add a downstream step to create one final aggregate result.

Use `<parallel>` for several independent single-step operations. Use `<fork>`
for concurrent lanes containing multiple steps.

### 9.2 `<choose>`, `<when>`, `<otherwise>`, and `<result>`

Use `<choose>` for strict boolean decisions. Complex expressions belong in a
preceding script step.

```xml
<choose id="route" name="Select route">
  <when test="{{context.steps.isApproved}}">
    <step id="approved"><script>return { status: "approved" };</script></step>
    <result value="{{context.steps.approved}}" />
  </when>

  <otherwise>
    <step id="rejected"><script>return { status: "rejected" };</script></step>
    <result value="{{context.steps.rejected}}" />
  </otherwise>
</choose>
```

Result-producing profile:

- `<choose>` requires `id`; `name` and `description` are optional.
- One or more ordered `<when>` arms are required.
- Exactly one final `<otherwise>` is required.
- Each arm has one or more flow items followed by exactly one `<result>`.
- `test` must be one exact context reference whose runtime value is a JSON
  boolean. WOML does not coerce truthy values.
- The first true `<when>` wins; otherwise the fallback runs.
- The selected `<result>` value is published at
  `context.steps.<chooseId>`, giving later steps one predictable output.

Control-only profile:

- Omit the `id`.
- Arms are non-empty and omit `<result>`.
- The choice controls execution but publishes no merged output.

### 9.3 `<switch>`, `<case>`, and `<default>`

Use `<switch>` for exact string routing:

```xml
<switch id="delivery" value="{{context.steps.order.provider}}">
  <case value="express">
    <step id="express"><script>return { days: 1 };</script></step>
    <result value="{{context.steps.express}}" />
  </case>
  <case value="standard">
    <step id="standard"><script>return { days: 5 };</script></step>
    <result value="{{context.steps.standard}}" />
  </case>
  <default>
    <step id="unsupported"><script>return { supported: false };</script></step>
    <result value="{{context.steps.unsupported}}" />
  </default>
</switch>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `value` | Yes | One exact context reference; runtime value must be a string. |
| `id` | No | When present, enables a merged result at `context.steps.<id>`. |
| `name`, `description` | No | Available with the result-producing profile. |

There must be one or more unique non-empty `<case value="...">` arms and one
final `<default>`. Matching is exact, ordered, case-sensitive, and performs no
trim or coercion. There is no fallthrough. An ID-less switch has no `<result>`;
an ID-bearing switch requires exactly one final `<result>` in every arm.

### 9.4 `<fork>` and `<branch>`

Use a fork for several independent, concurrent, multi-step routes:

```xml
<fork id="distribution" join="instagram facebook">
  <branch id="tiktok">
    <step id="prepareTikTok"><script>return prepareTikTok();</script></step>
    <step id="publishTikTok"><script>return publish(context.steps.prepareTikTok);</script></step>
  </branch>

  <branch id="instagram">
    <step id="publishInstagram"><script>return publishInstagram();</script></step>
  </branch>

  <branch id="facebook">
    <step id="publishFacebook"><script>return publishFacebook();</script></step>
  </branch>
</fork>
```

`<fork>` attributes:

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Stable fork identity. |
| `join` | No | `all`, `none`, or a whitespace-separated list of branch IDs. Omission means `all`. |

`<branch>` requires `id` and optionally accepts `name` and `description`. It
must be non-empty and may contain multiple steps, choices, switches, parallel
groups, and approvals.

Execution rules:

- Branches begin concurrently; each branch remains sequential internally.
- A branch sees context available before the fork and earlier outputs in that
  branch, never sibling outputs.
- The main continuation waits only for selected joined branches.
- Only joined branch outputs become visible to the continuation. Completion
  timing never makes an unjoined output visible.
- `join="none"` releases the continuation without waiting.
- Even after the continuation is released, the workflow itself does not report
  success until every fork-owned branch settles.
- A failure in a joined branch blocks the continuation. An unjoined failure
  does not block it, but the overall workflow still settles as failed after all
  branches finish.
- Nested forks anywhere inside a fork-owned branch subtree are rejected in v1.
- A terminal fork preserves the last earlier main-route value. A workflow made
  only of a result-less fork is invalid.

### 9.5 `<approval>`

```xml
<approval
  id="managerApproval"
  name="Manager approval"
  description="Approve the calculated refund."
  timeout="24h"
  on-timeout="reject"
>
  <notify>
    <telegram
      chats="123456789"
      bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
    />
  </notify>

  <when-approved>
    <step id="sendRefund"><script>return issueRefund();</script></step>
  </when-approved>

  <when-rejected>
    <step id="recordRejection"><script>return { rejected: true };</script></step>
  </when-rejected>
</approval>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Waiting-node and `context.steps.<id>` output identity. |
| `name`, `description` | No | Human-facing decision context. |
| `timeout` | No | Maximum durable wait. Omission means no WOML deadline. |
| `on-timeout` | No | `reject` or `fail`; defaults to `fail` and requires `timeout`. |

Child order is fixed: optional `<notify>`, required `<when-approved>`, required
`<when-rejected>`. Decision arms may be empty; an empty selected arm is a
successful no-op.

WOML persists the wait and opaque capability before notification. Restart does
not lose it. The first valid decision wins. Matching repeats are idempotent;
an opposing later decision conflicts. The approval result becomes:

```json
{
  "decision": "approved",
  "source": "human",
  "decidedAt": "2026-08-04T12:00:00.000Z"
}
```

`source` is `human` or `timeout`. `on-timeout="reject"` publishes a rejected
result and runs `<when-rejected>`. `on-timeout="fail"` fails the run without
executing either arm.

The public decision mechanism is HTTP:

```http
POST /api/v1/approvals/{token}/decision
Content-Type: application/json

{ "decision": "approved" }
```

Valid decisions are exactly `approved` and `rejected`. There is no npm API or
`woml.resume()` function.

## 10. Notifications

### 10.1 `<notify>` behavior

`<notify>` accepts one or more built-in or explicitly imported custom provider
tags. In an approval, every delivery has its own single-use capability, but all
deliveries settle the same approval. One successful delivery is enough to wait
for a decision; if every configured delivery fails, the run fails rather than
waiting invisibly.

In a lifecycle hook, notifications are informational. They never receive
decision authority and failure becomes a lifecycle warning rather than a new
business outcome.

### 10.2 Slack notification

Approval:

```xml
<slack
  channels="#approvals #engineering"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>
```

Lifecycle notification adds required `message`:

```xml
<slack
  channels="#incidents"
  message="Workflow {{lifecycle.workflow.id}} failed: {{lifecycle.failure.code}}"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>
```

`channels` is whitespace-separated and accepts lowercase `#channel` aliases or
Slack conversation IDs. Duplicate destination/credential combinations are
rejected.

### 10.3 Telegram notification

```xml
<telegram chats="-1001234567890,123456789" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
```

`chats` is a comma-separated list of positive or negative numeric chat IDs.
Approval notifications forbid `message`; lifecycle notifications require it.

### 10.4 Discord notification

```xml
<discord channels="200000000000000001,200000000000000002" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />
```

`channels` is a comma-separated list of 17–20 digit channel IDs. Approval
notifications forbid `message`; lifecycle notifications require it.

### 10.5 WhatsApp notification

```xml
<whatsapp
  recipients="15551234567,15557654321"
  access-token="{{secrets.WHATSAPP_ACCESS_TOKEN}}"
  phone-number-id="123456789012345"
  template="woml_approval_v1"
  language="en_US"
/>
```

Recipients are comma-separated international numbers containing 8–16 digits,
without `+` or spaces. Template names use lowercase letters, digits, and
underscores. Approval templates must provide the approved Approve/Reject
button order. Lifecycle notifications additionally require `message`, passed
as the first template body parameter. Meta does not let WOML edit a delivered
approved-template message after resolution, so WOML records a bounded warning
instead of claiming it updated the message.

### 10.6 Message templates

Lifecycle messages may include bounded scalar `{{context...}}` and
`{{lifecycle...}}` placeholders. Secret interpolation is forbidden. Approval
message wording is assembled from approval metadata and runtime decision data;
credentials never enter message content.

### 10.7 Set up Slack

Slack uses two credentials because WOML connects through
[Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/):

- a bot token (`xoxb-...`) authorizes messages, channel lookup, and events; and
- an app-level token (`xapp-...`) opens the Socket Mode connection.

This setup does not require a public callback URL.

#### 1. Create and configure the Slack app

1. Open [Slack API apps](https://api.slack.com/apps) and select **Create New
   App**.
2. Choose **From an app manifest**, select the workspace, and use this manifest:

```json
{
  "display_information": {
    "name": "WOML",
    "description": "Run and approve WOML workflows from Slack.",
    "background_color": "#16a34a"
  },
  "features": {
    "bot_user": {
      "display_name": "WOML",
      "always_online": false
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "chat:write",
        "chat:write.public",
        "app_mentions:read",
        "channels:read",
        "groups:read",
        "im:history"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "bot_events": ["app_mention", "message.im"]
    },
    "interactivity": { "is_enabled": true },
    "socket_mode_enabled": true,
    "token_rotation_enabled": false
  }
}
```

3. Open **Event Subscriptions** and confirm **Enable Events** is on. The bot
   events must include `app_mention` and `message.im`.
4. Open **Interactivity & Shortcuts** and confirm interactivity is enabled.
   Approval buttons depend on it.
5. Open **Socket Mode** and confirm it is enabled.
6. Open **Basic Information → App-Level Tokens**, create a token with the
   `connections:write` scope, and copy the resulting `xapp-...` token.
7. Open **OAuth & Permissions**, install the app to the workspace, and copy the
   **Bot User OAuth Token** beginning with `xoxb-`.

If scopes or event subscriptions change later, reinstall the app to the
workspace and replace the stored bot token when Slack issues a new one.

#### 2. Add the bot to the channel

Create or open a test channel such as `#woml-testing`, then run this inside the
channel:

```text
/invite @WOML
```

Private channels always require an explicit invite. Inviting the bot is also
recommended for public channels.

#### 3. Store the credentials

Run these commands from the project that will run the workflow:

```bash
woml secrets set SLACK_BOT_TOKEN
woml secrets set SLACK_APP_TOKEN
```

Paste the `xoxb-...` value into the first prompt and the `xapp-...` value into
the second. Do not place either token directly in a `.woml` file or commit it
to source control.

#### 4. Test a Slack trigger

Save this as `slack-test.woml`:

```xml
<woml>
  <workflow id="slack-test" name="Slack Test" version="1.0.0">
    <triggers>
      <slack
        id="messageReceived"
        events="app-mention,direct-message"
        channels="woml-testing"
        bot-token="{{secrets.SLACK_BOT_TOKEN}}"
        app-token="{{secrets.SLACK_APP_TOKEN}}"
      />
    </triggers>

    <steps>
      <step id="capture" name="Capture Slack message">
        <script>
          return {
            text: context.payload.text,
            senderId: context.payload.senderId,
            conversationId: context.payload.conversationId
          };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

Validate and start it:

```bash
woml check slack-test.woml
woml run slack-test.woml
```

Wait for `Slack workspace ... is ready for triggers`, then send
`@WOML hello` in `#woml-testing` or send the bot a direct message. WOML should
accept a Slack trigger, run `capture`, and print the normalized message data.

The trigger's `channels` attribute uses comma-separated names **without** `#`.
Notification destinations use whitespace-separated aliases **with** `#`, for
example:

```xml
<approval id="review" timeout="24h" on-timeout="reject">
  <notify>
    <slack
      channels="#woml-testing"
      bot-token="{{secrets.SLACK_BOT_TOKEN}}"
      app-token="{{secrets.SLACK_APP_TOKEN}}"
    />
  </notify>
  <when-approved />
  <when-rejected />
</approval>
```

#### 5. Diagnose Slack setup failures

- No events arrive: make sure **Enable Events** is on and the bot events include
  `app_mention` and `message.im`.
- Authentication fails: verify that the bot token starts with `xoxb-`, the app
  token starts with `xapp-`, and the app token has `connections:write`.
- Channel lookup reports `missing_scope`: add `channels:read` and
  `groups:read`, then reinstall the app. `channels:history` is not a replacement
  for `channels:read`.
- A private channel is not found: invite `@WOML` to it or use its Slack
  conversation ID.
- Messages send but buttons do nothing: enable **Interactivity & Shortcuts**.
- An updated app still behaves like the old configuration: reinstall it to the
  workspace and update the stored token.

### 10.8 Set up Telegram

Telegram needs one Bot API token for triggers, messages, approvals, and
lifecycle notifications. It does not need an app token or a public callback
URL; WOML receives messages through long polling.

#### 1. Create the bot

1. Open Telegram and start a conversation with the verified
   [@BotFather](https://t.me/BotFather) account.
2. Send `/newbot`.
3. Choose a display name.
4. Choose a unique username ending in `bot`, such as `my_woml_bot`.
5. Copy the Bot API token returned by BotFather. Treat it like a password.

#### 2. Store and verify the token

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml telegram doctor
```

The doctor command verifies that Telegram accepts the token without exposing
it in workflow source or terminal output.

#### 3. Test a Telegram trigger and reply

Save this as `telegram-test.woml`:

```xml
<woml>
  <workflow
    id="telegram-test"
    name="Telegram Test"
    description="Replies to every message received by the bot."
    version="1.0.0"
  >
    <triggers>
      <telegram
        id="messageReceived"
        events="message"
        bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
      />
    </triggers>

    <steps>
      <step id="reply" name="Reply in Telegram">
        <script>
          return services.telegram.send({
            botToken: secrets.TELEGRAM_BOT_TOKEN,
            conversationId: context.payload.conversationId,
            replyToMessageId: context.payload.messageId,
            text: `WOML received: ${context.payload.text}`
          }, { name: "telegram-test-reply" });
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

Run it:

```bash
woml check telegram-test.woml
woml run telegram-test.woml
```

Open the bot in Telegram, press **Start** or send `/start`, then send `hello`.
WOML should accept the message and the bot should reply with
`WOML received: hello`.

The run output includes the normalized `conversationId`. Use that numeric value
as a notification destination. You can verify it before using approvals:

```bash
woml telegram doctor --destination <conversationId>
```

An approval notification then looks like this:

```xml
<approval id="review" timeout="24h" on-timeout="reject">
  <notify>
    <telegram
      chats="<conversationId>"
      bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
    />
  </notify>
  <when-approved />
  <when-rejected />
</approval>
```

For groups and channels, the ID is commonly negative. Copy the exact value;
do not remove the minus sign.

#### 4. Configure group messages when needed

Telegram bots use privacy mode in groups by default. In that mode they receive
commands, replies directed to them, and other explicitly relevant messages,
but not every ordinary group message. If the workflow genuinely needs all
group messages:

1. Open BotFather and send `/setprivacy`.
2. Select the bot and disable privacy mode.
3. Remove and re-add the bot to the group so Telegram applies the change.

Only disable privacy when the automation requires it; narrower access is safer
and less noisy.

#### 5. Diagnose Telegram setup failures

- `Unauthorized`: replace `TELEGRAM_BOT_TOKEN` with the current BotFather
  token.
- Nothing happens in a private chat: press **Start** first; bots cannot begin a
  conversation with a user.
- The trigger repeatedly rejects the same update: stop other processes using
  the same bot token. Only one long-polling consumer should own that bot.
- Group messages are missing: mention/reply to the bot or review privacy mode.
- Notification delivery fails: verify the numeric chat ID with
  `woml telegram doctor --destination ...` and make sure the bot is still a
  member of the group or channel.

### 10.9 Set up Discord

Discord uses one bot token for Gateway triggers, messages, approvals, and
lifecycle notifications. The application's **Public Key is not the bot
token**.

#### 1. Create and install the Discord bot

1. Open the [Discord Developer Portal](https://discord.com/developers/applications),
   select **New Application**, and name it.
2. Open the application's **Bot** page. Generate or reset the token, copy it,
   and store it immediately. Discord may show it only once.
3. On the same page, enable the privileged **Message Content Intent**. WOML
   needs it to read message text.
4. Open **Installation** and enable a **Guild Install** with the `bot` scope.
5. Grant the bot **View Channels**, **Send Messages**, and
   **Read Message History**.
6. Copy the installation link, open it, select a test server, and authorize the
   app.

WOML does not require slash commands, so `applications.commands` is not needed
for the v1 message trigger.

#### 2. Find a channel ID

1. In Discord, open **User Settings → Advanced**.
2. Enable **Developer Mode**.
3. Right-click the channel and select **Copy Channel ID**.

Keep the 17–20 digit value for diagnostics, channel filtering, and
notifications.

#### 3. Store and verify the token

```bash
woml secrets set DISCORD_BOT_TOKEN
woml discord doctor --destination <channelId>
```

This verifies both bot authentication and delivery access to the selected
channel.

#### 4. Test a Discord trigger and reply

Save this as `discord-test.woml`:

```xml
<woml>
  <workflow
    id="discord-test"
    name="Discord Test"
    description="Replies when the bot is mentioned or receives a direct message."
    version="1.0.0"
  >
    <triggers>
      <discord
        id="messageReceived"
        events="app-mention,direct-message"
        channels="<channelId>"
        bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
      />
    </triggers>

    <steps>
      <step id="reply" name="Reply in Discord">
        <script>
          return services.discord.send({
            botToken: secrets.DISCORD_BOT_TOKEN,
            conversationId: context.payload.conversationId,
            replyToMessageId: context.payload.messageId,
            text: `WOML received: ${context.payload.text}`
          }, { name: "discord-test-reply" });
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

Replace both `<channelId>` placeholders, then run:

```bash
woml check discord-test.woml
woml run discord-test.woml
```

Mention the bot in the configured server channel and send `hello`, or send it a
direct message. WOML should accept the Gateway event and reply in the same
conversation.

Use the same channel ID for an approval notification:

```xml
<approval id="review" timeout="24h" on-timeout="reject">
  <notify>
    <discord
      channels="<channelId>"
      bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
    />
  </notify>
  <when-approved />
  <when-rejected />
</approval>
```

#### 5. Diagnose Discord setup failures

- Authentication fails: store the bot token from the **Bot** page, not the
  Public Key, Application ID, or Client Secret.
- The bot is online but message text is empty: enable **Message Content
  Intent**, then restart WOML.
- Mentions do not arrive: confirm the bot is installed in the server and can
  view that channel.
- Replies or notifications fail: grant **Send Messages** and **Read Message
  History**, and verify the channel using `woml discord doctor --destination`.
- Direct messages do not arrive: confirm the user and server privacy settings
  allow DMs to the bot.
- Events duplicate or reconnect repeatedly: make sure only the intended WOML
  runtime is using that workflow and token.

## 11. Lifecycle

A workflow has at most one `<lifecycle>`. Hook source order does not matter;
WOML normalizes semantic order.

| Hook | Optional `steps` filter | Runs when |
| --- | :---: | --- |
| `<on-start>` | No | Run is durably admitted, before the business DAG. |
| `<on-step-start>` | Yes | Logical step begins before its first attempt. |
| `<on-step-success>` | Yes | Logical step eventually succeeds. |
| `<on-step-failure>` | Yes | Logical step exhausts attempts. |
| `<on-step-complete>` | Yes | Logical step settles as success or failure. |
| `<on-success>` | No | Business outcome is success. |
| `<on-error>` | No | Business outcome is failure, including workflow timeout. |
| `<on-cancel>` | No | Durable cancellation wins. |
| `<on-complete>` | No | Outcome hook settles and the run is finalizing. |

Each hook may occur once and contains one or more source-ordered `<script>` or
`<notify>` actions. Step filters are whitespace-separated step IDs; omission
means every executable step, including nested steps.

Lifecycle scripts receive normal script bindings plus:

```js
lifecycle.event
lifecycle.workflow.id
lifecycle.workflow.outcome
lifecycle.step?.id
lifecycle.step?.outcome
lifecycle.step?.attempts
lifecycle.failure?.code
lifecycle.failure?.message
```

They cannot create `context.steps` values, choose a branch, recover a step, or
rewrite the workflow result. Failures become durable lifecycle warnings. The
run finalizes with lifecycle status `completed` or `completed_with_warnings`.

Reusable step/provider definitions accept only `<on-success>`, `<on-error>`,
and `<on-complete>`. Their hooks are observational and script-only.

## 12. Attribute references and templates

Exact references preserve JSON type:

```xml
<when test="{{context.steps.isEligible}}">
```

Mixed templates always produce strings and are accepted only by attributes
whose contract permits templates:

```xml
message="Order {{context.payload.orderId}} failed"
```

Reference grammar:

```text
{{context.payload}}
{{context.payload.property.nested}}
{{context.steps.stepId}}
{{context.steps.stepId.property.nested}}
{{secrets.UPPERCASE_NAME}}
```

No whitespace, bracket access, optional chaining, operators, calls, or
fallbacks are allowed inside declarative references. A referenced output must
exist and dominate the consumer in the compiled DAG. Missing properties fail
with `WOML_REFERENCE_NOT_AVAILABLE`; they do not become `undefined` or an empty
string. Context references validate data access but do not create hidden graph
edges.

## 13. Local modules

### 13.1 Import

```xml
<imports>
  <module name="pricing" from="./modules/pricing.ts" />
</imports>
```

`name` is the service alias. `from` is a static relative `.js` or `.ts` path.
The module uses named ESM exports:

```ts
export function total(price: number, quantity: number) {
  return price * quantity;
}
```

Call it as:

```js
return services.pricing.total(context.payload.price, context.payload.quantity);
```

Supported:

- named ESM exports;
- static relative `.js` and `.ts` imports inside the local module graph;
- pure functions and functions using the injected `services` global;
- deterministic bundling and immutable recovery artifacts.

Unsupported:

- default exports;
- CommonJS;
- dynamic imports;
- npm/package imports;
- runtime package installation; and
- initialization side effects.

Local modules receive `services` and native Fetch. They do not automatically
receive `context`, `attempt`, or `secrets`; pass required values explicitly.
This makes secret and data dependencies visible at the workflow call site.

`woml check` and `woml run` refresh `woml-env.d.ts` automatically. `woml
types` is only for an explicit refresh or custom output path.

## 14. Reusable WOML definitions

### 14.1 `<props>` and `<prop>`

Props belong only in reusable definition files, outside the top-level reusable
step/provider. They are invalid in runnable workflow documents.

```xml
<props>
  <prop name="price" required="true" />
  <prop name="api-token" required="true" secret="true" />
</props>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `name` | Yes | Kebab-case public attribute name; exposed as lower camel case under `props`. |
| `required` | No | Boolean requirement declaration. |
| `secret` | No | Requires an exact `{{secrets.NAME}}` invocation value and restricts exposure. |

### 14.2 Reusable custom step

Definition:

```xml
<woml>
  <props>
    <prop name="price" required="true" />
  </props>
  <step name="Calculate tax" description="Apply the project tax rule.">
    <script>
      return { total: Number(props.price) * 1.2 };
    </script>
  </step>
</woml>
```

Import and invoke:

```xml
<imports>
  <module name="calculate-tax" from="./calculate-tax.woml" />
</imports>

<steps>
  <calculate-tax id="tax" price="{{context.payload.price}}" retry="3" />
  <step id="finish"><script>return context.steps.tax;</script></step>
</steps>
```

The invocation behaves as one normal durable step. It requires an invocation
`id`, accepts ordinary retry attributes, publishes a result under that ID, and
has its own reusable lifecycle.

### 14.3 `<provider kind="notification">`

```xml
<woml>
  <props>
    <prop name="api-token" required="true" secret="true" />
    <prop name="destination" required="true" />
  </props>
  <provider kind="notification">
    <script>
      const response = await services.http.request({
        url: "https://messaging.example.com/messages",
        method: "POST",
        headers: { authorization: `Bearer ${props.apiToken}` },
        json: {
          destination: props.destination,
          text: notification.message,
          actions: notification.actions
        },
        idempotency: {
          header: "Idempotency-Key",
          value: notification.idempotencyKey
        }
      });
      return { messageId: String(response.data.id) };
    </script>
  </provider>
</woml>
```

Only `kind="notification"` exists in v1. Provider-trigger extensions are not
yet public. Use the imported custom tag directly inside `<notify>`. The script
transports the bounded `notification` object; it does not own the approval
decision. Custom tags cannot accept children or generate hidden control-flow
structure.

## 15. Built-in services

Every managed failure is catchable as `WomlServiceError` with stable fields:

```js
error.code
error.service
error.operation
error.callId
error.retryable
error.ambiguous
error.details
```

Give repeated effectful operations in one step stable `{ name: "..." }`
options. Names identify logical effects and must not depend on attempt number or
loop order.

The default managed-capability boundary accepts 1 MiB input, 4 MiB result, an
8 MiB transport frame, and a 30-second operation timeout unless a particular
service documents a smaller limit or the request supplies a supported timeout.

### 15.1 Native `fetch()`

```js
const response = await fetch("https://api.example.com/data");
const data = await response.json();
```

This is Bun's standard Fetch API: it returns `Response`, supports streams, and
does not throw merely because the status is non-2xx. WOML records redacted
observations, but Bun owns the actual request/body. Use it for Web API
compatibility and streaming. Prefer managed HTTP when you need Rust-owned
limits, status policy, cancellation, and durable operation outcomes.

### 15.2 `services.http.request()`

```js
const response = await services.http.request({
  url: "https://api.example.com/customers",
  method: "POST",
  headers: { authorization: `Bearer ${secrets.API_TOKEN}` },
  query: { source: "woml" },
  json: { customerId: "customer-42" },
  timeout: "10s",
  acceptedStatus: { minimum: 200, maximum: 299 },
  redirect: "follow",
  maximumRedirects: 10,
  idempotency: {
    header: "Idempotency-Key",
    value: attempt.idempotencyKey
  }
}, { name: "create-customer" });
```

Request fields:

| Field | Meaning |
| --- | --- |
| `url` | Required HTTP(S) URL. |
| `method` | Defaults to `GET`. |
| `headers` | String header map. Credentials remain private operation input. |
| `query` | Primitive query values. |
| `json`, `text`, `bytesBase64` | Mutually exclusive body forms. |
| `responseType` | `json` (default), `text`, `bytes`, or `storage`. |
| `timeout` / `timeoutMs` | Request deadline; default 30 seconds. |
| `acceptedStatus` | Accepted status interval; default 200–299. |
| `redirect` | `follow`, `error`, or `manual`. |
| `maximumRedirects` | Defaults to 10. |
| `storage` | Direct-to-storage target when `responseType="storage"`. |
| `idempotency` | External idempotency header/value contract. |

Return value:

```js
{
  status,
  ok,          // true only for native HTTP 2xx
  headers,
  data,
  url,
  redirected
}
```

Rust owns pooling, TLS validation, redirect handling, decompression, parsing,
timeouts, cancellation, response limits, and operation events. The local
profile allows reachable HTTP(S) destinations, including private and loopback
addresses; production users must apply egress controls when untrusted input
can influence URLs.

### 15.3 `services.db()`

```js
const db = services.db({
  driver: "sqlite", // or "postgres"
  connection: "./.woml/app.sqlite"
});
```

Configuration accepts exactly `driver` and `connection`. Never point SQLite at
WOML's own runtime-state database. PostgreSQL connections should normally come
from a secret.

Methods:

```js
await db.query({ text, values? }, options?);
await db.execute({ text, values? }, options?);
await db.read({ table, columns?, where?, orderBy?, limit? }, options?);
await db.insert({ table, values }, options?);
await db.update({ table, values, where }, options?);
await db.delete({ table, where }, options?);
await db.transaction({ operations }, options?);
```

SQLite uses `?` parameters; PostgreSQL uses `$1`, `$2`, and so on. Never
interpolate untrusted values into SQL text. `update` and `delete` helpers
require non-empty `where`. An intentional bulk write uses explicit SQL.

Reads return `{ rows, rowCount }`. Mutations return
`{ rowsAffected, lastInsertId }`; PostgreSQL returns `lastInsertId: null`.
Transactions contain 1–100 query/execute/CRUD operations, run on one
connection, and commit only if every operation succeeds. Nested transactions
and NoSQL/document drivers are not available.

Limits include 256 KiB SQL, 10,000 result rows, 256 columns, 100 transaction
operations, 1 MiB capability input, and 4 MiB capability result data.

### 15.4 `services.storage`

Rust-owned object storage is for files and larger durable values that should
not be copied into every context projection.

```js
const object = await services.storage.put({
  key: "reports/daily.json",
  value: { customers: 42 }
});

const loaded = await services.storage.get({
  key: object.key,
  responseType: "json",
  ifVersion: object.version
});
```

Methods:

| Method | Behavior |
| --- | --- |
| `put({ key, value | text | bytesBase64, contentType?, overwrite?, ifVersion? })` | Atomically writes and returns an object reference. Create-only by default. |
| `get({ key, responseType?, ifVersion? })` | Verifies checksum and returns `{ object, data }`. |
| `head({ key })` | Returns the reference or null without body. |
| `list({ prefix?, limit?, cursor? })` | Key-ordered references; default 100, maximum 1,000. |
| `delete({ key, ifVersion? })` | Removes one object; missing unconditional delete returns `deleted: false`. |

References contain contract/version, logical key, content-derived version,
SHA-256 checksum, size, and content type. Keys are logical names, not paths;
absolute paths, traversal, backslashes, empty segments, NULs, and control
characters are rejected. Local object maximum is 64 MiB. Back up the
`objects-v1` directory beside the state database together with that database.

### 15.5 `services.cache`

Cache is workflow-scoped, expiring optimization data. Correctness must tolerate
a miss, expiry, eviction, or different state location.

```js
const cached = await services.cache.get("customer:42");
if (cached.hit) return cached.value;

const value = await loadCustomer();
await services.cache.set("customer:42", value, { ttl: "15m" });
return value;
```

| Method | Result/behavior |
| --- | --- |
| `get(key)` | `{ hit: true, value, expiresAt }` or `{ hit: false }`. |
| `set(key, value, { ttl?, name? })` | Replaces value; returns `{ stored: true, expiresAt }`. |
| `delete(key, options?)` | Returns `{ deleted }`. |
| `has(key)` | Returns `{ present }`. |
| `increment(key, amount?, options?)` | Atomic safe-integer addition; missing starts at zero. |
| `setIfAbsent(key, value, options?)` | Atomic one-winner initialization. |

TTL defaults to `5m`, ranges from 1 ms through 30 days, and may be milliseconds
or duration text. The local cache stores at most 10,000 entries, 64 MiB total,
256 KiB per value, and 256 UTF-8 bytes per key. It removes expired entries then
evicts least-recently-used entries. Unexpired values survive local restart but
are never durable business state.

### 15.6 `services.state`

State is small permanent JSON memory shared by future runs of the same workflow
ID and state location.

```js
const current = await services.state.get("order-count");
const result = await services.state.increment(
  "order-count",
  1,
  { name: "increment-order-count" }
);
```

| Method | Signature and behavior |
| --- | --- |
| `get<T>(key)` | `{ found: true, value, version, updatedAt }` or `{ found: false }`. |
| `has(key)` | `{ present, version? }`. |
| `set(key, value, { name, ifVersion? })` | Durable versioned write. |
| `delete(key, { name, ifVersion? })` | Durable conditional delete. |
| `increment(key, amount, { name, ifVersion? })` | Atomic durable counter. |
| `setIfAbsent(key, value, { name })` | Atomic one-winner durable initialization. |

Every mutation requires a stable name. Retry with the same identity reattaches
to the original committed result. Use `ifVersion` for compare-and-set. State
does not expire, evict, enter context, or appear in normal run inspection. It
is not transparently encrypted; protect the state file and backups and do not
store secrets in state. Limits are 256 UTF-8 bytes per key, 256 KiB canonical
JSON per value, 10,000 live keys, and 64 MiB of canonical values per workflow
scope.

### 15.7 `services.events.emit()`

```js
const publication = await services.events.emit(
  "customer.updated",
  { customerId: "customer-42" },
  { name: "publish-customer-update" }
);
```

It directly and durably fans out to all loaded `<event>` subscribers without
HTTP or a control token. The result contains publication ID, event name,
accepted/duplicate/rejected counts, status, depth, and safe per-subscriber
delivery results. No subscribers is a successful no-op. Payloads must be
top-level JSON objects no larger than 1 MiB. Limits: 1,000 subscribers per
name, lineage depth 32, and cycle rejection.

### 15.8 `services.workflows.call()`

```js
const risk = await services.workflows.call(
  "calculate-risk",
  { customerId: context.payload.customerId },
  { name: "calculate-customer-risk", timeout: "30s" }
);
```

The target must be active in the same runtime or another local process sharing
the same state database. The payload becomes the child's complete
`context.payload`. `call()` waits for terminal child status and resolves to the
child's final JSON value. A child must return JSON, including explicit `null`;
missing/undefined output is an error.

Calls are independently durable, deduplicate retries, reject cycles, and apply
the child's own runtime policy. A parent waiting for a child releases its
concurrency slot. Synchronous call rejects targets containing Human Approval
because the Bun continuation cannot be serialized across a long durable wait.
Cancelling the waiting parent does not silently cancel the independent child.
Workflow-call payloads are limited to 1 MiB and child results to 4 MiB.

### 15.9 `services.workflows.start()`

```js
const child = await services.workflows.start(
  "send-report",
  { reportId: context.steps.report.id },
  { name: "start-report-delivery" }
);
```

`start()` waits only for durable admission/dispatch, then returns:

```js
{ workflowId, runId, duplicate }
```

The parent continues while the child runs. The child's later failure does not
retroactively fail the completed parent step. Use the returned run ID with
`woml get` or log following.

### 15.10 `services.telegram.send()`

```js
await services.telegram.send({
  botToken: secrets.TELEGRAM_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: "Hello from WOML",
  replyToMessageId: context.payload.messageId
}, { name: "telegram-reply" });
```

### 15.11 `services.discord.send()`

```js
await services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: "Hello from WOML",
  replyToMessageId: context.payload.messageId
}, { name: "discord-reply" });
```

Telegram and Discord return `{ provider, conversationId, messageId,
acceptedAt, threadId? }`.

### 15.12 `services.whatsapp.send()`

```js
await services.whatsapp.send({
  accessToken: secrets.WHATSAPP_ACCESS_TOKEN,
  phoneNumberId: "123456789012345",
  conversationId: context.payload.conversationId,
  template: {
    name: "woml_reply_v1",
    language: "en_US",
    parameters: ["Hello from WOML"]
  }
}, { name: "whatsapp-reply" });
```

WhatsApp v1 sends approved templates, not arbitrary proactive free-form text.
The return shape matches the other messaging services.

`services.queue` and `services.slack.send()` are not public v1 services.
Slack remains available as a trigger and notification provider.

## 16. Secrets

### 16.1 Local secret commands

```bash
woml secrets set <NAME>
woml secrets list
woml secrets delete <NAME>
```

Names use uppercase symbolic form such as `PAYMENTS_API_TOKEN`. `set` prompts
without echoing the plaintext value. `list` prints names only. Workflows refer
to configured values with `{{secrets.NAME}}` in supported attributes or
`secrets.NAME` in scripts.

Run secret commands from the project whose secret store you intend to use.
Never commit `.woml/`, `.env`, mounted secret files, or literal credentials.

### 16.2 Production secret sources

The default is the local OS credential store. Production can select:

```bash
WOML_SECRETS_PROVIDER=env woml run workflows/

WOML_SECRETS_PROVIDER=files \
WOML_SECRETS_DIRECTORY=/run/secrets \
woml run workflows/
```

Environment names map to `WOML_SECRET_<NAME>`. The files provider reads one
secure regular file named exactly `<NAME>` from an absolute protected
directory. `WOML_SECRETS_PROVIDER=production` checks mounted files, then
environment, then the OS store. Conflicting values across sources fail
activation. WOML resolves only names referenced by selected definitions.

## 17. CLI reference

Human-readable colored output is default. Commands offering `--json` expose a
stable machine-readable form. Relative paths resolve from the current working
directory. Default durable state is `.woml/state.sqlite`.

### 17.1 `woml check`

```bash
woml check <workflow.woml|directory>... [--config <path>] [--json]
```

Parses, validates, compiles, checks cross-workflow calls, packages module
graphs, verifies referenced secret names when production config is supplied,
and refreshes editor declarations. It does not execute scripts, activate
triggers, connect providers, or create workflow runs. Direct `.woml` files in a
directory are loaded non-recursively.

Use `--config` for production preflight, including listener/storage checks and
an aggregated missing-secret report. Use `--json` in CI.

### 17.2 `woml run`

```bash
woml run <workflow.woml|directory>... \
  [--config <path>] [--host <address>] [--port <port>] \
  [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>] [--json] [--verbose] \
  [--color=auto|always|never] [--background|-d]
```

Activates one or more workflows as one deployment unit. It validates the full
set, pins exact definitions/module artifacts, prepares recovery, starts
providers and listeners behind a closed readiness gate, then opens admission
atomically. It remains active for future trigger occurrences.

Important options:

| Option | Meaning |
| --- | --- |
| `--config` | Runtime configuration JSON. |
| `--host`, `--port` | Override public trigger listener. |
| `--state` | Select durable SQLite authority; use consistently across related processes/commands. |
| `--trigger` | Select a manual trigger when needed. |
| `--resume` | Continue one recoverable existing run against its stored immutable definition. |
| `--approval-port` | Override local approval listener port. |
| `--json` | Machine-readable command output where supported. |
| `--verbose` | Additional safe diagnostics. |
| `--color` | Auto, always, or never. |
| `--background`, `-d` | Detach only after genuine readiness. |

Multiple explicit files are supported:

```bash
woml run parent.woml child.woml
```

A directory is supported:

```bash
woml run workflows/
```

Reusable definition files may be present as dependencies, but only runnable
workflow profiles activate.

### 17.3 `woml test`

```bash
woml test <workflow.woml> [--state <path>] \
  [--trigger <manualTriggerId>] [--resume <runId>] \
  [--approval-port <port>]
```

Executes one manual occurrence and exits. This is for integration tests and CI,
not normal automation hosting.

### 17.4 `woml types`

```bash
woml types <workflow.woml|directory> [--output <path>]
```

Explicitly refreshes built-in and imported service declarations. `check` and
`run` already generate the default `woml-env.d.ts`; use this command for a
custom path or editor-only refresh.

### 17.5 `woml inspect`

```bash
woml inspect [--state <path>] [--no-color]
```

Opens the live colored terminal view of runtime health, workflows, recent runs,
queues, waits, and failures. It is the htop-like operations experience and does
not open a browser dashboard.

### 17.6 `woml list`

```bash
woml list [--workflow <workflowId>] [--status <status>] \
  [--limit <1-200>] [--state <path>] [--json]
```

Lists recent durable runs with optional workflow/status filters. It uses the
active authenticated local runtime when available and safe offline projection
otherwise.

### 17.7 `woml get`

```bash
woml get <runId> [--state <path>] [--json]
```

Shows one redacted run: status, steps, attempts, waits, control-flow selection,
lifecycle state, policy state, cancellation, and bounded workflow-call
relations. It intentionally omits payloads, full context, output values,
secrets, credentials, idempotency keys, and stack traces.

### 17.8 `woml cancel`

```bash
woml cancel <runId> [--state <path>] [--json]
```

Records a durable cancellation request. Rust stops new work, signals active
scripts/capabilities, invalidates approval capabilities, preserves committed
effects/results, settles ambiguity honestly, then runs `<on-cancel>` and
`<on-complete>`. It does not roll back external effects or automatically cancel
independent child workflows.

### 17.9 `woml stop`

```bash
woml stop [--state <path>] [--json]
```

Authenticates to and gracefully stops the background runtime owning the
selected state boundary.

### 17.10 Follow logs

```bash
woml <run-id|workflow-id> --logs \
  [--state <path>] [--config <path>] [--json] \
  [--color=auto|always|never]
```

Prints matching historical output and follows new records. Ctrl+C exits the
viewer without stopping the background automation.

### 17.11 `woml emit`

```bash
woml emit <eventName> \
  --id <publisherEventId> \
  --data @<jsonFile> \
  --server <url> \
  --token-secret <NAME>
```

Publishes to an authenticated public event endpoint. `--id` is the stable
publisher occurrence identity; repeating the same ID/data deduplicates, while
reusing it with changed data conflicts. The token value is loaded by symbolic
secret name. Applications may call the same HTTP endpoint directly; this CLI
is an operator convenience.

### 17.12 Provider doctors

```bash
woml telegram doctor \
  [--token-secret <NAME>] [--destination <chatId>] \
  [--json] [--color=auto|always|never]

woml discord doctor \
  [--token-secret <NAME>] [--destination <channelId>] \
  [--json] [--color=auto|always|never]

woml whatsapp doctor \
  [--access-token-secret <NAME>] [--app-secret <NAME>] \
  [--verify-token-secret <NAME>] [--phone-number-id <id>] \
  [--callback-url <https-url>] \
  [--json] [--color=auto|always|never]
```

Doctor commands authenticate, verify permissions/identity, and optionally
check a destination or callback without creating a workflow run or sending a
message. Credentials are redacted from human and JSON output.

Slack setup currently relies on startup diagnostics and the included Slack app
manifest rather than a public `woml slack doctor` CLI command.

### 17.13 `woml backup`

```bash
woml backup <backup-directory> [--state <path>] [--json]
```

Creates a coherent online SQLite snapshot plus versioned manifest and
checksums. Back up external application databases separately. Back up WOML
object storage together with the state boundary when used.

### 17.14 `woml restore`

```bash
woml restore <backup-directory> [--state <path>] [--replace] [--json]
```

Offline operation that verifies manifest/checksums before restoring. It
refuses a live target. `--replace` is required to replace existing state and
retains the previous database as a reported rollback copy.

### 17.15 `woml prune`

```bash
woml prune --before <duration> [--state <path>] [--dry-run] [--compact] [--json]
```

Removes terminal run history older than the threshold while preserving active
or recoverable runs, referenced immutable definitions, durable user state, and
other protected data. Start with `--dry-run`. `--compact` reclaims SQLite space
after successful pruning; deletion alone does not guarantee file shrinkage.

## 18. Runtime configuration

`woml.runtime.json` is optional. Safe defaults keep local use simple.

```json
{
  "schemaVersion": 1,
  "deploymentName": "order-automation",
  "statePath": "./data/state.sqlite",
  "public": { "host": "127.0.0.1", "port": 3000 },
  "admin": { "host": "127.0.0.1", "port": 3001 },
  "logging": {
    "format": "text",
    "level": "info",
    "directory": "./logs"
  },
  "workers": 4,
  "shutdownTimeoutMs": 30000,
  "observability": { "health": true, "metrics": true },
  "retention": {
    "enabled": true,
    "succeededAfterDays": 30,
    "failedAfterDays": 90,
    "cancelledAfterDays": 30,
    "maintenanceHourUtc": 3
  },
  "backup": { "directory": "./backups" }
}
```

| Field | Constraints and meaning |
| --- | --- |
| `schemaVersion` | Required and exactly `1`. |
| `deploymentName` | Lowercase segmented identity, maximum 128. |
| `statePath` | Durable state path, relative to the config file when relative. |
| `public` | Trigger listener host/port. |
| `admin` | Local operations listener host/port; production admin remains loopback-only. |
| `logging.format` | `text` or `json`. |
| `logging.level` | `error`, `warn`, `info`, or `debug`. |
| `logging.directory` | Runtime log destination. |
| `workers` | 1–256 Bun worker capacity. |
| `shutdownTimeoutMs` | 1,000–300,000 ms graceful drain. |
| `observability.health`, `.metrics` | Enable local operations endpoints. |
| `retention.enabled` | Enables automatic retention. |
| Retention day fields | 1–3,650 days per terminal outcome. |
| `maintenanceHourUtc` | 0–23. |
| `backup.directory` | Configured backup directory. |

Precedence is explicit CLI option, reviewed `WOML_RUNTIME_*` environment
variable, config file, then safe default.

## 19. Runtime, durability, and recovery semantics

### 19.1 Run truth

The append-only event log is authoritative. Context, terminal presentation,
run lists, and inspection are projections derived from those events. Mutable
summary tables may be rebuilt and are not a second source of truth.

### 19.2 Immutable definitions

Every run binds to the exact compiled workflow and module artifacts active at
admission. Editing source affects future runs only. Recovery never silently
rebinds an existing run to changed files.

### 19.3 Crash policy

If a step or managed effect has a recorded start without a definitive terminal
event, a restart does not prove whether an external side effect occurred. WOML
fails that ambiguous work closed unless the reviewed capability has a durable
proof allowing safe reattachment. Completed derivation is folded again; an
external effect is not blindly replayed.

### 19.4 Duplicate trigger policy

Durable trigger identity plus canonical payload determine duplication. The same
identity and payload reuse the original run. The same identity with changed
payload is an idempotency conflict. Provider acknowledgements occur only after
durable admission.

### 19.5 Atomic deployment activation

All supplied files/directories form one deployment. Providers and listeners
start behind a closed readiness gate. WOML hashes and rechecks source before it
opens admission. Partial startup is closed if a required provider fails or
source changes during activation.

### 19.6 Foreground and background

Foreground is recommended under systemd, Docker, and Kubernetes because the
supervisor already owns process lifetime. `--background` is useful for a
directly managed workstation or VPS. Both use the same production Rust host and
durable semantics.

## 20. Diagnostics and exit behavior

Authoring errors provide:

```ts
type WomlDiagnostic = {
  code: string;
  phase: "parse" | "validation" | "compile" | "runtime";
  message: string;
  file: string;
  location: {
    start: { line: number; column: number; offset: number };
    end?: { line: number; column: number; offset: number };
  };
  hint?: string;
};
```

Line/column are one-based; byte offset is zero-based. Locations point into the
original `.woml`, including translated script locations when available.
Diagnostics may include several sorted errors, with one primary diagnostic.

Representative codes:

| Code | Meaning |
| --- | --- |
| `WOML_UNKNOWN_ELEMENT`, `WOML_UNKNOWN_ATTRIBUTE` | Unsupported or misplaced syntax. |
| `WOML_DUPLICATE_ID` | Structural identity collision. |
| `WOML_INVALID_REFERENCE`, `WOML_UNKNOWN_REFERENCE` | Malformed or unknown declarative reference. |
| `WOML_REFERENCE_NOT_DOMINATING` | Output is not guaranteed before the consumer. |
| `WOML_REFERENCE_NOT_AVAILABLE` | Runtime nested property is missing. |
| `WOML_BRANCH_TEST_NOT_BOOLEAN` | `<when>` resolved to a non-boolean. |
| `WOML_SWITCH_VALUE_INVALID` | Switch input is not a string. |
| `WOML_TRIGGER_SCHEMA_INVALID` | Ingress payload failed its JSON Schema. |
| `WOML_TRIGGER_IDEMPOTENCY_CONFLICT` | Same occurrence ID reused with different data. |
| `WOML_POLICY_QUEUE_FULL` | Durable policy queue reached capacity. |
| `WOML_WORKFLOW_TIMED_OUT` | Total workflow deadline won. |
| `WOML_WORKFLOW_TARGET_NOT_FOUND` | Called/started workflow is not actively owned. |
| `WOML_WORKFLOW_CALL_CYCLE` | Call lineage would cycle. |
| `WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED` | Synchronous call target contains approval. |

A non-zero CLI exit means the requested finite operation failed. Long-lived
`run`, `inspect`, and log-follow commands normally end through Ctrl+C or an
explicit stop.

## 21. Security and data visibility

- `.woml` source contains symbolic secret names, never values.
- Run inspection excludes payloads, context, results, secret values,
  credentials, operation/idempotency keys, provider bodies, and stack traces.
- Managed operation events contain bounded safe metadata, not request/response
  bodies, SQL, database parameters, object bodies, or state/cache values.
- A script can deliberately return or transmit sensitive data; WOML cannot
  infer business sensitivity, so authors remain responsible for what their
  code returns and sends.
- The state SQLite file is hardened to owner-only mode on Unix but is not
  transparently encrypted. Protect disks, directories, snapshots, and backups.
- Native Fetch and managed HTTP in the local profile are not an SSRF sandbox.
- Bun worker isolation is not a hostile multi-tenant security boundary.
- Public webhooks, event publication, WhatsApp callbacks, and reverse proxies
  require deliberate authentication/TLS deployment.
- Local administration is state/runtime scoped and loopback-only in v1; do not
  expose the state file or wrap commands in an unauthenticated remote API.

## 22. Explicit v1 limitations

The following are not silently supported:

- arbitrary `after`, `from`, `to`, or `depends-on` DAG edges;
- structural loops, durable for-each, batching, race, or first-success groups;
- declarative `<http>`, `<db>`, `<storage>`, or other capability tags in steps;
- per-step timeout attributes;
- `parallel on-error="continue"`;
- multi-step direct children inside `<parallel>`;
- nested forks inside a fork-owned branch subtree;
- npm imports, package registry modules, default exports, CommonJS, or dynamic
  module installation;
- arbitrary structural custom tags or child-generating components;
- custom provider triggers (`kind="trigger"`);
- external schema-file references;
- `context.run`, `context.env`, secret enumeration, or bracket-style WOML
  references;
- a JavaScript `woml.resume()` API;
- synchronous workflow calls that wait through Human Approval;
- automatic parent-to-child cancellation propagation or compensation/sagas;
- NoSQL/document database drivers;
- `services.queue`;
- `services.slack.send()`; and
- a managed multi-node/distributed production control plane.

Use local JavaScript for ordinary in-step loops and transformations. That does
not create durable per-item nodes, retries, or inspection.

## 23. Choosing the right primitive

| Need | Use |
| --- | --- |
| One readable unit of work | `<step>` + `<script>` |
| Several independent single steps at once | `<parallel>` |
| Several independent multi-step lanes | `<fork>` + `<branch>` |
| Strict boolean routing | `<choose>` |
| Exact string routing | `<switch>` |
| Human decision | `<approval>` |
| Temporarily reusable value | `services.cache` |
| Small permanent workflow memory | `services.state` |
| Application records and queries | `services.db()` |
| Files or larger durable objects | `services.storage` |
| Familiar/streaming HTTP | `fetch()` |
| Supervised bounded HTTP | `services.http.request()` |
| Notify every subscriber of a fact | `services.events.emit()` |
| Wait for one child workflow's answer | `services.workflows.call()` |
| Start a child and continue | `services.workflows.start()` |
| Reusable data transformation/API wrapper | Local JS/TS module |
| Reusable durable project step | Reusable WOML step |
| Project-specific approval transport | Reusable notification provider |

## 24. Documentation map

This consolidated reference is backed by more focused guides:

- [Language reference](language-reference.md)
- [CLI reference](cli-reference.md)
- [Getting started](getting-started.md)
- [Production triggers](woml-production-triggers.md)
- [Lifecycle and run control](woml-lifecycle-and-run-control.md)
- [Runtime policies](woml-runtime-policies.md)
- [Services](woml-services.md)
- [HTTP](woml-http-services.md)
- [Database](woml-database.md)
- [Storage](woml-storage.md)
- [Cache](woml-cache.md)
- [Durable state](woml-durable-state.md)
- [Internal events](woml-events-service.md)
- [Workflow calls](woml-workflow-calls.md)
- [Modules](woml-modules.md)
- [Reusable definitions](woml-reusable-definitions.md)
- [Notifications](woml-notifications.md)
- [Communication providers](woml-communication-providers.md)
- [Production runtime](woml-production-runtime.md)
- [Observability](woml-observability.md)
- [Backup and restore](woml-backup-and-restore.md)
- [Retention and maintenance](woml-retention-and-maintenance.md)
- [Data security](woml-data-security.md)

Machine schemas and protocol documents under `docs/schemas/` and
`docs/protocols/` expose the lower-level versioned contracts used by the
compiler, engine, event store, hosts, and release gates. They are public for
auditability but are not required to write an ordinary workflow.
