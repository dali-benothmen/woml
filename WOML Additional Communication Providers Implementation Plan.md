# WOML Additional Communication Providers Implementation Plan

Status: In progress. ACP0 through ACP5 were completed on 2026-08-16.
Telegram now validates, lowers to Model v15 / Definition Package v10, and runs
end to end through the durable production runtime. Discord authoring, imported
module discovery, validation, Model v15 / Definition Package v10 lowering, and
production execution are also complete. WhatsApp remains unavailable until its
provider phases. Slack
triggers and notifications, custom notification providers, Human Approval,
durable capability execution, production runtime hosting, and cross-platform
release packaging form the proven baseline.

## 1. Product Outcome

This milestone makes Discord, Telegram, and WhatsApp complete built-in WOML
communication providers.

After ACP0-ACP10 are complete, a workflow author can:

- start a workflow from an inbound Telegram, Discord, or WhatsApp message;
- build a conversational automation or AI agent around `context.payload`;
- send ordinary messages and replies from workflow scripts;
- send Human Approval requests to any supported communication provider;
- resolve one approval from any delivered provider while every other delivery
  converges on the same final decision;
- send informational lifecycle notifications without changing the workflow's
  business outcome;
- configure credentials with `woml secrets set` and reference them directly in
  provider attributes;
- diagnose credentials, permissions, destinations, subscriptions, and
  connectivity from the terminal;
- run the same providers in foreground and background production runtimes;
- recover safely after restart without duplicating accepted triggers,
  approval decisions, or known message deliveries; and
- continue using project-owned custom notification providers when a built-in
  provider is not available.

The product remains simple:

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml run agent.woml
```

There is no mandatory account connection command and no provider-specific npm
package for workflow authors to install.

## 2. Provider Scope and Delivery Order

The providers are delivered as three complete vertical slices in this order:

1. **Telegram** — the simplest complete communication provider. Its HTTP Bot
   API and long-polling trigger path let authors test locally without exposing a
   public callback URL.
2. **Discord** — adds server channels, direct messages, mentions, Gateway
   events, and interactive approval buttons.
3. **WhatsApp** — adds official Meta Cloud API messages, mandatory webhook
   ingress, phone-number routing, templates, and business-message constraints.

Each provider must support all three product surfaces before its slice is
considered complete:

| Surface | Purpose |
| --- | --- |
| Trigger | A provider event durably starts a workflow. |
| Notification | Approval and lifecycle messages are delivered through WOML's existing notification authority. |
| Messaging capability | A script sends or replies to an ordinary message under Rust supervision. |

We will not build all frontend syntax first and leave execution for later.
Telegram must work end to end before Discord begins, and Discord must work end
to end before WhatsApp begins.

## 3. Approved Product Experience

### 3.1 Telegram conversational workflow

The following is the intended author experience. ACP0 freezes the exact event
and destination vocabularies before the syntax becomes a public contract.

```xml
<woml>
  <workflow
    id="telegram-agent"
    name="Telegram Support Agent"
    description="Answers customer questions received by the support bot."
    version="1.0.0"
  >
    <triggers>
      <telegram
        id="customerMessage"
        events="message"
        bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
      />
    </triggers>

    <steps>
      <step id="answer" name="Prepare answer">
        <script>
          const response = `You said: ${context.payload.text}`;

          return services.telegram.send({
            botToken: secrets.TELEGRAM_BOT_TOKEN,
            conversationId: context.payload.conversationId,
            text: response,
            replyToMessageId: context.payload.messageId
          });
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The terminal explains how to trigger the active workflow:

```text
TRIGGER

  ● Telegram
    Bot @woml_support_bot · Messages
    Send a message to the bot to start a run

  ● Listening for Telegram updates · Press Ctrl+C to stop
```

### 3.2 Discord conversational workflow

```xml
<triggers>
  <discord
    id="agentMessage"
    events="app-mention,direct-message"
    channels="123456789012345678,234567890123456789"
    bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
  />
</triggers>
```

```js
return services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: `Hello ${context.payload.senderName ?? 'there'}`,
  replyToMessageId: context.payload.messageId
});
```

Discord channel IDs are the unambiguous v1 destination identity. A human name
may be shown in diagnostics after lookup, but it is not used as durable
routing authority because channel names can repeat across servers.

### 3.3 WhatsApp conversational workflow

```xml
<triggers>
  <whatsapp
    id="customerMessage"
    events="message"
    phone-number-id="123456789012345"
    verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
    app-secret="{{secrets.WHATSAPP_APP_SECRET}}"
  />
</triggers>
```

```js
return services.whatsapp.send({
  phoneNumberId: '123456789012345',
  accessToken: secrets.WHATSAPP_ACCESS_TOKEN,
  conversationId: context.payload.conversationId,
  text: 'Thanks. We received your request.'
});
```

The credential uses WOML's existing restricted `secrets.NAME` script binding.
The compiler discovers literal secret reads, and the runtime injects only the
referenced values into that invocation. The value may cross the in-memory
capability boundary but never enters the compiled model, workflow context,
durable events, SQLite, fixtures, or terminal output.

WhatsApp uses the official Meta Cloud API only. WOML will not automate a
personal WhatsApp Web session or depend on an unofficial client.

### 3.4 Multi-provider Human Approval

```xml
<approval
  id="approveDeployment"
  name="Approve production deployment"
  timeout="24h"
  on-timeout="reject"
>
  <notify>
    <slack
      channels="#approvals"
      bot-token="{{secrets.SLACK_BOT_TOKEN}}"
      app-token="{{secrets.SLACK_APP_TOKEN}}"
    />

    <telegram
      chats="-1001234567890"
      bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
    />

    <discord
      channels="123456789012345678"
      bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
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

Every delivery receives its own capability token, but all deliveries resolve
one durable approval. The first valid decision wins. Telegram, Discord, Slack,
WhatsApp, and custom-provider deliveries must then show or record the same
final state.

### 3.5 Lifecycle notification

```xml
<lifecycle>
  <on-error>
    <notify>
      <telegram
        chats="-1001234567890"
        message="Workflow {{lifecycle.workflow.id}} failed: {{lifecycle.failure.code}}"
        bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
      />
    </notify>
  </on-error>
</lifecycle>
```

Lifecycle messages are informational. They never receive approval authority
and a delivery failure remains a bounded lifecycle warning rather than
rewriting the workflow's business result.

## 4. Product Decisions

### 4.1 Built-ins are runtime adapters, not `.woml` modules

Slack, Telegram, Discord, and WhatsApp are first-class WOML providers. They are
implemented like the current Slack provider because they need trusted access
to:

- long-lived connections and callback listeners;
- signature and token verification;
- durable trigger admission and deduplication;
- approval capability resolution;
- retries, rate limits, timeout classification, and idempotency;
- safe secret resolution;
- message identity storage and convergence updates; and
- runtime startup, shutdown, restart, and observability.

User-authored `.woml` provider modules remain supported for project-specific
notifications. They do not become an alternate trigger engine or receive raw
runtime authority in this milestone.

### 4.2 Three provider roles remain separate

The shared implementation must not create one oversized provider interface.
It defines three focused adapter contracts:

1. **Trigger adapter** — receives, verifies, normalizes, and forwards inbound
   occurrences to Rust.
2. **Notification adapter** — delivers approval or lifecycle messages and can
   update an actionable message after resolution.
3. **Messaging capability adapter** — executes an explicit workflow step call
   such as `services.telegram.send()`.

One built-in provider may implement all three contracts, but their authority
remains separate. An informational notification can never resolve an approval,
and an ordinary messaging call can never admit a trigger by itself.

### 4.3 Bun speaks provider protocols; Rust owns workflow truth

Bun/TypeScript owns:

- provider HTTP and WebSocket communication;
- provider payload decoding;
- signature and credential checks that require raw transport bytes;
- destination lookup and safe display metadata;
- connection lifecycle and provider-specific acknowledgements; and
- conversion to or from versioned WOML protocol messages.

Rust owns:

- occurrence identity and durable trigger admission;
- run creation, policy admission, and execution;
- capability attempt identity and terminal outcome;
- retry and idempotency decisions;
- notification delivery intent and result;
- approval tokens, decisions, deadlines, and selected routes;
- cancellation and recovery; and
- event-log and store authority.

The core does not learn Telegram, Discord, or WhatsApp wire payloads. It sees
versioned normalized trigger, notification, and capability messages.

### 4.4 Native platform features, no required provider SDK packages

The preferred adapters use Bun's native `fetch` and WebSocket support against
official provider APIs. A provider SDK is introduced only if a reviewed
protocol area is impractical or unsafe to maintain ourselves. Any SDK must be
locked, packaged, license-reviewed, and hidden from WOML authors.

### 4.5 Credentials stay declarative

Users configure secrets with existing commands:

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml secrets set DISCORD_BOT_TOKEN
woml secrets set WHATSAPP_ACCESS_TOKEN
woml secrets set WHATSAPP_APP_SECRET
woml secrets set WHATSAPP_VERIFY_TOKEN
```

Provider markup references exact symbolic values such as
`{{secrets.TELEGRAM_BOT_TOKEN}}`. There is no required `woml connect` flow.
Secrets never enter compiled definitions, workflow context, durable events,
SQLite, terminal output, or diagnostic fixtures.

Messaging services follow the existing database and HTTP capability model:
the request receives a literal `secrets.NAME` value through the restricted
script binding. ACP0 freezes the exact request property names and verifies that
compiler discovery, per-invocation injection, protocol redaction, and durable
storage exclusions remain intact.

### 4.6 One normalized communication payload

New communication triggers target a stable common payload vocabulary:

```js
context.payload.provider
context.payload.event
context.payload.text
context.payload.senderId
context.payload.senderName
context.payload.conversationId
context.payload.conversationType
context.payload.messageId
context.payload.replyToMessageId
context.payload.threadId
context.payload.occurredAt
```

Optional fields are omitted rather than filled with invented empty values.
Provider-specific stable identifiers may live under a bounded
`context.payload.providerData` object only after ACP0 reviews each field.
Credentials, raw envelopes, signatures, access tokens, entire user profiles,
and unbounded attachments never enter context.

The existing Slack payload is already published. ACP0 must define an additive
compatibility migration that preserves its current fields while providing the
common aliases; existing Slack workflows must continue to run unchanged.

### 4.7 Provider differences remain visible where they matter

WOML normalizes common message concepts, not provider policy:

- Telegram chat IDs, Discord channel IDs, and WhatsApp phone identities remain
  distinct destination types.
- WhatsApp template rules are not pretended away as ordinary free-form text.
- Discord Gateway reconnect/resume is not modeled as Telegram polling.
- Provider rate-limit and permission diagnostics retain provider-specific
  actions.

A future generic `services.messaging` facade is not part of this milestone.
The explicit `services.telegram`, `services.discord`, and
`services.whatsapp` names keep behavior understandable.

### 4.8 Custom providers remain supported

Imported `<provider kind="notification">` definitions can coexist with every
built-in provider inside the same `<notify>` block. Their existing props,
lifecycle, security, retry, and approval contracts do not change.

Custom trigger providers and custom `services.<provider>` implementations are
separate future extension work. Ordinary `.woml` modules must not be granted
listener, signature-verification, or trigger-admission authority accidentally.

### 4.9 Existing provider aliases do not break silently

The repository already contains a custom notification provider imported with
the alias `telegram`. Adding a built-in tag must not reinterpret that existing
workflow silently.

The reviewed resolution rule is contextual:

- an explicitly imported local provider alias wins inside `<notify>`;
- without that import, `<telegram>`, `<discord>`, `<whatsapp>`, and `<slack>`
  resolve to their built-ins;
- a local alias that shadows a built-in receives a clear non-fatal check
  diagnostic and can be renamed when the author wants both; and
- provider tags under `<triggers>` always resolve to built-in trigger grammar,
  because notification-only modules are illegal there.

ACP0 must separately fixture JavaScript/TypeScript module aliases that would
produce `services.telegram`, `services.discord`, or `services.whatsapp` and
freeze whether the explicit local import shadows the built-in service or must
be renamed. No existing module may change meaning without a diagnostic.

### 4.10 Hosts start from declared runtime needs

A built-in host is activated from the compiled workflow requirements, not only
from `<triggers>`:

- a trigger starts its inbound listener/connection;
- a pending actionable notification starts the callback path needed for its
  decision buttons;
- an informational notification or ordinary `send()` starts only the outbound
  transport it needs; and
- shared credential identities reuse one safe connection across loaded
  workflows.

Approval callbacks remain notification decisions. Starting their listener does
not register a workflow trigger or permit an arbitrary provider message to
create a run.

## 5. Authoring Contract to Freeze in ACP0

### 5.1 Trigger tags

ACP0 must freeze exact attributes for:

```xml
<telegram ... />
<discord ... />
<whatsapp ... />
```

Every trigger requires:

- `id` as the workflow-local stable trigger identity;
- an explicit supported event set;
- exact symbolic credential references;
- optional provider-appropriate destination filters; and
- no executable children.

Unknown attributes, duplicate events, duplicate destinations, literal secret
values, unsupported event combinations, and credentials in non-credential
attributes are compile errors with code, file, line, column, and message.

### 5.2 Notification tags

The same provider names are legal as direct `<notify>` children in approval
and lifecycle scopes. Scope determines required attributes:

- approval notification text derives from the approval identity and approved
  presentation contract;
- lifecycle notification requires an explicit `message`;
- one tag may target several destinations using a deterministic list;
- one destination becomes one durable delivery; and
- one tag uses one credential set.

WhatsApp is the exception that requires a reviewed template/session contract.
ACP0 must not claim that proactive free-form WhatsApp text always works.

### 5.3 Messaging capability methods

The minimum public service for each provider is:

```js
await services.telegram.send({ ... });
await services.discord.send({ ... });
await services.whatsapp.send({ ... });
```

Each request includes its provider credential through a literal
`secrets.NAME` read. Computed secret names, whole-object access, dynamic aliases,
and forwarding the `secrets` object remain compile errors under the existing
Script Bindings contract.

The minimum result is provider-neutral and JSON-safe:

```json
{
  "provider": "telegram",
  "conversationId": "-1001234567890",
  "messageId": "42",
  "acceptedAt": "2026-08-16T10:00:00.000Z"
}
```

`send()` may accept an optional reply target. Public edit/delete/media methods
are not added until their semantics, idempotency, limits, and provider support
are reviewed. Internal approval-message convergence may still update a message
through the private notification adapter.

### 5.4 Message content

V1 supports bounded UTF-8 text. Each adapter enforces both WOML's common byte
budget and the provider's current limit before making an external request.
Provider markup may interpolate bounded scalar `context.*` and `lifecycle.*`
values where that scope already allows interpolation.

Secret interpolation, executable markup inside message attributes, raw HTML
mode, arbitrary provider JSON, and unbounded media are rejected in v1.

## 6. Provider Trigger Semantics

### 6.1 Telegram

The first Telegram trigger uses long polling:

- WOML authenticates the bot and reports its safe username;
- one shared poller is created per bot credential identity;
- several loaded workflows may subscribe through that poller;
- updates are filtered and normalized in Bun;
- Rust durably accepts or recognizes the occurrence before the poll offset is
  advanced;
- restart resumes without intentionally losing or duplicating an update; and
- an existing Telegram webhook is detected and reported with an actionable
  diagnostic rather than silently replacing it.

Telegram webhooks may be added later only if real deployment demand shows an
advantage. Long polling and webhooks are mutually exclusive for one bot and
must never compete.

### 6.2 Discord

The first Discord trigger uses one shared Gateway connection per bot credential
identity:

- connection heartbeat, sequence, reconnect, resume, and invalid-session
  behavior are provider-adapter responsibilities;
- intents are minimal and diagnosed before the runtime reports ready;
- self/bot messages, edits, unsupported events, and filtered channels do not
  create occurrences;
- app mentions and direct messages normalize to the common payload;
- provider event identity plus workflow/trigger identity forms the durable
  deduplication key; and
- approval component interactions are verified and routed only to the
  notification decision boundary.

Slash commands and arbitrary Discord interactions are not silently treated as
ordinary messages. ACP0 either freezes a minimal command contract or leaves
public slash-command triggers for a later reviewed increment.

### 6.3 WhatsApp

WhatsApp uses the existing production HTTP listener:

- WOML exposes a stable callback route and prints it in the trigger header;
- the Meta verification handshake validates the configured verification token;
- every POST is checked against the configured app secret using the exact raw
  request bytes;
- the phone-number identity selects matching workflow triggers;
- inbound messages and supported button replies normalize to the common
  payload;
- status callbacks do not accidentally start message workflows;
- Rust durably accepts or recognizes the occurrence before success is
  acknowledged; and
- invalid signatures, unknown phone identities, and malformed payloads are
  rejected without echoing sensitive content.

The runtime must explain the public HTTPS/reverse-proxy requirement. It does
not pretend a loopback-only URL can receive Meta callbacks.

## 7. Notification and Approval Semantics

Existing Human Approval rules remain authoritative:

1. Rust creates one approval and separate delivery identities.
2. Each provider receives a distinct decision capability.
3. A provider message exposes Approve and Reject through its native supported
   interaction format.
4. The first valid decision is committed durably.
5. Every delivered provider is updated or given a durable bounded warning.
6. Later clicks observe the existing result and cannot change it.

Delivery policy remains:

- multiple built-in and custom providers are allowed;
- one delivery failure does not prevent successful providers from delivering;
- the approval may wait when at least one required delivery succeeds;
- all configured deliveries failing produces the existing actionable
  notification failure;
- lifecycle delivery failures become warnings; and
- an ambiguous send is not blindly replayed merely because no message ID was
  recorded.

Provider-specific interaction rules:

- Telegram uses callback-query buttons and must answer callbacks promptly
  after durable resolution.
- Discord uses message components and must acknowledge interactions within the
  provider deadline while keeping Rust authoritative.
- WhatsApp uses reviewed interactive/template buttons; template approval and
  session-window constraints must be explicit to the author.

## 8. Messaging Capability Semantics

Ordinary provider messages execute as managed capabilities:

```text
script request
  -> Script Host capability message
  -> Rust attempt/idempotency authority
  -> Bun provider adapter
  -> provider API
  -> validated bounded result
  -> durable terminal capability event
  -> script result
```

The existing retry and idempotency rules apply. A stable logical call identity
prevents duplicate replay after a known terminal result. Provider rate limits
may schedule a reviewed retry using safe retry timing. A transport failure
after a request may have reached the provider is classified as ambiguous and
fails closed unless the provider offers a reviewed idempotency or query
mechanism.

Messages sent by `services.*.send()` do not become workflow triggers in the
same process unless the provider independently delivers a valid inbound event.
Self/bot-message filtering prevents common automation loops.

## 9. Protocol, Model, Event, and Store Contracts

ACP0 must produce independently validated, versioned artifacts for:

- Communication Trigger Payload v1;
- Communication Trigger Host Protocol v1;
- Communication Notification Adapter Protocol v1;
- Communication Messaging Capability Protocol v1;
- provider-specific credential, destination, event, message identity, and
  failure schemas;
- compiled workflow model changes;
- definition-package changes for discovered provider capabilities;
- durable event-vocabulary changes; and
- presentation/diagnostic changes.

Model v14, Event v13, Store v14, the current Notification Provider Host, and
the current Script Host are immutable existing contracts. Adding provider
values to a closed enum or changing a message shape requires a reviewed version
bump. ACP0 must determine the next exact versions from the actual schemas; no
implementation phase may mutate an older version in place.

The store version should remain unchanged if new providers can use existing
generic trigger, delivery, capability, and message-identity records. A store
bump is allowed only for a real persistence-shape change, not because a new
provider name exists.

## 10. Errors and Diagnostics

Every provider receives an actionable diagnostic command consistent with the
existing Slack experience:

```bash
woml telegram doctor
woml discord doctor
woml whatsapp doctor
```

Diagnostics check only relevant safe facts.

Telegram:

- token authentication and bot identity;
- polling/webhook conflict;
- chat accessibility where a destination is configured;
- ability to send a bounded test message only when explicitly requested; and
- last safe polling failure.

Discord:

- bot authentication and application identity;
- configured Gateway intents;
- server/channel visibility and send permission;
- missing message-content capability when required; and
- connection/reconnect state.

WhatsApp:

- access-token validity and safe business/phone identity;
- callback URL and verification readiness;
- app-secret signature configuration;
- destination/template readiness where testable; and
- public HTTPS requirement.

Provider errors carry:

- a stable WOML code;
- safe provider category;
- actionable message;
- retryability/ambiguity classification where relevant; and
- source location for authoring failures.

Raw provider responses, message bodies, tokens, signatures, decision
capabilities, webhook verification values, and recipient personal information
never appear in diagnostics or default terminal output.

## 11. Compatibility and Migration

- Existing Slack workflows compile and execute unchanged.
- Existing custom notification providers compile and execute unchanged.
- `<notify>` retains document order and one-delivery-per-destination identity.
- Provider tags remain invalid outside their reviewed trigger/notification
  scopes.
- Existing `services.http`, native `fetch`, and user modules remain available;
  the new messaging services do not replace them.
- Historical runs remain readable after provider additions.
- A newer provider/model/protocol version fails closed on an older runtime with
  an actionable upgrade error.
- Foreground, background, multi-file, and directory activation use the same
  provider hosts and durable authorities.
- Cross-platform npm packages contain every required Bun provider host; no
  provider works only from the source checkout.

## 12. Implementation Phases

### 12.1 Phase summary

| Phase | Product result |
| --- | --- |
| ACP0 | The shared contracts, syntax, payloads, provider limitations, and fixtures are reviewed and frozen. |
| ACP1 | Slack runs unchanged on a reusable communication-provider foundation. |
| ACP2 | Telegram syntax and service calls validate and lower into versioned executable models. |
| ACP3 | Telegram triggers, notifications, approvals, and messages work end to end. |
| ACP4 | Discord syntax and service calls validate and lower into versioned executable models. |
| ACP5 | Discord triggers, notifications, approvals, and messages work end to end. |
| ACP6 | WhatsApp syntax, templates, credentials, callbacks, and service calls are validated and lowered. |
| ACP7 | WhatsApp triggers, notifications, approvals, and messages work end to end through the official Cloud API. |
| ACP8 | Multiple built-in/custom providers converge safely on shared approvals and lifecycle behavior. |
| ACP9 | Diagnostics, security, recovery, rate limits, compatibility, and operational behavior are hardened. |
| ACP10 | Examples, documentation, packaging, benchmarks, and release gates make the feature publishable. |

### ACP0 — Freeze communication contracts and reviewed fixtures

Status: **Completed on 2026-08-16.** The normative protocol, six JSON schemas,
synthetic reviewed fixtures, and independent Bun/Rust conformance tests now
freeze this phase. The exact decisions live in
`docs/protocols/additional-communication-providers-v1.md`.

Changes:

- Freeze exact Telegram, Discord, and WhatsApp trigger attributes and event
  vocabularies.
- Freeze approval and lifecycle notification attributes for every provider.
- Freeze messaging capability request/result/failure shapes.
- Freeze messaging credential properties using the existing restricted
  literal `secrets.NAME` binding.
- Freeze the common communication trigger payload and additive Slack
  compatibility aliases.
- Freeze provider destination identities, filters, ordering, and duplicate
  rules.
- Freeze contextual resolution and diagnostics for existing local provider and
  service-module aliases named Telegram, Discord, or WhatsApp.
- Freeze Telegram polling, Discord Gateway, and WhatsApp webhook acknowledgement
  and deduplication behavior.
- Freeze WhatsApp session/template and approval-button requirements.
- Freeze provider-neutral and provider-specific failure taxonomies.
- Decide exact Model, Definition Package, Event, Host Protocol, and presentation
  version bumps.
- Add reviewed valid, invalid, duplicate, retry, ambiguity, approval,
  lifecycle, and recovery fixtures.

Result:

Every layer has a reviewed target and no implementation phase needs to invent
public syntax, context fields, provider policy, or durable events.

Gate:

TypeScript and Rust independently validate the frozen artifacts; fixtures use
synthetic messages and identities and contain no real credentials, signatures,
capability tokens, personal data, or unbounded provider payloads.

### ACP1 — Build the shared communication-provider foundation

Status: **Completed on 2026-08-16.** Provider-neutral trigger, notification,
messaging, and role-registry interfaces now exist. Slack runs through that
foundation without changing its public protocol, payload, syntax, or runtime
behavior. Fake adapters cover ordered startup, rollback, reverse shutdown,
duplicate registration, and role separation.

Changes:

- Extract focused trigger, notification, and messaging adapter interfaces from
  the current Slack implementation.
- Preserve one shared transport/connection per credential identity where the
  provider supports it.
- Generalize provider-host startup, shutdown, reconnect progress, safe
  identities, message delivery, resolution updates, and failures.
- Generalize compiler/model/provider registries without replacing closed old
  schema versions.
- Keep Rust provider-neutral and route only versioned protocol messages.
- Migrate Slack to the new internal foundation without changing its markup,
  payload, events, diagnostics, or runtime behavior.
- Add fake adapter conformance suites reusable by all built-ins.

Result:

WOML has one reusable communication foundation proven by the already-working
Slack provider, while trigger, notification, and messaging authority remain
separate.

Gate:

All existing Slack trigger, approval, lifecycle, recovery, diagnostics,
packaging, and real manual smoke-test paths remain byte/behavior compatible;
provider-neutral tests cannot resolve approvals or admit triggers through the
wrong adapter role.

### ACP2 — Add Telegram authoring and lowering

Status: **Completed on 2026-08-16.** Telegram triggers, approval and lifecycle
notifications, and `services.telegram.send()` now have source-aware validation,
generated editor types/snippets, immutable Model v15 and Definition Package
v10 lowering, local-module discovery, and explicit local-alias compatibility.
Reviewed compile/check fixtures make no network requests. ACP2 deliberately
froze the authoring and lowering boundary before ACP3 activated its runtime.

Changes:

- Parse and validate Telegram trigger tags.
- Parse Telegram approval and lifecycle notification tags.
- Discover and type `services.telegram.send()` in scripts and imported
  modules.
- Lower exact symbolic credentials, events, filters, destinations, and
  capabilities into the reviewed compiled model/package.
- Add source-aware errors for invalid token references, chats, events,
  placements, messages, and duplicates.
- Add generated editor declarations and TextMate/VS Code awareness for the new
  tag and service.
- Add compile/check fixtures without making a network request.

Result:

Authors can write and validate complete Telegram WOML files, and every accepted
construct has an executable lowering target.

Gate:

Parser, grammar, compiler, model, package, module analysis, diagnostics,
autocomplete, and compatibility fixtures pass against the reviewed Model v15
and Definition Package v10 artifacts.

### ACP3 — Execute Telegram end to end

Status: **Completed on 2026-08-16.** One shared long-polling transport now
serves Telegram triggers and approval callbacks, while the notification host
delivers approval/lifecycle messages and Rust supervises
`services.telegram.send()`. Polling offsets advance only after durable
acceptance, approval decisions retain one shared authority across providers,
and uncertain sends fail closed.

Changes:

- Build the shared Telegram long-polling host and safe bot identity lookup.
- Normalize supported updates and durably admit trigger occurrences in Rust.
- Advance polling offsets only after durable acceptance/recognition.
- Deliver approval buttons and lifecycle messages.
- Resolve callback queries through the existing approval authority and update
  delivered messages after the final decision.
- Execute `services.telegram.send()` as a supervised capability.
- Implement rate-limit timing, retry, ambiguity, message-result validation,
  shutdown, and restart behavior.
- Add Telegram trigger/header/run presentation and actionable runtime errors.

Result:

A user can message a Telegram bot, run a workflow, receive a reply or approval,
and continue the workflow from the Telegram decision.

Gate:

Fake-API conformance, durable duplicate/restart, approval race, retry,
ambiguous-send, lifecycle, background-runtime, and clean-package tests pass;
an opt-in real bot/channel manual journey is documented and succeeds.

### ACP4 — Add Discord authoring and lowering

Status: **Completed on 2026-08-16.** Discord triggers, approval and lifecycle
notifications, `services.discord.send()`, imported-module discovery, local
alias compatibility, Model v15 / Definition Package v10 lowering, source-aware
diagnostics, editor declarations, and VS Code snippets are implemented. ACP4
deliberately keeps activation closed with `WOML_DISCORD_RUNTIME_UNAVAILABLE`;
the Gateway and REST runtime begins in ACP5.

Changes:

- Parse and validate Discord trigger tags and reviewed event filters.
- Parse Discord approval and lifecycle notification tags.
- Discover and type `services.discord.send()`.
- Validate exact channel IDs, credentials, events, placements, and duplicate
  destinations.
- Lower Discord triggers, deliveries, and capability requirements into the
  reviewed model/package.
- Add generated editor declarations and VS Code syntax awareness.
- Freeze or explicitly defer public slash-command trigger syntax.

Result:

Authors can write and validate complete Discord workflows without guessing
Gateway, channel, or credential rules.

Gate:

Frontend, schema, fixtures, module analysis, diagnostics, editor, and backward
compatibility tests pass; no invalid Discord workflow reaches activation.

### ACP5 — Execute Discord end to end

Status: **Completed on 2026-08-16.** Discord now runs through one shared
Gateway connection per bot credential with heartbeat, sequence tracking,
session resume, bounded reconnect, and safe shutdown. App mentions and direct
messages enter durable Rust admission; approvals use Discord components and
converge after the first durable decision; lifecycle notifications and
`services.discord.send()` use supervised REST delivery with structured
provider diagnostics. Fake Gateway/REST, resume, interaction-ordering,
idempotency, compiler/CLI, script-host, and durable-admission tests cover the
release boundary.

Changes:

- Build a shared Discord Gateway connection with heartbeat, sequence, resume,
  reconnect, and safe shutdown.
- Authenticate the bot, validate required intents, and resolve safe display
  metadata.
- Normalize app mentions/direct messages and durably admit occurrences.
- Filter self/bot/edited/deleted/unapproved channel events.
- Deliver approval components and lifecycle messages.
- Route component decisions to Rust and converge delivered messages.
- Execute `services.discord.send()` under capability supervision.
- Implement provider rate limits, retry timing, ambiguity, reconnect recovery,
  and terminal presentation.

Result:

A Discord mention or direct message can drive a durable WOML automation, and
the workflow can reply or request approval in Discord.

Gate:

Fake Gateway/REST conformance, heartbeat/reconnect/resume, duplicate, intent,
permissions, approval race, rate-limit, lifecycle, background, and packaged
tests pass; an opt-in real server/channel manual journey is documented.

### ACP6 — Add WhatsApp authoring, template, and callback contracts

Changes:

- Parse and validate WhatsApp trigger tags, phone-number identity, and symbolic
  verification/signature credentials.
- Parse WhatsApp approval and lifecycle notification tags.
- Freeze free-form session messages versus approved template messages.
- Validate template name, language, variable shape, recipient, and approval
  button requirements without contacting Meta at compile time.
- Discover and type `services.whatsapp.send()` with a credential-safe request.
- Lower triggers, notifications, template metadata, and capability requirements
  into reviewed artifacts.
- Add the callback route, verification-handshake contract, and raw-body
  signature-verification boundary.
- Add generated editor declarations and VS Code syntax awareness.

Result:

Authors can write a truthful WhatsApp workflow that distinguishes what can be
sent inside a customer session from what requires an approved template.

Gate:

Frontend, template, callback, signature-fixture, schema, editor, and secret
boundary tests pass; invalid or misleading WhatsApp delivery shapes are
rejected before activation.

### ACP7 — Execute WhatsApp end to end

Changes:

- Implement Meta callback verification and signed webhook ingestion on the
  production listener.
- Normalize supported inbound messages and button replies.
- Durably admit occurrences before successful webhook acknowledgement.
- Execute free-form/session and reviewed template messaging through the
  official Cloud API.
- Deliver lifecycle and Human Approval notifications using the frozen valid
  WhatsApp profiles.
- Resolve approval replies/buttons through Rust and record convergence status.
- Correlate accepted message IDs and safe delivery/failure status callbacks.
- Implement rate limits, token/permission failure, ambiguity, restart,
  cancellation, and public-endpoint presentation.

Result:

A customer can message the configured WhatsApp business number, start a WOML
run, receive a response or approval request, and continue the durable workflow.

Gate:

Signed webhook, handshake, duplicate, phone routing, template/session,
approval, lifecycle, delivery-status, rate-limit, restart, public-runtime, and
clean-package tests pass; an opt-in Meta test-number journey is documented.

### ACP8 — Complete cross-provider convergence and composition

Changes:

- Allow Slack, Telegram, Discord, WhatsApp, and custom providers in one
  deterministic `<notify>` block.
- Prove first-decision-wins across simultaneous provider callbacks.
- Update every known provider message after approval resolution without
  changing the decision if an update fails.
- Preserve at-least-one-success approval notification behavior.
- Compose provider triggers and sends with retry, choose/switch, parallel,
  fork/branch, approvals, workflow call/start, lifecycle, runtime policies,
  cancellation, durable state, background execution, backup, and retention.
- Present provider deliveries, decisions, warnings, and message steps without
  leaking capabilities or personal data.
- Keep custom provider lifecycle and props behavior unchanged.

Result:

Communication providers behave as one coherent WOML feature rather than four
independent integrations.

Gate:

Reversed timing, mixed success/failure, duplicate decisions, timeout,
cancellation, provider outage, workflow restart, nested control flow,
multi-workflow, and custom-provider composition tests pass.

### ACP9 — Harden diagnostics, security, recovery, and operations

Changes:

- Implement `woml telegram doctor`, `woml discord doctor`, and
  `woml whatsapp doctor` with safe actionable checks.
- Fuzz inbound payloads, signatures, Unicode, message sizes, nesting, IDs,
  callback data, and malformed provider responses.
- Enforce connection, subscriber, payload, delivery, result, and diagnostic
  budgets.
- Verify credentials, signatures, capability tokens, and access tokens never
  enter events, logs, errors, or stores. Prove accepted normalized message
  content and routing identities persist only as bounded workflow input or
  output where execution requires them, remain absent from diagnostics and
  operational logs, and obey backup, retention, and prune policy.
- Test provider outage, rate limit, token rotation, revoked permissions,
  process crash, network ambiguity, reconnect, backup/restore, prune, and
  retained-run inspection.
- Test terminal color/plain/JSON behavior and background log following.
- Test Linux/macOS/Windows packaging and shutdown behavior.
- Preserve machine-readable error and existing CLI contracts.

Result:

Provider failures are understandable and recoverable without weakening WOML's
durability, privacy, or exactly-one approval authority.

Gate:

Security, adversarial, diagnostic, recovery, compatibility, cross-platform,
background, backup/retention, and resource-budget suites pass with no skipped
provider-host coverage.

### ACP10 — Package, document, benchmark, and publish

Changes:

- Add one manually runnable conversational example per provider.
- Add one multi-provider approval/lifecycle example.
- Document provider application/bot setup, required permissions, secrets,
  destination IDs, local testing, production requirements, and troubleshooting.
- Document trigger payloads and every public messaging request/result.
- Update WOML syntax, architecture, security, production runtime,
  notifications, triggers, services, terminal, VS Code, and release docs.
- Add descriptive provider-foundation, Telegram, Discord, WhatsApp,
  cross-provider, diagnostics, package, and release test commands.
- Benchmark idle connection cost, inbound admission, outbound send overhead,
  callback resolution, reconnect, concurrent providers, and memory per
  credential identity.
- Verify a clean installed cross-platform `woml-cli` contains every provider
  host and executes the fake-provider conformance journey.

Result:

Discord, Telegram, and WhatsApp become documented, packaged, supportable WOML
features rather than repository-only integrations.

Gate:

Frontend, Rust, N-API, provider-host, real opt-in smoke, security, recovery,
diagnostics, editor, docs, package, cross-platform, benchmark, typecheck, and
warning-denying Clippy gates pass.

## 13. Verification Matrix

| Area | Required proof |
| --- | --- |
| Existing Slack | Every existing trigger, approval, lifecycle, diagnostic, and package test remains unchanged. |
| Telegram trigger | Polling, filtering, durable acceptance, offset advancement, duplicates, and restart are correct. |
| Discord trigger | Gateway heartbeat/resume/reconnect, intent/filter behavior, duplicates, and shutdown are correct. |
| WhatsApp trigger | Verification handshake, raw-body signature, phone routing, durable acknowledgement, and status filtering are correct. |
| Payload | Common fields are stable, bounded, JSON-safe, and contain no raw envelopes or credentials. |
| Notifications | Multiple destinations/providers preserve order and stable delivery identities. |
| Approval | First decision wins globally; every provider converges or records a bounded warning. |
| Lifecycle | Informational delivery cannot receive decision authority or rewrite business outcome. |
| Messaging | `send()` attempts, result validation, timeout, retry, rate limit, ambiguity, and cancellation are durable. |
| Secrets | Only symbolic references compile; resolved values never persist or render. |
| Diagnostics | Authentication, permissions, destinations, subscriptions, and callback readiness are actionable and redacted. |
| Custom providers | Imported notification providers still compose with all built-ins. |
| Control flow | Providers work inside sequential, choose/switch, parallel, fork, approval, call/start, and lifecycle paths. |
| Runtime | Foreground, background, multi-file, restart, cancellation, policies, backup, retention, and prune remain correct. |
| Presentation | Trigger instructions, step results, provider warnings, approval state, logs, plain, color, and JSON are truthful. |
| Package | Clean installations on all supported native targets contain and load the provider hosts. |
| Performance | Idle connections and concurrent provider traffic stay within reviewed budgets without slowing unrelated workflows materially. |

## 14. Explicit Non-Goals

This milestone does not add:

- a mandatory `woml connect` or browser OAuth flow;
- arbitrary raw provider API calls encoded as WOML tags;
- a generic `services.messaging` facade;
- custom trigger-provider execution authority;
- custom service-provider registration from `.woml` files;
- personal WhatsApp Web automation or unofficial WhatsApp clients;
- Discord voice, presence, moderation, member administration, or Activities;
- Telegram payments, games, Mini Apps, or MTProto client accounts;
- WhatsApp commerce catalogs, Flows, payments, campaigns, or contact-center UI;
- automatic creation of provider apps, bots, business accounts, channels, or
  approved WhatsApp templates;
- broad media/file support before storage, limits, and retention are reviewed;
- provider message-history synchronization;
- a hosted WOML relay service;
- weakening signature verification to make local callbacks easier;
- widening the existing literal-only `secrets.NAME` script binding;
- changing custom providers from notification-only extensions; or
- unrelated engine performance optimization.

## 15. Expected File Areas

| Area | Expected locations |
| --- | --- |
| WOML syntax and lowering | `woml/src/compiler.ts`, model/package schemas, source diagnostics, generated declarations |
| Shared communication adapters | focused modules under `woml-cli/src`, extracted from the current Slack-specific hosts |
| Telegram adapter | focused trigger/notification/messaging transport and fake conformance adapter under `woml-cli/src` |
| Discord adapter | focused Gateway/REST trigger/notification/messaging transport and fake conformance adapter under `woml-cli/src` |
| WhatsApp adapter | focused webhook/Cloud API trigger/notification/messaging transport and fake conformance adapter under `woml-cli/src` |
| Rust authority | `core/woml-engine` trigger ingress, capability, notification, approval, event, fold, recovery, and presentation modules |
| Native bridge | `core/woml-native/src/bridge.rs`, strict TypeScript decoders in `woml-cli/src/rust-executor.ts` |
| Production hosting | focused runtime-host composition modules; `cli.ts` remains orchestration only |
| Protocols and schemas | `docs/protocols`, `docs/schemas`, reviewed JSON fixtures |
| Diagnostics | provider doctor modules and stable safe diagnostic contracts |
| Editor | `woml-vscode` grammar, snippets, declarations, and provider diagnostics |
| Tests | descriptive frontend, Rust, provider-host, runtime, package, and manual opt-in suites |
| Examples and docs | provider examples plus trigger, notification, service, security, operations, and release documentation |

## 16. Risks and Guardrails

### A shared abstraction can erase important provider differences

Only common durable concepts are shared. Provider templates, identifiers,
transport behavior, permissions, and rate limits remain explicit adapter
contracts.

### Refactoring Slack can break the one working provider

ACP1 is behavior-preserving and independently gated before any new provider
depends on it. Existing Slack syntax and payloads do not change in place.

### Provider retries can duplicate messages

Known terminal calls are idempotent. Ambiguous network outcomes fail closed
unless the provider offers a reviewed mechanism that proves safe replay.

### Inbound provider redelivery can duplicate runs

Provider event identity, workflow ID, and trigger ID produce one durable
occurrence identity before a run is admitted.

### Communication payloads can become unstable or expose too much data

The common payload is versioned and bounded. Raw envelopes and undocumented
provider fields are not passed through for convenience.

### Multiple providers can race one approval

Rust's existing compare-and-commit decision authority remains the only place a
decision becomes final. Provider callbacks merely present requests.

### WhatsApp can look easier than its real business constraints

Template, session, business-account, public HTTPS, and phone-number rules are
shown explicitly. WOML rejects configurations it cannot honor truthfully.

### Persistent provider connections can consume excessive resources

Connections are shared by credential identity, subscriber counts are bounded,
idle/reconnect costs are measured, and shutdown closes every host cleanly.

### Provider code can turn `cli.ts` into a monolith

Each transport, adapter, diagnostic, normalization, and host concern lives in a
focused module. The CLI composes them but does not implement their protocols.

### Real credentials cannot be used in ordinary CI

Deterministic fake transports and official-protocol fixtures provide release
coverage. Real provider journeys are opt-in, redacted, and never required to
run untrusted pull requests.

## 17. Global Roadmap After This Milestone

1. **Initial WOML release and product feedback** — validate the complete
   language, runtime, editor, packaging, and communication experience with real
   workflow authors before expanding the surface further.
2. **Performance Profiling and Optimization** — after the initial release,
   measure startup, compilation, N-API, serialization, worker hosts, provider
   connections, short/large workflows, and concurrency, then optimize only
   proven bottlenecks. The optional Rust `quick-xml` investigation remains
   benchmark-gated and must never create two competing compilers.
3. **Communication extension contracts, if demanded** — review custom trigger
   and custom messaging-capability extensions only when real projects need
   providers that notification-only `.woml` modules cannot cover.
4. **Additional providers based on demand** — consider Microsoft Teams, email,
   SMS, or other adapters only from concrete product demand rather than growing
   the built-in surface speculatively.

The JavaScript-chaining SDK and legacy Cronflow execution/package surfaces are
already retired. Editor/theme work, legacy audit/removal, cross-platform native
release packaging, and all earlier WOML workflow/runtime milestones remain the
completed baseline and are not repeated as future work.

## 18. Definition of Done

This milestone is complete only when:

- Telegram, Discord, and WhatsApp each support a real trigger, approval and
  lifecycle notifications, and `services.<provider>.send()`;
- every provider construct has accepted syntax, executable lowering, and a
  production runtime implementation;
- existing Slack and custom-provider behavior remains compatible;
- provider credentials are configured through WOML secrets, only explicitly
  referenced values are injected into a script invocation, and no value enters
  context or durable state;
- inbound messages use a stable normalized bounded `context.payload`;
- provider occurrences are deduplicated before run creation;
- outbound message attempts obey existing retry, idempotency, cancellation,
  timeout, and ambiguity rules;
- one approval delivered to several providers has exactly one durable decision;
- lifecycle notification failures cannot change business outcomes;
- Telegram polling, Discord Gateway, and WhatsApp webhook restart behavior is
  proven;
- WhatsApp template/session restrictions are truthful and tested;
- provider diagnostics identify setup problems without leaking secrets or
  personal message content;
- foreground, background, multi-workflow, control-flow, workflow-call,
  runtime-policy, backup, retention, and inspection behavior composes;
- terminal output and log following present providers professionally in color,
  plain, and JSON modes;
- fake conformance adapters cover every release path and opt-in real journeys
  are documented;
- clean installed cross-platform packages contain all required hosts; and
- documentation, examples, security tests, benchmarks, typecheck, Clippy, and
  release gates pass.

## 19. ACP0 Review Gate

Before ACP1 implementation begins, review and approve:

1. exact Telegram trigger, notification, and `send()` syntax;
2. exact Discord trigger, notification, and `send()` syntax;
3. exact WhatsApp trigger, template/session, notification, and `send()` syntax;
4. messaging-service credential properties and preservation of the existing
   literal-only `secrets.NAME` binding;
5. the normalized communication payload and Slack compatibility aliases;
6. provider destination identity and multi-destination list rules;
7. supported inbound event sets, including whether Discord commands enter v1;
8. Telegram polling acknowledgement and webhook-conflict behavior;
9. Discord Gateway resume, intents, and interaction acknowledgement behavior;
10. WhatsApp callback verification, signature, phone routing, template, and
    public endpoint behavior;
11. cross-provider approval delivery, first-decision, update, and failure
    rules;
12. messaging retry, idempotency, rate-limit, cancellation, and ambiguous-send
    rules;
13. exact Model, Definition Package, Event, Host Protocol, and presentation
    versions; and
14. the safe provider error and diagnostic vocabulary.

ACP0 is complete only when these decisions exist as versioned schemas,
protocol documents, and reviewed fixtures—not only prose in this plan.

## 20. Official Provider References

- [Telegram Bot API](https://core.telegram.org/bots/api)
- [Discord Bots and Companion Apps](https://docs.discord.com/developers/platform/bots)
- [Discord Interactions](https://docs.discord.com/developers/platform/interactions)
- [Discord Gateway](https://docs.discord.com/developers/events/gateway)
- [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api/)
