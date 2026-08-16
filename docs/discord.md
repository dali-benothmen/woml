# Discord in WOML

Discord authoring is available as an ACP4 preview. WOML validates and lowers
Discord triggers, approval notifications, lifecycle notifications, and
`services.discord.send()` into the frozen Model v15 contract. Network execution
starts in ACP5.

## Configure the bot token

Store the Discord bot token outside the workflow:

```bash
woml secrets set DISCORD_BOT_TOKEN
```

Never put the literal token in a `.woml` file. ACP4 does not contact Discord,
so a token is not needed merely to run `woml check`.

## Trigger authoring

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

The supported event names are `app-mention` and `direct-message`. The optional
`channels` filter accepts numeric Discord channel IDs containing 17 to 20
digits. Enable Discord Developer Mode and copy the channel ID; names such as
`general` are intentionally rejected. Slash commands are deferred.

## Approval and lifecycle notifications

An approval targets one or more channels while sharing one durable decision:

```xml
<notify>
  <discord
    channels="200000000000000001,200000000000000002"
    bot-token="{{secrets.DISCORD_BOT_TOKEN}}"
  />
</notify>
```

Lifecycle notifications use the same destination and credential attributes and
also require `message`.

## Script messaging

```javascript
return services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: 'Hello from WOML',
  replyToMessageId: context.payload.messageId
}, { name: 'reply-to-message' });
```

The request accepts only `botToken`, `conversationId`, `text`, and the optional
`replyToMessageId`. The optional operation options object accepts `name`.

## ACP4 command behavior

Validate a Discord workflow now:

```bash
woml check workflow.woml
```

The command reports that Discord authoring and lowering are valid. Running the
workflow before ACP5 fails with `WOML_DISCORD_RUNTIME_UNAVAILABLE`; WOML never
pretends that the bot is connected or silently ignores Discord operations.
