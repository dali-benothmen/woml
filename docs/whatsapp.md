# WhatsApp in WOML

WOML uses Meta's official WhatsApp Cloud API. A WhatsApp workflow can receive
messages, send approved templates, deliver lifecycle notifications, and resolve
Human Approval buttons without installing a provider package.

## Required Meta values

From the Meta application and WhatsApp Business setup, collect:

- the **Phone Number ID** (this is not the displayed phone number);
- a production access token with WhatsApp messaging permission;
- the Meta application secret; and
- a private verification token that you choose.

Store credentials without writing them into `.woml` files:

```bash
woml secrets set WHATSAPP_ACCESS_TOKEN
woml secrets set WHATSAPP_APP_SECRET
woml secrets set WHATSAPP_VERIFY_TOKEN
```

Use the Phone Number ID as the literal `phone-number-id` attribute. Replace the
placeholder value in [the runnable example](../examples/whatsappWorkflow.woml)
before starting it.

## Callback setup

Run the workflow:

```bash
woml run examples/whatsappWorkflow.woml
```

The terminal shows `/callbacks/whatsapp`. Meta requires a publicly reachable
HTTPS callback, so production must expose that route through the deployment's
HTTPS domain or reverse proxy. Configure the same value stored as
`WHATSAPP_VERIFY_TOKEN`, subscribe the Meta application to WhatsApp `messages`,
and keep the application secret only in WOML's secret store.

WOML verifies `X-Hub-Signature-256` over the exact raw callback bytes before it
parses JSON. It records a trigger occurrence durably before acknowledging the
message, and repeated Meta deliveries with the same message ID reuse the same
run.

## Templates

Outside an open customer-service session, WhatsApp requires a Meta-approved
template. The example expects `woml_reply_v1` with one body variable.

Human Approval expects an approved template such as `woml_approval_v1` with:

1. approval name;
2. approval description;
3. workflow name;
4. deadline; and
5. two quick-reply buttons in this order: **Approve**, **Reject**.

Lifecycle notification templates receive the rendered WOML message as their
first body parameter. Template names and parameter counts must match the
approved Meta template exactly.

## Script payload and result

An inbound message exposes the stable fields under `context.payload`:

- `provider` (`"whatsapp"`);
- `event` (`"message"`);
- `senderId`;
- `conversationId` (the sender number);
- `conversationType` (`"direct"`);
- `messageId`;
- `phoneNumberId`;
- `text` when the incoming message contains text;
- `occurredAt` (plus the original provider `timestamp`); and
- `replyToMessageId` when present.

`services.whatsapp.send()` returns the provider, conversation ID, accepted
message ID, and acceptance time. Tokens are never copied into the workflow
result, event metadata, terminal output, or diagnostics.
