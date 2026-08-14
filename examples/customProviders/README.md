# Custom Telegram notification provider

This opt-in example sends an informational workflow-success message through a
user-authored `.woml` provider. WOML supervises the provider invocation; the
provider uses the managed HTTP service and returns only Telegram's message ID.

Configure the two values once:

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml secrets set TELEGRAM_CHAT_ID
```

Then run:

```bash
woml run examples/customProviders/telegramWorkflow.woml
```

The bot token is bound only when the provider runs and is not written into the
compiled workflow or durable event history.
