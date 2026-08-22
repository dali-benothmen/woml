---
name: woml
description: Create, explain, validate, and repair WOML workflow automation files. Use when a task involves `.woml` files, WOML triggers, steps, embedded JavaScript, context references, control flow, approvals, lifecycle hooks, services, providers, secrets, local modules, or WOML CLI operations.
---

# WOML Workflows

Build readable, executable WOML workflows from the user's automation goal.

## Authoring workflow

1. Identify the trigger, input payload, required steps, external systems, and expected final result. Ask only when a missing product decision would materially change the workflow.
2. Inspect nearby `.woml` files and project modules when modifying an existing project. Preserve its naming and credential conventions.
3. Choose the smallest WOML structure that expresses the automation clearly. Keep ordinary sequencing in `<steps>` and add control-flow elements only when the behavior needs them.
4. Put business logic in `<script>` as ordinary asynchronous JavaScript. Do not wrap it in a function and do not add CDATA.
5. Give every workflow and executable item a stable, descriptive ID. Add `name` and `description` when they improve terminal output for a real user.
6. Return compact JSON-compatible values from steps. Persist only data that downstream steps need.
7. Put credentials in WOML secrets and reference them symbolically. Never place literal credentials in source.
8. Run `woml check <path>` after creating or changing a workflow when the CLI is available. Repair source-located diagnostics before presenting the result.
9. Activate or trigger the workflow only when the user requested execution. `woml run` is a long-lived automation runtime, not merely a compile check.

## References

Read only the references needed for the request:

- Read [references/tags.md](references/tags.md) before creating or structurally editing any `.woml` document. It is the complete released tag and attribute vocabulary.
- Read [references/context-and-scripts.md](references/context-and-scripts.md) when writing JavaScript, declarative references, lifecycle actions, or reusable-definition code.
- Read [references/services.md](references/services.md) before calling a built-in service or choosing where workflow data belongs.
- Read [references/modules.md](references/modules.md) when importing JS/TS, defining reusable custom steps, or creating custom notification providers.
- Read [references/providers.md](references/providers.md) for Slack, Telegram, Discord, WhatsApp, approval notifications, lifecycle notifications, or provider messaging.
- Read [references/cli.md](references/cli.md) before giving operational commands or invoking the CLI beyond an ordinary `woml check`.
- Read [references/diagnostics.md](references/diagnostics.md) when validation, execution, providers, services, or runtime operations fail.
- Read [references/patterns.md](references/patterns.md) when choosing between sequential steps, parallel work, conditional routing, forks, modules, workflow calls, or data stores.

## Essential invariants

- A runnable document is `<woml>` containing optional `<imports>` and exactly one `<workflow>`.
- Workflow child containers may be ordered freely; prefer `<config>`, `<lifecycle>`, `<triggers>`, then `<steps>` for readability.
- Workflow IDs use lowercase kebab-case. Trigger, step, choice, switch, parallel, and approval IDs use JavaScript-safe lower camel case and must remain stable.
- Use `context.payload` for trigger or parent-workflow input and `context.steps.<id>` for completed outputs. Do not author new code with the deprecated `context.trigger` alias.
- `context.run` is not a public WOML binding.
- Declarative references use exact syntax such as `{{context.steps.validate.approved}}`. They are not JavaScript expressions.
- Complex conditions belong in a named script step that returns a boolean; `<when test>` consumes that boolean reference.
- A result-producing `<choose>` or `<switch>` publishes one predictable value at `context.steps.<controlId>` through an explicit `<result>` in every arm.
- `<for-each>` iterates an exact array reference durably. Inside its body, use `context.item` and `context.iteration`; publish one ordered per-item value with a final `<result>`.
- Retry is a `<step>` attribute, never a `<retry>` tag.
- Local JS/TS modules use named exports and appear as `services.<moduleName>`. Pass `context`, `attempt`, and secret values from the calling script because modules receive only `services` automatically.
- Prefer `services.http.request()` for managed HTTP behavior; native `fetch()` remains available when Fetch compatibility or streaming matters.
- Use `<for-each>` when every item needs durable attempts, bounded concurrency, recovery, or inspection. Use an ordinary JavaScript loop for small, pure in-step transformations.
- Do not invent elements, attributes, service methods, context fields, or provider configuration. Validate unfamiliar surfaces against the project's current WOML documentation.

## Output expectations

When asked to create a workflow:

- write or update the requested `.woml` file;
- add a small local JS/TS module only when reuse or testability justifies it;
- explain required secrets and the exact commands to configure them;
- provide the exact `woml check` and `woml run` commands; and
- explain how to trigger the workflow, including the generated HTTP route or provider setup when applicable.

Do not claim a workflow works merely because it looks valid. Report whether it was checked, executed, or left untested.
