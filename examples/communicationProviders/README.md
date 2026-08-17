# Communication Provider Examples

These examples use real provider accounts and are intended for manual local
testing. `woml check` is always offline; `woml run` contacts only the providers
referenced by the selected workflow.

## Conversational workflows

| Provider | Example | Start the conversation |
| --- | --- | --- |
| Telegram | `../telegramWorkflow.woml` | Send a text message to the configured bot. |
| Discord | `../discordWorkflow.woml` | Mention the bot in an allowed channel or send it a direct message. |
| WhatsApp | `../whatsappWorkflow.woml` | Send a message to the configured WhatsApp Business number through Meta's callback. |

Configure secrets first:

```bash
woml secrets set TELEGRAM_BOT_TOKEN
woml secrets set DISCORD_BOT_TOKEN
woml secrets set WHATSAPP_ACCESS_TOKEN
woml secrets set WHATSAPP_APP_SECRET
woml secrets set WHATSAPP_VERIFY_TOKEN
```

Then replace the documented destination or Phone Number ID placeholders and
run one workflow:

```bash
woml run examples/telegramWorkflow.woml
woml run examples/discordWorkflow.woml
woml run examples/whatsappWorkflow.woml
```

## Multi-provider approval and lifecycle

`../multiProviderApprovalWorkflow.woml` sends one approval through Slack,
Telegram, Discord, and WhatsApp. Replace every placeholder destination and
configure all referenced secrets before running it:

```bash
woml secrets set SLACK_BOT_TOKEN
woml secrets set SLACK_APP_TOKEN
woml run examples/multiProviderApprovalWorkflow.woml
```

Press Enter to start the run. Approve or reject from any delivered provider.
The first valid decision wins globally; later provider actions cannot reverse
it. Every provider is then asked to deliver the informational completion
message. WhatsApp truthfully records a bounded warning because approved
template messages cannot be edited after delivery.

Never commit real tokens, recipient IDs, private callback URLs, or provider
response bodies. The numeric values in these examples are intentionally
fictional placeholders.

