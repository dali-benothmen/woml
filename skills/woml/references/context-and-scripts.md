# Context and Script Bindings

Read this reference whenever a workflow contains JavaScript or declarative references.

## Ordinary step scripts

`<script>` is the body of an asynchronous JavaScript function:

```xml
<step id="greet">
  <script>
    const name = context.payload.name ?? "World";
    return { message: `Hello ${name}` };
  </script>
</step>
```

Use ordinary statements, `await`, loops, conditions, exceptions, imports supported by Bun, and `return`. Do not add CDATA, `<script>` attributes, a function wrapper, or TypeScript syntax inside the tag.

## `context`

The deeply read-only public context is:

```js
context.payload
context.steps.<stepOrResultId>
```

- `context.payload` is the input from any trigger, workflow call, or workflow start.
- `context.steps.<id>` appears only after that executable/result-producing item succeeds.
- New source must not use deprecated `context.trigger`.
- `context.run` is not public and must not be generated.
- Mutating `context` does not persist. Return a value instead.

The current script sees only outputs allowed by the compiled DAG. Do not read a later step, a parallel sibling, an unselected choice route, or an unjoined fork branch.

## Step output

A successful script return must be JSON-compatible:

```js
return null;
return true;
return { customerId: "customer-42", total: 144 };
return ["a", "b"];
```

Do not return `undefined`, functions, symbols, `BigInt`, class/client instances, circular objects, or non-finite numbers. Return only what downstream work needs because successful output becomes durable workflow context.

## `attempt`

Every normal/reusable step script receives:

```js
attempt.number          // one-based current attempt
attempt.maxAttempts     // maximum total attempts
attempt.idempotencyKey  // stable across retries of this logical step
```

Use `attempt.idempotencyKey` with external APIs that accept an idempotency key. Do not build operation identity from `attempt.number`.

## `services`

`services` is a deeply read-only namespace containing built-ins and imported JS/TS module aliases. Read [services.md](services.md) before calling a built-in and [modules.md](modules.md) before importing project code.

Managed service clients are invocation resources. Never return a client into context.

## `secrets`

Inside scripts, access only literal statically discoverable names:

```js
secrets.PAYMENTS_API_TOKEN
```

The frontend exposes only names referenced by the workflow. Do not enumerate `secrets`, use computed property access, return a secret, put it in a URL query, log it, or interpolate it into notification text.

In supported attributes, use exact symbolic syntax:

```xml
bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
```

Literal credentials and mixed secret templates are invalid.

## Native `fetch()`

Bun's standard Fetch API is available. It returns a normal `Response`, supports familiar request/streaming behavior, and does not throw only because a response is non-2xx. WOML observes the operation with redacted durable metadata. Prefer `services.http.request()` when managed parsing, limits, status policy, timeout, storage streaming, or stable operation naming is useful.

## Workflow lifecycle binding

Lifecycle scripts receive normal `context`, `attempt`, `services`, and `secrets`, plus:

```js
lifecycle.event
lifecycle.workflow.id
lifecycle.workflow.outcome       // after outcome: succeeded | failed | cancelled
lifecycle.step.id                 // step hooks only
lifecycle.step.outcome            // when settled
lifecycle.step.attempts
lifecycle.failure.code            // failure hooks
lifecycle.failure.message
```

Lifecycle event values are engine-normalized (`run_start`, `step_start`, `step_success`, `step_failure`, `step_complete`, `run_success`, `run_failure`, `run_cancel`, `run_complete`). Lifecycle actions are observational: returned values do not enter `context.steps`, failures become warnings, and hooks cannot change a run outcome.

## Reusable-definition bindings

Reusable custom step scripts receive:

- `props` containing declared invocation props in camel case;
- `attempt`;
- `services`, including imported modules;
- `fetch()`; and
- only secrets passed through secret props.

Reusable provider scripts additionally receive `notification`, containing the bounded message, stable idempotency identity, and approval actions when applicable.

Reusable lifecycle scripts receive `props`, `lifecycle`, `services`, and `fetch()`. Their lifecycle shape is invocation-owned:

```js
lifecycle.hook       // on-success | on-error | on-complete
lifecycle.outcome    // succeeded | failed | cancelled
lifecycle.result     // successful result when available
lifecycle.error.code
lifecycle.error.message
```

Provider lifecycle scripts do not receive workflow `context` or arbitrary secrets.

## Local JS/TS module bindings

A module file receives `services` automatically. It does not receive `context`, `attempt`, or `secrets`. Pass required values explicitly:

```xml
<script>
  return services.crm.findCustomer(
    context.payload.customerId,
    secrets.CRM_TOKEN,
    attempt.idempotencyKey
  );
</script>
```

## Declarative references

Exact references contain no internal whitespace:

```xml
{{context.payload.customerId}}
{{context.steps.loadCustomer.name}}
```

They preserve JSON type. A `<when test>` must resolve to a boolean; `<switch value>` must resolve to a string.

The following are not declarative WOML expressions:

```text
{{ context.payload.id }}
{{context.steps.a?.value}}
{{context.steps.a.value > 10}}
{{context.steps["a"]}}
```

Compute complex logic in a script step and reference its returned value.

## Script safety checklist

- Return bounded JSON, not entire HTTP responses or database clients.
- Use parameterized SQL and provider-supported idempotency.
- Keep correctness-critical effects in normal steps, not lifecycle hooks.
- Do not assume a retry proves an interrupted external effect did not happen.
- Do not log secrets or sensitive payloads.
- Use stable managed-operation names when one step can make multiple logical writes/calls.
