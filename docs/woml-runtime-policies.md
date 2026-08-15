# WOML Runtime Policies

Runtime Policies v1 are the workflow-level safety controls executed by the
Rust core. They let an author bound simultaneous work, pace new executions,
set a total workflow deadline, and choose the durable scheduling lane without
putting scheduling code inside business steps.

## Authoring `<config>`

`<config>` is optional. Put it once, before `<lifecycle>`, `<triggers>`, and
`<steps>`:

```xml
<workflow id="process-order" name="Process order" version="1.0.0">
  <config
    concurrency="4"
    rate-limit="100/1m"
    timeout="10m"
    queue="orders"
  />
  <!-- triggers and steps -->
</workflow>
```

Each attribute is independently optional, but `<config>` must contain at least
one:

- `concurrency` limits actively executing runs of this workflow ID across all
  WOML processes sharing the same state database.
- `rate-limit` limits first execution starts using a strict rolling window. It
  does not count queue admission or a resumed approval/retry/call wait twice.
- `timeout` is the total deadline from the first execution start. Queue time is
  excluded; approval, retry, child-workflow, script, and lifecycle time after
  the start are included.
- `queue` names the durable scheduling lane. It does not create a messaging
  queue and does not merge concurrency capacity between workflow IDs.

Use `woml check workflow.woml` to see the normalized executable policy before
activation.

## What happens when work arrives

Every supported ingress follows the same sequence:

1. Rust durably admits the run and records it as `queued`.
2. The scheduler checks queue order, workflow concurrency, and rate capacity.
3. Rust records the first execution start and fixes the timeout deadline.
4. The existing DAG executor runs the selected steps.
5. Durable events remain authoritative; queue rows, claims, and summaries can
   be rebuilt after a restart.

This path is shared by manual, webhook, Slack, schedule, interval, event,
`services.workflows.call()`, and `services.workflows.start()` admission.

The queue is work-conserving FIFO: it preserves oldest-eligible ordering while
allowing another workflow in the same lane to proceed when the oldest item is
blocked by its own workflow limit.

## Observing and controlling policy runs

Normal `woml run` output explains when a run is queued, becomes eligible,
starts, or reaches its timeout. Direct commands work without reopening source:

```bash
woml list --status queued
woml list --workflow process-order --json
woml get run_... --json
woml cancel run_...
```

Policy runs use Run List v2 and Run Inspection v3. Inspection exposes the safe
queue/wait/deadline state, but never scheduler owner IDs, payloads, contexts, or
secrets. Cancellation of a queued run prevents its business steps from
starting; cancellation of active work follows the normal fail-closed run-control
contract.

## Queue saturation

One local state location accepts at most 10,000 queued policy runs. At that
ceiling no new run is created, and the original source identity remains safe to
retry. `WOML_POLICY_QUEUE_FULL` is mapped to each producer:

| Producer | Behavior |
| --- | --- |
| Manual run | The command fails with an actionable diagnostic. |
| Webhook | HTTP 503 and `Retry-After: 1`. |
| Slack | WOML does not acknowledge the event, allowing provider redelivery. |
| Schedule/interval | The durable occurrence cursor is not advanced. |
| Named event | That subscriber delivery remains retryable; fan-out deduplication is preserved. |
| Workflow Call/Start | The managed operation returns a retryable failure. |

Backpressure is deliberate. WOML never discards an older run to make space and
never invents a run ID for rejected work.

## Lifecycle, retries, approval, and Workflow Calls

An execution slot represents active work, not the entire lifetime of a run.
Retry delay, Human Approval, and synchronous Workflow Call waiting release the
slot. Resume reacquires it. This lets another eligible run make progress while
one automation is waiting on time or a person.

A workflow timeout is a failure with code `WOML_WORKFLOW_TIMED_OUT`. When it
wins the durable race, WOML stops active work, runs workflow `on-error`, then
`on-complete`. A previously committed success/cancellation cannot be rewritten
by a late timer.

Called and started workflows apply their own `<config>` under their own
workflow ID. The parent does not lend its limits to the child. A synchronous
parent releases its active slot while it waits and reacquires it before
continuing.

## Recovery and upgrades

Use the same state location after restart:

```bash
woml run workflows/ --state .woml/state.sqlite
```

Store v12 reconstructs queued truth and rate history from immutable Event v11
history. Expired scheduler claims coordinate ownership only; they are never
proof that an ambiguous script or external side effect is safe to replay. Such
an interrupted attempt follows WOML's fail-closed recovery rule.

Models v1-v11, Events v1-v10, Run List v1, and Run Inspection v2 remain valid.
Adding `<config>` deliberately compiles that workflow to Model v12. Do not
change the policy or definition of a workflow ID while it has an active run;
activation fails with `WOML_POLICY_CONFLICT` instead of mixing policies.

Before upgrading a deployment, stop writers cleanly, back up the SQLite state
file together with its WAL/SHM files when present, install the new package, and
start all processes against the same explicit state path. Unknown future store
versions and corrupt required schema fail closed without rewriting history.

## Deployment checklist

1. Put the state database on durable local storage available to every WOML
   process that coordinates the same workflows.
2. Use one explicit absolute `--state` path in service configuration.
3. Keep filesystem permissions restricted to the WOML service account.
4. Monitor queued counts, wait reasons, timeout failures, queue-full responses,
   and process restarts.
5. Set producer retry behavior for HTTP 503 and retryable event/call failures.
6. Treat large sustained queues as capacity or downstream-health signals.
7. Back up and restore the event store as one SQLite database unit.
8. Run the publication gate for the exact source revision before deployment.

## Performance and publication gate

From `woml-cli`, build and run the repeatable local benchmark:

```bash
bun run build
bun run benchmark:runtime-policies
```

The versioned report measures the no-`<config>` baseline, policy admission and
execution, a shared durable burst, list/get, rolling-window eligibility,
timeout detection, and bounded long-lived memory growth. Bun worker startup is
reported separately from policy overhead.

The complete publication gate is:

```bash
bun run test:rp7
```

It builds the native package, validates frozen schemas and historical fixtures,
runs adversarial Rust and CLI tests, installs the packed CLI into a clean
consumer, enforces Clippy/type checks and performance budgets, inspects package
contents, and scans public artifacts for active WOML secret values.

The exact frozen machine contracts remain in
[Runtime Policies v1 Contracts](protocols/runtime-policies-v1.md).
