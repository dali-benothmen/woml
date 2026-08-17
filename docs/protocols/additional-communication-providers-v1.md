# WOML Additional Communication Providers v1

Status: **frozen in ACP0 and implemented through ACP10**.

This document freezes the public and internal shapes shared by Slack,
Telegram, Discord, and WhatsApp. Telegram became executable in ACP3, Discord
in ACP5, and WhatsApp in ACP7; ACP10 packages and publishes all three without
changing these frozen shapes.

## 1. Version decision

| Contract | Decision |
| --- | --- |
| Communication Trigger Payload | v1, new |
| Communication Trigger Host | v1, new and asynchronous |
| Communication Notification Adapter | v1, new internal adapter boundary |
| Communication Messaging | v1, new logical profile over Capability Call v1 |
| Compiled Workflow Model | v15 reserved; Model v14 remains immutable |
| Definition Package | v10 reserved; Package v9 remains immutable |
| Run Event Vocabulary | v14 reserved; Event v13 remains immutable |
| Durable Store | remains v14; no persistence shape change is needed |
| Script Host | remains v8 |
| Capability Call | remains v1 |
| Run Presentation | remains v1 because provider identifiers are open strings |
| Slack notification protocols | v1/v2 remain immutable and compatible |
| Slack trigger protocol | v1 remains immutable and compatible |

The complete Model v15 and Package v10 schemas are introduced when ACP2 adds
the first executable consumer. ACP0 freezes their communication fragments in
`communication-provider-model.v1.schema.json`; implementation may not change
those fragments while assembling the complete schemas.

Event v14 preserves WOML's single durable vocabulary. The logical facts in
`communication-provider-run-event.v1.schema.json` map as follows:

| Communication fact | Persisted Event v14 representation |
| --- | --- |
| delivery requested | existing `notification_delivery_requested` and attempt event |
| delivery succeeded | existing `notification_delivery_succeeded` with a provider-neutral message identity |
| delivery failed | existing `notification_delivery_failed` |
| interaction received | existing `notification_decision_accepted` after Rust commits the decision |
| message sent | existing capability `operation_succeeded` result and safe metadata |

The complete additive event schema is
`docs/schemas/run-event.v14.schema.json`. WOML does not persist parallel
`communication_*` events for the same side effect.

## 2. Exact authoring syntax

List attributes are comma-separated, trimmed, non-empty, ordered, and reject
duplicates. Every credential value below is exactly one literal
`{{secrets.NAME}}` reference. Plaintext credentials and computed secret names
are invalid.

### 2.1 Triggers

```xml
<telegram
  id="agentMessage"
  events="message"
  bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
/>

<discord
  id="agentMessage"
  events="app-mention,direct-message"
  channels="123456789012345678,234567890123456789"
  bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
/>

<whatsapp
  id="customerMessage"
  events="message"
  phone-number-id="123456789012345"
  verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
  app-secret="{{secrets.WHATSAPP_APP_SECRET}}"
/>
```

Exact trigger attributes:

| Tag | Required | Optional | v1 events |
| --- | --- | --- | --- |
| `telegram` | `id`, `events`, `bot-token` | none | `message` |
| `discord` | `id`, `events`, `bot-token` | `channels` | `app-mention`, `direct-message` |
| `whatsapp` | `id`, `events`, `phone-number-id`, `verify-token`, `app-secret` | none | `message` |

Trigger tags are empty elements and are valid only as direct children of
`<triggers>`. Telegram commands are ordinary `message` events in v1. Discord
slash commands and public interaction triggers are deferred. WhatsApp status
callbacks never create workflow runs.

### 2.2 Notifications

```xml
<notify>
  <telegram
    chats="-1001234567890,123456789"
    bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
  />
  <discord
    channels="123456789012345678"
    bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
  />
  <whatsapp
    recipients="15551234567"
    access-token="{{secrets.WHATSAPP_ACCESS_TOKEN}}"
    phone-number-id="123456789012345"
    template="woml_approval_v1"
    language="en_US"
  />
</notify>
```

Exact notification attributes:

| Tag | Approval required | Lifecycle required | Forbidden |
| --- | --- | --- | --- |
| `telegram` | `chats`, `bot-token` | same plus `message` | trigger attributes |
| `discord` | `channels`, `bot-token` | same plus `message` | trigger attributes |
| `whatsapp` | `recipients`, `access-token`, `phone-number-id`, `template`, `language` | same plus `message` | free-form delivery without a template |

Approval tags derive their presentation from the approval's name and
description, so `message` is forbidden there. Lifecycle notifications require
`message`. For WhatsApp, `message` is supplied as the first body-template
parameter; the configured template remains the external delivery contract.
One destination creates one durable delivery in source order.

The WhatsApp approval template must expose exactly two quick-reply buttons
whose provider payloads are `woml_approve` and `woml_reject`. WOML supplies the
approval name, description, workflow name, and deadline as four ordered body
parameters. A provider/template mismatch is an actionable activation or
delivery error; WOML never silently falls back to proactive free-form text.

### 2.3 Messaging services

```js
await services.telegram.send({
  botToken: secrets.TELEGRAM_BOT_TOKEN,
  conversationId: '-1001234567890',
  text: 'Build completed',
  replyToMessageId: '41'
}, { name: 'build-completed' });

await services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: '123456789012345678',
  text: 'Build completed',
  replyToMessageId: '234567890123456789'
});

await services.whatsapp.send({
  accessToken: secrets.WHATSAPP_ACCESS_TOKEN,
  phoneNumberId: '123456789012345',
  conversationId: '15551234567',
  template: {
    name: 'build_completed',
    language: 'en_US',
    parameters: ['build-42']
  }
});
```

Telegram and Discord require `botToken`, `conversationId`, and `text`, and
accept optional `replyToMessageId`. WhatsApp requires `accessToken`,
`phoneNumberId`, `conversationId`, and the exact `template` object shown above.
Unknown keys are rejected. V1 does not expose edit, delete, media, Discord
embeds, Telegram parse modes, or unreviewed raw provider JSON.

Every messaging call accepts an optional second `{ name }` operation-options
object. The stable lowercase name participates in WOML's existing durable
idempotency identity; it does not change the provider request body.

Successful calls return:

```json
{
  "provider": "telegram",
  "conversationId": "-1001234567890",
  "messageId": "42",
  "acceptedAt": "2026-08-16T10:00:00.000Z"
}
```

`threadId` is added only when the provider returns a real thread identity.

## 3. Normalized trigger payload

Every new communication trigger exposes the bounded Payload v1 object as
`context.payload`. Common required fields are `provider`, `event`, `text`,
`senderId`, `conversationId`, `messageId`, and `occurredAt`. Optional common
fields are omitted rather than invented.

Slack keeps its already-published `type`, `userId`, `channelId`, `messageTs`,
`threadTs`, and `teamId` fields while receiving common aliases additively.
Existing Slack source therefore does not change. Raw envelopes, HTTP headers,
signatures, tokens, entire profiles, and attachments are forbidden.

The normalized accepted message and routing identifiers are workflow input and
therefore may be persisted in the event-sourced run context. They are bounded,
excluded from default operational logs and diagnostics, and governed by WOML
retention/prune policy. Credentials and raw provider envelopes are never run
context.

## 4. Provider transport and acknowledgement

- Telegram v1 uses long polling. One poller is shared per bot credential
  identity. A configured Telegram webhook is reported and never overwritten.
  Its offset advances only after Rust returns durable accepted-or-duplicate.
- Discord v1 uses one Gateway connection per bot credential identity. The
  adapter owns heartbeat, resume, reconnect, intents, and bot-message
  filtering. Run admission still belongs to Rust.
- WhatsApp v1 uses the existing production HTTP listener and official Cloud
  API only at the stable `/callbacks/whatsapp` route. Verification checks the
  configured token. POST signatures are
  checked against the exact raw bytes before JSON decoding. HTTP success is
  returned only after durable accepted-or-duplicate admission.

The Trigger Host protocol is multiplexed from v1: several occurrences may be
in flight, `receiptId` correlates outcomes, and outcomes may arrive out of
order. Provider callbacks and offsets are never acknowledged after a rejected
or uncertain admission.

## 5. Delivery, approval, and failure rules

Each destination has its own delivery and decision capability. All providers
for one approval converge on the same durable approval: the first valid
decision wins and later interactions observe that result. Provider adapters
may transport a decision but cannot decide or resume a run themselves.

Stable failure kinds are `request_invalid`, `secret_not_found`,
`authentication_failed`, `permission_denied`, `destination_not_found`,
`rate_limited`, `provider_unavailable`, `transport_failed`,
`response_invalid`, and `size_limit_exceeded`. Every failure includes a stable
WOML code, safe message, `retryable`, and `ambiguous`. An ambiguous send fails
closed and is not blindly replayed.

Provider rate-limit responses may include `retryAfterMs`. Existing WOML
attempt/idempotency authority owns retries; an adapter never creates an
unrecorded retry loop.

## 6. Name resolution and compatibility

- An explicitly imported local notification provider alias wins inside
  `<notify>` and produces a non-fatal shadowing diagnostic when it has a
  built-in name.
- Without that import, provider names resolve to built-ins.
- Provider tags under `<triggers>` always use built-in trigger grammar.
- An explicitly imported JS/TS module alias wins for `services.<name>` and
  produces the same shadowing diagnostic. The built-in remains reachable after
  the author renames the local alias.
- Existing custom providers, Slack markup, Slack payload fields, protocols,
  events, diagnostics, and runtime behavior remain unchanged.

## 7. Host activation and security

Provider hosts start from declared requirements: inbound triggers start a
listener; pending actionable notifications start their callback path; outbound
notifications and `send()` start only outbound transport. Shared credential
identities reuse safe connections where supported.

Secrets are resolved only for an invocation, are never emitted by protocols,
and are redacted from failures. Safe account identifiers may be logged;
message bodies, recipients, raw provider responses, signatures, decision
capabilities, and credentials may not appear in default diagnostics.

The normative machine-readable artifacts are the six
`docs/schemas/communication-*.v1.schema.json` files, the assembled
`compiled-workflow-model.v15.schema.json` and
`woml-definition-package.v10.schema.json` contracts, and the reviewed fixtures
in `woml/tests/fixtures/communication-providers`.
