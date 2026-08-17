# WOML v1.0 Language Reference

This document defines the public WOML v1.0 authoring language implemented by
the `woml` command. It covers workflow documents, reusable definitions,
triggers, steps, control flow, approvals, lifecycle hooks, runtime policies,
references, modules, and script bindings.

WOML is a frontend for WOML's language-neutral compiled workflow model. The
WOML compiler validates this source and lowers it to that model; the execution
core never parses or interprets WOML.

For a guided first workflow, start with [Getting started](getting-started.md).
For command-line behavior, see the [CLI reference](cli-reference.md). Protocol
and schema files under `docs/protocols/` and `docs/schemas/` are maintainer
contracts rather than prerequisites for authoring a workflow.

## 1. Design Principles

The fundamental syntax follows these rules:

1. Structure expresses control flow.
2. Stable step and approval IDs express output identity.
3. Document order expresses sequential dependency within a steps container.
4. Context references express data access, not hidden dependency edges.
5. JavaScript appears only inside `<script>` bodies, never in attributes.
6. A successful step return or resolved approval becomes a named
   `context.steps` value.
7. There is no implicit `last` step or positional parallel-result array.
8. Unknown elements and attributes are compile errors.
9. The syntax remains independent of database, HTTP-client, Slack, and other
   capability vocabularies.

The words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** describe normative
requirements for the v1.0 language.

### 1.1 Supported v1.0 surface

WOML v1.0 executes:

- manual, webhook, schedule, interval, event, Slack, Telegram, Discord, and
  WhatsApp triggers;
- sequential steps, retries, parallel groups, conditional choices, exact-string
  switches, forked multi-step branches, and human approvals;
- workflow and step lifecycle hooks;
- concurrency, rate-limit, timeout, and queue runtime policies;
- local JavaScript/TypeScript modules, reusable WOML steps, and reusable
  notification providers;
- HTTP, SQL database, storage, cache, events, durable state, messaging, and
  workflow call/start services; and
- durable foreground/background operation, recovery, inspection, backup, and
  retention through the CLI.

Unknown elements and attributes are rejected. Designed future syntax is not
accepted early, so a workflow that passes `woml check` targets the implemented
v1.0 authoring surface.

## 2. WOML Is XML-Like, Not XML

WOML uses familiar markup structure, but an ordinary `<script>` body may contain
raw JavaScript operators such as `<`, `>`, and `&&`. A WOML document therefore
is not required to be well-formed XML and MUST NOT include an XML declaration.

Authors write:

```xml
<script>
  if (score < 0.8 && enabled) {
    return { accepted: true };
  }

  return { accepted: false };
</script>
```

Authors do not write CDATA wrappers and do not entity-escape ordinary
JavaScript.

The frontend lexer has at least two modes:

- **Markup mode** parses WOML elements and attributes.
- **Raw-content mode** preserves `<script>` JavaScript and inline `<schema>`
  JSON without source rewriting.

Raw-content termination is deterministic in v1.0:

- The first literal, case-sensitive `</script>` terminates a `<script>` body.
- The first literal, case-sensitive `</schema>` terminates a `<schema>` body.
- The terminator is recognized without interpreting JavaScript or JSON strings,
  comments, escapes, regular expressions, or template literals.
- The exact terminator text therefore MUST NOT occur inside the raw content.
  JavaScript that needs to construct the text can split it, for example
  `"</scr" + "ipt>"`.
- The frontend MUST preserve the raw body exactly and MUST retain an offset map
  to the original source for diagnostics.

This raw-text rule is part of the v1.0 grammar. A future language version may
introduce a JavaScript-aware terminator only through a reviewed compatibility
change.

Outside raw-content elements:

- Element and attribute names are lowercase and case-sensitive.
- Multiword element and attribute names use kebab-case.
- Attribute values MUST be quoted.
- Duplicate attributes are invalid.
- Comments use `<!-- comment -->`.
- Attribute text uses the normal markup entities `&amp;`, `&quot;`, `&apos;`,
  `&lt;`, and `&gt;` when needed.

## 3. Complete Example

```xml
<woml>
<workflow
  id="content-moderator"
  name="AI Content Moderator"
  description="Analyze submitted content and select a review path"
  version="1.0.0">

  <config
    concurrency="4"
    timeout="10m"
    rate-limit="100/1m"
    queue="moderation"
  />

  <lifecycle>
    <on-success>
      <script>
        console.log("Content moderation completed");
      </script>
    </on-success>

    <on-error>
      <script>
        console.error("Content moderation failed");
      </script>
    </on-error>
  </lifecycle>

  <triggers>
    <webhook
      id="moderateContent"
      path="/webhooks/moderate-content"
      method="POST"
      auth="bearer"
      secret="{{secrets.MODERATION_WEBHOOK_TOKEN}}">

      <schema>
        {
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "type": "object",
          "required": ["contentId", "content", "userId"],
          "properties": {
            "contentId": { "type": "string" },
            "content": { "type": "string" },
            "imageUrl": { "type": "string" },
            "userId": { "type": "string" }
          },
          "additionalProperties": false
        }
      </schema>
    </webhook>
  </triggers>

  <steps>
    <step
      id="preprocessContent"
      name="Preprocess content"
      description="Normalize the submitted content before analysis">
      <script>
        const { content } = context.payload;

        return {
          originalContent: content,
          normalizedContent: content.trim().toLowerCase()
        };
      </script>
    </step>

    <parallel
      id="contentAnalysis"
      name="Analyze content"
      description="Run the independent analysis steps concurrently"
      concurrency="3"
      on-error="fail-fast">
      <step id="analyzeText">
        <script>
          return analyzeText(
            context.steps.preprocessContent.originalContent
          );
        </script>
      </step>

      <step id="analyzeImage">
        <script>
          return analyzeImage(context.payload.imageUrl);
        </script>
      </step>

      <step id="analyzeContext">
        <script>
          return analyzeContext(
            context.steps.preprocessContent.originalContent
          );
        </script>
      </step>
    </parallel>

    <step id="combineAnalysis">
      <script>
        return {
          text: context.steps.analyzeText,
          image: context.steps.analyzeImage,
          contextual: context.steps.analyzeContext
        };
      </script>
    </step>

    <step id="needsHumanReview">
      <script>
        const { risk, confidence } = context.steps.combineAnalysis;

        return (
          (risk > 0.3 && confidence < 0.8) ||
          (risk > 0.7 && confidence < 0.9)
        );
      </script>
    </step>

    <choose
      id="reviewPath"
      name="Select review path"
      description="Choose human or automatic review">
      <when test="{{context.steps.needsHumanReview}}">
        <step id="prepareHumanReview">
          <script>
            return {
              analysis: context.steps.combineAnalysis,
              requestedAt: new Date().toISOString()
            };
          </script>
        </step>

        <approval
          id="humanApproval"
          name="Human content approval"
          description="Ask a moderator to approve or reject the content"
          timeout="24h"
          on-timeout="reject">

          <notify>
            <slack
              channels="#approvals #moderators"
              bot-token="{{secrets.SLACK_BOT_TOKEN}}"
              app-token="{{secrets.SLACK_APP_TOKEN}}"
            />
          </notify>

          <when-approved>
            <step id="recordHumanApproval">
              <script>
                return {
                  contentId: context.payload.contentId,
                  approved: true
                };
              </script>
            </step>
          </when-approved>

          <when-rejected>
            <step id="recordHumanRejection">
              <script>
                return {
                  contentId: context.payload.contentId,
                  approved: false
                };
              </script>
            </step>
          </when-rejected>
        </approval>

        <result value="{{context.steps.humanApproval}}" />
      </when>

      <otherwise>
        <step id="makeAutomaticDecision">
          <script>
            return makeDecision(context.steps.combineAnalysis);
          </script>
        </step>

        <result value="{{context.steps.makeAutomaticDecision}}" />
      </otherwise>
    </choose>
  </steps>
</workflow>
</woml>
```

## 4. Document Structure

A WOML file contains exactly one `<woml>` root. It contains exactly one
runnable `<workflow>` or one reusable top-level `<step>` or `<provider>`
definition. The wrapper remains required, so tools can classify a file before
compilation. Only the workflow profile is runnable; reusable files are imported
dependencies.

`<workflow>` contains optional singleton `<config>`, `<lifecycle>`, and
`<triggers>` containers plus exactly one `<steps>` container. These containers
may appear in any order. Omitting `<triggers>` declares a call-only workflow.
Authors are encouraged to use config, lifecycle, triggers, then steps for
readability, but order never changes behavior.

The structural grammar is:

```text
document       := workflow-document
                | reusable-step-document
                | provider-document

workflow-document
               := <woml> imports? workflow </woml>

reusable-step-document
               := <woml> imports? props? reusable-step reusable-lifecycle? </woml>

provider-document
               := <woml> imports? props? notification-provider reusable-lifecycle? </woml>

imports        := <imports> module+ </imports>
module         := <module name=module-alias from=relative-js-ts-or-woml-source />

props          := <props> prop+ </props>
prop           := <prop name=kebab-name required=boolean? secret=boolean? />

reusable-step  := <step name=text? description=text?> script </step>
notification-provider
               := <provider kind="notification"> script </provider>
reusable-lifecycle
               := <lifecycle> reusable-lifecycle-hook+ </lifecycle>
reusable-lifecycle-hook
               := on-success | on-error | on-complete

workflow       := <workflow workflow-attributes>
                    workflow-child+
                  </workflow>
workflow-child := config | lifecycle | triggers | steps

config         := <config config-attributes />

lifecycle      := <lifecycle>
                    lifecycle-hook+
                  </lifecycle>

lifecycle-hook := <hook steps=step-id-list?> (script | notify)+ </hook>

triggers       := <triggers> trigger+ </triggers>

trigger        := manual
                | webhook
                | slack
                | telegram
                | discord
                | whatsapp
                | schedule
                | interval
                | event

steps          := <steps> steps-item+ </steps>

steps-item     := step
                | parallel
                | choose
                | switch
                | fork
                | approval

step           := <step step-attributes>
                    operation
                  </step>

operation      := script

parallel       := <parallel parallel-attributes>
                    step+
                  </parallel>

choose         := <choose choice-attributes>
                    when+
                    otherwise
                  </choose>

when           := <when test="context-reference">
                    steps-item+
                    result
                  </when>

otherwise      := <otherwise>
                    steps-item+
                    result
                  </otherwise>

switch         := <switch switch-attributes>
                    case+
                    default
                  </switch>

case           := <case value="string">
                    steps-item+
                    result?
                  </case>

default        := <default>
                    steps-item+
                    result?
                  </default>

result         := <result value="context-reference" />

approval       := <approval approval-attributes>
                    notify?
                    when-approved
                    when-rejected
                  </approval>

notify         := <notify>
                    (slack | telegram | discord | whatsapp | custom-provider)+
                  </notify>

slack          := <slack
                    channels="slack-destination-list"
                    bot-token="secret-reference"
                    app-token="secret-reference"
                  />

telegram       := <telegram
                    chats="telegram-chat-id-list"
                    bot-token="secret-reference"
                  />

when-approved  := <when-approved>
                    steps-item*
                  </when-approved>

when-rejected  := <when-rejected>
                    steps-item*
                  </when-rejected>
```

Local `.js` and `.ts` module graphs are validated, bundled as deterministic ESM,
and exposed through `services.<alias>.<function>()`. Each step receives fresh
module state; initialization effects are rejected; Fetch and built-in service
calls retain their tracked runtime boundaries. Durable definitions bind the
exact compiled artifacts so recovery does not read changed source files. See
[Modules](woml-modules.md) for authoring and
[Module System v1](protocols/module-system-v1.md) for the maintainer contract.

This is a structural grammar. Identifier and raw-content tokenization are fixed
by Sections 2 and 5. Attribute-reference tokenization is defined in Section 15.

## 5. Identifiers

Six identifier roles exist:

- A workflow ID identifies the workflow definition.
- A trigger ID identifies a trigger within that definition.
- A step ID identifies an executable DAG node and its output in context.
- A parallel ID identifies a fork/join structure for diagnostics and events.
- A choice ID identifies a conditional structure, its merged output, diagnostics, and events.
- An approval ID identifies a durable waiting node, its decision, and its output
  in context.

All IDs MUST be non-empty. Workflow IDs and trigger IDs MUST be unique within
their respective namespaces. Step, parallel, choice, and approval IDs share one
structural namespace and MUST be unique across the whole workflow.

Step IDs are part of the JavaScript-facing API:

```js
context.steps.preprocessContent
```

Approval IDs use the same JavaScript-facing output namespace:

```js
context.steps.humanApproval
```

Trigger, step, parallel, choice, and approval IDs MUST be JavaScript-safe
lower-camel identifiers matching:

```text
[a-z][A-Za-z0-9]*
```

Quoted or bracket context paths are not part of WOML v1.0. This keeps JavaScript
access and `{{context...}}` access identical.

Workflow IDs MUST use lowercase kebab-case matching:

```text
[a-z][a-z0-9]*(?:-[a-z0-9]+)*
```

For example, `content-moderator` is a workflow ID and `preprocessContent` is a
step ID.

Generated timestamps, array positions, display names, and source line numbers
MUST NOT be used as durable step identities.

### 5.1 Structural name and description

`<step>`, `<parallel>`, result-producing `<choose>`, and `<approval>` use optional `name` and
`description` attributes:

```xml
<step
  id="generateReport"
  name="Generate report"
  description="Build the report returned to the publishing step">
  <script>
    return generateReport(context.payload);
  </script>
</step>
```

Rules:

- `name` and `description` are optional string attributes. When present, each
  must contain at least one non-whitespace character.
- `<name>` and `<description>` child elements are invalid.
- They are descriptive metadata and never become `context.steps` values.
- An `id` remains required even when a name is present; display names are not
  execution identities.

## 6. `<workflow>`

`<workflow>` defines one workflow.

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Workflow ID | Stable workflow identity. |
| `name` | No | String | Human-readable display name. |
| `description` | No | String | Short human-readable description. |
| `version` | No | Version string | User-defined workflow version. It does not select the WOML grammar. |

When present, workflow `name`, `description`, and `version` must each contain
at least one non-whitespace character.

```xml
<workflow
  id="daily-report"
  name="Daily Report"
  description="Generate and publish the daily report"
  version="1.2.0">
  ...
</workflow>
```

Runtime settings such as concurrency and timeout MUST NOT also appear on
`<workflow>`. They have one canonical location: `<config>`.

`name`, `description`, and `version` lower to
`metadata.name`, `metadata.description`, and `metadata.version` on the
compiled workflow. The v1.0 compiler rejects a `tags` attribute rather than
silently discarding it.

## 7. `<config>`

`<config>` contains workflow-level runtime policy. It is optional and may occur
at most once.

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `concurrency` | No | Positive integer | Maximum concurrently active runs of this workflow. |
| `timeout` | No | Duration | Maximum duration of one complete run. |
| `rate-limit` | No | Rate | Maximum run starts per duration window. |
| `queue` | No | Non-empty string | Logical execution queue. |

```xml
<config
  concurrency="4"
  timeout="10m"
  rate-limit="100/1m"
  queue="moderation"
/>
```

The rate syntax is:

```text
rate := positive-integer "/" duration
```

For example, `100/1m` means at most 100 run starts in one minute.
Runtime Policy v1 fixes the algorithm as a strict rolling window. Only the
first execution start consumes rate capacity; resuming a durable wait does not.

The initial duration syntax is:

```text
duration := positive-number ("ms" | "s" | "m" | "h" | "d")
```

Bare numeric durations are invalid because their unit would be ambiguous.

`<config>` contains data only and requires at least one attribute. Lifecycle
scripts MUST NOT be nested inside it. The compiler lowers the policy outside
the business DAG, and the Rust scheduler enforces it for every ingress. No
policy is silently ignored.

Runtime Policy v1 limits counts to 1,000,000, durations to whole milliseconds
from 1 ms through 365 days, and queue names to 128-character lowercase dot,
underscore, or kebab identifiers. Named queues are work-conserving FIFO
admission lanes; they do not create shared concurrency pools and are unrelated
to the postponed `services.queue` capability.

## 8. `<lifecycle>`

`<lifecycle>` declares workflow-level lifecycle behavior. It is separate from
`<config>` because its children execute code rather than describe runtime data.

```xml
<lifecycle>
  <on-start>
    <script>
      console.log(`Starting ${lifecycle.workflow.id}`);
    </script>
  </on-start>

  <on-step-failure steps="chargeCustomer createInvoice">
    <notify>
      <slack
        channels="#incidents"
        message="Step {{lifecycle.step.id}} failed with {{lifecycle.failure.code}}"
        bot-token="{{secrets.SLACK_BOT_TOKEN}}"
        app-token="{{secrets.SLACK_APP_TOKEN}}"
      />
    </notify>
  </on-step-failure>

  <on-success>
    <script>
      console.log("Workflow completed");
    </script>
  </on-success>

  <on-error>
    <script>
      console.error("Workflow failed");
    </script>
  </on-error>

  <on-complete>
    <script>
      console.log(`Final outcome: ${lifecycle.workflow.outcome}`);
    </script>
  </on-complete>
</lifecycle>
```

Structural rules:

- `<lifecycle>` is optional and may occur at most once.
- Hooks may appear in any source order. WOML recognizes them by name and
  normalizes them to the semantic execution order: `on-start`, step hooks,
  the matching workflow outcome hook, then `on-complete`.
- Every hook is optional and may occur at most once.
- A hook contains one or more source-ordered `<script>` or `<notify>` actions.
- Step hooks accept an optional whitespace-separated `steps` filter containing
  executable step IDs. Omission means all executable steps.
- Lifecycle scripts do not create `context.steps` outputs.
- Lifecycle scripts receive the read-only `lifecycle` binding. It is unavailable
  in normal step scripts.
- Lifecycle Slack, Telegram, Discord, and WhatsApp notifications are informational. Slack requires
  `channels`, `message`, `bot-token`, and `app-token`; Telegram requires
  `chats`, `message`, and `bot-token`; Discord requires `channels`, `message`,
  and `bot-token`; WhatsApp requires `recipients`, `access-token`,
  `phone-number-id`, `template`, `language`, and `message`. They never create
  approval buttons or decision capabilities.
- Notification messages use WOML Template v1: bounded literal text and scalar
  `{{context...}}` or `{{lifecycle...}}` placeholders. Secrets are forbidden in
  message content.

Lifecycle definitions lower outside the business DAG and execute through the
same durable event authority as workflow steps. Hook failures and outcomes are
inspectable and recoverable; hooks are never silently ignored.

## 9. `<triggers>`

`<triggers>` contains one or more workflow entry points. Omitting the entire
container declares a call-only workflow for `services.workflows.call()`.
Writing an empty `<triggers />` container is invalid; omission is the one
canonical call-only source shape.

All triggers in one workflow start the same `<steps>`. WOML v1.0 does not expose
an attribute that routes different triggers to different entry nodes. Workflows
with different graphs MUST be separate workflow definitions.

This corrects an ambiguity in the TypeScript SDK where triggers and steps are
stored in unrelated flat arrays.

Call-only source compiles with an empty trigger list and is registered by its
workflow ID. `services.workflows.call()` waits for the child's terminal result;
`services.workflows.start()` returns the admitted child run ID immediately.
Synchronous calls cannot target a workflow containing Human Approval because a
Bun script continuation cannot be serialized across a long durable wait. Rust
rejects that target before child admission.

### 9.1 `<manual>`

```xml
<manual id="manualRun" />
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |

Manual CLI input becomes `context.payload`.

### 9.2 `<webhook>`

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

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `path` | Yes | Exact absolute route path | Static route registered by the runtime; parameters, wildcards, repeated slashes, and `/_woml` are forbidden. |
| `method` | No | HTTP method | Defaults to `POST`; Production Trigger HTTP v1 executes POST only. |
| `auth` | Yes | `bearer` or `none` | Makes protected or deliberately public exposure explicit. |
| `secret` | For bearer | Exact secret reference | Required as `{{secrets.NAME}}` with bearer auth and forbidden with `auth="none"`. |

`<webhook>` may contain at most one `<schema>`. An inline schema is standard JSON
Schema Draft 2020-12. The compiler MUST parse and validate the schema before the
workflow can be registered.

When a schema exists, the webhook payload MUST validate before a run is created.
The normalized successful payload becomes `context.payload`.

When payload validation fails:

- The HTTP runtime returns status `400 Bad Request`.
- No workflow run is created and no run event is appended.
- The response body is JSON with the stable code
  `WOML_TRIGGER_SCHEMA_INVALID`, a human-readable message, and validation issues
  addressed by JSON Pointer when available.
- Transport access logs or security audit logs MAY record the rejected request,
  but they are not workflow-run events.

The minimum response shape is:

```json
{
  "error": {
    "code": "WOML_TRIGGER_SCHEMA_INVALID",
    "message": "Webhook payload does not match the declared schema",
    "issues": [
      {
        "path": "/orderId",
        "message": "Required property is missing"
      }
    ]
  }
}
```

Framework instances, middleware callbacks, Zod values, and arbitrary validation
callbacks from the TypeScript SDK are not part of the fundamental WOML grammar.

External schema files, such as `<schema src="..." />`, are deferred until file
resolution and portability rules are designed.

### 9.3 `<slack>`

```xml
<slack
  id="agentMessage"
  events="app-mention,direct-message"
  channels="woml-testing,agent-support"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `events` | Yes | Comma-separated event set | One or both of `app-mention` and `direct-message`. |
| `channels` | No | Comma-separated channel set | Limits app mentions to named channels; omission accepts every visible channel. |
| `bot-token` | Yes | Exact secret reference | Symbolic Slack bot token. |
| `app-token` | Yes | Exact secret reference | Symbolic Slack Socket Mode app token. |

The normalized trigger value contains safe message, user, channel, thread, and
workspace identifiers. Bot/self messages, edits, deletes, provider envelopes,
and credentials never enter `context.payload`. Slack event ingestion uses one
shared Socket Mode connection per credential pair and durable Rust admission.

### 9.3.1 `<telegram>`

```xml
<telegram
  id="agentMessage"
  events="message"
  bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
/>
```

Telegram v1 supports the single `message` event. The trigger is empty and is
valid only directly inside `<triggers>`. Its bot token is one exact symbolic
secret reference. The runtime uses long polling and durable admission.

### 9.3.2 `<discord>`

```xml
<discord
  id="agentMessage"
  events="app-mention,direct-message"
  channels="200000000000000001,200000000000000002"
  bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
/>
```

Discord v1 supports `app-mention` and `direct-message`. `channels` is optional;
when present it is a comma-separated list of numeric Discord channel IDs with
17 to 20 digits. Channel names are rejected because they are mutable display
labels, not durable routing identities. `bot-token` is one exact secret
reference. The runtime uses a shared resumable Gateway connection
and supervised REST operations. Slash-command syntax is explicitly deferred.
`woml check` remains offline and does not read credentials or contact Discord.

### 9.3.3 `<whatsapp>`

```xml
<whatsapp
  id="customerMessage"
  events="message"
  phone-number-id="123456789012345"
  verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
  app-secret="{{secrets.WHATSAPP_APP_SECRET}}"
/>
```

WhatsApp v1 supports the single inbound `message` event. The numeric
`phone-number-id` is Meta's durable Phone Number ID, not the display phone
number. Verification and signature credentials are exact symbolic secret
references. Status callbacks never start runs.

Approval and lifecycle notifications require `recipients`, `access-token`,
`phone-number-id`, `template`, and `language`; lifecycle notifications also
require `message`, which becomes the first template body parameter. Proactive
free-form delivery is rejected rather than silently violating WhatsApp's
template boundary. Script messaging uses the same reviewed shape through
`services.whatsapp.send({ accessToken, phoneNumberId, conversationId,
template: { name, language, parameters } })`.

The stable callback route is `/callbacks/whatsapp`; handshake and exact raw-body
signature verification are conformance-tested. The runtime validates signed
callbacks and supervises Cloud API delivery.

### 9.4 `<schedule>`

```xml
<schedule
  id="everySixHours"
  cron="0 */6 * * *"
  timezone="UTC"
  on-missed="skip"
/>
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `cron` | Yes | Cron expression | Schedule expression. |
| `timezone` | No | IANA timezone | Evaluation timezone; defaults to `UTC`. |
| `on-missed` | No | `skip` or `run-once` | Restart policy; defaults to `skip`. |

WOML schedules use **WOML Cron v1**: five numeric fields
(`minute hour day-of-month month day-of-week`) separated by single spaces.
Wildcards, lists, inclusive ranges, and `/step` are supported. Seconds, names,
macros, wrapping ranges, and Quartz-only tokens are rejected. Day-of-week uses
`0` or `7` for Sunday; restricted day-of-month and day-of-week fields use POSIX
OR semantics.

`timezone` must be a canonical IANA identifier and defaults to `UTC`.
Nonexistent DST wall times are skipped and repeated wall times create one
occurrence for each UTC instant. The durable Rust scheduler owns the clock and
SQLite cursor. `woml run` remains active, reports the next planned UTC instant,
and safely continues according to `on-missed` after restart.

### 9.5 `<interval>`

```xml
<interval id="everyFiveMinutes" every="5m" on-missed="skip" />
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `every` | Yes | Duration | Interval between run starts. |
| `on-missed` | No | `skip` or `run-once` | Restart policy; defaults to `skip`. |

An interval is a first-class trigger. The compiler MUST NOT translate it into a
cron expression if doing so would change its semantics.

### 9.6 `<event>`

```xml
<event id="orderCreated" name="order.created">
  <schema>
    { "type": "object", "required": ["orderId"] }
  </schema>
</event>
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `name` | Yes | Non-empty string | Event name consumed by the workflow. |
| `secret` | No | Secret reference | Optional public HTTP publisher bearer credential, written as `{{secrets.NAME}}`. Omit it for internal-only publication. |

`<event>` may contain at most one inline Draft 2020-12 `<schema>` using the
same source rules as webhook schemas.

The frontend compiles this trigger to `trigger.event` with literal `name`, an
optional symbolic `secret` reference, and optional literal `schema` fields. The name is
256 characters or fewer, begins with a
lowercase letter, and contains at least two lowercase alphanumeric segments
separated by one `.`, `_`, or `-`. Examples are `order.created`,
`payment_failed`, and `agent-response`.

When `secret` is present, authenticated publication is available at
`POST /_woml/events/{eventName}` with a required `Event-ID`, a bearer control
credential, and one top-level JSON object of at most 1 MiB. Publication fans
out in loaded workflow and trigger order. Each matching subscriber validates
and admits independently, so the response may be `accepted`, `partial`, or
`rejected`. The Rust runtime serves this endpoint, and `woml emit` provides
the built-in secret-store-backed publisher client. Without `secret`, no public
publisher route is opened for that event name.

`services.events.emit(name, payload, options?)` publishes
directly through Rust without HTTP or a control token, derives stable
idempotency from the managed operation identity, returns safe per-subscriber
run results, and stores bounded hidden lineage for cycle/depth protection.

At startup, `woml run` resolves only the optional symbolic secrets referenced by loaded
event triggers. A resolved secret value is held in memory for authentication and is
never written into WOML, Model v7, runtime events, or durable state.

## 10. `<steps>` and Sequential Execution

`<steps>` is the root executable container and contains one or more step items.
The name describes the workflow's executable body; structural `<parallel>` and
`<choose>` elements are valid step items even though they are not executable
operations themselves.

```xml
<steps>
  <step id="first">...</step>
  <step id="second">...</step>
</steps>
```

Within any sequential steps container, document order creates dependency edges:

```text
first -> second
```

`<steps>` has no attributes. It may contain `<step>`, `<parallel>`, `<choose>`,
and `<approval>` elements.

Empty and control-only behavior is explicit:

- `<steps></steps>` and a self-closing `<steps />` are invalid because the root
  container requires at least one step item.
- Every `<when>` and `<otherwise>` arm requires at least one step item followed
  by exactly one `<result>`. A result-producing choice whose cases are structurally empty is
  invalid.
- Approval decision arms may be empty. After a decision, an empty selected arm
  is a successful no-op and execution continues after the approval.
- A workflow whose only item is an approval is valid: it waits durably, records
  the decision output at `context.steps.<approvalId>`, runs the selected arm if
  non-empty, and then completes.
- A result-producing choice requires `<otherwise>`, so a
  successful choice always selects one route and publishes one stable merged result.
- Structural containers do not need to contain a `<script>` specifically, but
  the lowered graph must contain at least one reachable executable or durable
  control node. A graph made only of grouping metadata is invalid.

WOML v1.0 does not expose arbitrary `after`, `depends-on`, `from`, or `to`
attributes. The frontend still produces a DAG, but the source language describes
that DAG through structured sequencing, parallelism, and conditional flow.

## 11. `<step>` Operations

A `<step>` is one identifiable executable node. It contains exactly one
operation. The only fundamental step operation in this document is `<script>`.
An `<approval>` is a first-class control-flow item rather than an operation
wrapped in `<step>`.

```xml
<step
  id="greetUser"
  name="Greet user"
  description="Build the greeting returned to downstream steps"
  retry="3"
  retry-backoff="exponential"
  retry-delay="1s"
  retry-max-delay="30s">
  <script>
    return {
      message: `Hello ${context.payload.name}`
    };
  </script>
</step>
```

### 11.1 Step attributes

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Step ID | Stable DAG-node and output identity. |
| `name` | No | String | Human-readable display name. |
| `description` | No | String | Human-readable description. |
| `retry` | No | Integer `1`–`10` | Maximum total attempts, including the first attempt. |
| `retry-backoff` | No | `fixed` or `exponential` | Backoff strategy; defaults to `exponential` when `retry > 1`. |
| `retry-delay` | No | Positive duration up to `24h` | Fixed delay or initial exponential delay; defaults to `1s`. |
| `retry-max-delay` | No | Positive duration up to `24h` | Exponential cap; invalid with fixed backoff. |

Human-readable name and description are attributes, as defined in Section 5.1.
They lower to `metadata.name` and `metadata.description` on the compiled node.

Retry is always a `<step>` attribute. WOML has no `<retry>` element. Omission
and `retry="1"` both mean one attempt, omit `retryPolicy`, and preserve the
older compiled-model version. A value greater than one compiles to Model v6 and
executes through the durable Rust retry scheduler. Failed attempts, scheduled
retries, and terminal outcomes are recorded before the engine advances. A
safely scheduled retry can be continued with
`woml run ... --state ... --resume <runId>` without replaying completed work.

The complete attribute form is:

```xml
<step
  id="callModel"
  retry="3"
  retry-backoff="exponential"
  retry-delay="1s"
  retry-max-delay="30s">
  ...
</step>
```

Backoff attributes require `retry > 1`. Exponential backoff uses multiplier 2
and no jitter. Its default maximum is the greater of `30s` and
`retry-delay`. Fixed backoff rejects `retry-max-delay`. Every duration must
resolve to a whole number of milliseconds from `1ms` through `24h`.

The frozen failure policy retries only a definitive `script_threw` outcome.
Timeouts, invalid results, size failures, process crashes, interruptions, and
cancellation fail closed. The stable effect key and recovery rules are defined
in `docs/protocols/retry-idempotency-v1.md`.

### 11.2 Script semantics

`<script>` contains the body of an asynchronous JavaScript function. Authors
write ordinary statements and may use `await`, loops, conditions, exceptions,
and returned objects without adding a function wrapper.

The base published CLI profile injects exactly one binding:

- `context` is the fixed read-only, JSON-compatible data projection for the
  current workflow run.

Model v6 freezes a second read-only binding for the retry runtime:

- `attempt.number` is the current one-based attempt number.
- `attempt.maxAttempts` is the compiled maximum total attempt count.
- `attempt.idempotencyKey` is stable across every attempt of the same logical
  step in one run.

The Bun Script Host v3 supplies this binding to every Model v6 invocation. It is
separate from context and does not define `context.run`.

Script Bindings v1 provides the capability profile:

- `services` is the read-only namespace for WOML-owned capabilities.
- `secrets` exposes only literal `secrets.NAME` values proven necessary by the
  frontend; the Model v8 definition records names only.

Using either binding or native `fetch()` selects the capability-aware script
profile. There is no fallback that runs an untracked managed service call.
Service clients and secret values never become context or persisted step
output. Start with [Services](woml-services.md).

Native `fetch()` preserves Bun's standard `Request`/`Response` behavior.
`services.http.request()` returns `{ status, ok, headers, data, url,
redirected }` and supplies managed pooling, parsing, status policy, limits,
timeout, cancellation, and durable operation events. Its complete authoring,
failure, idempotency, security, and deployment contract is documented in
`docs/woml-http-services.md`.

`services.db({ driver: "sqlite", connection })` returns a managed Database v1
handle with parameterized query/execute, reviewed CRUD helpers, and atomic
transaction batches. Rust owns SQLite execution and prevents the handle from
opening WOML's runtime-state database. The full contract, limits, recovery
policy, and example are documented in `docs/woml-database.md`.

`services.storage` provides Rust-owned `put`, `get`, `head`, `list`, and
`delete` operations over durable local objects. It returns portable references
containing a logical key, content-derived version, SHA-256 checksum, size, and
content type. Managed HTTP `responseType: "storage"` streams a response into
the same store without copying its body through Bun or context. The complete
contract, limits, integrity policy, and example are documented in
`docs/woml-storage.md`.

`services.cache` provides workflow-scoped `get`, `set`, `delete`, `has`,
`increment`, and `setIfAbsent` operations over expiring JSON values. Rust owns
TTL enforcement, atomic mutations, bounds, LRU eviction, and the namespace.
Unexpired entries survive local restarts, but cache misses and eviction are
normal and cache must never be used as durable business state. The complete
contract and example are documented in `docs/woml-cache.md`.

`services.state` provides workflow-scoped durable `get`, `has`, `set`,
`delete`, `increment`, and `setIfAbsent` over bounded JSON. Mutations require a
stable name, support retry reattachment, and expose monotonic versions plus
`ifVersion` compare-and-set. State never expires, evicts, falls back to cache,
or enters `context`; the complete contract and operational guide are
documented in `docs/protocols/durable-state-v1.md` and
`docs/woml-durable-state.md`.

`services.events.emit(name, payload, options?)` directly fans out a named event
to loaded `<event>` subscribers without HTTP or a publisher credential. Rust
derives a stable publication identity, durably admits child runs, and protects
against duplicate retry, cycles, and unbounded lineage. The complete contract
and internal-only example are documented in `docs/woml-events-service.md`.

The public v1.0 context paths are:

```text
context.payload
context.steps.<stepId>
```

`context.payload` is the author-facing input regardless of whether a webhook,
Slack message, schedule, event, manual run, Workflow Call, or Workflow Start
created the run. Rust retains the old `trigger` field only inside frozen event,
projection, and Script Host transport contracts. `context.trigger` remains a
deprecated runtime compatibility alias for already compiled scripts; new WOML
source and documentation use `context.payload`.

`context.run` is not present in WOML v1.0. A runtime MUST NOT expose internal run
fields through that name. A future WOML version may add `context.run` only after
its minimal public schema and versioning policy are approved.

The current SDK mapping is:

```text
ctx.payload -> context.payload
ctx.last    -> explicit context.steps.<stepId>
```

A script contributes downstream data only by returning a JSON value. That value
enters the context projection only after the handler succeeds:

```text
return value -> successful handler outcome -> context.steps.<stepId>
```

The runtime appends the versioned success event and then folds event history
into the context projection. Storage changes how the projection is
reconstructed, not the script-facing context contract.

The following do not persist:

- Mutations to the supplied `context` object.
- Local variables.
- Script globals.
- Functions or clients.
- `undefined`, functions, symbols, `BigInt`, circular data, or other non-JSON
  values.

`<script>` has no attributes in the fundamental grammar. Timeout and retry are
step policies and belong on `<step>`.

## 12. `<approval>`, `<notify>`, and Decision Arms

`<approval>` is a first-class durable control-flow item. It records that a run
is waiting for a decision, optionally declares notification deliveries,
suspends the run, and selects exactly one continuation after the decision
arrives. It is not an attribute on `<step>` and is not wrapped by `<step>`.

```xml
<approval
  id="contentApproval"
  name="Content approval"
  description="Ask a moderator to approve or reject the calculated amount"
  timeout="24h"
  on-timeout="reject">

  <notify>
    <slack
      channels="#approvals #engineering"
      bot-token="{{secrets.SLACK_BOT_TOKEN}}"
      app-token="{{secrets.SLACK_APP_TOKEN}}"
    />
  </notify>

  <when-approved>
    <step id="publishApprovedContent">
      <script>
        return publish(context.steps.calc);
      </script>
    </step>
  </when-approved>

  <when-rejected>
    <step id="recordRejection">
      <script>
        return { rejected: true };
      </script>
    </step>
  </when-rejected>
</approval>
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Structural ID | Stable waiting-node identity and `context.steps` output key. |
| `name` | No | String | Human-readable approval name. |
| `description` | No | String | Human-readable explanation of the requested decision. |
| `timeout` | No | Duration | Maximum durable waiting period; omission means no WOML-level deadline. |
| `on-timeout` | No | Enum | `reject` or `fail`; defaults to `fail`. |

Like the other structural elements, `<approval>` does not accept child metadata
tags.

`on-timeout` without `timeout` is invalid because the policy could never take
effect. Executable approval durations must use `ms`, `s`, `m`, `h`, or `d` and
resolve to a positive safe integer number of milliseconds.

The child order is fixed:

1. Optional `<notify>` containing one or more built-in Slack/Telegram/Discord or
   explicitly imported custom-provider deliveries.
2. Required `<when-approved>`.
3. Required `<when-rejected>`.

Each decision arm is a sequential steps container and may contain zero or more
`<step>`, `<parallel>`, `<choose>`, or nested `<approval>` items. Exactly one arm
runs. After the selected arm completes, execution continues after the approval.
An empty arm is a valid no-op continuation.

### 12.1 Durable waiting and notification

The frozen runtime contract says that when execution reaches an approval it:

1. Generates a cryptographically opaque token scoped to this run and approval.
2. Persists the waiting state and token identity before any notification side
   effect occurs.
3. Delivers the approval through an executable notification surface.
4. Suspends further scheduling along this path until a decision or timeout is
   durably recorded.

A process restart leaves the approval waiting. An in-memory Promise, callback,
or timer is never the authoritative state. The token is runtime identity; it is
not WOML source text and is not a compiled constant.

Notification delivery is supervised, persisted, and recoverable. Provider
credentials remain symbolic secret references in compiled definitions; literal
credentials are rejected. See [Notifications](woml-notifications.md) and the
[notification contract](protocols/notification-contracts-v1.md).

One `<slack>` tag targets one credential set and one or more destinations:

```text
alias           := #[a-z0-9][a-z0-9_-]{0,79}
conversation-id := [CG][A-Z0-9]{8,31}
channels        := destination (whitespace+ destination)*
secret          := {{secrets.[A-Z][A-Z0-9_]*}}
```

Destination order is preserved. A duplicate destination within the same Slack
credential set is invalid, including duplicates repeated across tags. Every
destination becomes a separate durable delivery and
message, but all of them resolve the same approval. The first valid decision
wins. Literal credentials, interpolation, and context/service references in
notification credential attributes are invalid. JavaScript-style
`secrets.NAME` is a separate Model v8 script binding and is not attribute
syntax.

One built-in `<telegram>` tag targets one bot credential and one or more
comma-separated numeric chat IDs:

```xml
<telegram
  chats="-1001234567890,123456789"
  bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
/>
```

Approval Telegram tags forbid `message`; lifecycle Telegram tags require it.
Every chat becomes one ordered delivery while every delivery for the approval
shares the same first-valid-decision-wins authority.

One built-in `<discord>` tag targets one bot credential and one or more
comma-separated numeric channel IDs:

```xml
<discord
  channels="200000000000000001,200000000000000002"
  bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
/>
```

Approval Discord tags forbid `message`; lifecycle Discord tags require it.
Every channel becomes one ordered delivery and all deliveries share the same
first-valid-decision-wins approval authority.

### 12.2 Resolving an approval

A reviewer resolves a waiting approval through the WOML HTTP endpoint:

```http
POST /api/v1/approvals/{token}/decision
Content-Type: application/json

{
  "decision": "approved"
}
```

Only the exact decisions `"approved"` and `"rejected"` are accepted in v1.0.
HTTP is the public decision mechanism. WOML does not expose a `woml.resume()`
package API or an in-script control-flow function.

The runtime validates the token, atomically records the decision, and makes the
approval output available as `context.steps.<approvalId>`. The initial output
shape is:

```json
{
  "decision": "approved",
  "source": "human",
  "decidedAt": "2026-08-04T12:00:00.000Z"
}
```

`decision` is `approved` or `rejected`; `source` is `human` or `timeout`.
`decidedAt` is the timestamp recorded by the runtime. A successful human
decision selects the matching arm. An unknown, expired, or conflicting token is
rejected.
Repeating the same already-recorded decision is idempotent; attempting a
different decision for the same approval is a conflict.

### 12.3 Timeout behavior

With `on-timeout="reject"`, expiry durably records a rejected decision with
`source: "timeout"`, publishes the approval output, and runs
`<when-rejected>`. With `on-timeout="fail"`, expiry fails the run and neither
decision arm executes. Omitting `on-timeout` uses `fail`.

Timeout processing and HTTP decisions must compete through one atomic
persistence decision so only one terminal outcome wins.

## 13. `<parallel>`

`<parallel>` runs its direct child steps concurrently and joins after they
finish.

```xml
<parallel
  id="fieldData"
  name="Load field data"
  description="Fetch independent weather and soil readings"
  concurrency="2"
  on-error="wait-all">
  <step id="loadWeather">
    <script>
      return loadWeather(context.payload.fieldId);
    </script>
  </step>

  <step id="loadSoil">
    <script>
      return loadSoil(context.payload.fieldId);
    </script>
  </step>
</parallel>
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Structural ID | Stable identity for the parallel fork/join. |
| `name` | No | String | Human-readable display name. |
| `description` | No | String | Human-readable description. |
| `concurrency` | No | Positive integer | Maximum simultaneously running child steps; defaults to the number of children. |
| `on-error` | No | Enum | `fail-fast` or `wait-all`; defaults to `fail-fast`. |

Structural and data rules:

- `<parallel>` contains one or more direct `<step>` children. A one-step
  parallel is a valid degenerate fork/join, which keeps generated WOML stable
  when a computed branch list happens to contain one item.
- Optional `name` and `description` attributes describe the group.
- When present, `concurrency` MUST NOT exceed the number of child steps.
- A parallel group may appear in root `<steps>` or inside a choice arm.
- A direct parallel child cannot be a choice, parallel group, approval, or
  other control container in this profile.
- All children receive the same context view from immediately before the fork.
- A child MUST NOT reference a sibling's output.
- Each successful child writes its own `context.steps.<stepId>` output.
- `<parallel>` does not create an implicit aggregate output.
- The parallel ID is not a `context.steps` key; only its child steps produce
  context outputs.
- The next flow item becomes ready only after the group reaches its terminal
  outcome.
- After a successful join, every child output is available downstream.
- A root parallel group cannot be the final workflow item in the current CLI
  profile because the group has no aggregate output. Authors add a downstream
  result-building step. A group inside a choice arm may be followed by that
  arm's `<result>` selecting a guaranteed child output.

Error policies:

- `fail-fast` stops scheduling unstarted children, requests cancellation of
  active children, and fails the group after their terminal outcomes are known.
- `wait-all` allows every child to reach a terminal outcome, then fails the
  group if any child failed.

Both designed policies fail the group when a child fails. The previously
considered value `continue` is reserved because continuing downstream would
require defined semantics for missing failed-step outputs.

The compiler represents the children as independent model-v3 DAG nodes using
the frozen `engine.parallel-start`, ordered child edges, ordered join edges,
and `engine.parallel-join` identities. Rust validates this structure at its
model boundary and executes groups with bounded concurrent scheduling. The
versioned event log records group start/completion and policy-specific failure;
protocol-v2 cancellation terminates only the addressed active Workers while
leaving unrelated invocations alive.

Multi-step concurrent lanes are deferred. A future design may add an explicit
`<sequence>` child. Concurrent multi-step routes use the separately reviewed
`<fork>` and `<branch>` vocabulary once that runtime milestone is executable.

## 14. `<choose>`, `<when>`, and `<otherwise>`

`<choose>` represents mutually exclusive conditional flow and replaces the
TypeScript SDK's `.if()`, `.elseIf()`, `.else()`, and `.endIf()` marker chain.

```xml
<choose
  id="alertRoute"
  name="Select alert handling"
  description="Route the analysis by alert severity">
  <when test="{{context.steps.isCritical}}">
    <step id="handleCritical">
      <script>
        return handleCritical(context.steps.analysis);
      </script>
    </step>

    <result value="{{context.steps.handleCritical}}" />
  </when>

  <when test="{{context.steps.needsReview}}">
    <step id="requestReview">
      <script>
        return requestReview(context.steps.analysis);
      </script>
    </step>

    <result value="{{context.steps.requestReview}}" />
  </when>

  <otherwise>
    <step id="acceptAutomatically">
      <script>
        return { accepted: true };
      </script>
    </step>

    <result value="{{context.steps.acceptAutomatically}}" />
  </otherwise>
</choose>
```

### 14.1 Choice structure

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Structural ID | Stable identity for the conditional structure. |
| `name` | No | String | Human-readable display name. |
| `description` | No | String | Human-readable description. |

- The currently executable result-producing `<choose>` requires an `id` attribute.
- The ID is both the stable structural identity and the key of the merged
  successful result at `context.steps.<choiceId>`.
- Optional `name` and `description` attributes describe the choice.
- It contains one or more `<when>` elements.
- It contains exactly one `<otherwise>` in the result-producing profile.
- `<otherwise>` MUST be last.
- Each `<when>` and `<otherwise>` contains one or more step items followed by
  exactly one `<result>`.
- `<when>` elements are evaluated in document order.
- The first test that resolves to `true` is selected.
- At most one case executes.
- `<otherwise>` is selected only when every `<when>` is false.
- When no `<when>` matches, `<otherwise>` is selected.
- The compiler derives deterministic internal identities for individual
  `<when>` and `<otherwise>` arms from the stable choice ID and case ordinal in
  the immutable compiled definition. Source line/column values are diagnostic
  metadata only and never durable identities. Arm identities are not
  user-facing context keys.

### 14.2 The `test` attribute

`test` is required on `<when>` and contains exactly one context reference:

```xml
<when test="{{context.steps.needsReview}}">
```

At runtime, the referenced value MUST be the JSON boolean `true` or `false`.
WOML does not coerce strings, numbers, arrays, objects, or null to booleans. The
frontend can validate the reference and its graph position, but it cannot prove
the return type of arbitrary JavaScript.

These forms are invalid:

```xml
<!-- Arbitrary JavaScript is not allowed in attributes. -->
<when test="context.steps.analysis.risk > 0.3">

<!-- A mixed template is a string, not a boolean reference. -->
<when test="Result: {{context.steps.needsReview}}">
```

Complex conditions belong in named script steps:

```xml
<step id="needsReview">
  <script>
    const { risk, confidence } = context.steps.analysis;
    return risk > 0.3 && confidence < 0.8;
  </script>
</step>

<choose id="reviewRoute">
  <when test="{{context.steps.needsReview}}">
    ...
  </when>
</choose>
```

### 14.3 Conditional outputs

Steps in unselected cases do not produce successful outputs in `context.steps`.
Every arm therefore ends with a typed `<result>` reference:

```xml
<choose id="decisionRoute">
  <when test="{{context.steps.needsReview}}">
    <step id="humanDecision">...</step>
    <result value="{{context.steps.humanDecision}}" />
  </when>

  <otherwise>
    <step id="automaticDecision">...</step>
    <result value="{{context.steps.automaticDecision}}" />
  </otherwise>
</choose>

<step id="publishDecision">
  <script>
    return publish(context.steps.decisionRoute);
  </script>
</step>
```

`<result>` has one required `value` attribute and no children. In the first
executable result-choice profile, `value` contains exactly one context reference and
therefore preserves the referenced JSON type. Literal values, mixed templates,
fallbacks, and arbitrary JavaScript are not accepted in `<result>`.

The reference must be guaranteed to exist on the selected arm before the
result. It may target a step or completed nested choice earlier in the same arm,
or a value that dominates the outer choice. It may not target another arm.

After the selected arm completes, the engine resolves the selected `<result>`
and publishes that JSON value at `context.steps.<choiceId>`. The choice ID is
therefore the only output key that downstream declarative references can use
unconditionally. Route-specific step outputs remain available to JavaScript but
are not guaranteed across every route.

The choice result is pure engine-owned derivation. It does not execute through
Bun and cannot perform an external side effect.

### 14.4 Frozen lowering identities

The WOML frontend lowers a result-producing choice into the language-neutral Compiled Workflow
Model v2 using these identities:

| Item | Frozen compiled identity |
|---|---|
| Selector node | `__woml_branch__<choiceId>__select` (historical compiled identity retained for compatibility) |
| Selector handler | `engine.branch-select` |
| `<when>` arm and selector-edge ID | `<choiceId>:when:<zeroBasedIndex>` |
| `<otherwise>` arm and selector-edge ID | `<choiceId>:otherwise` |
| Result/join node ID | `<choiceId>` |
| Result/join handler | `engine.branch-result` |

The selector's outgoing edges are ordered exactly like the source cases. Each
`<when>` edge carries a strict `boolean` condition and the historical compiled `branchId` field.
The final `<otherwise>` edge carries `condition.kind = "always"` and the same
historical `branchId` field. Ordinary sequencing and join edges do not carry it.

The result node inputs are an object keyed by durable arm ID. Each value is the
compiled context-reference expression from that arm's `<result>`. Generated
selector and arm IDs occupy a namespace that user-authored JavaScript-safe IDs
cannot enter.

Optional choice `name` and `description` lower to descriptive metadata on the
selector node. They never affect selection, identity, result publication, or
the definition's event vocabulary.

Source positions, display names, timestamps, and random values never become
compiled or durable choice identities.

### 14.5 Fork and branch authoring and Model v13 compilation

The frontend accepts and validates the minimal concurrent-route syntax:

```xml
<fork id="distribution" join="instagram facebook">
  <branch id="tiktok">
    <step id="publishTikTok">...</step>
  </branch>

  <branch id="instagram">
    <step id="publishInstagram">...</step>
  </branch>

  <branch id="facebook">
    <step id="publishFacebook">...</step>
  </branch>
</fork>
```

- `<fork>` requires `id` and accepts only the optional `join` attribute.
- It contains one or more direct, non-empty `<branch>` elements.
- Each branch requires an ID local to that fork and may contain multiple flow
  items. Branch bodies currently accept steps, choices, parallel groups, and
  approvals.
- An omitted `join` and `join="all"` wait for every branch. `join="none"`
  waits for none. A whitespace-separated branch-ID list waits only for those
  branches. Selected IDs are canonicalized to document order.
- A fork branch ID may be reused by a different fork. Executable and structural
  IDs inside branch bodies remain unique across the workflow.
- Nested forks are rejected anywhere inside a fork-owned branch subtree in the
  first profile.
- A branch can read context available before the fork and outputs created
  earlier in that same branch. It cannot read a sibling branch output.
- After the fork, only outputs guaranteed by joined branches are visible.
  Unjoined outputs remain unavailable regardless of which branch finishes
  first.
- A terminal fork preserves the last earlier value-producing main-route
  result. A workflow whose only terminal structure is a fork is rejected.

An ID-less control-only `<choose>` has non-empty arms that omit
`<result>`. It controls execution but publishes no `context.steps` result. The
existing `<choose id="...">` profile remains the form for a path-stable merged
result.

`validateWoml` accepts this authoring profile, and `compileWoml` lowers it to
Compiled Workflow Model v13. The graph records deterministic fork ownership,
branch terminals, selected join membership, per-script context visibility, and
one workflow-settlement boundary while preserving the main-route result.
`woml check` therefore succeeds for valid fork source. Rust independently
validates the Model v13 graph and executes all three join profiles durably:
`join="all"`, a selected list of branch IDs, and `join="none"`. Each branch
remains sequential internally while ready branches overlap through the
multiplexed Bun host. A selected or non-blocking join may release the main route
while unjoined work continues, but the workflow-settlement boundary prevents a
successful run outcome from being published until every owned branch settles.
Compiled context visibility—not completion timing—determines which step outputs
each script can see. Event v12 fork-open, branch-settlement, and join-settlement
facts persist in the existing append-only Store v14. Run Inspection v4 and the
recovery work profile are rebuilt from those events and never expose payloads,
context, outputs, or secrets. Control-only and result-producing choices execute
identically on the main route and inside a branch. A failed or cancelled branch
settles durably; joined failure prevents the continuation from running, while
unjoined failure never blocks the selected join. The run itself becomes
terminal only after every opened branch settles, so a partial main-route value
is never printed as a successful result for a failed run. Recovery resumes safe
pending work and fails a started effect without a terminal event closed rather
than replaying it.

The complete executable example is
[`examples/forkDistributionWorkflow.woml`](../examples/forkDistributionWorkflow.woml).
Run it with `woml test examples/forkDistributionWorkflow.woml` for one result,
or use it as part of a long-lived `woml run` deployment.

### 14.6 Exact-string switch routing

`<switch>` is the compact value-routing form. It compares one exact context
reference against ordered string cases and executes exactly one route:

```xml
<switch id="delivery" value="{{context.steps.order.provider}}">
  <case value="slack">
    <step id="sendSlack">...</step>
    <result value="{{context.steps.sendSlack}}" />
  </case>

  <case value="email">
    <step id="sendEmail">...</step>
    <result value="{{context.steps.sendEmail}}" />
  </case>

  <default>
    <step id="unsupportedProvider">...</step>
    <result value="{{context.steps.unsupportedProvider}}" />
  </default>
</switch>
```

- `value` is required and MUST contain exactly one available `context` reference.
- The runtime value MUST be a JSON string. WOML performs no coercion,
  trimming, or case folding. Other JSON types fail with
  `WOML_SWITCH_VALUE_INVALID`.
- One or more `<case>` arms are required. Their non-empty string values MUST be
  unique; matching is exact and case-sensitive.
- Exactly one final `<default>` is required. There is no fallthrough.
- Every arm contains one or more ordinary flow items. Existing recursive
  placement rules continue to govern nested choices, switches, parallels,
  approvals, and forks.
- Without `id`, the switch is control-only and its arms MUST NOT contain
  `<result>`.
- With `id`, optional `name` and `description` become available and every arm
  MUST end in exactly one `<result>`. The selected JSON value is published at
  `context.steps.<switchId>` for downstream steps.
- The frontend lowers the selector to Model v14 `engine.choice-select`, stores
  its selector and exact case table in the compiled choice descriptor, and
  uses `engine.choice-result` only for the result-producing profile.
- Rust records the selected arm through the existing durable
  `choice_selected` event. Recovery reuses that event and never evaluates the
  selector again.

## 15. Attribute Values and Context References

Every attribute has a declared type. WOML does not treat every resolved value as
a string.

### 15.1 Literal attributes

Literal text is parsed according to the attribute declaration:

```xml
<config concurrency="4" timeout="10m" />
```

Here, `concurrency` becomes an integer and `timeout` becomes a duration. Invalid
values are compile errors.

### 15.2 Exact references preserve type

An attribute value containing exactly one reference preserves the referenced
JSON type:

```xml
<when test="{{context.steps.isEligible}}">
```

If `isEligible` is a boolean, `test` receives a boolean rather than the string
`"true"`.

Future declarative capability attributes use the same rule:

```xml
<operation count="{{context.steps.calculate.total}}" />
```

If `total` is a number and `count` accepts numbers, it remains a number.

### 15.3 Mixed templates are strings

An attribute containing literal text plus one or more references always produces
a string:

```xml
<operation message="Processed {{context.steps.calculate.total}} items" />
```

`<operation>` is illustrative, not accepted syntax.

### 15.4 Reference grammar

The frozen language grammar defines exact references with no internal
whitespace:

```text
reference       := "{{" context-path "}}"
context-path    := payload-path | step-path
payload-path    := "context.payload" ("." property-id)*
step-path       := "context.steps." structural-id ("." property-id)*
structural-id   := [a-z][A-Za-z0-9]*
property-id     := [A-Za-z_$][A-Za-z0-9_$]*
```

Bracket access, optional chaining, operators, function calls, and fallbacks are
not WOML references. A referenced step must exist and dominate the consumer in
the lowered DAG. A missing nested property at runtime produces
`WOML_REFERENCE_NOT_AVAILABLE`; it never becomes `undefined` or an empty string.

The result-producing choice profile resolves exact references in `<when test>` and
`<result value>` without passing them through JavaScript. Scripts continue to
read `context.steps.<id>` directly through the injected JavaScript context.
Mixed templates are accepted only by attributes whose contract explicitly
allows templates, such as lifecycle notification messages.

## 16. Static Validation

The WOML frontend MUST reject a document before run creation when any of these
conditions hold.

### 16.1 Structural errors

- The document has no `<workflow>` root or has more than one root.
- Required root children are missing, duplicated, or under the wrong parent.
- An element appears under an invalid parent.
- A required attribute is missing.
- An attribute is duplicated.
- An element or attribute is unknown.
- A singleton child occurs more than once.
- `<steps>` contains no step items.
- `<parallel>` contains no steps.
- `<parallel>` directly contains anything other than `<step>`.
- A root workflow ends with `<parallel>` without a downstream result-building
  step.
- `<choose>` has no `<when>`.
- `<choose>` has no `<otherwise>` in the executable result-choice profile.
- A `<when>` or `<otherwise>` arm contains no step items.
- `<otherwise>` is duplicated or is not last.
- A result-choice arm has no `<result>`, has more than one `<result>`, or places
  `<result>` anywhere except last.
- A step contains zero or multiple operations.
- `<approval>` is missing `<when-approved>` or `<when-rejected>`, duplicates
  either arm, or declares them out of order.
- `<approval>` contains more than one `<notify>`, or `<notify>` is empty.
- `<notify>` contains anything other than supported built-in Slack, Telegram,
  Discord, or WhatsApp tags or explicitly imported custom notification-provider tags.
- A structural element contains `<name>` or `<description>` child elements
  instead of the corresponding attributes.

### 16.2 Identity errors

- A workflow, trigger, step, parallel, branch, or approval ID is empty or
  malformed.
- Trigger IDs are duplicated.
- A step, parallel, branch, or approval ID is duplicated anywhere in the
  structural namespace, including across conditional cases, approval arms, and
  parallel groups.

### 16.3 Value errors

- A duration has no unit or uses an unsupported unit.
- `concurrency` or a rate count is not a positive integer.
- Parallel `concurrency` exceeds its direct child count.
- `retry` is not an integer from `1` through `10`.
- A retry backoff attribute appears without `retry > 1`.
- `retry-backoff` is not `fixed` or `exponential`.
- A retry delay is zero, malformed, exceeds `24h`, or does not resolve to a
  whole number of milliseconds.
- `retry-max-delay` is smaller than `retry-delay` or is used with fixed
  backoff.
- An enum attribute contains an unsupported value.
- Inline webhook JSON Schema is malformed or invalid.
- Approval `timeout` is invalid or `on-timeout` is not `reject` or `fail`.
- Approval declares `on-timeout` without a `timeout`.
- A `<when>` test is not exactly one context reference.
- A `<result>` value is not exactly one context reference.

### 16.4 Reference and dependency errors

- A WOML attribute reference targets a nonexistent step.
- A WOML attribute reference targets a later sequential step.
- A WOML attribute in one parallel child references a sibling.
- A branch test references a value not guaranteed before the branch.
- A WOML attribute in one selected case references a node from another case.
- A branch result references a value that is not guaranteed earlier in its own
  selected arm or before the branch.
- A downstream WOML attribute unconditionally references an output that only
  exists on some conditional paths instead of the stable branch result.
- An approval arm or downstream item references a route-local output that is
  not guaranteed; the approval's own stable decision output is the portable
  cross-route value.

Context references do not automatically create graph dependencies. Structural
order and nesting create the DAG; references are validated against it.

The compiler SHOULD diagnose statically recognizable direct JavaScript accesses
such as `context.steps.missingStep`, but arbitrary JavaScript is not the WOML
reference grammar. Dynamic property access remains a runtime concern and cannot
be the only signal used to create a dependency edge.

### 16.5 Lowered graph well-formedness

After lowering and before registration or execution, the frontend MUST verify:

- The graph is directed and acyclic.
- Node IDs and edge IDs are unique.
- Every edge endpoint names a node in the same graph.
- Every entry node exists and has no incoming edge.
- Every node is reachable from an entry node.
- Every source step or durable control item lowers exactly once; no source item
  is reachable through two duplicate structural paths.
- Global structural IDs remain unique across every nested branch case,
  parallel group, and approval arm.
- Joining nested containers does not create an unreachable node or a cycle.

The first CLI executable profile additionally requires exactly one terminal
node: one node with no outgoing edge. Its result is the command's workflow
result. DAGs with multiple terminal nodes remain valid in the general compiled
model but are unavailable to this CLI profile until workflow-output semantics
are explicitly designed.

Structured sequencing makes cycles impossible for a correct compiler, but the
compiler still checks the emitted artifact. This protects the model boundary
from compiler bugs and from compiled models produced by future non-WOML
frontends.

## 17. Diagnostic Contract

Parse, validation, and compile failures are part of the WOML public interface.
Every frontend diagnostic MUST contain:

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

Rules:

- `code` is a stable machine-readable identifier such as
  `WOML_UNKNOWN_ELEMENT`, `WOML_DUPLICATE_ID`, or
  `WOML_REFERENCE_NOT_AVAILABLE`.
- `line` and `column` are one-based; `offset` is a zero-based UTF-8 source byte
  offset.
- `message` explains what is wrong in author language and names the relevant
  element, attribute, or ID.
- `hint`, when present, gives one concrete correction and is not a second error.
- A frontend MAY return multiple diagnostics. The first diagnostic is primary;
  all remaining diagnostics are sorted by source position.
- Locations always refer to the original `.woml` file. Raw-body extraction or
  XML-safe placeholders MUST retain an offset map; diagnostics must never point
  into a rewritten temporary document.
- A JavaScript parse or runtime error points to the `<script>` body when an exact
  inner location is unavailable. When Bun provides a reliable inner line and
  column, the runtime translates it back to the original WOML coordinates.

The executable result-choice profile freezes these source diagnostics. Legacy
conditional `<branch>` files retain their historical `WOML_BRANCH_*`
validation codes and additionally receive
`WOML_DEPRECATED_CONDITIONAL_BRANCH` as a warning:

| Code | Phase | Meaning and primary location |
|---|---|---|
| `WOML_CHOOSE_WHEN_REQUIRED` | validation | `<choose>` contains no `<when>`; points to the choice opening tag. |
| `WOML_CHOOSE_OTHERWISE_REQUIRED` | validation | `<choose>` has no fallback; points to the choice opening tag. |
| `WOML_CHOOSE_OTHERWISE_ORDER` | validation | `<otherwise>` is duplicated or not last; points to the offending tag. |
| `WOML_CHOOSE_RESULT_REQUIRED` | validation | An arm has zero or multiple `<result>` children; points to the arm or duplicate result. |
| `WOML_CHOOSE_RESULT_ORDER` | validation | `<result>` is not the final arm child; points to the misplaced result. |
| `WOML_INVALID_REFERENCE` | validation | `test` or `value` is not exactly one frozen WOML context reference; points to the attribute value. |
| `WOML_UNKNOWN_REFERENCE` | compile | A reference names an unknown structural ID; points to that ID inside the attribute. |
| `WOML_REFERENCE_NOT_DOMINATING` | compile | The referenced output is later, in another arm, or otherwise not guaranteed; points to that reference. |
| `WOML_BRANCH_TEST_NOT_BOOLEAN` | runtime | A selected test reference resolved to a non-boolean JSON value; points to the `test` value. |
| `WOML_REFERENCE_NOT_AVAILABLE` | runtime | A compiled reference path has a missing property; points to the consuming `test` or `value`. |
| `WOML_BRANCH_SELECTION_INVALID` | runtime | The compiled/event branch identity is inconsistent; reported at the branch opening tag. |

General diagnostics such as `WOML_DUPLICATE_ID`, `WOML_UNKNOWN_ATTRIBUTE`, and
`WOML_INVALID_DAG` continue to apply. Historical compiled handler and runtime
error names containing `branch` remain unchanged so stored definitions and
events do not change identity.

The CLI renders the primary diagnostic in this stable form:

```text
hello.woml:14:18 [WOML_REFERENCE_NOT_AVAILABLE] Step "missing" is not available here
```

Human formatting may improve without changing the diagnostic object or code.
Webhook input-schema failures use the transport response contract in Section
9.2 because their location is the request payload, not WOML source.

## 18. v1.0 boundaries

The following syntax is intentionally not part of WOML v1.0:

- arbitrary DAG-edge attributes such as `after`, `from`, or `depends-on`;
- structural loops, for-each, batching, race, or first-success groups;
- declarative HTTP/database/storage tags inside `<step>`;
- npm/package imports or dynamic module installation;
- external schema-file references;
- `context.env`, `context.run`, or dynamic secret enumeration;
- a package-level `woml.resume()` API; and
- long approval-waiting synchronous workflow calls.

Authors may use ordinary JavaScript inside `<script>` for local computation.
Durable external effects should use the reviewed `services` capabilities. New
language constructs require their own versioned compilation, event, recovery,
and diagnostic contracts; the v1.0 compiler rejects them instead of guessing.

`<parallel on-error>` accepts only `fail-fast` and `wait-all`. Both fail the
group when any child fails. There is no missing-output continuation mode.

## 19. Minimal workflow

This workflow exercises manual triggering and direct context threading between
two sequential script steps:

```xml
<woml>
<workflow version="1.0.0" id="hello" name="Hello WOML">
  <triggers>
    <manual id="start" />
  </triggers>

  <steps>
    <step
      id="a"
      name="Choose greeting name"
      description="Use the trigger name or default to World">
      <script>
        const name = context.payload.name ?? "World";

        return {
          x: name
        };
      </script>
    </step>

    <step id="b">
      <script>
        return {
          message: `Hello ${context.steps.a.x}`
        };
      </script>
    </step>
  </steps>
</workflow>
</woml>
```

For `woml run hello.woml`, press Enter to create a run. The manual payload is
`{}` and the final result is:

```json
{ "message": "Hello World" }
```

The engine builds `context.steps` by folding durable run events; scripts never
receive an authoritative mutable workflow state object. Use `woml test
hello.woml` when an automated test or CI job needs a one-shot execution that
exits.
