# WOML Provider Extension Architecture

Status: Deferred post-v1 architecture direction. WOML v1 keeps its existing
built-in providers supported. This document does not authorize an extraction,
new extension runtime, or removal of an existing provider.

## 1. Why This Note Exists

WOML already provides the universal automation primitives that most external
systems can use:

- manual triggers;
- webhooks;
- events;
- schedules;
- intervals; and
- supervised HTTP capabilities.

Building Slack, Discord, WhatsApp, Telegram, and every future communication
integration directly into WOML creates an expanding product promise. Once
several providers are built in, users reasonably expect Microsoft Teams,
Signal, email, SMS, Messenger, and many regional services to receive the same
support.

That would make provider demand control the core roadmap. Each built-in also
adds credentials, permissions, callback security, API-version maintenance,
rate limits, connection recovery, diagnostics, documentation, and support
work.

The preferred long-term product is therefore an extensible workflow platform,
not an ever-growing closed provider catalog.

## 2. Recommended Product Shape

WOML should have three provider layers.

### 2.1 Core automation primitives

Manual, webhook, event, schedule, and interval triggers remain built in. They
are provider-neutral and belong to the workflow language.

### 2.2 One bundled reference provider

Telegram is the strongest candidate for the bundled communication provider:

- setup is comparatively simple;
- bot tokens are easy to obtain;
- messages and approval buttons are supported;
- it demonstrates conversational triggers, notifications, approvals, and
  messaging; and
- it gives a new user a complete experience without an extension install.

Keeping one provider built in is a product convenience and a conformance
reference, not a promise to bundle every popular platform.

### 2.3 Optional provider extensions

Slack, Discord, WhatsApp, Microsoft Teams, and future providers should be able
to live as optional official or community modules behind one stable Provider
Extension API. Existing implementations should be reused or extracted, not
discarded.

The intended product message is:

> WOML includes universal automation primitives and Telegram out of the box.
> Additional platforms are added through provider modules.

## 3. What the Current Module System Can Do

The current module system is useful and supported. It can create:

- JavaScript or TypeScript utility and service modules;
- reusable WOML steps;
- custom notification providers;
- approval notifications using the bounded `notification.actions.approve.url`
  and `notification.actions.reject.url` values supplied by WOML;
- workflow lifecycle notifications; and
- supervised outbound HTTP operations with explicit secrets and idempotency.

Rust already owns the durable intent, retry identity, approval capability, and
terminal delivery result for a custom notification provider. Provider code
only transports the bounded message and returns an optional message ID.

This is real extensibility, but it is workflow-level extensibility rather than
a complete runtime-integration extension system.

## 4. What the Current Module System Cannot Do

A custom provider cannot currently become a first-class built-in equivalent.
It cannot register:

- a custom declarative trigger such as `<teams events="message" />`;
- provider authentication and signature verification before a run is created;
- a persistent connection, polling loop, subscription, or reconnect policy;
- a normalized provider trigger payload contract;
- native interactive callback handling owned by the provider runtime;
- a managed messaging capability such as `services.teams.send()` as part of
  the same provider definition;
- provider message updates after an approval settles;
- a provider-specific diagnostic command such as `woml teams doctor`; or
- a unified installable provider package containing all these surfaces.

The compiled workflow graph and Rust runtime only recognize the extension
points that have already been reviewed. Changing a provider's `kind` string
cannot safely create new trigger, connection, callback, or persistence
semantics.

## 5. Can Microsoft Teams Be Built Today?

Partially, yes. A complete first-class Microsoft Teams provider cannot be built
with the current public module system.

| Teams capability | Possible now? | Current approach |
| --- | --- | --- |
| Send a simple notification | Yes | A custom notification provider calls a Teams webhook or Microsoft Graph through `services.http.request()` and returns a message ID when available. |
| Send workflow lifecycle notifications | Yes | Import the custom provider and use it inside lifecycle `<notify>`. |
| Send approval choices | Yes, with a limitation | Render WOML's approve/reject action URLs in a Teams Adaptive Card or message. The shared durable WOML approval remains authoritative. |
| Send ordinary messages from scripts | Yes, separately | Import a JavaScript/TypeScript module that wraps Microsoft Graph with `services.http`; it is not yet one unified provider package or automatically named `services.teams`. |
| Receive a basic external call | Workaround only | Teams can call a generic WOML webhook, but the workflow sees a webhook rather than a first-class Teams trigger. |
| Verify Teams/Bot Framework identity before run creation | No public provider extension | This requires a reviewed inbound-provider authentication boundary. Doing it in a workflow step occurs too late because a run already exists. |
| Receive native bot events through a provider host | No | Custom triggers and provider-owned connection/subscription hosts do not exist yet. |
| Handle native interactive callbacks | No first-class extension | Approval URLs work, but custom provider-native callback payloads are not routed through an extension contract. |
| Update every delivered message after settlement | No | Custom Provider v1 records the delivery receipt but has no reviewed update callback contract. |
| Expose `woml teams doctor` | No | Provider diagnostic extensions are not currently installable. |

Therefore it would be misleading to tell developers that they can reproduce
the built-in Slack, Discord, or WhatsApp experience with the current module
system. They can build useful Teams notifications and outbound messaging, but
not the complete trigger, interaction, operational, and recovery experience.

## 6. Target Provider Extension API

The future module system should support one provider package declaring only the
surfaces it implements:

```text
Provider extension
├── notification delivery
├── approval interaction handling
├── messaging capabilities
├── inbound triggers
├── message settlement updates
└── diagnostics
```

Provider authors should not need to understand or manually reproduce the Rust
engine. WOML should supervise the extension behind stable contracts.

The author supplies provider-specific code for:

- request construction;
- response normalization;
- event normalization;
- signature or identity verification using reviewed APIs;
- provider-specific rate-limit information; and
- safe diagnostics.

WOML retains authority over:

- attempts and timeouts;
- idempotency and ambiguous-effect handling;
- durable event admission and deduplication;
- approval capability creation and first-decision-wins convergence;
- secret injection and redaction;
- cancellation and recovery;
- payload and result limits;
- lifecycle and message-update scheduling; and
- event, store, and inspection contracts.

## 7. Required Technical Contracts

Before provider extensions can replace built-ins, the following contracts need
to be designed and versioned:

1. **Provider manifest** — identity, version, exported surfaces, runtime
   compatibility, required permissions, secret references, and configuration.
2. **Notification contract** — the existing Custom Notification Provider v1
   remains the starting point.
3. **Interaction contract** — authenticated inbound approval actions and
   provider acknowledgement deadlines.
4. **Message-update contract** — removal or replacement of actions after
   approval settlement.
5. **Messaging-capability contract** — provider operations exposed under
   `services.<provider>` while Rust keeps effect authority.
6. **Trigger contract** — registration, connection ownership, signature or
   token verification, normalized payloads, durable admission, deduplication,
   acknowledgement, and reconnect behavior.
7. **Diagnostic contract** — safe read-only checks and stable human/JSON
   results without exposing provider responses or secrets.
8. **Permission model** — declared network destinations, secret access,
   callback routes, persistent connections, and any local resources.
9. **Packaging and trust** — local development first, followed by reviewed
   package locking, integrity, provenance, and an installation story.
10. **Compatibility contract** — an unsupported extension or protocol version
    fails closed with an actionable upgrade error.

These are runtime extension contracts, not arbitrary custom markup. Custom
structural tags remain a different future design problem because they can
change the workflow DAG itself.

## 8. Suggested Migration of Existing Providers

If this direction is approved later:

1. Keep Telegram bundled as the reference provider.
2. Generalize the provider runtime using the already proven Slack, Discord,
   and WhatsApp behaviors as conformance fixtures.
3. Extract Slack, Discord, and WhatsApp into optional official extensions.
4. Preserve their existing syntax through imports or provide a clear
   pre-release migration if the language has not yet been published.
5. Prove optional extensions retain triggers, notifications, approvals,
   messaging, diagnostics, restart recovery, security, and durable semantics.
6. Open the same contracts to local and community providers.
7. State explicitly that WOML does not guarantee an official module for every
   communication platform.

The existing provider implementations remain valuable: they are test cases
for the extension API and can become official optional modules.

## 9. Product Risks and Guardrails

- Do not remove built-ins and then claim parity before the extension API can
  reproduce their behavior.
- Do not let provider modules create hidden workflow graph nodes or persistence
  semantics.
- Do not execute provider callbacks in ordinary workflow scripts when
  authentication must happen before durable run creation.
- Do not give extension code unrestricted access to every secret or network
  destination.
- Do not make Bun provider code the persistence authority; Rust remains the
  supervisor and durable authority.
- Do not make a closed provider enum the permanent language boundary.
- Do not promise a public provider registry before local versioned extensions
  are secure and useful.

## 10. Relationship to WOML v1

WOML v1 ships the communication-provider implementations that exist today.
They remain supported product behavior while the extension architecture is
deferred. A future extraction into optional extensions must preserve syntax or
provide an explicit compatibility path, and must first prove equivalent
triggers, notifications, approvals, messaging, diagnostics, security, and
recovery.

The extension system is a later product decision, not a prerequisite for the
initial release. It must not weaken the current durable contracts or turn
provider distribution into a second workflow execution authority.

## 11. Open Decisions for Later Review

- Is Telegram definitively the only bundled communication provider?
- Should optional official providers use imports, a future package command, or
  both?
- Does one extension package export triggers, notifications, services, and
  diagnostics together?
- How are callback routes allocated without collisions?
- Which authentication algorithms can extensions request safely?
- Can a provider run a persistent connection, and who applies its resource
  budget?
- How are provider-specific message updates recovered after a process crash?
- What is the minimum permission model needed before third-party extensions
  are enabled?
- Which existing provider syntax remains compatible during extraction?
