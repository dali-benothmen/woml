# WOML Notification Providers Implementation Plan

Status: planned — begins after Human Approval A7; no notification provider is
executable yet

## 1. Product Outcome

Human Approval must reach reviewers where they already work. The workflow
author describes the delivery channels inside `<notify>`, while WOML handles
credentials, delivery reliability, provider interactions, one shared decision,
and workflow continuation.

The intended authoring experience is:

```xml
<approval
  id="publishApproval"
  name="Publish article"
  description="Review the article before publishing"
  timeout="24h"
  on-timeout="reject">
  <notify>
    <slack channel="#approvals" />
    <discord channel="content-review" />
    <whatsapp recipient="contentManager" />
  </notify>

  <when-approved>
    ...
  </when-approved>

  <when-rejected>
    ...
  </when-rejected>
</approval>
```

The reviewer sees provider-native Approve and Reject controls. A browser page,
local HTTP API, and curl remain useful development and fallback interfaces, but
they are not the primary production notification experience.

Webhook notification tags are explicitly deferred from this milestone. The
first milestone covers built-in provider tags only.

## 2. Product Principles

1. `<approval>` owns one human decision.
2. `<notify>` describes where that one decision is delivered.
3. Multiple provider tags never create multiple approvals.
4. The first valid decision from any provider wins atomically.
5. Rust remains the only authority that records the decision and selects a
   route.
6. Provider adapters send messages, authenticate provider interactions, and
   translate them to `approved` or `rejected`; they never execute workflow
   routes.
7. Provider credentials never appear in `.woml`, compiled models, context,
   events, terminal errors, or provider-visible payloads.
8. Provider delivery and provider-message updates are external effects and
   require durable idempotency and recovery behavior.
9. The workflow language describes product intent; deployment-specific secrets
   and callback configuration remain outside the workflow definition.

## 3. One Approval, Many Deliveries

One approval request may have several delivery records:

```text
                         +-- Slack delivery + credential
one approval request ---+-- Discord delivery + credential
                         +-- WhatsApp delivery + credential
                                      |
                                      v
                            one durable resolution
```

Every delivery has its own capability credential and provider message identity,
but all credentials bind to the same `(runId, approvalId, requestId)`.

The shared resolution rules are:

- the first valid `approved` or `rejected` decision commits exactly one
  `approval_resolved` event;
- the selected route executes exactly once;
- an identical later decision is idempotent;
- an opposite later decision reports that another decision already won;
- the shared timeout applies to every delivery;
- after resolution, WOML asks every successfully delivered provider to disable
  or update its message;
- a message-update failure is reported and retried, but never reverses the
  durable human decision; and
- notification credentials remain delivery capabilities, not workflow data.

## 4. Proposed Source Syntax

`<notify>` is an optional child of `<approval>` before the two decision arms.
When present, it contains one or more supported provider tags.

```xml
<approval id="releaseApproval" timeout="2h" on-timeout="reject">
  <notify>
    <slack connection="company" channel="#release-approvals" />
    <slack connection="company" channel="#engineering-leads" />
    <discord connection="community" channel="release-review" />
  </notify>

  <when-approved>...</when-approved>
  <when-rejected>...</when-rejected>
</approval>
```

The same provider may appear more than once when it targets different
destinations. Provider order is preserved for diagnostics and deterministic
delivery identity; providers may still deliver concurrently.

The initial notification message is generated from the approval `name`,
`description`, workflow identity, and deadline. Custom templates, arbitrary
message bodies, comments, attachments, quorum, and provider-specific forms are
deferred so the first profile stays predictable.

Exact provider attributes remain an N0 contract decision. Likely routing
attributes include:

| Provider     | Likely workflow attributes                               |
| ------------ | -------------------------------------------------------- |
| `<slack>`    | `connection` optional; `channel` required or defaulted   |
| `<discord>`  | `connection` optional; `channel` required or defaulted   |
| `<whatsapp>` | `connection` optional; `recipient` required or defaulted |

No token, signing secret, application secret, private key, or provider password
is a valid WOML attribute.

## 5. Why Connections Exist

Most useful provider information should be attributes because it is part of the
workflow's behavior:

```xml
<slack channel="#approvals" />
```

Examples of appropriate tag attributes are:

- channel or destination;
- recipient alias;
- connection name when several accounts/workspaces exist; and
- later, reviewed non-secret presentation options.

Credentials are different. Putting them in attributes would place secrets in a
source file commonly committed to Git, read by AI tools, copied between
environments, compiled, reviewed, and shared. It would also duplicate the same
credential across many workflows and make rotation require source edits.

Therefore, `connection` separates reusable environment configuration from
workflow intent:

```xml
<slack connection="company" channel="#approvals" />
```

`woml connect` is only one convenient way to create that configuration:

```bash
woml connect slack company
```

It is not required by the language. The same connection may be supplied by:

- a guided `woml connect` command for local users;
- environment or secret-manager configuration in CI;
- a self-hosted deployment configuration; or
- a future managed WOML dashboard.

For the simplest project with one configured Slack workspace, the connection
may be the environment's default and the author writes only:

```xml
<slack channel="#approvals" />
```

When several workspaces exist, the author selects one explicitly:

```xml
<slack connection="customerSupport" channel="#escalations" />
```

This gives authors simple attributes without turning `.woml` into a secret
store.

## 6. Provider Adapter Boundary

The core must not contain Slack, Discord, or WhatsApp API rules. The frontend
lowers each provider tag into a versioned generic notification operation with a
registered handler. A provider adapter is responsible for:

- resolving its named/default connection without exposing credentials;
- validating provider-specific destination configuration;
- sending an approval message with native decision controls;
- returning a durable provider message identity;
- authenticating interactions using the provider's reviewed mechanism;
- translating an interaction to the shared WOML decision contract;
- updating or disabling the provider message after resolution; and
- returning structured, secret-safe failures.

Rust owns:

- durable delivery intent and state;
- delivery attempt identity and idempotency keys;
- separate credentials bound to one approval request;
- the atomic first-decision-wins transaction;
- timeout races;
- workflow continuation; and
- recovery scheduling.

The exact provider host boundary, compiled-model version, event version,
interaction transport, and idempotency-key derivation are frozen in N0 rather
than selected implicitly during implementation.

## 7. Delivery and Failure Policy to Review in N0

The proposed v1 default is:

1. WOML durably creates one approval request and one delivery intent per
   provider tag.
2. Providers may attempt delivery concurrently.
3. At least one delivery must succeed.
4. If one succeeds and another fails, the workflow remains waiting and reports
   the failed channel without losing the valid delivery.
5. If every delivery fails after the approved retry policy, WOML produces an
   explicit notification-delivery failure instead of silently waiting for a
   reviewer who was never reached.
6. A recovered delivery attempt uses the frozen idempotency contract so a crash
   does not silently send duplicate provider messages.
7. Resolution-message updates are durable follow-up work. They are retried but
   cannot undo the decision.

N0 must decide whether an all-delivery failure fails the run immediately or
enters a recoverable operational state. No implementation phase may choose that
silently.

## 8. Implementation Phases

### Prerequisite — Human Approval A7

Changes:

- Complete the current approval composition, crash-boundary, security,
  packaging, and recovery verification.
- Keep notification providers out of A7 so the base approval milestone closes
  independently.

Result:

The shared decision authority is stable before adding provider effects.

### N0 — Freeze notification contracts and reviewed fixtures

Changes:

- Freeze `<notify>` placement, child rules, provider identity, duplicate-target,
  and shared-resolution semantics.
- Freeze the compiled notification representation and versioning decision.
- Freeze notification delivery, attempt, success, failure, message-update, and
  recovery events with a schema version.
- Freeze the provider adapter request/result/error contract.
- Freeze per-delivery credential binding and redaction rules.
- Freeze delivery and message-update idempotency-key derivation.
- Freeze at-least-one-delivery behavior and the all-delivery-failed result.
- Freeze provider interaction authentication and callback/connection ownership
  as an explicit per-provider profile.
- Add reviewed Slack, Discord, WhatsApp, multi-provider, partial-failure,
  all-failure, decision-race, and redaction fixtures.

Result:

Every layer targets one reviewed provider-neutral delivery contract before
provider code is written.

Gate:

The fixtures prove several deliveries converge on one approval request and one
resolution, with no provider secret or plaintext runtime credential in compiled
or event artifacts.

### N1 — Validate and lower provider tags

Changes:

- Teach the WOML frontend to validate `<notify>` and the executable built-in
  provider tags.
- Validate placement, order, supported attributes, connection references,
  destinations, duplicate identities, and unsupported nested content at the
  original source location.
- Lower provider tags into deterministic generic notification operations.
- Extend graph, namespace, dominance, reachability, and reference validation
  without teaching the core provider markup.
- Keep webhook notification explicitly staged.

Result:

Readable provider tags become deterministic operations that frontend and Rust
interpret identically.

Gate:

Exact compiled fixtures and source diagnostics pass for single-provider,
multi-provider, repeated-provider, nested-approval, and invalid workflows.

### N2 — Build connections and the provider registry

Changes:

- Add named and default provider connections outside WOML source.
- Support guided local configuration, environment/secret-manager injection,
  self-hosted configuration, and a future managed configuration source through
  one connection interface.
- Add provider capability registration and startup validation.
- Ensure resolved secrets are short-lived runtime values and never persisted in
  models, context, events, or diagnostics.
- Report missing, ambiguous, invalid, and unavailable connections before a
  delivery is claimed successful.

Result:

Authors can use simple routing attributes while operators can configure and
rotate provider credentials independently.

Gate:

The same WOML file works with different development and production connections,
and secret-leak tests pass across every artifact and failure path.

### N3 — Add durable delivery and recovery infrastructure

Changes:

- Create one durable delivery intent and separate capability credential per
  provider tag.
- Add an outbox-style dispatcher so recorded delivery work is not lost between
  transaction commit and provider execution.
- Add delivery attempts, idempotency, retry scheduling, partial/all failure
  aggregation, and provider message identity persistence.
- Add durable post-resolution message-update work.
- Recover safely before send, during an uncertain send, after provider success,
  after the shared decision, and during message update.
- Prove provider delivery never starts a workflow route and provider failure
  never fabricates a human decision.

Result:

Notification effects survive crashes and multiple deliveries remain one
approval.

Gate:

Fake-provider tests prove no lost delivery intent, no unreviewed duplicate send,
one shared resolution, deterministic partial failure, and safe recovery.

### N4 — Implement Slack end to end

Changes:

- Add Slack connection validation and destination resolution.
- Send a native approval message with Approve and Reject controls.
- Implement the reviewed Slack interaction transport and authentication.
- Map Slack actions to the shared Rust decision authority.
- Store only the provider message identity required for idempotent updates.
- Update or disable the Slack message after human decision or timeout.
- Add Slack-specific delivery, interaction, signature/authentication, recovery,
  duplicate, conflict, timeout, and redaction tests.

Result:

A reviewer can approve or reject a real WOML workflow entirely inside Slack.

Gate:

The packaged product completes send, click, Rust resolution, selected-route
continuation, and message update without exposing a raw decision URL to the
reviewer.

### N5 — Prove shared multi-provider behavior

Changes:

- Run Slack with deterministic fake Discord and WhatsApp adapters against the
  same approval request.
- Test first-provider wins, opposite simultaneous decisions, identical repeats,
  partial delivery failure, all-delivery failure, timeout, restart, and message
  update failure.
- Verify every provider credential binds to the same request but remains a
  separate revocable delivery capability.
- Verify every successful provider message reflects the final shared decision.

Result:

Multiple tags behave like several doors to one decision, never like separate
approvals.

Gate:

Race and recovery tests contain exactly one resolution and exactly one selected
route execution regardless of provider count.

### N6 — Implement Discord

Changes:

- Add Discord connection, delivery, native controls, authenticated interaction,
  message update, recovery, and diagnostics using the existing provider
  boundary.
- Replace the fake Discord adapter in multi-provider tests.
- Reject any implementation pressure that would make core decision semantics
  provider-specific.

Result:

The provider architecture supports a second real interactive platform without
changing the shared approval contract.

Gate:

Slack and Discord can race on one approval, one decision wins, and both messages
converge on the final state.

### N7 — Implement WhatsApp and close the milestone

Changes:

- Add the reviewed WhatsApp connection, template/destination validation,
  provider-native decision interaction, authentication, and follow-up behavior.
- Replace the fake WhatsApp adapter in multi-provider tests.
- Run full syntax, compiled model, event, Rust, provider, security, recovery,
  packaging, and clean-install verification.
- Document connection setup, default connections, multi-provider semantics,
  operational failures, and the browser/API fallback.
- Keep generic webhook notification, email, SMS, custom templates, comments,
  forms, quorum, RBAC, and escalation outside this milestone.

Result:

WOML Human Approval reaches reviewers through several built-in real-world
providers while retaining one durable decision authority.

Gate:

Slack, Discord, and WhatsApp pass the same provider-neutral conformance suite,
multi-provider workflows resolve exactly once, and no provider credential leaks.

## 9. Verification Matrix

| Area                | Required proof                                                                        |
| ------------------- | ------------------------------------------------------------------------------------- |
| Syntax              | Valid provider tags compile; invalid placement and attributes retain source location. |
| Shared decision     | Any provider may win; exactly one approval resolution and route execution occur.      |
| Multiple delivery   | One delivery per tag with deterministic identity and separate credential.             |
| Partial failure     | At least one successful delivery keeps the approval reachable and visible.            |
| Total failure       | Every failed delivery produces the frozen explicit operational result.                |
| Idempotency         | Retry and recovery do not silently duplicate provider messages.                       |
| Recovery            | Send, success, decision, and message-update crash boundaries reopen safely.           |
| Timeout             | One timeout resolves every provider view consistently.                                |
| Connections         | Default/named connections work without putting secrets in WOML.                       |
| Authentication      | Forged provider interactions fail before Rust decision resolution.                    |
| Message convergence | Every delivered message is updated or carries a visible update failure.               |
| Redaction           | Secrets and plaintext runtime credentials never enter durable/public artifacts.       |
| Compatibility       | Existing approval workflows without `<notify>` keep the local fallback behavior.      |
| Packaging           | Built-in provider assets and connection tooling ship in a clean installation.         |

## 10. Explicit Non-Goals

This milestone does not add:

- generic `<webhook>` notification delivery;
- arbitrary third-party provider plugins;
- notification secrets inside WOML attributes;
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
- provider tag attributes and destination identity grammar;
- default connection selection and ambiguity behavior;
- the minimum stored provider message identity;
- interaction transport ownership for local, hosted, and self-hosted runtimes;
- how reviewer identity is represented without silently defining production
  authentication/RBAC;
- whether post-resolution update failures appear only operationally or through a
  versioned workflow inspection surface; and
- token revocation and retention after every delivery converges.

No implementation phase may resolve these with an undocumented default.
