# WOML Notification Providers Implementation Plan

Status: N0 and N1 complete; ready for N2 — Slack, secret, delivery, event, and
provider-host contracts are frozen, and secure secret references plus
`woml secrets` are implemented; Discord, WhatsApp, and generic webhook
notification remain separate later milestones

## 1. Product Outcome

Human Approval must reach reviewers where they already work. The first vertical
milestone sends one approval to one or more Slack channels, accepts a native
Slack Approve or Reject action, records exactly one durable decision in Rust,
continues the selected WOML route, and updates every delivered Slack message.

The intended authoring experience is:

```xml
<approval
  id="publishApproval"
  name="Publish article"
  description="Review the article before publishing"
  timeout="24h"
  on-timeout="reject">
  <notify>
    <slack
      channels="#approvals #engineering"
      bot-token="{{secrets.SLACK_BOT_TOKEN}}"
      app-token="{{secrets.SLACK_APP_TOKEN}}"
    />
  </notify>

  <when-approved>
    ...
  </when-approved>

  <when-rejected>
    ...
  </when-rejected>
</approval>
```

The reviewer sees native Slack Approve and Reject controls. The existing browser
page, local HTTP API, and curl remain development and fallback interfaces.

The Slack adapter ships with WOML; users do not install an npm provider package
and do not configure a `woml connect` abstraction. They create and install a
Slack app from the supplied WOML Slack manifest, store its credentials with
`woml secrets`, and reference those secret names directly in markup.

## 2. Product Principles

1. `<approval>` owns one human decision.
2. `<notify>` describes where that one decision is delivered.
3. Multiple Slack channels and Slack tags never create multiple approvals.
4. The first valid decision from any delivered Slack message wins atomically.
5. Rust remains the only authority that records the decision and selects a
   route.
6. The Slack adapter sends messages, authenticates interactions, and translates
   them to `approved` or `rejected`; it never executes workflow routes.
7. Secret values never appear in `.woml`, compiled models, context, events,
   SQLite workflow history, terminal output, errors, or provider-visible
   payloads. Secret names may appear as typed symbolic references.
8. Provider delivery and provider-message updates are external effects and
   require durable idempotency and recovery behavior.
9. `{{secrets.NAME}}` is a runtime-only secret reference, never string
   interpolation and never durable workflow data.
10. The first profile exposes secrets only to reviewed secret-bearing tag
    attributes. It does not add a `secrets` JavaScript global.

## 3. One Approval, Many Deliveries

One approval request may have several Slack delivery records:

```text
                         +-- #approvals delivery + message identity
one approval request ---+-- #engineering delivery + message identity
                         +-- #management delivery + message identity
                                      |
                                      v
                            one durable resolution
```

Every channel has its own durable delivery identity, decision capability, and
Slack message identity. Every delivery binds to the same
`(runId, approvalId, requestId)`. Channels under one `<slack>` reuse the same
Slack bot/app secret references; Slack credentials belong to the app/workspace,
not to an individual channel.

The shared resolution rules are:

- the first valid `approved` or `rejected` decision commits exactly one
  `approval_resolved` event;
- the selected route executes exactly once;
- an identical later decision is idempotent;
- an opposite later decision reports that another decision already won;
- the shared timeout applies to every delivery;
- after resolution, WOML asks the Slack adapter to disable or update every
  successfully delivered message;
- a message-update failure is reported and retried, but never reverses the
  durable human decision; and
- notification credentials remain delivery capabilities, not workflow data.

## 4. Proposed Source Syntax

`<notify>` is an optional child of `<approval>` before the two decision arms.
The first executable profile accepts one or more `<slack>` children only.

```xml
<approval id="releaseApproval" timeout="2h" on-timeout="reject">
  <notify>
    <slack
      channels="#release-approvals #engineering-leads"
      bot-token="{{secrets.COMPANY_SLACK_BOT_TOKEN}}"
      app-token="{{secrets.COMPANY_SLACK_APP_TOKEN}}"
    />

    <slack
      channels="#customer-approvals"
      bot-token="{{secrets.CUSTOMER_SLACK_BOT_TOKEN}}"
      app-token="{{secrets.CUSTOMER_SLACK_APP_TOKEN}}"
    />
  </notify>

  <when-approved>...</when-approved>
  <when-rejected>...</when-rejected>
</approval>
```

One `<slack>` targets one Slack credential set and one or more channels in that
workspace. A second `<slack>` is used for another workspace or app installation.
Tag order and channel order are preserved for source diagnostics and
deterministic delivery identity; deliveries may still execute concurrently.

The `channels` attribute is an HTML-like whitespace-separated list. It must
contain at least one unique channel token. The compiler lowers it to an ordered
array. Repeated whitespace is insignificant and duplicate channels within the
same Slack credential set are rejected.

The first grammar accepts channel aliases such as `#approvals` and Slack
conversation IDs. N0 freezes the exact identity grammar and normalization rules.

The initial notification message is generated from the approval `name`,
`description`, workflow identity, and deadline. Custom templates, arbitrary
message bodies, comments, attachments, quorum, and provider-specific forms are
deferred so the first profile stays predictable.

The Slack v1 attributes are:

| Attribute   | Required | Meaning                                                              |
| ----------- | -------: | -------------------------------------------------------------------- |
| `channels`  |      Yes | Ordered whitespace-separated channel destinations.                   |
| `bot-token` |      Yes | Exact `{{secrets.NAME}}` reference to the installed Slack bot token. |
| `app-token` |      Yes | Exact `{{secrets.NAME}}` reference to the Socket Mode app token.     |

Literal credential values, partial interpolation, `{{context...}}`, and
`{{services...}}` are invalid for secret-bearing attributes. Custom message
templates, arbitrary bodies, comments, attachments, and per-channel overrides
are deferred so the first message shape stays predictable.

## 5. Secret Reference and Storage Contract

The accepted attribute form is exactly:

```xml
bot-token="{{secrets.SLACK_BOT_TOKEN}}"
```

Secret names use `[A-Z][A-Z0-9_]*`. A secret reference lowers to a typed symbolic
value, never the secret value:

```json
{
  "kind": "secretReference",
  "name": "SLACK_BOT_TOKEN"
}
```

The initial CLI surface is:

```bash
woml secrets set SLACK_BOT_TOKEN
woml secrets list
woml secrets delete SLACK_BOT_TOKEN
```

`set` reads the value through a hidden interactive prompt and never accepts a
plaintext value as a normal command argument. `list` returns names and metadata
only. CI receives a reviewed non-interactive secret-provider path without
changing the WOML file.

The secret subsystem has one interface with environment-specific backends:

- local development uses the operating-system credential store;
- CI may resolve reviewed environment/secret-manager bindings;
- self-hosted and managed deployments may provide their own secure backend; and
- WOML never silently falls back to a plaintext project file.

Before any workflow effect starts, `woml run` preflights that all referenced
secret names exist and that each reference appears only in an allowed sink. The
Slack adapter resolves values again at delivery time so rotation does not
require recompiling the workflow.

Secret values are delivered directly from the runtime secret resolver to the
Slack adapter. Rust may schedule a typed secret reference but never receives,
persists, folds, logs, or returns the resolved value. `context`, events, outputs,
diagnostics, and definition hashes contain no secret values.

The name `secrets` is reserved for WOML attribute references now. Direct access
as `secrets.NAME` inside `<script>` remains deferred until services and secret
exfiltration/persistence behavior are reviewed.

## 6. Provider Adapter Boundary

The core must not contain Slack API or secret-store rules. The frontend lowers
each `<slack>` tag and its ordered channels into versioned generic notification
operations. The built-in Slack adapter is responsible for:

- resolving only the typed bot/app secret references assigned to it;
- validating Slack credentials and channel destinations;
- sending an approval message with native decision controls;
- returning a durable provider message identity;
- receiving Slack interactions through Socket Mode and acknowledging them;
- translating an interaction to the shared WOML decision contract;
- updating or disabling the provider message after resolution; and
- returning structured, secret-safe failures.

The Bun provider host owns the secret resolver and Slack protocol integration.
It receives symbolic references from Rust, resolves values only for the adapter
invocation, and must redact them from every result and error. Slack interaction
payloads carry an opaque per-delivery approval capability, never provider
credentials or workflow context.

Rust owns:

- durable delivery intent and state;
- delivery attempt identity and idempotency keys;
- one delivery capability per channel bound to one approval request;
- the atomic first-decision-wins transaction;
- timeout races;
- workflow continuation; and
- recovery scheduling.

The exact provider-host protocol, secret-resolution request, compiled-model
version, event version, and idempotency-key derivation are frozen in N0 rather
than selected implicitly during implementation.

## 7. Delivery and Failure Policy to Review in N0

The proposed v1 default is:

1. WOML durably creates one approval request and one delivery intent per Slack
   channel after expanding each ordered `channels` list.
2. Channel deliveries may execute concurrently within the reviewed Slack rate
   and concurrency limits.
3. At least one delivery must succeed.
4. If one channel succeeds and another fails, the workflow remains waiting and
   reports the failed channel without losing the valid delivery.
5. If every delivery fails after the approved retry policy, WOML produces an
   explicit notification-delivery failure instead of silently waiting for a
   reviewer who was never reached.
6. A recovered delivery attempt uses the frozen idempotency contract so a crash
   does not silently send a duplicate Slack message.
7. Resolution-message updates are durable follow-up work. They are retried but
   cannot undo the decision.

N0 must decide whether an all-delivery failure fails the run immediately or
enters a recoverable operational state. No implementation phase may choose that
silently.

## 8. Implementation Phases

### Prerequisite — Human Approval A7 — complete

Completed proof:

- Approval contracts, durable waiting, atomic resolution, timeout, recovery,
  composition, local HTTP fallback, and clean packaging are complete.
- Rust is the only decision and route-selection authority before Slack effects
  are introduced.

Result:

The shared decision authority is stable before adding provider effects.

### N0 — Freeze Slack, secret, delivery, and event contracts — complete

Changes:

- Freeze `<notify>` placement and the Slack-only executable child profile.
- Freeze `channels` tokenization, normalization, ordered expansion, duplicate
  detection, and delivery identity.
- Freeze exact `bot-token` and `app-token` typed secret-reference inputs.
- Freeze secret-name grammar, allowed sinks, local/CI backend interface,
  preflight behavior, redaction, and secret-host boundary.
- Freeze the compiled notification representation and versioning decision.
- Freeze notification delivery, attempt, success, failure, message-update, and
  recovery events with a schema version.
- Freeze the provider-host and Slack adapter request/result/error contracts.
- Freeze per-delivery credential binding and redaction rules.
- Freeze delivery and message-update idempotency-key derivation.
- Freeze at-least-one-delivery behavior and the all-delivery-failed result.
- Freeze Socket Mode ownership, acknowledgement, action routing, credential
  rotation, and message-update behavior.
- Add reviewed single-channel, multi-channel, multi-workspace, partial-failure,
  all-failure, decision-race, secret-redaction, and recovery fixtures.

Result:

Every layer targets one reviewed Slack-first delivery and secret contract before
provider code is written.

Gate:

The fixtures prove expanded channel deliveries converge on one approval request
and one resolution, while only secret names—not values—appear in the compiled
model and no secret value enters an event or public artifact.

Completed proof:

- Compiled Workflow Model v5, Run Event v5, and Notification Provider Host v1
  are frozen as separate JSON Schemas without modifying versions 1–4.
- `docs/protocols/notification-contracts-v1.md` freezes source identity,
  secret boundaries, UTF-8 byte framing, multiplexing, retry/idempotency,
  Socket Mode ownership, reviewer audit, partial failure, and all-failure
  behavior.
- `docs/protocols/notification-diagnostics-v0.1.md` freezes N0/N1 codes and
  reserves the N2–N6 source/runtime codes.
- The reviewed model fixture expands `#approvals #engineering` into two stable
  deliveries while retaining only `SLACK_BOT_TOKEN` and `SLACK_APP_TOKEN`
  symbolic names. Its canonical definition hash is
  `sha256:a02f094f7200f0e7e33bef7de2aba9b52638ac24adb9f017fd292764fbcb6988`.
- Partial-success and all-delivery-failed event histories prove that one
  successful destination keeps the approval waiting while zero successful
  destinations fails explicitly with `WOML_NOTIFICATION_DELIVERY_FAILED`.
- Provider-host fixtures cover ready, delivery, out-of-order correlation,
  structured failure, decision interaction, message update, multibyte UTF-8,
  and literal CRLF content without carrying resolved provider credentials.
- AJV conformance tests reject literal credentials, malformed delivery models,
  secret-bearing events, and unversioned provider messages.

### N1 — Build `{{secrets.NAME}}` and `woml secrets` — complete

Changes:

- Add the exact whole-attribute secret-reference grammar and typed frontend
  representation.
- Reject literal values, interpolation, invalid names, unsupported sinks, and
  missing secrets with safe source-located errors.
- Add the versioned `SecretStore` interface and the secure local backend without
  a plaintext fallback.
- Implement hidden-prompt `woml secrets set`, metadata-only `list`, and
  destructive-but-explicit `delete`.
- Add the reviewed CI secret-provider binding and ensure values never enter
  command arguments.
- Preflight required secret names before any workflow effect starts.
- Add secret rotation, absence, backend-unavailable, redaction, and artifact
  scanning tests.

Result:

WOML can reference secrets safely without placing values in source, compiled
models, context, events, or logs.

Gate:

Interactive local secret management and the CI backend pass the same conformance
suite, and no test can recover a plaintext value from durable/public artifacts.

Completed proof:

- The WOML frontend exports an exact typed `SecretReferenceExpression`, freezes
  `[A-Z][A-Z0-9_]*`, and rejects interpolation, malformed references, literals,
  and unsupported secret sinks with source locations and secret-safe messages.
- `woml secrets set NAME` reads only from a hidden interactive TTY prompt;
  plaintext values are rejected as command arguments. `list` prints names,
  backend, and update metadata only, and `delete` is explicit.
- The versioned local store uses Bun's OS-native Keychain/libsecret/Windows
  Credential Manager API under `dev.woml.cli.secrets.v1`; the CLI requires Bun
  1.3.14 or newer and has no plaintext-file fallback.
- CI selects the explicit read-only provider with
  `WOML_SECRETS_PROVIDER=env` and binds `WOML_SECRET_<NAME>` values. There is no
  automatic environment fallback during local development.
- Unique symbolic references can be preflighted before effects without
  returning their values. Empty values and values over 2048 UTF-8 bytes fail
  safely.
- Frontend, store, CLI, CI-provider, rotation, deletion, absence, redaction,
  typecheck, existing workflow regression, local approval HTTP, and clean
  packaged CLI tests pass.
- `<notify>` deliberately remains `WOML_FEATURE_NOT_EXECUTABLE`; N1 validates
  its secret-bearing attributes but N2 alone may validate and lower the Slack
  structure into Model v5.

### N2 — Validate and lower `<notify><slack>`

Changes:

- Validate `<notify>` order and placement inside `<approval>`.
- Validate Slack-only children, exact attributes, secret-reference types,
  non-empty channel lists, channel identity grammar, and duplicates at their
  original source locations.
- Expand one Slack tag into one deterministic delivery definition per channel
  while retaining common secret references and source ownership.
- Lower Slack markup into provider-neutral delivery operations without teaching
  Rust WOML syntax or Slack API rules.
- Extend graph, namespace, reachability, and model validation for notification
  operations.
- Keep Discord, WhatsApp, webhook, executable `<notify><script>`, literal
  credentials, and direct script secret access explicitly non-executable.

Result:

The agreed Slack markup becomes a deterministic engine-ready model with one
delivery per authored channel.

Gate:

Exact fixtures and source diagnostics pass for one channel, many channels,
multiple Slack workspaces, nested approvals, duplicates, and invalid secrets.

### N3 — Add durable delivery and recovery infrastructure

Changes:

- Create one durable delivery intent and separate capability credential per
  expanded Slack channel.
- Add an outbox-style dispatcher so recorded delivery work is not lost between
  transaction commit and provider execution.
- Add delivery attempts, idempotency, retry scheduling, partial/all failure
  aggregation, and minimal Slack message identity persistence.
- Add durable post-resolution message-update work.
- Recover safely before send, during an uncertain send, after provider success,
  after the shared decision, and during message update.
- Prove provider delivery never starts a workflow route and provider failure
  never fabricates a human decision.

Result:

Notification effects survive crashes and multiple deliveries remain one
approval.

Gate:

Deterministic fake-Slack tests prove no lost delivery intent, no unreviewed
duplicate send, one shared resolution, deterministic partial failure, and safe
recovery.

### N4 — Build the Slack provider host and conformance adapter

Changes:

- Add the versioned Bun provider-host boundary without putting Slack code in
  Rust.
- Resolve symbolic secret references only inside the Slack adapter invocation.
- Implement Slack request/result/error validation and complete redaction.
- Add a fake Slack transport with native-message, action, and update behavior.
- Route fake button actions through the existing Rust approval decision
  authority and never directly into workflow execution.
- Reuse one provider-host/Socket ownership record for deliveries sharing an app
  token while retaining separate channel delivery identities.
- Add contract tests for secret resolver failures, malformed adapter responses,
  late/duplicate actions, timeout, cancellation, and host crashes.

Result:

The complete Slack lifecycle works against a deterministic adapter before any
real external effect is trusted.

Gate:

Fake-Slack journeys complete send, click, Rust resolution, selected-route
continuation, and all-message update with no secret value crossing the frozen
boundary.

### N5 — Implement real Slack Socket Mode end to end

Changes:

- Ship a reviewed Slack app manifest enabling the minimum bot scope,
  interactivity, and Socket Mode configuration needed by WOML.
- Document the one unavoidable external setup: create the Slack app from the
  manifest, install it in a workspace, and store the resulting bot/app tokens
  with `woml secrets set`.
- Open and supervise the Socket Mode connection without requiring a public
  callback URL or `woml serve` for the local CLI profile.
- Send Block Kit approval messages with Approve and Reject actions to every
  expanded channel delivery.
- Bind every action to its delivery capability and pass the decision to Rust's
  existing atomic first-decision-wins operation.
- Persist only the channel/message identity needed for recovery and updates.
- Update or disable every delivered Slack message after human decision or
  timeout.
- Implement rate-limit, permission, channel-not-found, token-expiry, Socket
  reconnect, duplicate action, and safe error behavior.

Result:

A reviewer can approve or reject a real WOML workflow entirely inside Slack,
including when one tag targets several channels.

Gate:

The packaged product completes real send, click, Rust resolution,
selected-route continuation, and all-message convergence without a public
callback URL or exposed raw decision URL.

### N6 — Harden, package, and close the Slack milestone

Changes:

- Test approval notification at root, branch-arm, nested, and
  parallel-adjacent placements.
- Test one channel, many channels, multiple Slack tags, two credential sets,
  partial delivery failure, total failure, timeout, simultaneous decisions,
  and repeated actions.
- Test recovery before send, during uncertain send, after message creation,
  after decision commit, during selected-route execution, and during message
  update.
- Prove channel-order determinism, one shared resolution, one selected route,
  and eventual message convergence.
- Scan compiled models, events, context, SQLite/WAL files, logs, diagnostics,
  N-API/provider messages, package contents, and snapshots for secret values.
- Run frontend, Rust, provider host, CLI, typecheck, Clippy, clean-package, and
  real Slack acceptance verification without skipped native/provider tests.
- Document Slack app installation, channel permissions, secret management,
  rotation, CI setup, operational failures, and browser/API fallback.

Result:

Slack notification becomes a supported and publishable WOML feature. Discord
and WhatsApp remain unimplemented and cannot silently compile as Slack aliases.

Gate:

`woml run` from a clean installation sends one real approval to all authored
Slack channels, any one action resolves exactly once through Rust, every message
reflects the final state, and no secret value leaks.

## 9. Verification Matrix

| Area                | Required proof                                                                                           |
| ------------------- | -------------------------------------------------------------------------------------------------------- |
| Syntax              | Valid Slack/secret markup compiles; invalid placement and attributes retain source location.             |
| Shared decision     | Any delivered Slack message may win; exactly one resolution and route execution occur.                   |
| Multiple delivery   | One delivery per expanded channel with deterministic identity and separate capability.                   |
| Partial failure     | At least one successful delivery keeps the approval reachable and visible.                               |
| Total failure       | Every failed delivery produces the frozen explicit operational result.                                   |
| Idempotency         | Retry and recovery do not silently duplicate provider messages.                                          |
| Recovery            | Send, success, decision, and message-update crash boundaries reopen safely.                              |
| Timeout             | One timeout resolves every provider view consistently.                                                   |
| Secrets             | `set/list/delete`, local/CI backends, preflight, rotation, allowed sinks, and no plaintext leakage work. |
| Authentication      | Forged provider interactions fail before Rust decision resolution.                                       |
| Message convergence | Every delivered message is updated or carries a visible update failure.                                  |
| Redaction           | Secrets and plaintext runtime credentials never enter durable/public artifacts.                          |
| Compatibility       | Existing approval workflows without `<notify>` keep the local fallback behavior.                         |
| Packaging           | The built-in Slack adapter, app manifest, secret tooling, and provider host ship cleanly.                |

## 10. Explicit Non-Goals

This milestone does not add:

- generic `<webhook>` notification delivery;
- arbitrary third-party provider plugins;
- literal secret values or partial secret interpolation inside WOML attributes;
- a `woml connect` command or named/default connection abstraction;
- Discord or WhatsApp execution in the first Slack milestone;
- direct `secrets.NAME` access inside JavaScript;
- custom message templates or arbitrary executable notification scripts;
- reviewer comments, attachments, custom forms, or multiple decision values;
- quorum, voting, delegation, reassignment, or escalation;
- production RBAC, SSO, organization policy, or a complete approval inbox;
- email, generic SMS, or push-notification providers;
- state-changing GET links;
- provider-owned workflow route execution; or
- a requirement that ordinary hosted users manually run `woml serve`.

These require separate reviewed milestones.

## 11. Open Decisions Carried Into N0

N0 must explicitly settle:

- the compiled-model and event-version increments;
- whether all-delivery failure fails the workflow or enters a recoverable
  operational state;
- delivery and message-update retry limits and backoff;
- exact delivery/update idempotency keys and uncertain-effect recovery;
- exact Slack channel alias/ID grammar and normalization;
- local operating-system credential-store behavior across Linux, macOS, and
  Windows, including headless-machine errors;
- exact CI secret-provider binding and environment selection;
- provider-host secret-resolution framing and lifetime;
- the minimum stored provider message identity;
- Socket Mode lifecycle, reconnect ownership, and connection reuse;
- how reviewer identity is represented without silently defining production
  authentication/RBAC;
- whether post-resolution update failures appear only operationally or through a
  versioned workflow inspection surface; and
- token revocation and retention after every delivery converges.

No implementation phase may resolve these with an undocumented default.

## 12. Approval-Notification Roadmap After Slack

Slack is the only provider authorized by N0–N6. After the Slack milestone is
complete and reviewed, the remaining work in the Human Approval notification
scope is:

1. **Discord milestone** — freeze Discord secrets, installation, interaction
   transport, message identity, and signature behavior against the existing
   provider-neutral delivery conformance suite; then implement it end to end.
2. **Shared-provider milestone** — allow Slack and Discord deliveries on one
   approval and prove either may win while every message converges on one Rust
   decision.
3. **WhatsApp milestone** — review Meta onboarding, templates, phone/recipient
   identity, webhook authentication, and message-update limitations before any
   WhatsApp tag becomes executable.
4. **Generic webhook notification** — design only after signed delivery,
   callback authentication, metadata, retries, and secret payload policy are
   separately frozen.

No later provider may require changing the approval decision output,
`approval_resolved` authority, or selected-route semantics established by the
Slack milestone.

This provider roadmap is not the complete WOML product roadmap. It finishes the
notification side of Human Approval; it does not replace the larger engine and
language roadmap below. Generic webhook notification may be scheduled later
with services and capabilities and does not need to block unrelated WOML work.

## 13. Global WOML Roadmap After Approval Notifications

The complete product direction, carried forward from Section 15 of
`WOML Human Approval Implementation Plan.md`, is:

```text
Human Approval (complete)
        |
        v
Approval notifications
  Slack -> Discord -> shared providers -> WhatsApp
        |
        v
Retries and idempotency
        |
        v
Production triggers
        |
        v
Services and capabilities
        |
        v
Lifecycle and engine controls
        |
        v
Production runtime and operations
        |
        v
JavaScript SDK migration and retirement
```

The stages after approval notifications are:

1. **Retries and idempotency** — freeze idempotency-key derivation, duplicate
   effect behavior, retryable failures, durable scheduling, and backoff before
   enabling `retry` values greater than one.
2. **Production triggers** — implement webhook first, followed by schedule,
   interval, and event triggers with complete payload, validation, delivery,
   and failure contracts.
3. **Services and capabilities** — add registered HTTP, database, messaging,
   and other provider operations. Reuse the secret-reference boundary without
   persisting service clients or resolved secret values. Generic notification
   webhooks can be considered here once their security contract is frozen.
4. **Lifecycle and engine controls** — add lifecycle hooks, workflow
   cancellation, durable user state, and other engine-owned operations with
   explicit events and race behavior.
5. **Production runtime and operations** — add long-lived hosting,
   multi-workflow registration, deployment configuration, observability,
   retention, and later distributed queue and worker ownership.
6. **SDK migration and retirement** — publish migration tooling and remove the
   old JavaScript chaining SDK only after WOML reaches the agreed feature parity
   and existing users have a supported migration path.

This ordering preserves the overall destination: WOML becomes the primary
authoring language, the Rust engine remains the durable execution authority,
and the old JavaScript chaining SDK is retired only at the end of a safe
migration—not immediately after Slack ships.
