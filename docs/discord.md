# Discord in WOML

Discord is executable through the WOML production runtime. A single configured
bot can start workflows from mentions or direct messages, send supervised
messages, deliver lifecycle notifications, and collect durable approval
decisions through Discord buttons.

## Prepare a Discord bot

In the Discord Developer Portal:

1. Create an application and add a bot.
2. Enable **Message Content Intent** on the bot page.
3. Install the bot in the server with **View Channels**, **Send Messages**, and
   **Read Message History** permissions for the channels WOML will use.
4. Enable Discord Developer Mode when you need to copy a numeric channel ID.

Store the bot token outside the workflow:

```bash
woml secrets set DISCORD_BOT_TOKEN
```

Never put the literal token in a `.woml` file. `woml check` validates a Discord
workflow without reading the secret or contacting Discord.

## Receive messages

```xml
<triggers>
  <discord
    id="agentMessage"
    events="app-mention,direct-message"
    channels="200000000000000001,200000000000000002"
    bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
  />
</triggers>
```

The supported events are `app-mention` and `direct-message`. `channels` is an
optional allowlist for mentions; it accepts numeric Discord channel IDs with
17 to 20 digits. Direct messages remain available when `direct-message` is
listed. Names such as `general` are rejected because names can be renamed or
duplicated. Slash commands are not part of this version.

The script receives a normalized `context.payload` with the message text,
sender, conversation, message ID, and provider metadata. Discord's raw Gateway
event is not exposed as a public language contract.

## Send a reply

```javascript
return services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: 'Hello from WOML',
  replyToMessageId: context.payload.messageId
}, { name: 'reply-to-message' });
```

`services.discord.send()` is supervised by the Rust core. It supports stable
operation names, timeouts, bounded results, rate-limit retry guidance, and
fail-closed handling when Discord does not confirm whether a send succeeded.
The result contains `provider`, `conversationId`, `messageId`, and `acceptedAt`.

## Approval and lifecycle notifications

An approval can target one or more channels. Every delivery belongs to the same
durable approval, so the first valid Approve or Reject button wins and all
delivered Discord messages have their buttons removed during convergence.

```xml
<notify>
  <discord
    channels="200000000000000001,200000000000000002"
    bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
  />
</notify>
```

Lifecycle notifications use the same `channels` and `bot-token` attributes and
also require `message`. A lifecycle notification failure is observable but does
not change the workflow's business outcome.

## Run the examples

For a conversational trigger:

```bash
woml run examples/discordWorkflow.woml
```

Mention the installed bot in a server channel or send it a direct message. WOML
keeps one shared Gateway connection per bot token and safely resumes that
session after recoverable disconnects.

For approval, replace the example channel ID with your numeric Discord channel
ID, then run:

```bash
woml run examples/discordApprovalWorkflow.woml
```

Press Enter to start the manual workflow, then choose Approve or Reject in
Discord. The terminal records the waiting run and its final result.

## Actionable failures

- `WOML_DISCORD_AUTH_FAILED`: the bot token was rejected.
- `WOML_DISCORD_INTENTS_MISSING`: enable Message Content Intent and verify the
  bot's server permissions.
- `WOML_DISCORD_DESTINATION_INVALID`: the channel does not exist or the bot
  cannot access it.
- `WOML_DISCORD_RATE_LIMITED`: Discord requested a bounded retry delay.
- `WOML_DISCORD_DELIVERY_AMBIGUOUS`: WOML cannot safely prove whether a message
  was sent and will not replay the side effect automatically.

Bot messages and the bot's own messages are ignored, provider occurrences are
deduplicated before run creation, and interaction tokens remain memory-only.
