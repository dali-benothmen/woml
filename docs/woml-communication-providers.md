# WOML Communication Providers

WOML includes Slack, Telegram, Discord, and WhatsApp as communication
providers. Each built-in provider can participate in three product surfaces:

| Surface | Purpose |
| --- | --- |
| Trigger | Start a durable workflow from an inbound message. |
| Notification | Deliver Human Approval or informational lifecycle messages. |
| Messaging service | Send an ordinary message from a script under Rust supervision. |

All providers use secrets configured with `woml secrets set`; credentials never
belong in a `.woml` file. Telegram is the simplest local starting point.
WhatsApp has the most production setup because Meta requires an HTTPS callback
and approved outbound templates.

## Setup at a glance

| Provider | Required setup | WOML secrets | Destination identity |
| --- | --- | --- | --- |
| Slack | Socket Mode, event subscriptions, bot membership, required OAuth scopes | `SLACK_BOT_TOKEN`, `SLACK_APP_TOKEN` | Channel name or ID |
| Telegram | Bot created with BotFather | `TELEGRAM_BOT_TOKEN` | Numeric chat ID |
| Discord | Bot application, Message Content intent, View/Send/Read permissions | `DISCORD_BOT_TOKEN` | Numeric channel ID |
| WhatsApp | Meta app, WhatsApp Business number, public HTTPS callback, approved templates | `WHATSAPP_ACCESS_TOKEN`, `WHATSAPP_APP_SECRET`, `WHATSAPP_VERIFY_TOKEN` | Recipient number and Phone Number ID |

Provider-specific setup and troubleshooting are documented in
[Slack](woml-notifications.md), [Telegram](telegram.md),
[Discord](discord.md), and [WhatsApp](whatsapp.md).

## Configure and verify secrets

Set only the providers a workflow uses:

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml secrets set DISCORD_BOT_TOKEN
woml secrets set WHATSAPP_ACCESS_TOKEN
woml secrets set WHATSAPP_APP_SECRET
woml secrets set WHATSAPP_VERIFY_TOKEN
```

Run the read-only provider diagnostics before activating a workflow:

```bash
woml telegram doctor
woml discord doctor
woml whatsapp doctor \
  --phone-number-id 123456789012345 \
  --callback-url https://automation.example/callbacks/whatsapp
```

Add `--json` for stable machine-readable output. Doctor commands authenticate
and inspect safe provider metadata; they never send a message or print a token.

## Trigger authoring

Communication triggers live inside `<triggers>`:

```xml
<telegram
  id="messageReceived"
  events="message"
  bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}"
/>

<discord
  id="messageReceived"
  events="app-mention,direct-message"
  channels="200000000000000001"
  bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
/>

<whatsapp
  id="messageReceived"
  events="message"
  phone-number-id="123456789012345"
  verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
  app-secret="{{secrets.WHATSAPP_APP_SECRET}}"
/>
```

Telegram uses one shared long-polling connection per bot credential. Discord
uses one shared resumable Gateway connection per bot credential. WhatsApp uses
the production HTTP listener at `/callbacks/whatsapp`; Meta must be configured
with the same verification token and a public HTTPS URL for that route.

## Stable trigger payload

Every communication trigger exposes a normalized object through
`context.payload`. The common public fields are:

| Field | Meaning |
| --- | --- |
| `provider` | `slack`, `telegram`, `discord`, or `whatsapp` |
| `event` | Normalized event such as `message`, `app-mention`, or `direct-message` |
| `text` | Bounded message text |
| `senderId` | Provider sender identity |
| `senderName` | Optional safe display name |
| `conversationId` | Chat, channel, or direct-conversation identity |
| `conversationType` | `direct`, `group`, or `channel` when available |
| `messageId` | Stable provider message identity |
| `replyToMessageId` | Optional parent message identity |
| `threadId` | Optional thread identity |
| `occurredAt` | Normalized RFC 3339 provider timestamp |
| `providerData` | Small provider-specific scalar metadata |

Slack keeps its reviewed compatibility aliases (`userId`, `channelId`,
`messageTs`, `threadTs`, and `teamId`). Telegram stores the bot identity under
`providerData.botId`; Discord may expose guild metadata under `providerData`;
WhatsApp exposes `providerData.phoneNumberId`. Raw provider envelopes, headers,
signatures, tokens, and profiles are never copied into workflow context.

The exact frozen contract is
[Communication Trigger Payload v1](schemas/communication-trigger-payload.v1.schema.json).

## Messaging services

### Telegram

```js
const sent = await services.telegram.send({
  botToken: secrets.TELEGRAM_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: 'Hello from WOML',
  replyToMessageId: context.payload.messageId
}, { name: 'telegram-reply' });
```

### Discord

```js
const sent = await services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: 'Hello from WOML',
  replyToMessageId: context.payload.messageId
}, { name: 'discord-reply' });
```

### WhatsApp

```js
const sent = await services.whatsapp.send({
  accessToken: secrets.WHATSAPP_ACCESS_TOKEN,
  phoneNumberId: '123456789012345',
  conversationId: context.payload.conversationId,
  template: {
    name: 'woml_reply_v1',
    language: 'en_US',
    parameters: ['Hello from WOML']
  }
}, { name: 'whatsapp-reply' });
```

Telegram and Discord requests accept `botToken`, `conversationId`, `text`, and
optional `replyToMessageId`. WhatsApp v1 intentionally accepts only an approved
template request with `accessToken`, `phoneNumberId`, `conversationId`, and a
`template` containing `name`, `language`, and string `parameters`.

Every successful send returns:

```js
{
  provider,       // "telegram", "discord", or "whatsapp"
  conversationId,
  messageId,
  acceptedAt,     // RFC 3339
  threadId        // optional
}
```

The optional second argument gives the effect a stable name. WOML records the
attempt and bounded result, applies cancellation and result limits, honors
provider rate-limit timing, and fails an ambiguous network outcome closed
instead of claiming a send definitely failed.

## Approval and lifecycle notification

One `<notify>` can contain several providers and several destinations. In a
Human Approval, every delivered message receives a distinct capability token,
but all tokens resolve the same durable decision. Approving from Telegram, for
example, immediately makes a later Discord rejection unable to reverse it.

Lifecycle notifications are informational. They never receive decision
authority and a delivery failure cannot rewrite the workflow's business
outcome. See the runnable
[multi-provider example](../examples/multiProviderApprovalWorkflow.woml).

WhatsApp notification delivery uses reviewed Meta templates. Approval buttons
must match the frozen approve/reject ordering. Because an approved-template
message cannot be edited after delivery, convergence records a visible bounded
warning rather than claiming the WhatsApp message was updated.

## Local and production operation

Run the conversational examples from the project root:

```bash
woml run examples/telegramWorkflow.woml
woml run examples/discordWorkflow.woml
woml run examples/whatsappWorkflow.woml
```

Telegram works locally without inbound internet access. Discord makes an
outbound Gateway connection. WhatsApp requires the callback to be reachable
over HTTPS. For a background runtime, use normal WOML operations:

```bash
woml run examples/telegramWorkflow.woml --background
woml inspect
```

Use one runtime owner for each trigger workflow and persistent local state.
Back up the SQLite authority consistently, test restore on a copy, and rotate a
provider token by updating the same symbolic WOML secret before restarting the
runtime. Detailed outage, reconnect, retention, privacy, and recovery guidance
is in [Communication Provider Diagnostics and Operations](communication-provider-operations.md).

## Extension direction

Built-ins are the polished baseline, not a promise that WOML core will absorb
every provider. Project-owned reusable notification providers remain available
today. Broader trigger and messaging extensions are a future reviewed boundary;
see [Provider Extension Architecture Notes](../WOML%20Provider%20Extension%20Architecture%20Notes.md).

