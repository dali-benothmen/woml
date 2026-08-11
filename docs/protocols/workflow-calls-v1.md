# WOML Workflow Calls v1

Status: frozen by WC0 on 2026-08-10. WC1 compiled the source contract, WC2
implemented exact registration plus durable child admission, WC3 executed
same-runtime children, and WC4 completed retry-safe reattachment, cycle
rejection, single-executor claiming, and fail-closed recovery. WC5 implements
project-local cross-process ownership, wake-up, and pending-child recovery.
WC6 adds a separate progress surface, bounded parent/child inspection,
composition coverage, and pre-admission Human Approval rejection. WC7 hardens
stored identity verification, v9-to-v10 migration, clean packaging, benchmark
coverage, and publication of the local deployment/security profile without
changing this frozen v1 protocol.

## Author contract

```js
const result = await services.workflows.call(
  'calculate-risk',
  { customerId: context.payload.customerId },
  { name: 'customer-risk', timeout: '30s' }
);
```

The first argument is one activated workflow ID. The second is a top-level JSON
object that becomes the complete child `context.payload`. The optional `name`
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

`woml run` activates this definition without starting it. An empty `<triggers>`
container is invalid. No Workflow Calls v1 target may contain Human Approval
because an arbitrary Bun continuation cannot be durably serialized across a
long pause. Rust checks the selected immutable definition and rejects it before
admitting a child.

Compiled Workflow Model v10 represents call-only activation with exactly
`triggers: []`. It may retain unchanged Module Runtime v1 bindings. Models
v1-v9 remain immutable.

## Hot-path and event contracts

Workflow Calls use unchanged Capability Call v1 with:

```text
capability = workflows
operation  = call
```

Parent managed-operation history continues using unchanged Run Event v8. Safe metadata may
contain target workflow ID, target definition hash, child run ID, payload
digest, and lineage depth. It never contains payload or result values.

Run Event v9 adds one narrow, truthful `workflow_call` ingress for a called
child. Its `run_started` event stores the payload as the complete trigger value,
but stores the call key separately as engine metadata. It never invents a
manual, event, or other source trigger. All later event payloads retain their
Run Event v8 shapes. The child's eventual `run_succeeded.result` or
`run_failed` event remains the terminal authority.

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

The stored `parentAttempt` records the attempt that first admitted the child;
it is not part of the logical call identity. A later attempt with the same
step idempotency key, target, operation name, payload digest, and definition
hash observes that original child. Exactly one concurrent caller may move the
index from `admitted` to `running`; all others wait on authoritative child run
history.

Recovery reconciles the reconstructible index from child events. A child that
committed success stays successful even if the parent crashed before committing
its step result. The ambiguous parent attempt fails closed and is never replayed
as proof that its script did or did not finish.

Self-calls and indirect calls to a workflow already present in hidden lineage
fail with `WOML_WORKFLOW_CALL_CYCLE`. The maximum lineage depth remains 32.

The durable call index is a transactional uniqueness/routing index. Parent
operation history and child run history remain authoritative and the index
never stores a competing result.

## Routing

Same-runtime routing is a direct Rust registry lookup. Local cross-process
routing uses a project/state-scoped ownership lease and an authenticated
loopback wake-up message for an already admitted child. WOML creates the
session credential automatically; it is not a workflow secret or public API.

`woml run` accepts one or more file/directory operands. Every direct `.woml`
file selected by those operands is deduplicated and validated before the set is
activated as one runtime unit. Directory traversal is intentionally
non-recursive in v1. Duplicate workflow IDs fail startup.

Each local process registers its workflow IDs, exact definition hashes,
runtime ID, random loopback endpoint, session-credential hash, and renewable
ownership lease in durable store v10. The raw credential is derived from a
mode-`0600` project-local routing key beside the selected state database and is
never stored in the route table, workflow metadata, or user context. A
graceful stop releases ownership; a crash is handled by lease expiry.

The caller admits the child in shared durable state before sending the wake-up.
The target verifies the bearer credential, runtime ID, child identity, call
key, workflow ID, and definition hash before attempting the existing atomic
execution claim. The target scans admitted children every 250 ms, so a lost
wake-up remains recoverable. Wake-up acknowledgement never substitutes for the
child's durable terminal event.

Exactly one live owner may register a workflow ID. The selected child is pinned
to the owner's exact definition hash. Cross-machine routing and tenant service
identity are deferred to Production Runtime.

## Progress and run inspection

Workflow Call Progress v1 is a separate operator surface; it does not widen
Capability Call v1, Trigger Progress v1, or the durable event vocabulary. Its
`call_admitted` message contains only parent run/node IDs, target workflow ID,
child run ID, duplicate status, and time. Its `child_terminal` message contains
the parent ID, target ID, child ID, terminal status, and time. Its
`call_rejected` message contains the parent run/node IDs, requested target ID,
stable failure code, safe actionable message, and time; it has no child ID
because rejection happens before admission.

`woml runs get` returns one optional `parentCall` plus at most 50 `childCalls`
and a `childCallsTruncated` flag. Both surfaces forbid call keys, definition
hashes, payload digests, payloads, results, secrets, and runtime credentials.

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
- `docs/schemas/workflow-call-progress.v1.schema.json`
- `docs/schemas/compiled-workflow-model.v10.schema.json`
- `docs/schemas/run-event.v9.schema.json`
- `docs/schemas/woml-definition-package.v4.schema.json`
- `docs/schemas/woml-definition-package.v5.schema.json`
- `woml/tests/fixtures/workflow-calls/*`
- `woml/tests/fixtures/workflow-call-contracts/*`
