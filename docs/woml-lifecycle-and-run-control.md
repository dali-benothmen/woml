# Lifecycle and Local Run Control

WOML Lifecycle and Engine Controls let a workflow observe its own execution
and let a local operator list, inspect, and cancel durable runs. The feature is
implemented by the Rust runtime, Event v10, Store v11, isolated Bun workers,
and the direct WOML CLI commands.

Lifecycle hooks are observers. Use them for logging, notifications, audit
records, metrics, and best-effort cleanup. Keep anything required for business
correctness as an ordinary step in `<steps>`.

## Authoring lifecycle hooks

One optional `<lifecycle>` block is a direct child of `<workflow>`:

```xml
<workflow id="process-order" name="Process order" version="1.0.0">
  <lifecycle>
    <on-start>
      <script>
        console.log(`Starting ${lifecycle.workflow.id}`);
      </script>
    </on-start>

    <on-step-failure steps="charge createInvoice">
      <notify>
        <slack
          channels="#order-incidents"
          message="Step {{lifecycle.step.id}} failed: {{lifecycle.failure.code}}"
          bot-token="{{secrets.SLACK_BOT_TOKEN}}"
          app-token="{{secrets.SLACK_APP_TOKEN}}"
        />
      </notify>
    </on-step-failure>

    <on-success>
      <script>console.log('Order succeeded');</script>
    </on-success>

    <on-failure>
      <script>console.log(`Order failed: ${lifecycle.failure.code}`);</script>
    </on-failure>

    <on-cancel>
      <script>console.log('Order cancelled');</script>
    </on-cancel>

    <on-complete>
      <script>console.log(`Final outcome: ${lifecycle.workflow.outcome}`);</script>
    </on-complete>
  </lifecycle>

  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="charge"><script>return { charged: true };</script></step>
    <step id="createInvoice"><script>return { created: true };</script></step>
  </steps>
</workflow>
```

Hooks must appear in this canonical order when present:

1. `on-start`
2. `on-step-start`
3. `on-step-success`
4. `on-step-failure`
5. `on-step-complete`
6. `on-success`
7. `on-failure`
8. `on-cancel`
9. `on-complete`

Each hook contains one or more `<script>` or `<notify>` actions. Step hooks may
use a space-separated `steps` filter. Without the filter, a step hook observes
every executable step, including steps nested in branches, parallel groups,
and approval arms.

## What each hook means

| Hook | Runs when |
|---|---|
| `on-start` | The run is durably admitted, before the business DAG starts. |
| `on-step-start` | A logical step begins, before its first attempt. |
| `on-step-success` | A logical step eventually succeeds, including after retries. |
| `on-step-failure` | A logical step exhausts its allowed attempts. |
| `on-step-complete` | The logical step has settled as success or failure. |
| `on-success` | The durable business outcome is success. |
| `on-failure` | The durable business outcome is failure. |
| `on-cancel` | A cancellation request wins before the business outcome. |
| `on-complete` | The outcome-specific hook has settled and the run is finalizing. |

Step hooks observe logical steps, not individual retry attempts. A step that
fails twice and succeeds on its third attempt produces one start, one success,
and one complete hook. `lifecycle.step.attempts` reports the attempt count.

## Lifecycle script binding

Lifecycle scripts receive the normal read-only `context`, `attempt`,
`services`, and `secrets` bindings plus a deeply read-only `lifecycle` binding.
The available fields depend on the event:

- `lifecycle.event`
- `lifecycle.workflow.id`
- `lifecycle.workflow.outcome` after an outcome is decided
- `lifecycle.step.id`, `outcome`, and `attempts` for step hooks
- `lifecycle.failure.code` for failure hooks

`lifecycle` is unavailable in ordinary business-step scripts. Lifecycle
scripts cannot return a value into `context.steps`, change a branch, recover a
step, or rewrite the workflow result.

Hook failures are durable lifecycle warnings. They never turn a successful
business run into a failed run or a cancelled run into a failure. A run remains
`finalizing` until its required hook actions settle, then ends with lifecycle
status `completed` or `completed_with_warnings`.

## Cancellation semantics

Request cancellation with:

```bash
woml cancel run_...
```

The command writes a durable request and prints “cancellation requested.” It
does not claim that already committed external effects were rolled back. Rust
then:

1. stops admitting new steps, retries, and branches;
2. signals active scripts and managed capabilities that support cancellation;
3. invalidates waiting approval decision credentials;
4. preserves already committed results and external effects;
5. records ambiguous interrupted effects as warnings instead of replaying them;
6. decides the business outcome as cancelled;
7. runs `on-cancel`, followed by `on-complete`; and
8. finalizes the run as cancelled.

Cancelling a parent waiting in `services.workflows.call()` stops the parent
wait. It does not silently cancel the independently admitted child. A child
created by `services.workflows.start()` is also independent. Cancellation
propagation and compensation/sagas require future explicit contracts.

Repeated cancellation is safe. The command returns `already_requested` while
settlement is in progress and `already_cancelled` after finalization. A run
whose success or failure outcome is already decided rejects cancellation.

## List and inspect runs

The commands use `.woml/state.sqlite` by default. Pass `--state` whenever the
runtime uses another database:

```bash
woml list
woml list --workflow process-order --status running --limit 50
woml get run_...
woml cancel run_...
```

Add `--json` for the stable `woml.run-list/v1`,
`woml.run-inspection/v2`, and `woml.run-control.result/v1` contracts.

Inspection is intentionally redacted. It includes workflow/run identity,
status, business outcome, lifecycle status, bounded hook summaries, warning
codes, and cancellation state. It excludes context, trigger payloads, step and
workflow results, notification bodies, secrets, credentials, operation keys,
idempotency keys, and stack traces.

The initial control surface is local and state-file scoped. Anyone able to
write the SQLite state database is inside the trusted runtime boundary. Do not
expose the database or wrap these commands in an unauthenticated remote API.

## Recovery and shutdown

Event v10 is authoritative. The runtime rebuilds context and lifecycle state by
folding events; the Store v11 run-summary table used by `woml list` is a
rebuildable index.

After a crash:

- an admitted but unstarted hook may start once;
- an action recorded as started without a terminal event is treated as
  ambiguous and fails closed without automatic replay;
- a durable cancellation request continues toward the same cancelled outcome;
- an interrupted business effect is never described as rolled back; and
- `on-complete` remains the last lifecycle hook before finalization.

Stop a long-lived local runtime with `Ctrl+C` or `SIGTERM`. WOML stops trigger
hosts and Rust runtime threads gracefully. Durable runs retain their event
history and can be inspected after restart.

## Notifications and secrets

Lifecycle `<notify>` currently supports informational Slack delivery. It uses
the shared Slack transport but never receives an approval token or decision
callback. Notification failures become lifecycle warnings.

Keep credentials in the WOML secret store:

```bash
woml secrets set SLACK_BOT_TOKEN
woml secrets set SLACK_APP_TOKEN
```

Reference only symbolic names in source. Secret references are permitted in
credential attributes and forbidden in notification message templates.
Templates may interpolate bounded scalar `context.*` and `lifecycle.*` values.

## Compatibility boundaries

- Model v11, Event v10, Store v11, Lifecycle Binding v1, Lifecycle Progress
  v1, Notification Provider Host v2, Run List v1, Run Inspection v2, and Run
  Control v1 are versioned contracts.
- Model v1-v10 histories are never rewritten to invent lifecycle or cancelled
  states.
- Cancellation of a pre-v11 run fails with
  `WOML_RUN_CONTROL_VERSION_UNSUPPORTED`.
- The frozen inspection v2 contract does not expose raw results or compose the
  separate bounded workflow-call relation query. Either addition requires a
  new inspection schema version.
- Event v10 does not encode an approval-message `cancelled` resolution. WOML
  invalidates the decision capability but does not mislabel cancellation as
  rejection.

## Production checklist

Before deploying a long-lived WOML runtime:

1. Use an explicit absolute `--state` path on persistent storage.
2. Restrict the state directory to the WOML service account.
3. Configure secrets with `woml secrets set`; never put values in source or
   process arguments.
4. Exercise success, failure, cancellation, and restart paths in staging.
5. Verify every lifecycle action is observational rather than business
   critical.
6. Use step filters to avoid unnecessary notification volume.
7. Monitor `completed_with_warnings` and lifecycle warning codes.
8. Set retention and backup policy for the event database and its WAL files.
9. Preserve the exact CLI/core package version across all local processes that
   share one state database.
10. Run `bun run test:lec8` from `woml-cli` before publishing a build.

Hosted multi-node ownership, remote authenticated run control, cancellation
propagation, compensation, retention automation, and distributed scheduling
remain Production Runtime work rather than hidden behavior in this local
profile.

## Runtime Policy timeout interaction

The workflow `timeout` in `<config>` begins at first execution start and
includes subsequent lifecycle time and durable waits. If it wins before a
business outcome, WOML records `WOML_WORKFLOW_TIMED_OUT`, runs workflow
`on-failure`, then `on-complete`. Timeout remains failure, not cancellation, and
a previously committed outcome wins the race. Waiting for approval, retry, or a
synchronous child releases concurrency capacity without pausing the deadline.
See [WOML Runtime Policies](woml-runtime-policies.md).
