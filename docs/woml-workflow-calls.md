# Calling One WOML Workflow from Another

Use `services.workflows.call()` when one workflow needs the answer produced by
exactly one other workflow. The called workflow is an independent durable run,
but the calling script receives its final JSON result like a normal async
function result.

```js
const risk = await services.workflows.call('calculate-risk', {
  customerId: context.payload.customerId
});

return { score: risk.score };
```

The payload object becomes the child's complete `context.payload`. The value
returned by the child's last executable step becomes `risk`. A child must
return JSON, including `null`; missing or JavaScript `undefined` is an error.

Use `start()` when the parent should continue immediately after the child is
durably admitted:

```js
const started = await services.workflows.start('calculate-risk', {
  customerId: context.payload.customerId
});

return { childRunId: started.runId };
```

The `await` above waits only for durable admission and dispatch. The child keeps
running independently, and its later result or failure does not change the
already completed parent step.

| Operation | Parent waits for | Returned value |
| --- | --- | --- |
| `services.workflows.call()` | Child terminal status | Child's final JSON result |
| `services.workflows.start()` | Durable admission and dispatch | `{ workflowId, runId, duplicate }` |

## Activate the workflows

The simplest setup keeps both definitions in one folder and starts one WOML
runtime:

```bash
woml run examples/workflowCalls
```

Explicit files work too:

```bash
woml run parent.woml child.woml
```

The module-backed composition example proves that imports stay attached to the
called definition:

```bash
woml run examples/workflowCallsModule
```

Separate local processes are optional. They can discover each other when they
use the same durable state file:

```bash
woml run child.woml --state .woml/state.sqlite
woml run parent.woml --state .woml/state.sqlite
```

WOML creates private loopback routing credentials automatically. Do not add a
URL or secret to the workflow call.

## Progress and inspection

When a call begins, the active CLI prints both durable identities:

```text
Workflow call run_parent/calculateRisk started child run_child for "calculate-risk".
Workflow call child run_child for "calculate-risk" succeeded; parent run_parent.
```

Inspect either run's durable status later:

```bash
woml get run_parent --state .woml/state.sqlite
woml get run_child --state .woml/state.sqlite
```

The engine's separate workflow-call relation query contains `parentCall` for a
child and `childCalls` for a parent. Child lists are capped at 50 and set
`childCallsTruncated: true` when more exist. That relation query is not folded
into the frozen `woml.run-inspection/v2` JSON contract; doing so requires a
future inspection version. Inspection and progress never include the call
payload, result payload, secret values, definition hashes, payload digests, or
internal call keys.

## Composition

Called workflows use the normal Rust DAG engine. They may contain sequential
steps, branches, parallel groups, retries, local modules, native `fetch()`, and
the current built-in services. A parent may be started by manual, webhook,
Slack, schedule, interval, or named-event triggers; the call behavior is the
same after the run starts. Nested calls work in one runtime and across local
processes, up to the lineage-depth limit.

Human Approval is the one v1 exception. A target containing approval is
rejected by synchronous `call()` before a child run is admitted, with
`WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED`. Use the non-blocking `start()` operation
when launching that workflow from another workflow is appropriate; the parent
does not wait for its approval or terminal result.

Cancelling a parent waiting in `services.workflows.call()` stops the parent
wait but does not silently cancel the independently admitted child. Runs
created by `services.workflows.start()` are independent too. Inspect the child
using the run ID printed in Workflow Call progress. Propagating cancellation
requires a future explicit parent/child control contract; see
[Lifecycle and Local Run Control](woml-lifecycle-and-run-control.md).

## Common failures

- `WOML_WORKFLOW_TARGET_NOT_FOUND`: no active runtime currently owns that
  workflow ID. Load the target in the same command or start it against the same
  `--state` file.
- `WOML_WORKFLOW_TARGET_AMBIGUOUS`: two live local runtimes tried to own the
  same workflow ID. Stop one of them.
- `WOML_WORKFLOW_DEFINITION_MISMATCH`: the admitted child is pinned to a
  different definition than the active owner. Restore that definition or
  start a new parent run.
- `WOML_WORKFLOW_CALL_CYCLE`: the target already appears in the current call
  lineage.
- `WOML_WORKFLOW_CALL_TIMED_OUT`: the parent stopped waiting, but the admitted
  child remains independent and can be inspected by its printed run ID.
- `WOML_WORKFLOW_CALL_FAILED`: inspect the child run for its own failure code.

Retries and duplicate transports reconnect to the original child when the
logical operation identity and payload match. Repeated calls to the same target
from one step need stable, different `{ name: "..." }` values.

The frozen non-blocking contract is documented in
`docs/protocols/workflow-start-v1.md`.

For deployment, security, migration, shutdown, benchmarks, and the release
gate, see `docs/woml-workflow-calls-production.md`.
