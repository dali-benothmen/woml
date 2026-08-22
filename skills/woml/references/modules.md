# Modules and Reusable Definitions

Use JS/TS modules for reusable functions. Use reusable `.woml` definitions when the imported unit should appear as an executable custom step or notification-provider tag.

## Local JS/TS modules

```xml
<woml>
  <imports>
    <module name="pricing" from="./modules/pricing.ts" />
  </imports>
  <workflow id="calculate-order" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="total"><script>
        return { total: services.pricing.total(context.payload.items) };
      </script></step>
    </steps>
  </workflow>
</woml>
```

Module:

```ts
export function total(items: { price: number; quantity: number }[]) {
  return items.reduce((sum, item) => sum + item.price * item.quantity, 0);
}
```

Rules:

- Use named ESM exports. Default exports and CommonJS are rejected.
- Static relative `.js`/`.ts` imports are allowed inside the module graph.
- Dynamic imports, npm package imports, generators, and module initialization effects are unsupported.
- Module alias names are exposed under `services.<alias>.<export>()`.
- Modules receive only `services` automatically. Pass context, attempt identity, and individual secret values explicitly.
- Each invocation receives fresh module state; do not rely on mutable module globals.
- `woml check` and `woml run` refresh `woml-env.d.ts` automatically.

## Reusable custom step

Definition file:

```xml
<woml>
  <imports>
    <module name="pricing" from="./pricing.ts" />
  </imports>
  <props>
    <prop name="price" required="true" />
    <prop name="discount-token" required="true" secret="true" />
  </props>
  <step name="Calculate discount" description="Apply the customer discount.">
    <script>
      return services.pricing.discount(Number(props.price), props.discountToken);
    </script>
  </step>
  <lifecycle>
    <on-success><script>console.log(lifecycle.result.finalPrice);</script></on-success>
    <on-error><script>console.error(lifecycle.error.code);</script></on-error>
    <on-complete><script>console.log(lifecycle.outcome);</script></on-complete>
  </lifecycle>
</woml>
```

Workflow import and invocation:

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
  <step id="finish"><script>return context.steps.discount;</script></step>
</steps>
```

Prop names are kebab-case in source and camel-case on `props`. Context props must be available at the invocation position. Secret props accept only exact secret references. The invocation requires a normal step `id`, accepts retry and display metadata, and publishes its result under that ID.

## Custom notification provider

Definition:

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
        url: "https://provider.example/messages",
        headers: { authorization: `Bearer ${props.botToken}` },
        json: {
          chatId: props.chatId,
          text: notification.message,
          actions: notification.actions
        },
        idempotency: {
          header: "Idempotency-Key",
          value: notification.idempotencyKey
        }
      }, { name: "deliver-notification" });
      return { messageId: String(response.data.id) };
    </script>
  </provider>
</woml>
```

Use it:

```xml
<imports><module name="company-chat" from="./company-chat.woml" /></imports>

<approval id="review">
  <notify>
    <company-chat bot-token="{{secrets.COMPANY_CHAT_TOKEN}}" chat-id="approvals" />
  </notify>
  <when-approved />
  <when-rejected />
</approval>
```

The provider transports `notification.message` and `notification.actions`; WOML owns durable delivery, approval capability tokens, retries, and the first-valid-decision result. A successful provider returns a compact JSON result such as `{ messageId }`.

Only `kind="notification"` is released. Trigger providers, arbitrary structural tags, child content inside custom tags, package imports, and remote component URLs are unsupported.

## Reusable lifecycle

Reusable step/provider definitions may declare only:

- `<on-success>`;
- `<on-error>`; and
- `<on-complete>`.

Each hook is script-only and optional. Hooks are observational; failures become warnings and cannot change the already committed invocation/delivery outcome.

## Project behavior and recovery

- A folder passed to `woml check` or `woml run` loads direct `.woml` files non-recursively.
- Only workflow documents activate; directly running a reusable definition is an error.
- WOML packages exact module bundles, source maps, reusable definitions, and symbolic secret names with the compiled workflow.
- Recovery uses stored immutable artifacts rather than reading modified/deleted source files.
- `woml types` is optional because check/run already create editor declarations.

## When not to create a module

Keep short one-use logic in its `<script>`. Add a module when logic is reused, deserves isolated unit tests, or would obscure workflow structure. Split into another workflow when the unit should have its own run identity, retries, history, lifecycle, and operational ownership.
