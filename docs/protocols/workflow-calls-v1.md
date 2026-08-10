# WOML Workflow Calls v1

Status: frozen by WC0 on 2026-08-10. Execution begins in WC2; WC1 implements
only source validation, Model v10 lowering, packaging, diagnostics, and editor
types.

## Author contract

```js
const result = await services.workflows.call(
  'calculate-risk',
  { customerId: context.trigger.customerId },
  { name: 'customer-risk', timeout: '30s' }
);
```

The first argument is one activated workflow ID. The second is a top-level JSON
object that becomes the complete child `context.trigger`. The optional `name`
participates in stable operation identity; `timeout` is normalized to
`timeoutMs` before crossing the managed capability boundary.

The promise resolves directly to the child's final JSON result. JSON `null` is
valid. Missing or JavaScript `undefined` is not a result.

## Call-only source shape

Omitting `<triggers>` declares a call-only workflow:

```xml
<woml>
  <workflow id="calculate-risk">
    <steps>...</steps>
  </workflow>
</woml>
```

`woml run` will eventually activate this definition without starting it. An
empty `<triggers>` container is invalid, and a call-only v1 target cannot
contain Human Approval because an arbitrary Bun continuation cannot be durably
serialized across a long pause.

Compiled Workflow Model v10 represents call-only activation with exactly
`triggers: []`. It may retain unchanged Module Runtime v1 bindings. Models
v1-v9 remain immutable.

## Existing hot-path and event contracts

Workflow Calls use unchanged Capability Call v1 with:

```text
capability = workflows
operation  = call
```

They use unchanged Run Event v8 managed-operation events. Safe metadata may
contain target workflow ID, target definition hash, child run ID, payload
digest, and lineage depth. It never contains payload or result values.

The child's existing `run_started` event stores the payload as its trigger. The
child's `run_succeeded.result` or `run_failed` event is the terminal authority.
No workflow-call-specific Run Event v9 is introduced.

## Identity and recovery

The call key is derived from the parent step idempotency key, target workflow
ID, and automatic or explicit operation name. One accepted key binds:

- one payload digest;
- one target definition hash; and
- one child run ID.

A duplicate with the same inputs reattaches to the child. A duplicate with a
different payload or target fails with
`WOML_WORKFLOW_CALL_IDEMPOTENCY_CONFLICT`. A route retry after admission never
creates another child.

The durable call index is a transactional uniqueness/routing index. Parent
operation history and child run history remain authoritative and the index
never stores a competing result.

## Routing

Same-runtime routing is a direct Rust registry lookup. Local cross-process
routing uses a project/state-scoped ownership lease and an authenticated
loopback wake-up message for an already admitted child. WOML creates the
session credential automatically; it is not a workflow secret or public API.

Exactly one live owner may register a workflow ID. The selected child is pinned
to the owner's exact definition hash. Cross-machine routing and tenant service
identity are deferred to Production Runtime.

## Limits

- Request payload: 1 MiB.
- Result: 4 MiB.
- Hidden call lineage: 32.
- In-flight calls per invocation: 32 through Capability Call v1.
- Wait timeout: at most 24 hours and never beyond the calling step's remaining
  timeout.

Parent timeout does not silently cancel an admitted independent child. Public
cancellation and long approval-wait suspension remain Lifecycle and Engine
Controls work.

## Frozen artifacts

- `docs/schemas/workflow-call.v1.schema.json`
- `docs/schemas/workflow-call-index.v1.schema.json`
- `docs/schemas/workflow-call-routing.v1.schema.json`
- `docs/schemas/compiled-workflow-model.v10.schema.json`
- `docs/schemas/woml-definition-package.v4.schema.json`
- `docs/schemas/woml-definition-package.v5.schema.json`
- `woml/tests/fixtures/workflow-calls/*`
- `woml/tests/fixtures/workflow-call-contracts/*`
