# Migrating from the JavaScript SDK to WOML

WOML replaces workflow construction through JavaScript chaining with a markup
document. JavaScript remains available where it is useful—inside `<script>`—but
the workflow structure becomes readable data that the frontend can validate and
the Rust core can execute durably.

The JavaScript SDK remains in the repository during the migration. Do not mix
SDK and WOML definitions in one run. The SDK is retired only after WOML reaches
sufficient parity and the relevant production features have migration paths.

## Concept mapping

| JavaScript SDK concept | WOML equivalent |
|---|---|
| `cronflow.define({...})` | `<workflow id="..." name="..." version="...">` |
| Trigger registration | A tag inside `<triggers>` |
| Chained `.step(...)` | `<steps><step id="..."><script>...</script></step></steps>` |
| `ctx.payload` | `context.trigger` |
| Implicit `ctx.last` | Explicit `context.steps.<stepId>` |
| `.if()/.else()/.endIf()` | `<branch>`, `<when>`, and `<otherwise>` |
| Parallel chaining | `<parallel>` with child workflow items |
| Retry option/object | `retry` and backoff attributes on `<step>` |
| Human-in-the-loop callback | `<approval>` with a durable HTTP/provider decision |

WOML makes every downstream dependency explicit. Give each step a stable ID and
replace positional or “last result” access with the producing step's path:

```xml
<step id="prepare">
  <script>
    return { name: context.trigger.name ?? "World" };
  </script>
</step>

<step id="greet" retry="3">
  <script>
    return {
      message: `Hello ${context.steps.prepare.name}`,
      attempt: attempt.number
    };
  </script>
</step>
```

## Migration checklist

1. Move workflow metadata to `<workflow>` attributes and triggers beneath
   `<triggers>`.
2. Convert each chained operation to a stable `<step id="...">`.
3. Put custom JavaScript directly inside `<script>` without CDATA or a wrapper
   function.
4. Replace `ctx.payload` with `context.trigger` and every implicit previous
   result with `context.steps.<id>`.
5. Express branches and parallel work structurally, then verify every referenced
   step is reachable in the DAG.
6. Move retry configuration to `<step retry="...">` attributes. Use
   `attempt.idempotencyKey` for external APIs that support duplicate handling.
7. Run the workflow with an explicit state path and test the printed recovery
   command before deploying it.

## Current parity boundary

Sequential scripts, branch, parallel, Human Approval, Slack approval
notifications, secrets, durable retry, and production webhooks are available
through `woml run`. Schedule, interval, event, and Slack triggers, general
HTTP/database services, lifecycle controls, and the hosted production runtime
remain roadmap items. Keep an SDK workflow in place when it depends on one of
those unavailable features.
