# WOML Durable Recovery

WOML records run decisions and outcomes in SQLite so the Rust core can rebuild
a run without trusting an in-memory context object. Recovery folds the event
history, identifies the only legal next action, and either continues safely or
fails closed.

## Resuming a run

Use the exact recovery command printed by the CLI after a retry is first
scheduled or a Human Approval begins:

```bash
woml run "workflow.woml" \
  --state ".woml/state.sqlite" \
  --resume "run_..."
```

The workflow file and state path must identify the definition and database
bound to the original run. Do not edit the workflow and try to resume the old
run as though it used the new definition; each run is immutable against its
compiled definition.

## Recovery rules

| Durable state | Recovery result |
|---|---|
| Retry scheduled, next attempt not started | Wait until its recorded due time, then run that exact attempt. |
| Attempt started, no terminal outcome | Record `interrupted` and fail closed; do not replay an ambiguous effect. |
| Step succeeded | Keep its published output and never run that step again. |
| Retry exhausted or non-retryable failure | Keep the run failed; do not invent another attempt. |
| Run stopped after success but before downstream dispatch | Rebuild context from events and continue only the remaining nodes. |

An attempt failure and its next schedule are committed atomically. The engine
therefore cannot recover a half-decision where a retry was intended but its due
time or attempt number is missing.

## What is safe to retry

The first retry profile automatically retries only `script_threw`. It does not
retry timeouts, invalid/non-JSON or oversized results, oversized context, host
crashes, Worker crashes, cancellation, or interruption. Those outcomes cannot
prove that an external effect did not happen.

All attempts of one logical step share `attempt.idempotencyKey`. Pass it to an
external API that guarantees duplicate handling. With managed HTTP, use the
reviewed `idempotency` request field and a stable operation name. The key
enables that service to deduplicate; it does not turn an arbitrary side effect
into exactly-once execution.

A managed `operation_started` without `operation_succeeded` or
`operation_failed` is closed as `interrupted` with `ambiguous: true` before the
surrounding attempt is recovered. WOML never reconstructs a JavaScript
instruction pointer or releases an unpersisted service result after restart.

## Workflow Calls

A workflow call admits one independent child before executing it. Retries and
duplicate delivery with the same logical identity reconnect to that child;
they never manufacture another run. The mutable call index is reconstructed
from the child's authoritative events during startup recovery.

If the child succeeded but the parent crashed before durably committing its
step result, the child remains successful and inspectable. The ambiguous parent
attempt still fails closed. WOML does not replay the parent script or pretend
that receiving the child's result proves the parent finished using it.

Across local processes, the child is durable before WOML sends the private
loopback wake-up. The target periodically scans admitted children, so a lost
wake-up is repaired without another child admission. Renewable ownership leases
expire crashed targets; graceful shutdown removes their routes immediately.
The loopback acknowledgement is never treated as proof of child completion.

Use `woml get <runId> --state <path>` to move between the two independent
runs. A parent exposes at most 50 `workflowCalls.childCalls`; a child exposes
its optional `workflowCalls.parentCall`. The summaries intentionally omit
payloads, results, secrets, hashes, and internal routing identity.

Workflow Calls v1 rejects a selected definition containing Human Approval
before child admission. Approval workflows must be triggered independently
until a calling JavaScript continuation can be suspended durably.

## Operational output

Attempt failures, schedules, successes, and the recovery command are written to
stderr. Only the final JSON result is written to stdout. Exhaustion reports
`WOML_STEP_RETRIES_EXHAUSTED`, the final attempt count, the safe underlying
failure code, and the authored script location.

Secrets and runtime capabilities are forbidden from the event log, folded
context, progress protocol, errors, and durable step output.
