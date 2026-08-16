# Telegram in WOML

WOML supports Telegram as a workflow trigger, an approval and lifecycle
notification provider, and a supervised messaging service.

## Setup

1. Create a bot with Telegram's official `@BotFather` account and copy its bot
   token.
2. Store the token without putting it in a `.woml` file:

   ```bash
   woml secrets set TELEGRAM_BOT_TOKEN
   ```

3. For approval notifications, obtain the numeric Telegram chat ID. Send the
   bot a message, then inspect the official Bot API `getUpdates` response before
   starting WOML:

   ```text
   https://api.telegram.org/bot<BOT_TOKEN>/getUpdates
   ```

   Use `message.chat.id`. Group and channel IDs are usually negative. Keep the
   token private and do not paste it into logs, source files, or bug reports.

## Message-triggered reply

Run the included example:

```bash
woml run examples/telegramWorkflow.woml
```

WOML prints that the Telegram bot is ready and remains active. Open a direct
conversation with the bot and send a text message. The durable Rust runtime
admits one run before Telegram's polling offset advances, then the bot replies
in the same conversation.

Only human text messages trigger v1. Bot-authored messages are ignored to
prevent common automation loops. In groups, Telegram's bot privacy settings
determine which messages Telegram delivers to the bot.

## Approval buttons

Replace `chats="123456789"` in
`examples/telegramApprovalWorkflow.woml` with your numeric chat ID, then run:

```bash
woml run examples/telegramApprovalWorkflow.woml
```

Press Enter to start the manual workflow. Telegram receives one message with
Approve and Reject buttons. The first valid decision is committed by the same
durable approval authority used by every provider; the buttons are removed
after settlement and the workflow continues.

One bot token may be reused by triggers, notifications, lifecycle hooks, and
`services.telegram.send()`. Telegram does not require a separate app-level
token.

## Script messaging

Inside a `<script>`, send a text message with:

```javascript
return services.telegram.send({
  botToken: secrets.TELEGRAM_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: 'Hello from WOML',
  replyToMessageId: context.payload.messageId
}, { name: 'reply-to-message' });
```

The operation name is required when a step performs more than one effectful
operation. WOML supervises attempts, timeout, cancellation, result limits, and
durable completion. An uncertain network result fails closed instead of
silently sending a duplicate message.

## Common errors

- `WOML_SECRET_NOT_FOUND`: configure the named token with `woml secrets set`.
- `WOML_TELEGRAM_AUTH_FAILED`: BotFather token is invalid or was revoked.
- `WOML_TELEGRAM_DESTINATION_INVALID`: verify the numeric chat ID and ensure
  the bot belongs to or can access that chat.
- `WOML_TELEGRAM_RATE_LIMITED`: Telegram requested a safe delayed retry.
- `WOML_TELEGRAM_DELIVERY_AMBIGUOUS`: Telegram did not confirm whether a send
  occurred, so WOML deliberately did not replay it.
