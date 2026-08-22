# Communication Providers

Read this reference when authoring Slack, Telegram, Discord, WhatsApp, approval notifications, lifecycle notifications, or provider messaging. Never put literal tokens in `.woml` source.

## Capability matrix

| Provider | Trigger | Approval notification | Lifecycle notification | Script messaging |
| --- | :---: | :---: | :---: | :---: |
| Slack | Yes | Yes | Yes | No built-in service |
| Telegram | Yes | Yes | Yes | `services.telegram.send()` |
| Discord | Yes | Yes | Yes | `services.discord.send()` |
| WhatsApp | Yes | Yes | Yes | `services.whatsapp.send()` templates |
| Imported custom provider | No | Yes | Yes | Use its imported module/API design |

## Shared trigger payload

Communication triggers normalize input into `context.payload` with safe fields:

```js
{
  provider,
  event,
  text,
  senderId,
  senderName?,
  conversationId,
  conversationType?,
  messageId,
  replyToMessageId?,
  threadId?,
  occurredAt,
  providerData
}
```

Slack also retains compatibility aliases `userId`, `channelId`, `messageTs`, `threadTs`, and `teamId`. Never expect raw provider envelopes, request headers, signatures, credentials, or full user profiles in context.

## Slack

Required secrets:

```bash
woml secrets set SLACK_BOT_TOKEN
woml secrets set SLACK_APP_TOKEN
```

The Slack app needs Socket Mode, event subscriptions, bot membership in target channels, and OAuth scopes appropriate to its configured events/delivery. App tokens use Socket Mode; bot tokens authenticate Web API operations.

Trigger:

```xml
<slack
  id="agentMessage"
  events="app-mention,direct-message"
  channels="woml-testing,agent-support"
  bot-token="{{secrets.SLACK_BOT_TOKEN}}"
  app-token="{{secrets.SLACK_APP_TOKEN}}"
/>
```

- `events` is one or both of `app-mention` and `direct-message`.
- Optional `channels` limits app mentions; omission accepts all visible channels.
- Channel aliases/IDs must match the current Slack authoring contract.

Approval notification:

```xml
<notify>
  <slack channels="#approvals #engineering" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
</notify>
```

Lifecycle notification adds required `message`:

```xml
<notify>
  <slack channels="#incidents" message="Workflow {{lifecycle.workflow.id}} failed" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
</notify>
```

There is no released `services.slack.send()`. Do not generate it.

## Telegram

Create a bot with BotFather and store its token:

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml telegram doctor --destination -1001234567890
```

Trigger:

```xml
<telegram id="agentMessage" events="message" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
```

Approval notification uses comma-separated numeric chat IDs and forbids `message`:

```xml
<telegram chats="-1001234567890,123456789" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
```

Lifecycle notification requires `message`:

```xml
<telegram chats="-1001234567890" message="Workflow completed" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
```

Messaging:

```js
return services.telegram.send({
  botToken: secrets.TELEGRAM_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: "Hello from WOML",
  replyToMessageId: context.payload.messageId
}, { name: "telegram-reply" });
```

Telegram uses long polling and does not require a public inbound URL for local use.

## Discord

Create a Discord application/bot, enable the Message Content intent, grant View Channel, Send Messages, and Read Message History where needed, invite it to the server, then configure:

```bash
woml secrets set DISCORD_BOT_TOKEN
woml discord doctor --destination 200000000000000001
```

Trigger:

```xml
<discord
  id="agentMessage"
  events="app-mention,direct-message"
  channels="200000000000000001,200000000000000002"
  bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
/>
```

`channels` accepts numeric IDs (17–20 digits), not mutable names. It is optional.

Approval notification:

```xml
<discord channels="200000000000000001" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />
```

Lifecycle notification adds required `message`:

```xml
<discord channels="200000000000000001" message="Workflow completed" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />
```

Messaging:

```js
return services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: "Hello from WOML",
  replyToMessageId: context.payload.messageId
}, { name: "discord-reply" });
```

Discord uses an outbound resumable Gateway connection.

## WhatsApp

WhatsApp requires a Meta app, WhatsApp Business number, Phone Number ID, public HTTPS callback, callback verification token, app secret, Cloud API access token, and approved outbound templates.

```bash
woml secrets set WHATSAPP_ACCESS_TOKEN
woml secrets set WHATSAPP_APP_SECRET
woml secrets set WHATSAPP_VERIFY_TOKEN
woml whatsapp doctor \
  --phone-number-id 123456789012345 \
  --callback-url https://automation.example/callbacks/whatsapp
```

Trigger:

```xml
<whatsapp
  id="customerMessage"
  events="message"
  phone-number-id="123456789012345"
  verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
  app-secret="{{secrets.WHATSAPP_APP_SECRET}}"
/>
```

The callback route is `/callbacks/whatsapp`; Meta must use the same verification token. Status callbacks do not start workflow runs.

Approval notification:

```xml
<whatsapp
  recipients="15551234567"
  access-token="{{secrets.WHATSAPP_ACCESS_TOKEN}}"
  phone-number-id="123456789012345"
  template="approval_request_v1"
  language="en_US"
/>
```

Lifecycle notification additionally requires `message`, which becomes the first template body parameter:

```xml
<whatsapp
  recipients="15551234567"
  access-token="{{secrets.WHATSAPP_ACCESS_TOKEN}}"
  phone-number-id="123456789012345"
  template="workflow_status_v1"
  language="en_US"
  message="Workflow completed"
/>
```

Messaging uses approved templates only:

```js
return services.whatsapp.send({
  accessToken: secrets.WHATSAPP_ACCESS_TOKEN,
  phoneNumberId: "123456789012345",
  conversationId: context.payload.conversationId,
  template: {
    name: "woml_reply_v1",
    language: "en_US",
    parameters: ["Hello from WOML"]
  }
}, { name: "whatsapp-reply" });
```

Do not generate proactive free-form WhatsApp sends.

## Multiple approval providers

One `<notify>` may contain multiple provider tags and destinations. Each successful delivery gets its own capability token, but every token controls the same approval. The first valid approve/reject wins globally; another provider cannot reverse it.

WOML settles configured notification delivery before waiting. At least one delivery must succeed. If every delivery fails, the run fails instead of waiting invisibly.

Approval provider tags do not accept informational `message`; WOML builds the approval prompt and actions. Lifecycle provider tags require `message` and never carry decision authority.

## Custom notification providers

Import a `.woml` definition containing `<provider kind="notification">` and use the alias directly under `<notify>`. Read [modules.md](modules.md). Custom providers transport WOML's bounded notification object; they do not own approval tokens, durable retries, or decision settlement.

## Message and secret safety

- Notification `message` permits bounded scalar `{{context...}}` and `{{lifecycle...}}` templates.
- Secrets are forbidden in message content.
- Provider credentials must be exact `{{secrets.NAME}}` references.
- Doctor output, events, inspection, and normal logs redact credentials and full message bodies.
- Use stable operation names for script messaging when one step may send multiple logical messages.
