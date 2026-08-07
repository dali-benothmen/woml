# WOML v0.1 Fundamental Syntax

Status: design catalog draft; sequential scripts, conditional branches, and
bounded parallel groups are executable and publishable through the Rust-backed
CLI, including both failure policies and fail-fast Worker cancellation
Scope: fundamental workflow structure, triggers, script and approval steps,
parallel flow, conditional flow, configuration, and lifecycle hooks

This document defines the proposed fundamental authoring syntax for WOML v0.1.
It contains both a language-design catalog and an executable-profile contract.
A construct appearing in the design catalog is not automatically implemented,
publishable, or accepted by a runtime. Elements whose lowering or runtime
semantics remain open must not be silently implemented with guessed behavior.

WOML is a frontend for WOML's language-neutral compiled workflow model. The
WOML compiler validates this source and lowers it to that model; the execution
core never parses or interprets WOML.

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
requirements within this draft.

### 1.1 Feature maturity and publication

WOML separates syntax design from executable availability:

- **Designed** means the proposed source shape and author-facing intent are
  documented for review.
- **Executable** means the construct has an approved lowering, runtime
  semantics, diagnostics, and end-to-end tests.
- **Publishable** means an executable construct is included in an advertised
  runtime profile. A runtime MUST reject every designed-only construct with an
  unsupported-feature diagnostic.

The CLI profile grows only through reviewed vertical milestones. The original
walking skeleton proved sequential scripts; the current published profile also
includes conditional branches and bounded parallel groups:

| Feature | Design status | Current CLI profile |
|---|---|---|
| Workflow `id`/`name`/`description`, manual trigger, sequential steps | Frozen | Executable and publishable |
| `<script>` with `context.trigger` and `context.steps` | Frozen | Executable and publishable |
| `{{context...}}` attribute-reference grammar | Frozen | Executable for branch `test` and `result` |
| Workflow `tags`/`version` and step `retry`/`timeout` | Frozen, runtime-staged attributes | Unavailable; the attributes must be omitted |
| Webhook and inline payload schema | Designed | Unavailable |
| Config, lifecycle, schedule, interval, and event triggers | Designed | Unavailable |
| Branch | Frozen | Executable and publishable |
| Parallel | Frozen | Executable and publishable with bounded concurrency, `wait-all`, and `fail-fast` |
| Approval | Frozen; A1–A7 implemented and hardened | Executable and publishable in the local profile: `woml run` pauses durably, prints a local approval URL, accepts an HTTP decision through Rust, recovers, and continues only the selected route |
| `{{secrets.NAME}}` and `woml secrets` | Frozen; N1 implemented | Secret references, secure local/CI secret management, and typed Slack credential sinks are available |
| `<notify><slack>` approval delivery | Frozen; N0–N6 implemented and hardened | Executable and publishable: the built-in Slack provider delivers through Socket Mode, one action resolves durably in Rust, the selected route continues, and every delivered message converges |
| Database, HTTP, RAK, and other capabilities | Deferred | Unavailable |

The complete example in Section 3 demonstrates the design catalog; it is not a
claim that the first CLI profile can execute every element shown. The minimum
publishable example is in Section 20.

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

Raw-content termination is deterministic in v0.1:

- The first literal, case-sensitive `</script>` terminates a `<script>` body.
- The first literal, case-sensitive `</schema>` terminates a `<schema>` body.
- The terminator is recognized without interpreting JavaScript or JSON strings,
  comments, escapes, regular expressions, or template literals.
- The exact terminator text therefore MUST NOT occur inside the raw content.
  JavaScript that needs to construct the text can split it, for example
  `"</scr" + "ipt>"`.
- The frontend MUST preserve the raw body exactly and MUST retain an offset map
  to the original source for diagnostics.

This simple raw-text rule is part of the v0.1 grammar. A future WOML version may
introduce a JavaScript-aware terminator without changing v0.1 parsing.

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
<workflow
  id="content-moderator"
  name="AI Content Moderator"
  description="Analyze submitted content and select a review path"
  tags="ai,moderation,safety"
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

    <on-failure>
      <script>
        console.error("Content moderation failed");
      </script>
    </on-failure>
  </lifecycle>

  <triggers>
    <webhook
      id="moderateContent"
      path="/webhooks/moderate-content"
      method="POST">

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
      description="Normalize the submitted content before analysis"
      timeout="30s">
      <script>
        const { content } = context.trigger;

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
          return analyzeImage(context.trigger.imageUrl);
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

    <branch
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
                  contentId: context.trigger.contentId,
                  approved: true
                };
              </script>
            </step>
          </when-approved>

          <when-rejected>
            <step id="recordHumanRejection">
              <script>
                return {
                  contentId: context.trigger.contentId,
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
    </branch>
  </steps>
</workflow>
```

## 4. Document Structure

A WOML file contains exactly one `<workflow>` root.

The root children appear in this order:

1. Optional `<config>`.
2. Optional `<lifecycle>`.
3. Required `<triggers>`.
4. Required `<steps>`.

The order is deliberate. Metadata and runtime policy are declared before entry
points, and entry points are declared before executable steps.

The structural grammar is:

```text
document       := workflow

workflow       := <workflow workflow-attributes>
                    config?
                    lifecycle?
                    triggers
                    steps
                  </workflow>

config         := <config config-attributes />

lifecycle      := <lifecycle>
                    on-success?
                    on-failure?
                  </lifecycle>

on-success     := <on-success> script </on-success>
on-failure     := <on-failure> script </on-failure>

triggers       := <triggers> trigger+ </triggers>

trigger        := manual
                | webhook
                | schedule
                | interval
                | event

steps          := <steps> steps-item+ </steps>

steps-item     := step
                | parallel
                | branch
                | approval

step           := <step step-attributes>
                    operation
                  </step>

operation      := script

parallel       := <parallel parallel-attributes>
                    step+
                  </parallel>

branch         := <branch branch-attributes>
                    when+
                    otherwise
                  </branch>

when           := <when test="context-reference">
                    steps-item+
                    result
                  </when>

otherwise      := <otherwise>
                    steps-item+
                    result
                  </otherwise>

result         := <result value="context-reference" />

approval       := <approval approval-attributes>
                    notify?
                    when-approved
                    when-rejected
                  </approval>

notify         := <notify> slack+ </notify>

slack          := <slack
                    channels="slack-destination-list"
                    bot-token="secret-reference"
                    app-token="secret-reference"
                  />

when-approved  := <when-approved>
                    steps-item*
                  </when-approved>

when-rejected  := <when-rejected>
                    steps-item*
                  </when-rejected>
```

This is a structural grammar. Identifier and raw-content tokenization are fixed
by Sections 2 and 5. Attribute-reference tokenization is defined in Section 15.

## 5. Identifiers

Six identifier roles exist:

- A workflow ID identifies the workflow definition.
- A trigger ID identifies a trigger within that definition.
- A step ID identifies an executable DAG node and its output in context.
- A parallel ID identifies a fork/join structure for diagnostics and events.
- A branch ID identifies a conditional structure for diagnostics and events.
- An approval ID identifies a durable waiting node, its decision, and its output
  in context.

All IDs MUST be non-empty. Workflow IDs and trigger IDs MUST be unique within
their respective namespaces. Step, parallel, branch, and approval IDs share one
structural namespace and MUST be unique across the whole workflow.

Step IDs are part of the JavaScript-facing API:

```js
context.steps.preprocessContent
```

Approval IDs use the same JavaScript-facing output namespace:

```js
context.steps.humanApproval
```

Trigger, step, parallel, branch, and approval IDs MUST be JavaScript-safe
lower-camel identifiers matching:

```text
[a-z][A-Za-z0-9]*
```

Quoted or bracket context paths are not part of WOML v0.1. This keeps JavaScript
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

`<step>`, `<parallel>`, `<branch>`, and `<approval>` use optional `name` and
`description` attributes:

```xml
<step
  id="generateReport"
  name="Generate report"
  description="Build the report returned to the publishing step">
  <script>
    return generateReport(context.trigger);
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
| `tags` | No | Tag list | Comma-separated classification tags. |
| `version` | No | Version string | User-defined workflow version. It does not select the WOML grammar. |

When present, workflow `name`, `description`, and `version` must each contain
at least one non-whitespace character.

`tags` is a comma-separated list. Whitespace surrounding each item is removed.
Empty items and duplicate items are invalid.

```xml
<workflow
  id="daily-report"
  name="Daily Report"
  description="Generate and publish the daily report"
  tags="reporting,daily"
  version="1.2.0">
  ...
</workflow>
```

Runtime settings such as concurrency and timeout MUST NOT also appear on
`<workflow>`. They have one canonical location: `<config>`.

In the first CLI profile, `name`, `description`, and `version` lower to
`metadata.name`, `metadata.description`, and `metadata.version` on the
compiled workflow. The `tags` attribute remains staged because its compiled
representation is not yet frozen.

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
The rate-limiting algorithm and distributed coordination behavior are runtime
contracts; the source syntax only declares the count and window.

The initial duration syntax is:

```text
duration := positive-number ("ms" | "s" | "m" | "h" | "d")
```

Bare numeric durations are invalid because their unit would be ambiguous.

`<config>` contains data only. Lifecycle scripts MUST NOT be nested inside it.

The current compiled-workflow schema does not yet represent workflow-level
concurrency, timeout, rate limiting, or queue selection. Before the compiler may
accept `<config>`, these values need an approved language-neutral home—either in
the compiled definition or in a separately versioned registration policy. The
WOML frontend must not pass them to the core through WOML-specific fields.

## 8. `<lifecycle>`

`<lifecycle>` declares workflow-level lifecycle behavior. It is separate from
`<config>` because its children execute code rather than describe runtime data.

```xml
<lifecycle>
  <on-success>
    <script>
      console.log("Workflow completed");
    </script>
  </on-success>

  <on-failure>
    <script>
      console.error("Workflow failed");
    </script>
  </on-failure>
</lifecycle>
```

Structural rules:

- `<lifecycle>` is optional and may occur at most once.
- `<on-success>` is optional and may occur at most once.
- `<on-failure>` is optional and may occur at most once.
- Each lifecycle hook contains exactly one `<script>`.
- Lifecycle scripts do not create `context.steps` outputs.

This draft fixes the lifecycle syntax but does not yet approve the durable event
boundaries, retry behavior, or failure semantics of lifecycle execution. A
runtime MUST reject unsupported lifecycle hooks rather than silently ignore
them.

The current compiled-workflow schema also has no lifecycle representation. That
contract must be extended deliberately before lifecycle syntax can lower into an
executable model.

## 9. `<triggers>`

`<triggers>` contains one or more workflow entry points.

All triggers in one workflow start the same `<steps>`. WOML v0.1 does not expose
an attribute that routes different triggers to different entry nodes. Workflows
with different graphs MUST be separate workflow definitions.

This corrects an ambiguity in the TypeScript SDK where triggers and steps are
stored in unrelated flat arrays.

### 9.1 `<manual>`

```xml
<manual id="manualRun" />
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |

Manual CLI input becomes `context.trigger`.

### 9.2 `<webhook>`

```xml
<webhook id="newOrder" path="/webhooks/orders" method="POST">
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
| `path` | Yes | Absolute route path | Route registered by the runtime. |
| `method` | No | HTTP method | `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`; defaults to `POST`. |

`<webhook>` may contain at most one `<schema>`. An inline schema is standard JSON
Schema Draft 2020-12. The compiler MUST parse and validate the schema before the
workflow can be registered.

When a schema exists, the webhook payload MUST validate before a run is created.
The normalized successful payload becomes `context.trigger`.

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

### 9.3 `<schedule>`

```xml
<schedule
  id="everySixHours"
  cron="0 */6 * * *"
  timezone="UTC"
/>
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `cron` | Yes | Cron expression | Schedule expression. |
| `timezone` | No | IANA timezone | Evaluation timezone; defaults to `UTC`. |

The supported cron dialect must be named by the runtime specification before
`<schedule>` is considered executable.

### 9.4 `<interval>`

```xml
<interval id="everyFiveMinutes" every="5m" />
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `every` | Yes | Duration | Interval between run starts. |

An interval is a first-class trigger. The compiler MUST NOT translate it into a
cron expression if doing so would change its semantics.

### 9.5 `<event>`

```xml
<event id="orderCreated" name="order.created" />
```

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Trigger ID | Stable trigger identity. |
| `name` | Yes | Non-empty string | Event name consumed by the workflow. |

Transport, subscription, acknowledgement, and event-delivery guarantees are
outside the fundamental syntax and require a runtime contract.

## 10. `<steps>` and Sequential Execution

`<steps>` is the root executable container and contains one or more step items.
The name describes the workflow's executable body; structural `<parallel>` and
`<branch>` elements are valid step items even though they are not executable
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

`<steps>` has no attributes. It may contain `<step>`, `<parallel>`, `<branch>`,
and `<approval>` elements.

Empty and control-only behavior is explicit:

- `<steps></steps>` and a self-closing `<steps />` are invalid because the root
  container requires at least one step item.
- Every `<when>` and `<otherwise>` arm requires at least one step item followed
  by exactly one `<result>`. A branch whose cases are structurally empty is
  invalid.
- Approval decision arms may be empty. After a decision, an empty selected arm
  is a successful no-op and execution continues after the approval.
- A workflow whose only item is an approval is valid: it waits durably, records
  the decision output at `context.steps.<approvalId>`, runs the selected arm if
  non-empty, and then completes.
- The first executable branch profile requires `<otherwise>`, so a successful
  branch always selects one route and publishes one stable merged result.
- Structural containers do not need to contain a `<script>` specifically, but
  the lowered graph must contain at least one reachable executable or durable
  control node. A graph made only of grouping metadata is invalid.

WOML v0.1 does not expose arbitrary `after`, `depends-on`, `from`, or `to`
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
  retry="1"
  timeout="5s">
  <script>
    return {
      message: `Hello ${context.trigger.name}`
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
| `retry` | No | Integer `1` | Reserved retry policy; omission and `1` both mean one attempt in v0.1. |
| `timeout` | No | Duration | Maximum duration of each attempt. |

Human-readable name and description are attributes, as defined in Section 5.1.
They lower to `metadata.name` and `metadata.description` on the compiled node.

The frozen design grammar reserves `retry="1"` as the only v0.1 value, but the
first CLI executable profile rejects the `retry` attribute entirely with
`WOML_FEATURE_NOT_EXECUTABLE`. It never silently ignores the attribute. Any
value greater than one is also outside the v0.1 grammar. Multi-attempt retry
cannot become executable until retryable-error, backoff, interruption, and
idempotency contracts are approved together.

The fundamental syntax intentionally keeps retry as an attribute. When backoff
is added, it should remain attribute-based rather than introducing a second
retry representation. Candidate future attributes are:

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

These backoff attributes and `retry` values greater than one are reserved, not
part of the executable v0.1 grammar.

### 11.2 Script semantics

`<script>` contains the body of an asynchronous JavaScript function. Authors
write ordinary statements and may use `await`, loops, conditions, exceptions,
and returned objects without adding a function wrapper.

The first CLI executable profile injects exactly one binding:

- `context` is the fixed read-only, JSON-compatible data projection for the
  current workflow run.

`services` is unavailable in this profile. A script that references it receives
the normal JavaScript missing-binding failure. A later capability profile may
add a `services` binding; service clients and secrets must never become context
or persisted step output.

The complete v0.1 context paths are:

```text
context.trigger
context.steps.<stepId>
```

`context.run` is not present in WOML v0.1. A runtime MUST NOT expose internal run
fields through that name. A future WOML version may add `context.run` only after
its minimal public schema and versioning policy are approved.

The current SDK mapping is:

```text
ctx.payload -> context.trigger
ctx.last    -> explicit context.steps.<stepId>
```

A script contributes downstream data only by returning a JSON value. That value
enters the context projection only after the handler succeeds:

```text
return value -> successful handler outcome -> context.steps.<stepId>
```

The first CLI profile publishes that outcome to its in-memory projection. A
durable runtime first appends the versioned success event and then folds the
event history into the same projection. Storage changes how the projection is
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

1. Optional `<notify>` containing one or more `<slack>` deliveries.
2. Required `<when-approved>`.
3. Required `<when-rejected>`.

Each decision arm is a sequential steps container and may contain zero or more
`<step>`, `<parallel>`, `<branch>`, or nested `<approval>` items. Exactly one arm
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

N0 freezes the Slack-first syntax, compiled model v5, event vocabulary v5,
provider-host v1, delivery identity, failure behavior, and secret boundary in
`docs/protocols/notification-contracts-v1.md`. N1 implements secure secret
management. N2 implements source validation and Model v5 lowering. Until N3,
`woml run` stops after successful compilation with
`WOML_NOTIFICATION_RUNTIME_UNAVAILABLE`; it never silently ignores delivery.

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
wins. Literal credentials, interpolation, context/service references, and
`secrets.NAME` inside `<script>` are invalid.

### 12.2 Resolving an approval

A reviewer resolves a waiting approval through the WOML HTTP endpoint:

```http
POST /api/v1/approvals/{token}/decision
Content-Type: application/json

{
  "decision": "approved"
}
```

Only the exact decisions `"approved"` and `"rejected"` are accepted in v0.1.
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
      return loadWeather(context.trigger.fieldId);
    </script>
  </step>

  <step id="loadSoil">
    <script>
      return loadSoil(context.trigger.fieldId);
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
- A parallel group may appear in root `<steps>` or inside a branch arm.
- A direct parallel child cannot be a branch, parallel group, approval, or
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
  result-building step. A group inside a branch arm may be followed by that
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
`<sequence>` child, but `<branch>` is not used for that purpose.

## 14. `<branch>`, `<when>`, and `<otherwise>`

`<branch>` represents mutually exclusive conditional flow and replaces the
TypeScript SDK's `.if()`, `.elseIf()`, `.else()`, and `.endIf()` marker chain.

```xml
<branch
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
</branch>
```

### 14.1 Branch structure

| Attribute | Required | Type | Meaning |
|---|---:|---|---|
| `id` | Yes | Structural ID | Stable identity for the conditional structure. |
| `name` | No | String | Human-readable display name. |
| `description` | No | String | Human-readable description. |

- `<branch>` requires an `id` attribute.
- The ID is both the stable structural identity and the key of the merged
  successful result at `context.steps.<branchId>`.
- Optional `name` and `description` attributes describe the branch.
- It contains one or more `<when>` elements.
- It contains exactly one `<otherwise>` in the first executable branch profile.
- `<otherwise>` MUST be last.
- Each `<when>` and `<otherwise>` contains one or more step items followed by
  exactly one `<result>`.
- `<when>` elements are evaluated in document order.
- The first test that resolves to `true` is selected.
- At most one case executes.
- `<otherwise>` is selected only when every `<when>` is false.
- When no `<when>` matches, `<otherwise>` is selected.
- The compiler derives deterministic internal identities for individual
  `<when>` and `<otherwise>` arms from the stable branch ID and case ordinal in
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

<branch id="reviewRoute">
  <when test="{{context.steps.needsReview}}">
    ...
  </when>
</branch>
```

### 14.3 Conditional outputs

Steps in unselected cases do not produce successful outputs in `context.steps`.
Every arm therefore ends with a typed `<result>` reference:

```xml
<branch id="decisionRoute">
  <when test="{{context.steps.needsReview}}">
    <step id="humanDecision">...</step>
    <result value="{{context.steps.humanDecision}}" />
  </when>

  <otherwise>
    <step id="automaticDecision">...</step>
    <result value="{{context.steps.automaticDecision}}" />
  </otherwise>
</branch>

<step id="publishDecision">
  <script>
    return publish(context.steps.decisionRoute);
  </script>
</step>
```

`<result>` has one required `value` attribute and no children. In the first
executable branch profile, `value` contains exactly one context reference and
therefore preserves the referenced JSON type. Literal values, mixed templates,
fallbacks, and arbitrary JavaScript are not accepted in `<result>`.

The reference must be guaranteed to exist on the selected arm before the
result. It may target a step or completed nested branch earlier in the same arm,
or a value that dominates the outer branch. It may not target another arm.

After the selected arm completes, the engine resolves the selected `<result>`
and publishes that JSON value at `context.steps.<branchId>`. The branch ID is
therefore the only output key that downstream declarative references can use
unconditionally. Route-specific step outputs remain available to JavaScript but
are not guaranteed across every route.

The branch result is pure engine-owned derivation. It does not execute through
Bun and cannot perform an external side effect.

### 14.4 Frozen lowering identities

The WOML frontend lowers a branch into the language-neutral Compiled Workflow
Model v2 using these identities:

| Item | Frozen compiled identity |
|---|---|
| Selector node | `__woml_branch__<branchId>__select` |
| Selector handler | `engine.branch-select` |
| `<when>` arm and selector-edge ID | `<branchId>:when:<zeroBasedIndex>` |
| `<otherwise>` arm and selector-edge ID | `<branchId>:otherwise` |
| Result/join node ID | `<branchId>` |
| Result/join handler | `engine.branch-result` |

The selector's outgoing edges are ordered exactly like the source cases. Each
`<when>` edge carries a strict `boolean` condition and the public `branchId`.
The final `<otherwise>` edge carries `condition.kind = "always"` and the same
`branchId`. Ordinary sequencing and join edges do not carry a branch ID.

The result node inputs are an object keyed by durable arm ID. Each value is the
compiled context-reference expression from that arm's `<result>`. Generated
selector and arm IDs occupy a namespace that user-authored JavaScript-safe IDs
cannot enter.

Optional branch `name` and `description` lower to descriptive metadata on the
selector node. They never affect selection, identity, result publication, or
the definition's event vocabulary.

Source positions, display names, timestamps, and random values never become
compiled or durable branch identities.

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

### 15.4 Frozen v0.1 reference grammar

The frozen language grammar defines exact references with no internal
whitespace:

```text
reference       := "{{" context-path "}}"
context-path    := trigger-path | step-path
trigger-path    := "context.trigger" ("." property-id)*
step-path       := "context.steps." structural-id ("." property-id)*
structural-id   := [a-z][A-Za-z0-9]*
property-id     := [A-Za-z_$][A-Za-z0-9_$]*
```

Bracket access, optional chaining, operators, function calls, and fallbacks are
not WOML references. A referenced step must exist and dominate the consumer in
the lowered DAG. A missing nested property at runtime produces
`WOML_REFERENCE_NOT_AVAILABLE`; it never becomes `undefined` or an empty string.

The published branch profile resolves exact references in `<when test>` and
`<result value>` without passing them through JavaScript. Scripts continue to
read `context.steps.<id>` directly through the injected JavaScript context.
Mixed templates remain in the design catalog; their escaping rules must be
approved before a publishable profile accepts them.

## 16. Static Validation

The WOML frontend MUST reject a document before run creation when any of these
conditions hold.

### 16.1 Structural errors

- The document has no `<workflow>` root or has more than one root.
- Required root children are missing or out of order.
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
- `<branch>` has no `<when>`.
- `<branch>` has no `<otherwise>` in the executable branch profile.
- A `<when>` or `<otherwise>` arm contains no step items.
- `<otherwise>` is duplicated or is not last.
- A branch arm has no `<result>`, has more than one `<result>`, or places
  `<result>` anywhere except last.
- A step contains zero or multiple operations.
- `<approval>` is missing `<when-approved>` or `<when-rejected>`, duplicates
  either arm, or declares them out of order.
- `<approval>` contains more than one `<notify>`, or `<notify>` is empty.
- `<notify>` contains anything other than one or more `<slack>` tags in the
  Slack-first profile.
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
- `retry` is present with any value other than the integer `1`.
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

The executable branch profile freezes these branch-specific diagnostic codes:

| Code | Phase | Meaning and primary location |
|---|---|---|
| `WOML_BRANCH_WHEN_REQUIRED` | validation | `<branch>` contains no `<when>`; points to the branch opening tag. |
| `WOML_BRANCH_OTHERWISE_REQUIRED` | validation | `<branch>` has no fallback; points to the branch opening tag. |
| `WOML_BRANCH_OTHERWISE_ORDER` | validation | `<otherwise>` is duplicated or not last; points to the offending tag. |
| `WOML_BRANCH_RESULT_REQUIRED` | validation | An arm has zero or multiple `<result>` children; points to the arm or duplicate result. |
| `WOML_BRANCH_RESULT_ORDER` | validation | `<result>` is not the final arm child; points to the misplaced result. |
| `WOML_INVALID_REFERENCE` | validation | `test` or `value` is not exactly one frozen WOML context reference; points to the attribute value. |
| `WOML_UNKNOWN_REFERENCE` | compile | A reference names an unknown structural ID; points to that ID inside the attribute. |
| `WOML_REFERENCE_NOT_DOMINATING` | compile | The referenced output is later, in another arm, or otherwise not guaranteed; points to that reference. |
| `WOML_BRANCH_TEST_NOT_BOOLEAN` | runtime | A selected test reference resolved to a non-boolean JSON value; points to the `test` value. |
| `WOML_REFERENCE_NOT_AVAILABLE` | runtime | A compiled reference path has a missing property; points to the consuming `test` or `value`. |
| `WOML_BRANCH_SELECTION_INVALID` | runtime | The compiled/event branch identity is inconsistent; reported at the branch opening tag. |

General diagnostics such as `WOML_DUPLICATE_ID`, `WOML_UNKNOWN_ATTRIBUTE`, and
`WOML_INVALID_DAG` continue to apply and are not renamed for branches.

The CLI renders the primary diagnostic in this stable form:

```text
hello.woml:14:18 [WOML_REFERENCE_NOT_AVAILABLE] Step "missing" is not available here
```

Human formatting may improve without changing the diagnostic object or code.
Webhook input-schema failures use the transport response contract in Section
9.2 because their location is the request payload, not WOML source.

## 18. Remaining Open Design Decisions

Identifier grammar, raw-content termination, and conditional merge results are
closed by Sections 5, 2, and 14. The following items remain open and block only
the executable profiles that depend on them.

### 18.1 Lifecycle execution

Approve the event boundaries, ordering, retry behavior, timeout behavior, and
error recording for lifecycle scripts. In particular, decide whether hook
failure can change an already determined workflow outcome.

### 18.2 Continuing after parallel failure

Define an explicit failed-output or outcome model before adding
`on-error="continue"`. The current grammar accepts only `fail-fast` and
`wait-all`, both of which fail the parallel group when any child fails.

### 18.3 Approval notifications and production hosting

A0 froze the local HTTP, token, store, event, native-outcome, timeout, and
diagnostic contracts in `WOML Human Approval Implementation Plan.md` and
`docs/protocols/approval-*.md`. HTTP is the only public decision mechanism.

N0 freezes Slack notification and secret contracts in
`docs/protocols/notification-contracts-v1.md`; N1 implements the secret
reference primitive and secure secret-management CLI; N2 implements source
validation and Model v5 lowering. N3–N6 complete the durable outbox, built-in
provider host, real Socket Mode integration, recovery, packaging, and
publication hardening. The remaining approval-adjacent work is later product
expansion:

- Discord, shared-provider delivery, WhatsApp, and generic signed webhook
  notifications follow the Slack milestone.
- Remote hosting waits for TLS, reviewer authentication/authorization, and
  deployment ownership.
- Structured reviewer metadata, custom forms, and validated decision payloads
  require a later language version.
- Credential cleanup waits for an explicit retention/audit contract.

None of these changes the frozen two-decision flow or timeout behavior. WOML
will not add a package-level resume function.

## 19. Explicitly Deferred Syntax

The following concepts are not part of the fundamental grammar in this draft:

- Background `<action>` execution.
- Race/first-success concurrency.
- `while`, `for-each`, and batching.
- Subflows.
- Cache and circuit-breaker policies.
- Durable event waiting.
- Pause and cancellation syntax.
- Explicit arbitrary DAG edges.
- External schema files.
- Resolved secrets, direct script access through `secrets.NAME`, or
  `context.env`.
- RAK, packages, `<requires>`, and dynamic capability vocabularies.
- Declarative capability operations.
- Database, HTTP-client, non-approval Slack operations, email, and other
  integration operations.

These features require their own durable execution and context semantics. Their
absence from this grammar does not prevent the compiled workflow model from
supporting future DAG execution features.

## 20. Minimum Executable-Profile Example

The walking-skeleton workflow exercises raw script execution and direct context
threading between two sequential script steps:

```xml
<workflow version="0.1" id="hello" name="Hello WOML">
  <triggers>
    <manual id="start" />
  </triggers>

  <steps>
    <step
      id="a"
      name="Choose greeting name"
      description="Use the trigger name or default to World">
      <script>
        const name = context.trigger.name ?? "World";

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
```

For `woml run hello.woml`, the manual trigger payload is `{}`. The successful
context is:

```json
{
  "trigger": {},
  "steps": {
    "a": {
      "x": "World"
    },
    "b": {
      "message": "Hello World"
    }
  }
}
```

The Rust engine builds this projection by folding run events. Its durable mode
reconstructs the same public shape from persisted events; the context is never
an authoritative mutable object exposed to scripts.

The Phase 0 acceptance contracts are checked in at:

- `woml/tests/fixtures/hello.woml` — canonical source workflow.
- `woml/tests/fixtures/hello.compiled.v1.json` — exact compiled DAG.
- `woml/tests/fixtures/hello.context.v0.1.json` — context before and after each
  node.
- `woml-cli/tests/fixtures/hello.cli.v0.1.json` — exact public process contract.

The successful CLI result is compact JSON followed by one line feed on stdout,
empty stderr, and exit status `0`.
