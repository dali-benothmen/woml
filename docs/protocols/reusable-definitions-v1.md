# WOML Reusable Definitions Contracts v1

Status: published after SCP8; frontend, Rust, Bun hosts, durable recovery,
inspection, and clean-package gates implement this contract.

## Frozen boundaries

The frontend accepts exactly three document profiles:

```text
<woml> <imports>? <workflow> </woml>
<woml> <imports>? <props>? <step> <lifecycle>? </woml>
<woml> <imports>? <props>? <provider kind="notification"> <lifecycle>? </woml>
```

`<woml>` has no attributes. A document contains exactly one workflow, reusable
step, or notification provider. `<props>` is forbidden everywhere in a
workflow document. Reusable definitions are dependencies, not runnable
workflows; attempting to run one fails with `WOML_DEFINITION_NOT_RUNNABLE`.

Local `.woml` imports reuse `<module name="..." from="..." />`. Their aliases
are lowercase kebab-case and become empty custom tags. `.js` and `.ts` imports
retain lower-camel aliases beneath `services`. Workflow documents cannot be
imported; independently running workflows communicate through
`services.workflows.call()` and `services.workflows.start()`.

Props are declared in source order with `name`, optional `required`, and
optional `secret`. Names are kebab-case and map deterministically to lower-camel
JavaScript keys. Optional props are omitted. Non-secret props accept a literal
or one exact `context.payload`/`context.steps` reference. Secret props accept
only one symbolic `{{secrets.NAME}}` reference. Resolved secrets are never
compiled, persisted, inspected, or logged.

Reusable steps receive deeply read-only `props`, `context`, `attempt`, and
`services`. Providers receive `props`, `notification`, `attempt`, and
`services`. Neither receives a general `secrets` object. Reusable lifecycle
actions receive `props`, `lifecycle`, `services`, and `context` only for step
definitions. Allowed reusable hooks are `on-success`, `on-error`, and
`on-complete`, in that order. Hook failures are durable warnings and do not
replace the operation outcome. Reusable lifecycle actions are `<script>` only
in v1. `<notify>` remains available in workflow lifecycle hooks, including with
an imported custom provider, but is rejected inside a reusable definition
lifecycle because Model v14 has no frozen recoverable payload for that scope.

## Versioned artifacts

- Compiled Workflow Model v14 adds exact-string choices and reserves reusable
  invocation descriptors on top of the Model v13 DAG.
- Definition Package v9 pins every imported WOML definition and transitive
  JS/TS artifact.
- Reusable Script Binding v3 freezes invocation-local `props` without changing
  ordinary Script Binding v1 or Workflow Lifecycle Binding v2.
- Custom Notification Provider Protocol v1 is asynchronous and multiplexed.
  Requests and responses correlate by `invocationId`; completion order is not
  significant. Content-Length counts UTF-8 bytes.
- Run Event v13 adds reusable lifecycle progress. Custom step attempts and
  provider deliveries keep the existing step/delivery event authorities.
- Run Inspection v5 adds bounded, redacted reusable invocation status.
- Store v14 remains sufficient. Immutable packages fit the existing definition
  record and Event v13 fits the append-only event record; all new inspection
  state is a rebuildable projection. There is no Store v15 migration.

Historical Model v1-v13, Event v1-v12, Definition Package v1-v8, Inspection
v1-v4, and Store v1-v14 bytes remain immutable.

## Provider delivery and approval

A custom provider is one supervised logical delivery. Rust owns its delivery
ID, idempotency key, retry decision, cancellation, recovery, and approval
capabilities. Bun executes the frozen script. The provider returns `{}` or a
bounded `{ "messageId": "..." }` receipt; arbitrary response bodies are
rejected.

Approval providers receive opaque single-use approve and reject URLs belonging
to the same durable approval request. Each delivery has a separate capability,
but the first accepted decision resolves the shared approval. Informational
notifications receive no actions. Custom message edits after a decision are
outside v1.

The v1 failure kinds are `script_threw`, `timed_out`, `cancelled`, `non_json`,
`worker_crashed`, `host_crashed`, `context_too_large`, `result_too_large`,
`delivery_ambiguous`, `service_failed`, and `request_invalid`. The size-limit
mechanism is frozen; byte values remain deployment configuration. A host crash
with ambiguous external delivery fails closed unless the managed service has a
reviewed idempotent replay contract.

## Switch contract

`<switch>` is an official tag, not a custom structural component. It compares
one exact context reference against one or more unique, case-sensitive string
cases and one final default. It lowers to the existing choice authority and
records `choice_selected`; no new switch event exists. An ID-less switch is
control-only. An ID-bearing switch requires one final `<result>` in every arm
and publishes one path-stable result. Runtime execution starts in SCP2.

## Recovery and cancellation ordering

The definition digest and package root hash select the exact bytes used for
recovery; current project files are never reopened to replace them. A completed
operation is not replayed. An interrupted ambiguous effect follows the existing
fail-closed rule.

For success: commit operation success, run `on-success`, run `on-complete`, then
release downstream work. For permanent failure: commit operation failure, run
`on-error`, run `on-complete`, then settle workflow failure. On cancellation,
the normal cancellation outcome is authoritative; `on-error` does not run and
`on-complete` runs only within the existing bounded shutdown budget.

Diagnostics retain code, file, line, column, message, optional hint, and an
import chain. No diagnostic contains resolved props, secrets, messages, action
URLs, provider bodies, or stack traces.
