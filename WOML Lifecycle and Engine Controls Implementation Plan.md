# WOML Lifecycle and Engine Controls Implementation Plan

Status: LEC0 through LEC8 completed on 2026-08-11. Workflow-level lifecycle
scripts now run through isolated Bun workers under Rust supervision. `on-start`
runs before the business DAG; the matching outcome hook and `on-complete` run
after the durable business outcome; lifecycle failures remain visible warnings
without rewriting that outcome. Step hooks are executable as of LEC4, and
informational Slack lifecycle notifications are executable as of LEC5. Durable
cancellation is executable as of LEC6, and direct `list`, `get`, and `cancel`
commands are shipped as of LEC7.

LEC8 completed the clean-package journey, adversarial release matrix, schema
compatibility audit, migration/recovery gate, secret/package scan, performance
budgets, operator documentation, and the unified `bun run test:lec8`
publication command. Lifecycle and local run control are now supported WOML
features rather than staged syntax.

## 1. Product Outcome

WOML authors can declare one workflow-owned lifecycle block for durable
automation around step and workflow events, while operators can inspect, list,
and cancel individual runs through direct CLI commands.

The authoring experience is:

```xml
<woml>
  <workflow id="process-order" name="Process an order" version="1.0.0">
    <lifecycle>
      <on-start>
        <script>
          console.log('Order processing started');
        </script>
      </on-start>

      <on-step-failure steps="chargeCustomer createInvoice">
        <notify>
          <slack
            channels="#order-incidents"
            message="Step {{lifecycle.step.id}} failed"
            bot-token="{{secrets.SLACK_BOT_TOKEN}}"
            app-token="{{secrets.SLACK_APP_TOKEN}}"
          />
        </notify>
      </on-step-failure>

      <on-success>
        <notify>
          <slack
            channels="#orders"
            message="Order workflow completed successfully"
            bot-token="{{secrets.SLACK_BOT_TOKEN}}"
            app-token="{{secrets.SLACK_APP_TOKEN}}"
          />
        </notify>
      </on-success>

      <on-error>
        <notify>
          <slack
            channels="#order-incidents"
            message="Order workflow failed"
            bot-token="{{secrets.SLACK_BOT_TOKEN}}"
            app-token="{{secrets.SLACK_APP_TOKEN}}"
          />
        </notify>
      </on-error>

      <on-cancel>
        <script>
          console.log('Order workflow was cancelled');
        </script>
      </on-cancel>

      <on-complete>
        <script>
          console.log('Order workflow lifecycle is complete');
        </script>
      </on-complete>
    </lifecycle>

    <triggers>
      <webhook id="newOrder" path="/webhooks/orders" method="POST" auth="none">
        <schema>
          {
            "type": "object",
            "required": ["orderId"],
            "properties": {
              "orderId": { "type": "string" }
            }
          }
        </schema>
      </webhook>
    </triggers>

    <steps>
      <step id="chargeCustomer">
        <script>
          return { charged: true, orderId: context.payload.orderId };
        </script>
      </step>

      <step id="createInvoice">
        <script>
          return { invoiceCreated: true };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The operator experience is deliberately direct:

```bash
woml run process-order.woml
woml list
woml get run_...
woml cancel run_...
```

`woml run` activates definitions. `list`, `get`, and `cancel` operate on
individual durable executions. The old `woml runs get` spelling is not the
canonical product surface.

## 2. Product Principles

### 2.1 One lifecycle location

`<lifecycle>` is an optional direct child of `<workflow>`. Lifecycle hooks are
never nested inside `<step>`, `<parallel>`, `<branch>`, or `<approval>`.

The single location keeps cross-cutting behavior visible and prevents business
steps from becoming containers for hidden automation.

### 2.2 Hooks observe; workflow nodes decide

Lifecycle hooks are appropriate for notifications, audit records, metrics,
logging, and non-critical cleanup. They do not:

- add values to `context.steps`;
- select a branch;
- recover a failed step;
- mutate the workflow result;
- compensate a committed side effect; or
- rewrite success, failure, or cancellation.

If an operation is required for business correctness, it remains an explicit
normal step in the DAG.

### 2.3 Durable does not mean exactly once

Rust durably admits each logical hook once and gives every action a stable
identity. Managed services and notification deliveries reuse their approved
idempotency behavior. An arbitrary script or native Fetch side effect can still
be ambiguous after a crash. WOML keeps the existing fail-closed rule and does
not replay an ambiguous lifecycle script as proof that its side effect did not
happen.

### 2.4 Cancellation is not rollback

Cancellation stops new work and signals active cancellable operations. It does
not undo committed steps, revoke an HTTP request that already reached a remote
server, roll back an independent child workflow, or promise compensation.

### 2.5 Outcome and lifecycle health are separate

A business workflow may succeed while a Slack lifecycle notification fails.
The run retains its successful outcome and exposes the lifecycle failure as a
separate warning. This avoids turning observability failures into false business
failures.

## 3. Scope

### Included

- One optional workflow-level `<lifecycle>` block.
- `on-start`, `on-step-start`, `on-step-success`, `on-step-failure`,
  `on-step-complete`, `on-success`, `on-error`, `on-cancel`, and
  `on-complete`.
- Optional step filtering on step hooks through a whitespace-separated `steps`
  attribute.
- Lifecycle scripts with `context`, `lifecycle`, `services`, `secrets`, and
  native Fetch.
- Informational `<notify><slack>` actions inside lifecycle hooks.
- Durable hook admission, action attempts, recovery, and inspection.
- A separate business outcome and lifecycle-finalization projection.
- Durable, idempotent run cancellation.
- Cancellation of waiting approvals, pending retries, active scripts, and
  cancellable managed services.
- Direct `woml list`, `woml get`, and `woml cancel` commands.
- Safe lifecycle and cancellation progress in the active CLI.
- Model v11, Run Event v10, Store v11, Script Runtime Bindings v2, and
  Notification Provider Host v2 contracts.
- Compatibility execution for Models v1-v10 and their existing event streams.

### Not included

- Lifecycle blocks inside steps or control-flow structures.
- Lifecycle actions that write `context.steps` outputs.
- Step recovery, compensation, sagas, or transactions.
- `on-step-retry` or attempt-level hooks in the first profile.
- General pause/resume. Human Approval remains the reviewed durable suspension
  primitive.
- Automatic cancellation propagation into independent workflows created by
  `services.workflows.call()` or `services.workflows.start()`.
- Remote cancellation HTTP APIs or multi-tenant authorization.
- Workflow-level concurrency, rate limits, queue selection, and automatic run
  timeout from `<config>`.
- A public durable user-state service.
- Discord, WhatsApp, Telegram, or generic webhook notification providers.
- Guaranteed ordering between different steps finishing concurrently beyond
  their authoritative event sequence.

The postponed engine-policy and durable-state items remain immediately after
this milestone in the global roadmap.

## 4. Source-Language Contract

### 4.1 Workflow placement

`<workflow>` contains optional `<config>`, optional `<lifecycle>`, optional
`<triggers>` (omission continues to mean call-only), and exactly one required
`<steps>` container. These named containers may appear in any source order.

Only one of each named container is allowed.

### 4.2 Hook grammar

```text
lifecycle := <lifecycle>
               on-start?
               on-step-start?
               on-step-success?
               on-step-failure?
               on-step-complete?
               on-success?
               on-error?
               on-cancel?
               on-complete?
             </lifecycle>

hook      := <hook hook-attributes> lifecycle-action+ </hook>

lifecycle-action := script | notify
```

Lifecycle hooks may appear in any source order. Their names determine when they
execute; source position does not change lifecycle semantics. The compiler
normalizes hooks to deterministic semantic order. A hook may contain one or more
`<script>` or `<notify>` actions. Actions inside one hook execute in source
order. A failed action is recorded, but later actions in the same hook still run.

The step hooks accept one optional attribute:

```xml
<on-step-success steps="chargeCustomer createInvoice">
  ...
</on-step-success>
```

Rules for `steps`:

- it is valid only on `on-step-start`, `on-step-success`, `on-step-failure`, and
  `on-step-complete`;
- it is a non-empty whitespace-separated list of globally unique `<step id>`
  values;
- duplicates and unknown IDs are compile errors;
- omission means every executable `<step>` in the workflow;
- branch, parallel, approval, and lifecycle structural IDs are not step IDs.

Workflow hooks accept no attributes in the first profile.

### 4.3 Hook semantics

| Hook               |                              Cardinality | Trigger point                                                       |
| ------------------ | ---------------------------------------: | ------------------------------------------------------------------- |
| `on-start`         |                             Once per run | Settles before the first business step starts.                      |
| `on-step-start`    |                    Once per logical step | Admitted with the first attempt; it does not run again for retries. |
| `on-step-success`  |         Once per successful logical step | After the successful attempt commits its JSON output.               |
| `on-step-failure`  | Once per permanently failed logical step | After retries are exhausted or a non-retryable failure commits.     |
| `on-step-complete` |            Once per settled logical step | After success, permanent failure, or engine cancellation.           |
| `on-success`       |                             Once per run | After the business DAG decides success.                             |
| `on-error`       |                             Once per run | After the business DAG decides failure.                             |
| `on-cancel`        |                             Once per run | After cancellation becomes the business outcome.                    |
| `on-complete`      |                             Once per run | After the outcome-specific workflow hook settles.                   |

`on-start` is the only hook that blocks business-step scheduling. Step hooks are
durable observers and do not block downstream DAG nodes. Before the run becomes
terminal, Rust drains admitted step hooks, then runs the outcome-specific hook,
then `on-complete`.

`on-step-start` observes the committed logical-step start; it does not promise
that its actions finish before the business step. An author who requires setup
to finish first must model that setup as an explicit preceding step. The local
v1 runtime processes hook requests through one bounded per-run worker in durable
request-sequence order while the business DAG may continue independently.

For one logical step:

```text
success:  on-step-start -> step attempts -> on-step-success -> on-step-complete
failure:  on-step-start -> step attempts/retries -> on-step-failure -> on-step-complete
cancel:   on-step-start -> engine cancellation -> on-step-complete
```

For one workflow:

```text
success: on-success -> on-complete
failure: on-error -> on-complete
cancel:  on-cancel  -> on-complete
```

In a fail-fast parallel, the primary failed step receives `on-step-failure` and
`on-step-complete`. Engine-cancelled siblings receive `on-step-complete` with a
cancelled step outcome, but not `on-step-failure`. The workflow receives
`on-error`, not `on-cancel`, because its business outcome is failure.

### 4.4 Lifecycle scripts

A lifecycle `<script>` uses ordinary JavaScript and may use top-level `await`:

```xml
<on-step-failure>
  <script>
    await services.http.request({
      url: 'https://audit.example.com/failures',
      method: 'POST',
      json: {
        stepId: lifecycle.step.id,
        code: lifecycle.failure.code
      }
    });
  </script>
</on-step-failure>
```

Lifecycle script return values are ignored and never enter `context.steps`.
JavaScript `undefined` is therefore a valid lifecycle-script completion, unlike
a normal workflow step result. Result size remains bounded if an author returns
a value accidentally.

Lifecycle scripts have no automatic script retry in v1. Managed operations
inside them retain their own idempotency behavior. Ambiguous script execution
fails closed and becomes lifecycle health data without changing the business
outcome.

### 4.5 Informational notifications

`<notify>` currently has an approval-only domain contract. This milestone adds
a separate informational mode for lifecycle hooks:

```xml
<on-step-failure>
  <notify>
    <slack
      channels="#incidents #workflow-operations"
      message="Workflow step {{lifecycle.step.id}} failed with {{lifecycle.failure.code}}"
      bot-token="{{secrets.SLACK_BOT_TOKEN}}"
      app-token="{{secrets.SLACK_APP_TOKEN}}"
    />
  </notify>
</on-step-failure>
```

Lifecycle Slack rules:

- `channels`, `message`, `bot-token`, and `app-token` are required;
- one tag targets one Slack credential set and one or more channels;
- multiple Slack tags and shared-channel delivery rules reuse the notification
  provider foundation;
- no approval token, Approve button, Reject button, or callback capability is
  created;
- delivery attempts and provider message identities are durable and bounded;
- provider credentials are resolved at runtime and never enter the event log;
- message bodies are sent to the provider but are not persisted in run events.

WOML Template v1 is frozen in LEC0 for the `message` attribute. It supports
literal text plus bounded `{{...}}` placeholders from `context` and
`lifecycle`. Secret references are forbidden in message content and remain
valid only in explicitly credential-bearing attributes. A message placeholder
must resolve to a scalar value; objects, arrays, unavailable references, and
oversize output are actionable hook errors.

Approval Notification Provider Host v1 remains immutable. Informational
Notification Provider Host v2 reuses Slack connection/channel/delivery code but
uses a separate message and result contract.

## 5. Lifecycle Runtime Binding v1

Lifecycle code receives one deeply read-only `lifecycle` binding. It is present
only in lifecycle scripts and templates; ordinary step scripts do not gain an
undefined public contract.

The proposed frozen shape is:

```ts
type LifecycleBinding = Readonly<{
  event:
    | 'run_start'
    | 'step_start'
    | 'step_success'
    | 'step_failure'
    | 'step_complete'
    | 'run_success'
    | 'run_failure'
    | 'run_cancel'
    | 'run_complete';

  workflow: Readonly<{
    id: string;
    outcome?: 'succeeded' | 'failed' | 'cancelled';
  }>;

  step?: Readonly<{
    id: string;
    outcome?: 'succeeded' | 'failed' | 'cancelled';
    attempts: number;
  }>;

  failure?: Readonly<{
    code: string;
    message: string;
  }>;
}>;
```

The binding deliberately omits secrets, raw stack traces, provider errors,
definition hashes, internal event IDs, invocation IDs, operation keys, and
idempotency keys. Normal workflow data remains in `context.payload` and
`context.steps`. Secret values remain in `secrets` and are never copied into
`lifecycle`.

`context.run` remains unavailable. LEC0 reviews the exact lifecycle fields and
then freezes them as a versioned artifact before scripts can use them.

## 6. Compiled Workflow Model v11

Model v11 adds a top-level optional lifecycle definition rather than encoding
hooks as ordinary DAG nodes:

```ts
interface CompiledLifecycleDefinitionV1 {
  readonly profileVersion: 1;
  readonly hooks: readonly CompiledLifecycleHookV1[];
}

interface CompiledLifecycleHookV1 {
  readonly hookId: string;
  readonly event: LifecycleEventName;
  readonly stepIds?: readonly string[];
  readonly actions: readonly CompiledLifecycleActionV1[];
}

interface CompiledLifecycleActionV1 {
  readonly actionId: string;
  readonly handler: 'runtime.lifecycle-script' | 'notification.informational';
  readonly inputs: ValueExpression;
  readonly scriptRuntime?: ScriptRuntimeBindingsV2;
}
```

The exact schema is an LEC0 review artifact. The important boundaries are:

- the business DAG remains a DAG and does not gain invisible hook edges;
- hook and action IDs are deterministic from hook kind and source order;
- step filters reference globally unique compiled node IDs;
- lifecycle actions cannot be graph entry or terminal nodes;
- lifecycle actions cannot contribute context outputs;
- script actions carry exact required-secret and module bindings;
- notification actions carry symbolic secret references and template
  expressions, never resolved credentials.

During LEC1, lifecycle-bearing WOML definitions emit Model v11. Definitions
without lifecycle retain their existing model version so the current Event v9
Rust runtime remains usable between phases. LEC2 moves newly admitted
definitions to Model v11 once Event v10 is the durable authority. Models v1-v10
remain immutable and executable.

Already stored pre-v11 runs remain inspectable and resumable under their frozen
event versions. Public cancellation of a pre-v11 active run returns
`WOML_RUN_CONTROL_VERSION_UNSUPPORTED` rather than inventing a legacy failure.

## 7. Run Event v10 and Folded State

Run Event v10 adds an explicit lifecycle and control vocabulary. Recommended
event families are:

```text
run_cancellation_requested

lifecycle_hook_requested
lifecycle_action_attempt_started
lifecycle_action_succeeded
lifecycle_action_failed
lifecycle_hook_completed

run_outcome_decided
run_finalized
```

`run_outcome_decided` carries exactly one business outcome: succeeded with the
bounded final result, failed with the existing safe failure union, or cancelled
with its cancellation request identity. It does not mean lifecycle finalization
is finished.

`run_finalized` commits after all required lifecycle work has reached a terminal
hook status. It preserves the business outcome and records only bounded
lifecycle-health summaries. Hook result values and notification messages are
not stored in this event.

The folded run gains two axes:

```text
business outcome: undecided | succeeded | failed | cancelled
lifecycle status: idle | running | finalizing | completed_with_warnings | completed
```

The public run status is:

```text
not_started | running | waiting | cancelling | finalizing |
succeeded | failed | cancelled
```

For Models v1-v10, the existing `run_succeeded` and `run_failed` events retain
their current terminal meaning. Event v10 does not reinterpret historical
streams.

### 7.1 Durable hook identity

One hook invocation identity is derived from:

```text
run_id + hook_id + subject_kind + subject_id
```

For step hooks, `subject_id` is the compiled step ID. For workflow hooks it is
the run ID. One action attempt identity additionally includes `action_id` and
attempt number.

The request event is appended transactionally with the engine event that makes
the hook eligible. Duplicate recovery observes the same hook invocation rather
than admitting another one.

### 7.2 Finalization order

Rust uses this order:

1. Decide and persist the business outcome.
2. Drain all previously admitted step hooks.
3. Execute `on-success`, `on-error`, or `on-cancel` for that outcome.
4. Execute `on-complete`.
5. Append `run_finalized`.

If the process crashes during any stage, recovery folds the event log and
continues only unambiguous work. An action attempt recorded as started without a
terminal event fails closed and is not automatically replayed.

## 8. Durable Cancellation Contract

### 8.1 CLI command

```bash
woml cancel run_... [--state .woml/state.sqlite]
```

Cancellation is an idempotent durable command:

- a not-started, running, or waiting run with no decided business outcome
  accepts one request;
- a cancelling run, or a run finalizing an already decided cancellation,
  observes `already_requested`;
- an already cancelled run returns success and `already_cancelled`;
- a run finalizing an already decided success/failure returns
  `WOML_RUN_OUTCOME_ALREADY_DECIDED`;
- an already succeeded or failed run returns `WOML_RUN_ALREADY_TERMINAL`;
- an unknown run returns `WOML_RUN_NOT_FOUND`;
- a pre-v11 run returns `WOML_RUN_CONTROL_VERSION_UNSUPPORTED`.

The local first profile authorizes control through access to the selected state
database. Remote authentication and tenancy belong to Production Runtime.

### 8.2 Engine behavior

After `run_cancellation_requested` commits, Rust:

1. stops scheduling new DAG nodes and retries;
2. signals active Script Host invocations with the existing `run_cancelled`
   reason;
3. cancels active managed capabilities that support cancellation;
4. records ambiguous unsafe effects truthfully rather than claiming rollback;
5. closes waiting approval state and invalidates its decision capability;
6. asks the notification provider to disable/update outstanding approval
   messages as cancelled;
7. marks unstarted work as skipped by cancellation without manufacturing step
   failures;
8. decides the run outcome as cancelled;
9. executes `on-cancel`, then `on-complete`; and
10. finalizes the run as cancelled with any lifecycle warnings visible.

Cancellation of a parent waiting in `services.workflows.call()` stops the
parent wait but does not cancel the already admitted independent child. A run
started by `services.workflows.start()` is also independent. Propagation policy
is deferred until a future explicit parent/child control contract exists.

### 8.3 Crash and race behavior

- Cancellation racing step success is resolved by durable event sequence.
- A step success committed before the cancellation request remains committed.
- A cancellation request committed first prevents a later success from
  scheduling downstream work.
- Duplicate commands never append multiple logical cancellation requests.
- Runtime restart discovers requested-but-unfinished cancellation from the
  event log.
- Cancellation during an ambiguous external effect preserves an ambiguity
  warning and never retries the effect automatically.

## 9. Direct CLI Run Management

### 9.1 List

```bash
woml list
woml list --workflow process-order --status running --limit 50
woml list --json
```

The first profile defaults to the 20 most recently updated runs and caps
`--limit` at 200. Human output includes run ID, workflow ID, status, start time,
and last update time. JSON output uses a versioned bounded schema.

The list never includes context, results, failure messages, notification
messages, secrets, call keys, operation keys, or credentials.

### 9.2 Get

```bash
woml get run_...
woml get run_... --json
```

`get` replaces the canonical `woml runs get` spelling. Its frozen redacted v2
inspection exposes:

- business outcome;
- lifecycle status;
- bounded lifecycle hook/action summaries;
- cancellation request state;
- actionable failure/warning codes.

Human output is readable by default. `--json` is stable for scripts and AI
agents. It never includes raw context, payloads, results, or secrets. The
existing bounded workflow-call relation query remains a separate safe engine
surface; composing it into JSON inspection requires a future inspection schema
version rather than mutating v2 after freeze.

### 9.3 Cancel

```bash
woml cancel run_...
```

The CLI prints whether cancellation was accepted, already requested, or already
complete. It never requires the workflow source file because the immutable
definition and event stream are already stored.

### 9.4 Store v11 summary index

`woml list` must not fold every event stream for every invocation. Store v11
adds a rebuildable run-summary projection index containing only bounded operator
fields. Events remain authoritative. Migration reconstructs the index from
existing streams and proves that deleting/rebuilding it produces identical
summaries.

## 10. Error Surface

The frontend and runtime use stable codes with source locations or run IDs.
The initial catalog includes:

| Code                                    | Meaning                                                                                                 |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `WOML_LIFECYCLE_DUPLICATE`              | More than one lifecycle block or duplicate singleton hook exists.                                       |
| `WOML_LIFECYCLE_ACTION_REQUIRED`        | A hook has no executable action.                                                                        |
| `WOML_LIFECYCLE_ACTION_INVALID`         | A hook contains unsupported children.                                                                   |
| `WOML_LIFECYCLE_STEP_FILTER_INVALID`    | A step filter is empty, duplicated, or malformed.                                                       |
| `WOML_LIFECYCLE_STEP_UNKNOWN`           | A filter references no executable step.                                                                 |
| `WOML_LIFECYCLE_BINDING_UNAVAILABLE`    | A lifecycle-only binding is used in an ordinary step.                                                   |
| `WOML_LIFECYCLE_TEMPLATE_INVALID`       | Informational message interpolation is malformed.                                                       |
| `WOML_LIFECYCLE_TEMPLATE_VALUE_INVALID` | A placeholder is unavailable or not scalar.                                                             |
| `WOML_LIFECYCLE_ACTION_FAILED`          | A hook action failed without changing the business outcome.                                             |
| `WOML_RUN_NOT_FOUND`                    | A direct run command names no stored run.                                                               |
| `WOML_RUN_OUTCOME_ALREADY_DECIDED`      | Cancellation arrived after success/failure was durably decided but before lifecycle finalization ended. |
| `WOML_RUN_ALREADY_TERMINAL`             | A successful or failed run cannot be cancelled.                                                         |
| `WOML_RUN_CONTROL_VERSION_UNSUPPORTED`  | The stored run predates Event v10 controls.                                                             |
| `WOML_RUN_CANCELLATION_FAILED`          | The durable cancellation authority is unavailable.                                                      |

Messages and provider errors are bounded and redacted. Lifecycle warnings may
identify hook ID, action ID, step ID, provider, destination, and safe error code,
but never secret values, message bodies, raw stack traces, context payloads, or
provider tokens.

## 11. Versioned Artifacts Required Before Execution Code

LEC0 produces reviewable schemas and fixtures for:

- `docs/schemas/compiled-workflow-model.v11.schema.json`;
- `docs/schemas/run-event.v10.schema.json`;
- `docs/schemas/lifecycle-binding.v1.schema.json`;
- `docs/schemas/lifecycle-progress.v1.schema.json`;
- `docs/schemas/run-control.v1.schema.json`;
- `docs/schemas/run-inspection.v2.schema.json`;
- `docs/schemas/run-list.v1.schema.json`;
- `docs/schemas/notification-provider-host.v2.schema.json`;
- `docs/schemas/woml-template.v1.schema.json`;
- `docs/schemas/woml-definition-package.v6.schema.json` when Model v11 package
  storage requires it; and
- reviewed source, compiled-model, event-sequence, cancellation-race,
  notification, inspection, and migration fixtures.

The schema version numbers above are proposed targets for LEC0. If repository
inspection during LEC0 proves a different version boundary is necessary, the
artifact review changes the number before implementation—not after data is
written.

## 12. Implementation Phases

### Phase summary

| Phase           | What changes                                                                                          | Result after the phase                                                                                                                          |
| --------------- | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| LEC0 (complete) | Freeze lifecycle, notification, cancellation, inspection, model, event, and store contracts.          | Every expensive boundary is reviewable before runtime code.                                                                                     |
| LEC1 (complete) | Validate lifecycle WOML and lower lifecycle-bearing definitions to Model v11.                         | Authors receive exact syntax diagnostics and deterministic compiled lifecycle definitions without breaking the Event v9 runtime between phases. |
| LEC2 (complete) | Implement Event v10 folding, Store v11, hook identity, outcome/finalization state, and run summaries. | Rust can durably represent lifecycle and controls without executing hooks.                                                                      |
| LEC3 (complete) | Execute workflow-level lifecycle scripts.                                                             | `on-start`, outcome hooks, and `on-complete` work end to end.                                                                                   |
| LEC4 (complete) | Execute step lifecycle hooks across retries and control flow.                                         | Step start/success/failure/complete observers work correctly in sequential, branch, parallel, and approval workflows.                           |
| LEC5 (complete) | Deliver informational Slack lifecycle notifications.                                                  | Lifecycle hooks can notify real Slack channels without approval actions.                                                                        |
| LEC6 (complete) | Implement durable cancellation and propagation.                                                       | Active and waiting Event v10 runs can be cancelled safely and recovered.                                                                        |
| LEC7 (complete) | Ship direct `list`, `get`, and `cancel` CLI commands.                                                 | Operators manage runs without the `runs` namespace or workflow source files.                                                                    |
| LEC8 (complete) | Harden, migrate, document, benchmark, and publish.                                                    | Lifecycle and Engine Controls become a supported release feature.                                                                               |

### LEC0 — Freeze contracts and reviewed fixtures (completed)

Changes:

- Remove the temporary `.start()` child-result monitor from
  `workflow_calls.rs` and `cli.ts`; keep the delayed example and durable
  inspection test.
- Freeze lifecycle grammar, action order, step filtering, and template rules.
- Freeze Model v11, Event v10, Store v11, lifecycle binding, run-control,
  list/inspection, progress, and notification-host contracts.
- Freeze business-outcome versus lifecycle-health semantics.
- Freeze cancellation races for steps, retries, parallel, approval, managed
  services, Fetch, Workflow Call, and Workflow Start.
- Add reviewed positive and negative fixtures, including a complete event
  sequence for success, failure, and cancellation.

Result:

Frontend, Rust, Bun, notification, CLI, and durable-store work target the same
versioned shapes.

Gate:

Schema validation and deep-equality fixture tests pass. The contracts explicitly
answer when hooks run, what they can see, what they may persist, how they fail,
and how a run becomes terminal.

### LEC1 — Compile lifecycle syntax into Model v11 (completed)

Changes:

- Move lifecycle tags from staged to executable frontend elements.
- Validate order-independent named hooks, singleton rules, action children, step filters,
  Slack informational attributes, templates, references, secrets, and modules.
- Add Model v11 and Script Runtime Bindings v2 TypeScript types.
- Lower deterministic hook/action IDs and source-ordered actions outside the
  business graph.
- Emit Model v11 for lifecycle-bearing workflows and Definition Package v6
  where stored module packages require it. Move the lifecycle-free default to
  v11 in LEC2, when Rust can durably admit Event v10 streams.
- Add editor declarations for the read-only `lifecycle` binding.
- Preserve Models v1-v10 unchanged and executable.

Result:

`woml check` accepts valid lifecycle workflows and reports precise line/column
errors for invalid lifecycle source. The compiled definition matches reviewed
Model v11 fixtures byte-for-byte.

Gate:

Frontend tests cover all hook kinds, nested step discovery, duplicate IDs,
filtering, templates, scripts, modules, secrets, approval notifications versus
informational notifications, and compatibility compilation.

### LEC2 — Build the durable lifecycle and control authority (completed)

Changes:

- Implement Event v10 validation and folding.
- Add business outcome, lifecycle status, hook projections, cancellation state,
  and lifecycle warnings to `RunProjection`.
- Add Store v11 migration and the rebuildable bounded run-summary index.
- Atomically admit hook requests with the events that make them eligible.
- Implement one logical hook identity and one active action-attempt claim.
- Recover requested, running, ambiguous, and completed hook work fail-closed.
- Add Rust list, inspection-v2, and cancellation-authority APIs without yet
  exposing CLI commands.

Result:

Rust can persist, fold, inspect, migrate, and recover lifecycle/control state,
while Models v1-v10 retain their existing event behavior.

Gate:

Event/store tests cover sequence validation, duplicate admission, corruption,
rebuildable summaries, v10-to-v11 migration, unknown future versions, and every
crash boundary from hook request through finalization.

Completed implementation:

- Rust validates Model v11 lifecycle bindings and Event v10 payloads while
  keeping Models v1-v10 and Events v1-v9 on their original behavior.
- `RunProjection` now folds public status, business outcome, lifecycle health,
  cancellation, hooks, action attempts, and safe warnings from events alone.
- Store v11 migrates Store v10, rebuilds bounded run summaries from the event
  log, and updates that cache at the common append boundary.
- Cancellation and outcome decisions atomically admit their eligible hook
  request using deterministic logical identities.
- Recovery fails ambiguous started actions as interrupted and never replays
  them; requested work remains resumable and completed work remains complete.
- Rust exposes safe list, inspection-v2, cancellation, outcome, finalization,
  and summary-rebuild authorities without adding public CLI commands yet.
- New CLI admissions are promoted to Model v11/Event v10. Historical compiler
  fixtures remain immutable, and lifecycle-bearing `woml run` stays explicitly
  gated until LEC3 can execute its actions.

Verification:

- The focused LEC2 Rust suite covers Model/Event/Store versions, atomic and
  duplicate cancellation, atomic outcome-hook admission, lifecycle folding,
  summary rebuild, Store v10-to-v11 migration, lifecycle-free v11
  finalization, and fail-closed crash recovery.
- A native end-to-end `woml run hello.woml` smoke test produced the expected
  result and persisted this Event v10 sequence:
  `run_started`, step attempts, `run_outcome_decided`, `run_finalized`.
- The workspace compiles, the WOML CLI type-checks, lifecycle CLI staging tests
  pass, Store migration compatibility tests pass, and `woml-engine` Clippy is
  clean. One unrelated existing retry test cannot bind `127.0.0.1:0` in the
  restricted test environment; all tests reached before it passed.
- Existing Workflow Start remains asynchronous and reaches a successful child
  result under Model v11 admission. The complete 13-test module compatibility
  suite also passes, including Event v10 branch/parallel/retry composition,
  native Fetch tracking, and source-free durable resume.

### LEC3 — Execute workflow lifecycle scripts (completed)

Changes:

- Add lifecycle execution mode to Rust and the Bun Script Host.
- Inject the reviewed deeply read-only `lifecycle` binding only for lifecycle
  invocations.
- Allow an ignored/undefined lifecycle script return without weakening normal
  step result validation.
- Execute `on-start` before business scheduling.
- Enter finalization after business outcome, then run `on-success`,
  `on-error`, or `on-cancel`, followed by `on-complete`.
- Record action success/failure and preserve the original business outcome.
- Extend safe progress and inspection with lifecycle state.

Result:

A real `.woml` workflow runs workflow-level lifecycle JavaScript through Bun
under Rust supervision and finishes with a truthful business outcome plus
lifecycle health.

Gate:

End-to-end tests cover success, failure, hook script throw, timeout, non-JSON
return, managed services, Fetch, secrets, modules, Worker crash, host crash,
restart, and no `context.steps` mutation.

Implementation result (2026-08-11):

- Script Host Protocol v7 adds an explicit lifecycle execution mode and the
  frozen Lifecycle Binding v1 without changing step invocation behavior.
- Rust admits and executes workflow hooks in durable order, supervises their
  Bun workers, records each action attempt, and finalizes only after the
  outcome hook and `on-complete` settle.
- Lifecycle returns are ignored, including `undefined`; normal business-step
  JSON-result validation remains unchanged.
- Lifecycle scripts receive deeply read-only `context` and `lifecycle`
  bindings plus declared secrets, modules, managed services, and tracked
  Fetch. Managed operations use the lifecycle action's durable identity.
- Hook throws, timeouts, invalid results, Worker/host failures, and interrupted
  attempts fail closed as lifecycle warnings. They do not replace the already
  decided business result.
- Lifecycle Progress v1 is emitted through the existing CLI progress channel,
  and run inspection continues to expose durable hook/action health.
- `examples/lifecycleWorkflow.woml` is the manual end-to-end example. The CLI
  rejected step hooks and lifecycle notifications at the LEC3 boundary.

### LEC4 — Execute step lifecycle hooks

Status: **completed on 2026-08-11.**

Changes:

- Admit `on-step-start` once at the first logical attempt.
- Admit success/failure/complete hooks only at final logical settlement.
- Apply optional compiled step filters.
- Keep downstream DAG scheduling independent from step-hook execution.
- Drain admitted step hooks before workflow outcome hooks and finalization.
- Compose with retries, sequential steps, branches, parallels, approvals, and
  call-only workflows.
- Treat fail-fast sibling cancellation separately from permanent step failure.
- Admit step-hook requests atomically with their first-start or final-settlement
  event, using the frozen run/step/hook identity.
- Execute pending step observers in durable request order, prioritizing them
  ahead of workflow outcome hooks during finalization.
- Build `lifecycle.step`, `lifecycle.failure`, and read-only `context` from the
  durable hook-request snapshot rather than whichever context happens to be
  current when the observer runs.
- Include the step ID in Lifecycle Progress v1 terminal output.
- Allow lifecycle scripts in step hooks through `woml run`; informational
  notification actions through the same Model v11 lifecycle authority used by
  LEC5.

Result:

One workflow-level lifecycle block observes every selected business step without
being copied into each step tag or changing DAG outputs.

Retries produce one logical start and one final settlement observation. Selected
branch and parallel children are correlated independently; unselected routes
produce no hooks. Approval pause/resume drains already admitted observers and
continues without duplicate admission. Hook failures remain warnings and do not
change step output or workflow outcome.

Gate:

Tests prove once-per-logical-step behavior, no hook per retry, deterministic
parallel event correlation, filter correctness, approval pause/resume behavior,
and restart without duplicate hook admission.

Completed gates cover sequential filters and frozen snapshots, retry success,
interrupted-attempt recovery, engine-cancelled sibling classification,
branch-plus-parallel composition, approval pause/resume, CLI admission, and
step-correlated progress output.

### LEC5 — Add informational lifecycle notifications

Status: **completed on 2026-08-11.**

Changes:

- Implement Notification Provider Host v2 informational delivery.
- Reuse Slack Socket Mode transport, channel lookup, secret loading, provider
  retry, idempotency, diagnostics, and message identity storage.
- Render WOML Template v1 from the hook snapshot without persisting the rendered
  message or secret values.
- Support multiple Slack tags/channels in one lifecycle notification.
- Keep approval buttons, capability tokens, decision callbacks, and message
  resolution updates exclusive to approval mode.
- Surface partial provider failure as lifecycle warnings without rewriting the
  business outcome.

Result:

A successful, failed, cancelled, or step-level lifecycle event can send a real
informational Slack message using the existing WOML secret experience.

Gate:

Real-workspace and mock-adapter tests cover multi-channel delivery, duplicate
recovery, message templates, missing scopes, invalid channels, partial success,
provider crash, secret redaction, and approval-contract compatibility.

Completed automated gates cover Provider Host v2 isolation, informational
delivery without approval capabilities, multi-channel delivery, WOML Template
v1 rendering, durable bounded delivery/message identities, partial success,
secret redaction, and Provider Host v1 approval compatibility. The real Slack
smoke-test workflow is `examples/lifecycleSlackWorkflow.woml` and
uses the existing `woml secrets` experience.

### LEC6 — Implement durable run cancellation

Status: complete.

Changes:

- Expose an atomic Rust cancel command over Event v10 runs.
- Stop new nodes/retries after the request sequence wins.
- Route cancellation into active Script Host invocations and managed service
  tokens.
- Signal active pre-outcome lifecycle actions and settle them as cancelled
  lifecycle work before `on-cancel` begins.
- Settle running step, parallel, branch, and retry state truthfully.
- Make waiting approvals inactive and invalidate every decision capability;
  defer visible `cancelled` message updates according to the frozen-contract
  adjustment below.
- Preserve committed work and independent child workflows.
- Execute `on-cancel`, then `on-complete`, and finalize as cancelled.
- Recover cancellation after process crash or target-runtime restart.

Result:

An active or waiting workflow can be cancelled durably without pretending that
external side effects were rolled back.

Implemented behavior:

- Rust observes the durable request while scripts, lifecycle scripts, parallel
  children, and retry waits are active.
- Bun receives the frozen Script Host `run_cancelled` signal, and Rust cancels
  the invocation's managed capability tokens before settling the attempt.
- No new node or retry starts after cancellation wins the durable sequence.
- Active step attempts settle as engine-cancelled; step-complete, on-cancel,
  and on-complete lifecycle work then runs in that order before finalization.
- Waiting approval and Slack decision capabilities become unusable immediately.
- Restart recovery closes active attempts without replay and completes the same
  cancelled outcome.
- Already committed step results and independently started child workflows are
  preserved.

Reviewed contract adjustment:

Event v10 and Notification Provider Host v2 only encode approved, rejected, and
timeout message resolutions. LEC6 therefore invalidates approval capabilities
and treats a waiting request as inactive once its run is cancelling, while
retaining that request's historical `waiting` projection. It deliberately does
not mislabel an existing Slack approval message as rejected. A first-class
approval-cancelled projection and visible `cancelled` message update require an
explicit future Event/Provider protocol version, and remain on the post-LEC7
provider-hardening roadmap rather than silently changing a frozen contract.

Gate:

Race/crash tests cover every boundary before and after step start/success,
operation dispatch, approval wait, retry schedule, child admission, cancellation
request, cancellation signal, hook execution, and finalization.

The completed automated gate covers live script signalling, pre-outcome
lifecycle action settlement, parallel child signalling, retry suppression,
approval credential invalidation, and crash/restart continuation.

### LEC7 — Ship direct run-management commands (completed)

Status: **completed on 2026-08-11.**

Changes:

- Add `woml list`, `woml get`, and `woml cancel` parsers and Rust N-API calls.
- Provide human-readable output and versioned `--json` output.
- Add status/workflow/limit filters to `list`.
- Replace documentation and tests that teach `woml runs get`.
- Remove the `runs` namespace as the canonical CLI surface.
- Print safe cancellation and lifecycle progress in long-lived `woml run`.

Result:

Users and AI agents can discover, inspect, and cancel runs through short,
predictable commands without reopening the source workflow.

Gate:

Packaged CLI tests cover empty stores, filters, limits, JSON schemas, unknown
runs, already-terminal runs, repeated cancellation, shared state across two
processes, active approval cancellation, and redaction.

The completed gate combines packaged CLI coverage for a live two-process
cancellation with the LEC6 Rust approval/cancellation suite. Long-lived
`woml run` now treats an externally cancelled startup run as safe progress and
keeps the automation host active instead of reporting cancellation as a host
failure.

### LEC8 — Harden and publish Lifecycle and Engine Controls (completed)

Status: **completed on 2026-08-11.**

Changes:

- Run adversarial lifecycle identity, cancellation, storage, template, secret,
  notification, crash, and corruption tests.
- Benchmark lifecycle-disabled overhead, hook admission, list/get, cancellation
  detection, and finalization.
- Prove that workflows without lifecycle retain negligible execution overhead.
- Validate clean installation, packaging, schema compatibility, migration,
  shutdown, and recovery.
- Update language, architecture, notification, recovery, workflow-call,
  services, deployment, CLI, and SDK migration documentation.
- Add one Lifecycle and Engine Controls release gate.

Result:

Workflow-wide lifecycle automation and durable local run control are supported
features rather than staged syntax.

Gate:

Frontend, Rust, Bun, protocol/schema, TypeScript, Clippy, integration,
cross-process, approval, notification, crash, migration, packaging, benchmark,
and secret scans pass from a clean project.

The completed `bun run test:lec8` gate builds the release package, runs the
frontend lifecycle contracts, Script Host v7, Notification Provider Host v2,
Rust LEC2-LEC6 authority/runtime suites, direct CLI management, and a clean
consumer install that executes, inspects, cancels, and shuts down. It compiles
all 73 repository schemas together, audits package contents, scans public
artifacts for configured WOML secrets, runs TypeScript and Clippy with warnings
denied for `woml-engine`, and enforces the versioned local benchmark budgets for
lifecycle-disabled overhead, lifecycle finalization, list/get, cancellation
request, and cancellation detection.

## 13. Expected File Areas

| Area                  | Expected locations                                                               |
| --------------------- | -------------------------------------------------------------------------------- |
| Grammar and lowering  | `woml/src/compiler.ts`, `model.ts`, `script-analysis.ts`, `editor.ts`            |
| Model and fixtures    | `docs/schemas/compiled-workflow-model.v11.schema.json`, `woml/tests/fixtures/*`  |
| Event and projection  | `core/woml-engine/src/event.rs`, `projection.rs`, `engine.rs`                    |
| Runtime execution     | `core/woml-engine/src/runtime.rs`, lifecycle-specific Rust module                |
| Durable store         | `core/woml-engine/src/durable.rs`, Store v11 migration/tests                     |
| Script binding        | `woml-cli/src/script-host/worker.ts`, protocol/types/tests                       |
| General notifications | frontend lowering, notification host/protocol, shared Slack adapter              |
| Cancellation          | Rust runtime/host/capability tokens, approval settlement, workflow-call boundary |
| N-API boundary        | `core/src/woml_bridge.rs`                                                        |
| CLI                   | `woml-cli/src/cli.ts`, `rust-executor.ts`, packaged CLI tests                    |
| Contracts             | `docs/schemas/*`, `docs/protocols/*`, reviewed fixtures                          |
| Examples and docs     | new lifecycle/cancellation examples plus language/architecture/operator guides   |

## 14. Verification Matrix

| Area           | Required proof                                                                   |
| -------------- | -------------------------------------------------------------------------------- |
| Grammar        | One lifecycle location, order-independent named hooks, valid actions, and precise diagnostics. |
| Model          | Model v11 keeps lifecycle outside the business DAG and preserves v1-v10.         |
| Hook identity  | One logical subject admits each hook once across retries and crashes.            |
| Retry          | Step hooks observe the logical step, not every failed attempt.                   |
| Parallel       | Out-of-order steps correlate correctly; cancelled siblings are not failures.     |
| Approval       | Waiting runs cancel durably; old decisions cannot win afterward.                 |
| Outcome        | Hook failure never rewrites business success/failure/cancellation.               |
| Finalization   | Step hooks drain before outcome hooks; `on-complete` is last.                    |
| Scripts        | Lifecycle binding is read-only, bounded, and absent from normal scripts.         |
| Notifications  | Informational Slack reuses transport without approval capabilities.              |
| Cancellation   | New work stops, supported active work is signalled, committed work remains.      |
| Workflow calls | Parent cancellation does not silently cancel independent children.               |
| CLI            | Direct list/get/cancel work locally and across a shared state database.          |
| Recovery       | Requested/running/ambiguous hooks and cancellation recover fail-closed.          |
| Migration      | Store v10 data survives Store v11 migration without event rewriting.             |
| Security       | No context, messages, secrets, tokens, or internal identities leak.              |
| Performance    | Lifecycle-disabled workflows have negligible new overhead.                       |
| Packaging      | A clean consumer can run, observe, notify, inspect, and cancel.                  |

## 15. Risks and Guardrails

### Hooks can become hidden business logic

Lifecycle outputs never enter the DAG or context. Documentation directs required
business work into explicit normal steps.

### Step hooks can create an effect storm

Step filters, deterministic identities, bounded actions, provider retries, and
concurrency limits are mandatory. The first profile executes lifecycle actions
through one bounded Rust-owned worker per run rather than spawning unbounded
polling tasks.

### “Complete” can become ambiguous

Event v10 separates business outcome from lifecycle finalization. CLI status
shows `finalizing` until lifecycle work settles, then preserves the original
outcome with optional warnings.

### Cancellation can imply rollback

The CLI and docs say “cancellation requested” until the engine finalizes it.
Committed effects remain committed, and ambiguous external effects remain
visible.

### Informational notify can accidentally inherit approval authority

Provider Host v2 uses a separate informational contract and never receives an
approval token or decision callback.

### Lifecycle context can harden another accidental public API

The lifecycle binding is minimal, versioned, deeply read-only, and frozen in
LEC0. It does not silently expose `context.run` or internal events.

### Direct CLI commands can become an unauthenticated remote control plane

The first profile is local and state-file scoped. Remote control requires
Production Runtime identity, authorization, and audit design.

### Old runs cannot truthfully gain new terminal states

Models v1-v10 retain their frozen event streams. The CLI rejects cancellation
for pre-v11 runs instead of rewriting history or encoding cancellation as a
failure.

## 16. Global Roadmap After Lifecycle and Engine Controls

1. **Fork and Branch Execution** — rename conditional source flow to
   `<choose>` and add durable `<fork>`/`<branch>` routes with multi-step branch
   bodies and selective main-route joins.
2. **Additional Communication Providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when justified.
3. **Retire the JavaScript Chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

Completed roadmap milestones—Retries and Idempotency, Production Triggers,
Services and Capabilities, the essential Module System, Durable Workflow Calls,
Workflow Start, Lifecycle and Engine Controls, Runtime Policies, Durable State,
and Production Runtime and Operations—remain the baseline and are not repeated
as future work.

## 17. LEC0 Review Gate

No lifecycle execution or cancellation code begins until LEC0 answers and tests
these contract questions:

1. Does the exact lifecycle binding expose only the reviewed minimal fields?
2. Are step hooks once per logical step, including retries and parallel
   cancellation?
3. Is lifecycle action order deterministic without blocking the business DAG?
4. Can hook failures be inspected without changing the business outcome?
5. Does Event v10 represent outcome decision and finalization without
   reinterpreting old streams?
6. Is cancellation sequence-authoritative in every success/failure race?
7. Are approval tokens and messages safely invalidated on cancellation?
8. Does informational notification reuse transport without inheriting approval
   authority?
9. Are template interpolation, secret placement, and message limits explicit?
10. Can Store v11 summaries be deleted and rebuilt exactly from events?
11. Are direct CLI results bounded, versioned, redacted, and useful to AI
    agents?
12. Is every open item from `context.run`, durable state, workflow policies,
    and engine-control ownership either resolved here or explicitly deferred?

Once those artifacts are reviewed, LEC1 may begin with frontend compilation.

## 18. Definition of Done

Lifecycle and Engine Controls are complete when a user can:

1. author one workflow-level lifecycle block with workflow and step hooks;
2. run scripts and informational Slack notifications from those hooks;
3. observe exactly one logical hook per eligible step/workflow event;
4. see lifecycle warnings without losing the business outcome;
5. list and inspect durable runs through direct CLI commands;
6. cancel an active or approval-waiting Event v10 run;
7. restart the runtime without losing or duplicating unambiguous lifecycle and
   cancellation work; and
8. pass the packaged release gate from a clean project with no secret or
   context leakage.

The feature is not complete merely because the syntax parses or an in-memory
cancel flag stops one script. Rust events, recovery, inspection, notification,
CLI behavior, migration, and compatibility must agree on the same reviewed
contracts.
