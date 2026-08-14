# Reusable WOML Steps and Notification Providers

Reusable definitions let a project give a local `.woml` file a custom tag and
use that tag from ordinary workflows. They are source components, not running
workflows: only documents containing `<workflow>` activate triggers or own runs.

## Reusable custom step

Declare props above one top-level step:

```xml
<woml>
  <imports>
    <module name="pricing" from="./pricing.ts" />
  </imports>
  <props>
    <prop name="price" required="true" />
    <prop name="discount-token" required="true" secret="true" />
  </props>
  <step name="Calculate discount">
    <script>
      return services.pricing.discount(
        Number(props.price),
        props.discountToken
      );
    </script>
  </step>
  <lifecycle>
    <on-success>
      <script>console.log(`Discount: ${lifecycle.result.finalPrice}`);</script>
    </on-success>
    <on-error>
      <script>console.error(lifecycle.error.code);</script>
    </on-error>
    <on-complete>
      <script>console.log(lifecycle.outcome);</script>
    </on-complete>
  </lifecycle>
</woml>
```

Import it into a workflow and use its import name as the tag:

```xml
<imports>
  <module name="calculate-discount" from="./calculate-discount.woml" />
</imports>

<steps>
  <calculate-discount
    id="discount"
    price="{{context.payload.price}}"
    discount-token="{{secrets.DISCOUNT_TOKEN}}"
    retry="3"
  />
  <step id="finish"><script>
    return { total: context.steps.discount.finalPrice };
  </script></step>
</steps>
```

Every invocation has its own normal step ID, retry identity, result, lifecycle,
and inspection entry. Context props must already be visible at the usage
position; props never create hidden DAG edges.

## Custom notification provider

A notification provider receives explicit props plus a bounded `notification`
object. Rust owns durable delivery intent, retries, approval capabilities, and
the final delivery outcome; the provider script only transports the message.

```xml
<woml>
  <props>
    <prop name="bot-token" required="true" secret="true" />
    <prop name="chat-id" required="true" />
  </props>
  <provider kind="notification">
    <script>
      const response = await services.http.request({
        method: "POST",
        url: `https://api.example.test/bot/${props.botToken}/messages`,
        json: {
          chatId: props.chatId,
          text: notification.message,
          actions: notification.actions
        },
        idempotencyKey: notification.idempotencyKey
      });
      return { messageId: String(response.data.id) };
    </script>
  </provider>
  <lifecycle>
    <on-success>
      <script>console.log(`Delivered ${lifecycle.result.messageId}`);</script>
    </on-success>
    <on-error>
      <script>console.error(lifecycle.error.code);</script>
    </on-error>
    <on-complete>
      <script>console.log(lifecycle.outcome);</script>
    </on-complete>
  </lifecycle>
</woml>
```

Use the imported provider directly under `<notify>` in an approval or workflow
lifecycle hook. Multiple providers may share one approval; each transports a
different single-use capability for the same durable decision, and the first
valid decision wins.

Provider lifecycle scripts receive `props`, `lifecycle`, `services`, `fetch`,
and a redacting console. They do not receive workflow context or arbitrary
secrets. `on-success` or `on-error` runs first, followed by `on-complete`.
Lifecycle failures are warnings and never rewrite an already committed
delivery result. Reusable lifecycle hooks are script-only in v1; place custom
providers in a workflow lifecycle `<notify>` when a lifecycle must send a
notification.

## Switch

Use `<switch>` for exact, case-sensitive string routing. It has no JavaScript
fallthrough. An ID-less switch controls flow only; an ID-bearing switch
requires one `<result>` in every case and publishes a path-stable result.

```xml
<switch id="destination" value="{{context.payload.channel}}">
  <case value="email">
    <step id="email"><script>return { provider: "email" };</script></step>
    <result value="{{context.steps.email}}" />
  </case>
  <default>
    <step id="fallback"><script>return { provider: "slack" };</script></step>
    <result value="{{context.steps.fallback}}" />
  </default>
</switch>
```

## Commands and project behavior

```bash
woml check workflows/order.woml
woml run workflows/
woml get run_... --state .woml/state.sqlite --json
```

`woml check` validates and packages the complete dependency graph without
executing it. `woml run <folder>` validates every direct `.woml` document but
activates only workflows. One exact source snapshot is pinned atomically;
recovery uses stored Model v14 and Definition Package v9 artifacts rather than
reopening current files. Directly running a definition file is an error.

Editor data and `woml-env.d.ts` are refreshed automatically by `check` and
`run`. The custom tag exposes its declared props; imported JS/TS modules remain
under `services.<name>`.

## Security and operational limits

- Secret props must use an exact `{{secrets.NAME}}` reference.
- Only the secrets declared by an invocation cross its worker boundary.
- Props, provider messages, approval URLs, and secret values are excluded from
  events, inspection, and normal logs.
- Returning or intentionally transmitting a secret is still an
  author-controlled effect; WOML cannot infer business sensitivity.
- Local JavaScript is isolated per invocation, but Bun is not advertised as a
  hostile multi-tenant sandbox.
- Custom tags cannot contain children or generate hidden workflow structure.
- Provider `kind="trigger"`, package imports, arbitrary structural custom tags,
  and remote component URLs remain unsupported.

The publication gate is `bun run test:reusable-release`; the independent clean
package gate is `bun run test:reusable-package`, and performance measurements
are available through `bun run benchmark:reusable-definitions`.
