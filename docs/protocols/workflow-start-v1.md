# WOML Workflow Start v1

Status: frozen and executable on 2026-08-11.

`services.workflows.start()` durably admits exactly one child workflow and lets
the parent continue without waiting for the child's terminal result:

```js
const started = await services.workflows.start('send-invoice', {
  invoiceId: context.payload.invoiceId
});

return { childRunId: started.runId };
```

The `await` confirms durable admission and dispatch. It does not wait for child
completion. The returned public value is:

```ts
{
  workflowId: string;
  runId: string;
  duplicate: boolean;
}
```

The child receives the supplied object as `context.payload`. It remains an
independent run with its own immutable definition binding, event history,
status, result, and failure.

Workflow Start reuses the Workflow Call target registry, atomic call index,
hidden lineage, cycle/depth protection, exact definition pinning, local routing,
lost-wake-up recovery, and stable operation identity. The Capability Call v1
operation is `{ capability: "workflows", operation: "start" }`; its inner
request and response validate against
`docs/schemas/workflow-start.v1.schema.json`.

Repeating the same logical operation reattaches to the admitted child and
returns `duplicate: true`. Reusing its identity with different payload data
fails with the existing idempotency-conflict error. Repeated starts to the same
target in one step require distinct stable `{ name: "..." }` values.

A child failure after successful admission does not retroactively fail the
parent. Operators use the returned run ID with `woml runs get`. An interrupted
started attempt remains governed by WOML's fail-closed recovery policy.

Unlike synchronous `call()`, `start()` can launch a workflow that later waits
for Human Approval because no arbitrary parent JavaScript continuation must be
preserved. Cross-machine routing and cancellation propagation remain outside
the local v1 profile.
