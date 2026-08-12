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
| `ctx.payload` | `context.payload` |
| Implicit `ctx.last` | Explicit `context.steps.<stepId>` |
| `.if()/.else()/.endIf()` | `<branch>`, `<when>`, and `<otherwise>` |
| Parallel chaining | `<parallel>` with child workflow items |
| Retry option/object | `retry` and backoff attributes on `<step>` |
| Human-in-the-loop callback | `<approval>` with a durable HTTP/provider decision |
| Native SDK/Bun Fetch | Native `fetch()` inside `<script>` with durable redacted observation |
| Supervised HTTP helper | `services.http.request()` inside `<script>` |
| SDK database service | `services.db()` with a SQLite or PostgreSQL driver |
| Durable files/large values | `services.storage` and portable object references |
| Temporary reusable values | `services.cache` with an explicit TTL |
| Small durable workflow-owned memory | `services.state` with named mutations and versions |
| Internal workflow fan-out | `services.events.emit()` plus an `<event>` trigger |
| Reusable local JavaScript helper package | `<imports><module name="..." from="..." /></imports>` and `services.<name>` |
| Call one workflow and await its answer | `services.workflows.call(workflowId, payload)` |
| Start one workflow and continue | `services.workflows.start(workflowId, payload)` |
| SDK workflow callbacks/hooks | Workflow-owned `<lifecycle>` hooks |
| SDK run cancellation | `woml cancel <runId>` against durable local state |

WOML makes every downstream dependency explicit. Give each step a stable ID and
replace positional or “last result” access with the producing step's path:

```xml
<step id="prepare">
  <script>
    return { name: context.payload.name ?? "World" };
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
4. Replace `ctx.payload` with `context.payload` and every implicit previous
   result with `context.steps.<id>`.
5. Express branches and parallel work structurally, then verify every referenced
   step is reachable in the DAG.
6. Move retry configuration to `<step retry="...">` attributes. Use
   `attempt.idempotencyKey` for external APIs that support duplicate handling.
7. Run the workflow with an explicit state path and test the printed recovery
   command before deploying it.
8. Replace SDK webhook/schedule/provider registration with the corresponding
   WOML trigger tag. Keep credentials as `{{secrets.NAME}}` references and
   configure their values through `woml secrets set NAME`.
9. Move reusable local JavaScript/TypeScript functions into a named WOML module
   and call them through `services.<alias>`.
10. Split reusable durable workflow work into a call-only workflow and activate
    it with its parent, then use `services.workflows.call()` when the parent
    needs its terminal JSON result.
11. Move observational logging, notifications, metrics, and best-effort cleanup
    into `<lifecycle>`. Keep business-critical work as explicit steps.
12. Replace custom local run administration with `woml list`, `woml get`, and
    `woml cancel`; use `--json` for the versioned redacted contracts.
13. Replace SDK process-local concurrency/rate wrappers with one workflow-level
    `<config concurrency="..." rate-limit="..." timeout="..." queue="..." />`.
    Test burst admission and restart against the same explicit state path.
14. Replace process memory or SDK-local key/value maps with `services.state`
    only when the data is small and owned by one workflow ID. Give every
    mutation a stable name and use `ifVersion` for read/modify/write races.

## Current parity boundary

Sequential scripts, branch, parallel, Human Approval, Slack approval
notifications, secrets, durable retry, and manual, webhook, Slack, schedule,
interval, and named-event triggers are available through `woml run`.

Native Fetch; the Rust-managed HTTP, database, storage, cache, and internal
event services; local JavaScript/TypeScript modules; durable local Workflow
Calls; workflow and step lifecycle hooks; informational Slack lifecycle
notifications; durable local cancellation; and workflow-level Runtime Policies
are available. Durable User State provides small, workflow-scoped, versioned
JSON memory across runs. The separate `services.queue` capability, package modules,
additional messaging services, cross-machine workflow routing, remote run
control, and the hosted production runtime remain roadmap items. Keep an SDK
workflow in place when it depends on those unavailable capabilities. The SDK is
not retired merely because local lifecycle and control parity exists.

See [Lifecycle and Local Run Control](woml-lifecycle-and-run-control.md) for
hook ordering, warning semantics, cancellation races, and deployment guidance.
Workflow-level concurrency, rate limiting, durable scheduling lanes, and total
deadlines are documented in [WOML Runtime Policies](woml-runtime-policies.md).
Use [Choosing Where Workflow Data Lives](woml-data-guide.md) before migrating
SDK persistence into cache, state, storage, or a database.
