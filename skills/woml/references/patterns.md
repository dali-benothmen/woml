# WOML Automation Patterns

Choose the simplest pattern matching the automation.

## Transform a payload sequentially

Use ordinary steps when each result feeds the next:

```xml
<steps>
  <step id="normalize"><script>
    return { email: context.payload.email.trim().toLowerCase() };
  </script></step>
  <step id="message"><script>
    return { message: `Welcome ${context.steps.normalize.email}` };
  </script></step>
</steps>
```

## Fan out independent checks, then combine

Use `<parallel>` for independent single-step work and a normal downstream step to combine results. Never make parallel siblings depend on each other.

## Decide, then expose one stable result

Use a boolean-producing step followed by `<choose id="...">`. End every arm with `<result>` and make downstream work consume `context.steps.<choiceId>`.

Use `<switch>` instead when routing one string across several exact values. Use `<fork>` when multiple multi-step routes should run rather than selecting only one.

## Human approval

```xml
<approval id="review" name="Review publication" timeout="24h" on-timeout="reject">
  <notify>
    <telegram chats="-1001234567890" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
  </notify>
  <when-approved>
    <step id="publish"><script>
      return { published: true, draftId: context.steps.draft.id };
    </script></step>
  </when-approved>
  <when-rejected>
    <step id="recordRejection"><script>return { published: false };</script></step>
  </when-rejected>
</approval>
```

Approvals are durable control-flow items, not step attributes. Provider-specific configuration must be checked against current documentation. All notification deliveries for one approval share the same first-valid-decision authority.

## Reuse project code

Declare a local module:

```xml
<imports>
  <module name="pricing" from="./modules/pricing.ts" />
</imports>
```

Use named exports:

```ts
export function total(items: { price: number; quantity: number }[]) {
  return items.reduce((sum, item) => sum + item.price * item.quantity, 0);
}
```

Call it from WOML:

```xml
<step id="calculateTotal"><script>
  return { total: services.pricing.total(context.payload.items) };
</script></step>
```

Modules automatically receive `services`, but not `context`, `attempt`, or `secrets`. Pass those values explicitly when required. Use named ESM exports; do not use default exports or CommonJS.

## Split a large automation into workflows

Create a call-only child by omitting `<triggers>`. Load parent and child together:

```bash
woml run workflows/parent.woml workflows/calculate-risk.woml
```

Wait for its result:

```js
const risk = await services.workflows.call(
  "calculate-risk",
  { customerId: context.payload.customerId },
  { name: "calculate-customer-risk" }
);
return risk;
```

Or start it and continue immediately:

```js
return services.workflows.start(
  "send-follow-up",
  { customerId: context.payload.customerId },
  { name: "start-follow-up" }
);
```

Use `.call()` when the parent needs the child's final JSON result. Use `.start()` when the work is independent and the parent only needs admission information.

## Choose the correct data store

- Step output: data needed later in the same run.
- `services.cache`: disposable optimization data where misses are normal.
- `services.state`: small correctness data shared across future runs of one workflow.
- `services.db`: application records, queries, and transactions.
- `services.storage`: files, blobs, or larger durable values.

Never use cache as authoritative business state.

## Agent completion checklist

- IDs match the correct kebab-case or lower-camel grammar and are unique.
- Every reference points backward to a guaranteed available output.
- Conditional/switch routes expose a merged result when downstream work needs one.
- Scripts return JSON-compatible, intentionally small values.
- Credentials are symbolic secret references.
- Effectful retries have stable operation names or provider idempotency.
- The user receives required secret names and exact check/run/trigger commands.
- `woml check` passed, or the reason it could not run is stated explicitly.
