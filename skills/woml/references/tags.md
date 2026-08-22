# WOML Tag Reference

This is the complete released authoring vocabulary. Do not invent aliases for element or attribute names.

## Document profiles

Every file has exactly one `<woml>` root with no attributes. It contains optional `<imports>` followed by exactly one of:

- a runnable `<workflow>`;
- a reusable top-level `<step>`; or
- a reusable `<provider kind="notification">`.

Reusable definitions may also have top-level `<props>` before the definition and a restricted `<lifecycle>` after it. They are imported components, not runnable workflows.

```xml
<woml>
  <workflow id="hello-world" name="Hello World" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="greet"><script>return { message: "Hello World" };</script></step>
    </steps>
  </workflow>
</woml>
```

## Identity and common metadata

- Workflow IDs: lowercase kebab-case, `[a-z][a-z0-9]*(?:-[a-z0-9]+)*`.
- Trigger and structural IDs: lower camel case, `[a-z][A-Za-z0-9]*`.
- Step, parallel, choice, switch, fork, and approval identities must not collide in the workflow-wide structural namespace.
- `name` and `description` are optional attributes, never child tags.
- IDs are durable identities. Do not derive them from source lines, array positions, timestamps, or display names.

## Root and imports

### `<woml>`

Required document root. No attributes.

### `<imports>`

Optional direct child of `<woml>`. Must contain one or more `<module>` children and precede the workflow or reusable definition.

### `<module>`

```xml
<module name="pricing" from="./modules/pricing.ts" />
<module name="calculate-discount" from="./calculate-discount.woml" />
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `name` | Yes | Unique import alias. JS/TS modules become `services.<name>`; reusable WOML definitions become custom tags named from this alias. |
| `from` | Yes | Static relative path to `.js`, `.ts`, or `.woml`. |

The tag is empty. Remote URLs, npm package imports, and dynamic paths are not supported.

## Workflow containers

### `<workflow>`

```xml
<workflow id="daily-report" name="Daily Report" description="Build and publish the report." version="1.2.0">
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Stable workflow ID. |
| `name` | No | Terminal/editor display name. |
| `description` | No | Human-readable purpose. |
| `version` | No | User-owned workflow version; it does not select WOML grammar. |

Contains optional singleton `<config>`, `<lifecycle>`, and `<triggers>`, plus exactly one `<steps>`. These containers may appear in any order. Prefer config, lifecycle, triggers, steps for readability.

### `<config>`

```xml
<config concurrency="4" timeout="10m" rate-limit="100/1m" queue="reports" />
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `concurrency` | No | Positive maximum concurrently active runs. |
| `timeout` | No | Complete-run duration using `ms`, `s`, `m`, `h`, or `d`. |
| `rate-limit` | No | Positive count and duration, such as `100/1m`. |
| `queue` | No | Non-empty logical FIFO admission lane name. |

At least one attribute is required. `<config>` is empty and contains no hooks.

### `<triggers>`

Contains one or more trigger tags. Omit the entire container for a call-only workflow. `<triggers />` is invalid.

### `<steps>`

Contains one or more flow items in source order: `<step>`, `<parallel>`, `<for-each>`, `<choose>`, `<switch>`, `<fork>`, or `<approval>`. Ordinary adjacent items form sequential dependencies.

## Trigger tags

### `<manual>`

```xml
<manual id="start" />
```

Requires only `id`. The active CLI accepts keyboard input and exposes it as `context.payload`.

### `<webhook>` and `<schema>`

```xml
<webhook id="newOrder" path="/webhooks/orders" method="POST" auth="bearer" secret="{{secrets.ORDER_WEBHOOK_TOKEN}}">
  <schema>
    { "type": "object", "required": ["orderId"] }
  </schema>
</webhook>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Trigger ID. |
| `path` | Yes | Static absolute path; no wildcards, parameters, repeated slashes, or `/_woml`. |
| `method` | No | Defaults to `POST`; the released runtime executes POST. |
| `auth` | Yes | `bearer` or `none`. |
| `secret` | With bearer | Exact `{{secrets.NAME}}`; forbidden with `auth="none"`. |

The optional singleton `<schema>` contains raw JSON Schema Draft 2020-12. Invalid payloads receive HTTP 400 and create no run.

### `<schedule>`

```xml
<schedule id="daily" cron="0 8 * * *" timezone="Europe/Berlin" on-missed="run-once" />
```

Requires `id` and five-field numeric `cron`. `timezone` defaults to `UTC` and must be a canonical IANA zone. `on-missed` is `skip` or `run-once`, defaulting to `skip`.

### `<interval>`

```xml
<interval id="refresh" every="5m" on-missed="skip" />
```

Requires `id` and positive duration `every`. Optional `on-missed` is `skip` or `run-once`.

### `<event>`

```xml
<event id="orderCreated" name="order.created" secret="{{secrets.EVENT_CONTROL_TOKEN}}">
  <schema>{ "type": "object", "required": ["orderId"] }</schema>
</event>
```

Requires `id` and event `name`. Optional `secret` enables authenticated HTTP publication; omit it for internal-only `services.events.emit()` subscribers. An optional `<schema>` uses the webhook schema rules.

### Communication triggers

```xml
<slack id="agentMessage" events="app-mention,direct-message" channels="woml-testing" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
<telegram id="agentMessage" events="message" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
<discord id="agentMessage" events="app-mention,direct-message" channels="200000000000000001" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />
<whatsapp id="customerMessage" events="message" phone-number-id="123456789012345" verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}" app-secret="{{secrets.WHATSAPP_APP_SECRET}}" />
```

All require `id`, `events`, and their shown credential attributes. Slack `channels` is optional names/IDs; Discord `channels` is optional comma-separated numeric channel IDs. Read [providers.md](providers.md) before generating provider workflows.

## Executable and control-flow tags

### `<step>` and `<script>`

```xml
<step id="loadCustomer" name="Load customer" description="Read the customer record." retry="3" retry-backoff="exponential" retry-delay="1s" retry-max-delay="30s">
  <script>
    return await services.http.request({ url: context.payload.url });
  </script>
</step>
```

`<step>` requires `id`; optional attributes are `name`, `description`, `retry` (1–10), `retry-backoff` (`fixed` or `exponential`), `retry-delay`, and `retry-max-delay`. Backoff attributes require `retry > 1`; fixed backoff forbids `retry-max-delay`.

A fundamental step contains exactly one `<script>`. Script has no attributes and contains an asynchronous JavaScript function body. Write JavaScript directly—never CDATA and never a function wrapper.

### `<parallel>`

```xml
<parallel id="checks" name="Run checks" concurrency="2" on-error="wait-all">
  <step id="inventory"><script>return { available: context.payload.inStock };</script></step>
  <step id="risk"><script>return { approved: context.payload.riskScore < 70 };</script></step>
</parallel>
```

Requires `id` and one or more direct `<step>` children. Optional `name`, `description`, `concurrency`, and `on-error="fail-fast|wait-all"`. Children share the pre-fork context and cannot reference siblings. The parallel ID does not create an output; each child does.

### `<for-each>`

```xml
<for-each id="processItems" items="{{context.steps.load.items}}" concurrency="4">
  <step id="transformItem"><script>
    return { value: context.item, index: context.iteration.index };
  </script></step>
  <result value="{{context.steps.transformItem}}" />
</for-each>
```

| Attribute | Required | Meaning |
| --- | :---: | --- |
| `id` | Yes | Stable loop identity and aggregate output key. |
| `items` | Yes | One exact `context.payload` or visible `context.steps` reference resolving to an array. |
| `name`, `description` | No | Terminal and inspection metadata. |
| `concurrency` | No | Active iterations from 1 through 64; defaults to 1. |

The body contains one or more supported flow items and an optional final
`<result>`. Each iteration receives `context.item` and
`context.iteration = { index, total }`; body step outputs are local to that
iteration. A final result publishes ordered aggregate output at
`context.steps.<forEachId>` as `{ total, succeeded, results }`. Empty arrays
succeed immediately. One failed item stops new admissions, settles owned work,
and fails the loop. Nested `<for-each>`, `<fork>`, and `<approval>` are not
supported in the first loop profile.

### `<choose>`, `<when>`, `<otherwise>`, and `<result>`

```xml
<choose id="decision" name="Choose decision">
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

`<choose>` has one or more ordered `<when>` children and exactly one final `<otherwise>`. A result-producing choice requires `id`, optionally accepts `name` and `description`, and every arm ends with `<result value="{{context...}}" />`. It publishes the selected JSON value at `context.steps.<chooseId>`.

`test` must be one exact reference resolving to a JSON boolean. No coercion or JavaScript expressions. An ID-less control-only choose omits results and creates no merged output.

### `<switch>`, `<case>`, and `<default>`

```xml
<switch id="delivery" value="{{context.steps.order.provider}}">
  <case value="express">
    <step id="express"><script>return { days: 1 };</script></step>
    <result value="{{context.steps.express}}" />
  </case>
  <default>
    <step id="standard"><script>return { days: 4 };</script></step>
    <result value="{{context.steps.standard}}" />
  </default>
</switch>
```

`value` is one exact reference resolving to a string. One or more unique, non-empty, case-sensitive `<case value="...">` arms precede exactly one `<default>`. There is no fallthrough. With `id`, optional `name`/`description` are allowed and every arm must publish `<result>` to `context.steps.<switchId>`. Without `id`, omit results.

### `<fork>` and `<branch>`

```xml
<fork id="distribution" join="instagram facebook">
  <branch id="instagram">
    <step id="publishInstagram"><script>return { published: true };</script></step>
  </branch>
  <branch id="facebook">
    <step id="publishFacebook"><script>return { published: true };</script></step>
  </branch>
</fork>
```

`<fork>` requires `id`; optional `join` is omitted/`all`, `none`, or a whitespace-separated set of branch IDs. It contains one or more non-empty `<branch id="...">` children. A branch may contain multiple steps and supported control-flow items. Branches execute independently and cannot read sibling outputs. Only outputs from guaranteed joined branches are visible after the fork. Nested forks are not released.

### `<approval>`, `<notify>`, `<when-approved>`, and `<when-rejected>`

```xml
<approval id="review" name="Review content" timeout="24h" on-timeout="reject">
  <notify>
    <telegram chats="-1001234567890" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
  </notify>
  <when-approved>
    <step id="publish"><script>return { published: true };</script></step>
  </when-approved>
  <when-rejected>
    <step id="recordRejection"><script>return { published: false };</script></step>
  </when-rejected>
</approval>
```

`<approval>` requires `id`; optional `name`, `description`, `timeout`, and `on-timeout="reject|fail"` (default `fail`). `on-timeout` requires `timeout`. Child order is optional `<notify>`, required `<when-approved>`, required `<when-rejected>`. Decision arms may be empty or contain supported flow items. The durable decision becomes `context.steps.<approvalId>`.

`<notify>` contains one or more built-in or imported custom notification tags. Provider attributes differ between approvals and lifecycle notifications; read [providers.md](providers.md).

## Workflow lifecycle tags

```xml
<lifecycle>
  <on-start><script>console.log("Started");</script></on-start>
  <on-step-failure steps="chargeCustomer"><script>console.error(lifecycle.failure.code);</script></on-step-failure>
  <on-success><script>console.log("Succeeded");</script></on-success>
  <on-error><script>console.error("Failed");</script></on-error>
  <on-cancel><script>console.warn("Cancelled");</script></on-cancel>
  <on-complete><script>console.log(lifecycle.workflow.outcome);</script></on-complete>
</lifecycle>
```

The optional workflow `<lifecycle>` accepts each hook at most once, in any source order:

- `<on-start>`
- `<on-step-start steps="...">`
- `<on-step-success steps="...">`
- `<on-step-failure steps="...">`
- `<on-step-complete steps="...">`
- `<on-success>`
- `<on-error>`
- `<on-cancel>`
- `<on-complete>`

Each hook contains one or more `<script>` or `<notify>` actions. Only step hooks accept optional whitespace-separated `steps`. Hooks are observational and cannot rewrite the business result.

## Reusable definition tags

### `<props>` and `<prop>`

```xml
<props>
  <prop name="customer-id" required="true" />
  <prop name="api-token" required="true" secret="true" />
</props>
```

`<props>` contains one or more `<prop>`. A prop requires kebab-case `name`; optional `required` and `secret` are booleans. JavaScript receives camel-case properties such as `props.customerId`. Secret props must be supplied by exact secret reference.

### Reusable top-level `<step>`

```xml
<woml>
  <props><prop name="value" required="true" /></props>
  <step name="Double value"><script>return { value: Number(props.value) * 2 };</script></step>
</woml>
```

The definition step has optional `name` and `description`, no `id`, and exactly one script. Import it with `<module name="double-value" ... />`, then invoke `<double-value id="double" value="{{context.payload.value}}" retry="3" />`. The invocation behaves as a normal step and publishes `context.steps.double`.

### `<provider kind="notification">`

```xml
<woml>
  <props>
    <prop name="endpoint" required="true" />
    <prop name="token" required="true" secret="true" />
  </props>
  <provider kind="notification"><script>
    const response = await services.http.request({
      method: "POST",
      url: props.endpoint,
      headers: { authorization: `Bearer ${props.token}` },
      json: { text: notification.message, actions: notification.actions }
    }, { name: "deliver-notification" });
    return { messageId: String(response.data.id) };
  </script></provider>
</woml>
```

Only `kind="notification"` is released. Imported provider tags may appear only under `<notify>`. They receive `props` and `notification`; custom structural or trigger providers are not released.

Reusable step/provider lifecycle accepts only `<on-success>`, `<on-error>`, and `<on-complete>`, each containing scripts. It is separate from workflow lifecycle.

## Reference values

- Exact `{{context.payload...}}` and `{{context.steps.<id>...}}` references preserve JSON type.
- Mixed literal/reference templates produce strings only where that attribute explicitly permits templates, such as notification `message`.
- Exact secret references use `{{secrets.UPPERCASE_NAME}}` only in supported credential/secret-prop attributes.
- Missing paths, forward references, branch-unsafe references, bracket syntax, optional chaining, operators, calls, and fallback expressions are invalid declarative references.
