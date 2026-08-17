# Communication Provider Diagnostics and Operations

WOML provides read-only diagnostics for Telegram, Discord, and WhatsApp. The
commands authenticate only against the selected provider, return stable WOML
codes, and never send a test message.

```bash
woml telegram doctor
woml discord doctor
woml whatsapp doctor --phone-number-id 123456789012345 \
  --callback-url https://automation.example/callbacks/whatsapp
```

Use `--destination` to verify access to one Telegram chat or Discord channel.
Use `--json` for automation. Human output supports
`--color=auto|always|never` and respects `NO_COLOR`.

```bash
woml telegram doctor --destination -1001234567890 --json
woml discord doctor --destination 200000000000000001 --color=always
```

The exit status is `0` for healthy or degraded diagnostics, `1` when a check
fails, and `2` when the command arguments are invalid. JSON uses the stable
`woml.provider-doctor/v1` profile and contains only check IDs, statuses, WOML
codes, and safe messages.

## What the checks cover

- Telegram: configured token, bot authentication, webhook/long-polling
  conflict, and optional chat access.
- Discord: configured token, bot and application identity, Message Content
  intent, and optional channel visibility.
- WhatsApp: access token, app secret, verification token, Phone Number ID,
  and the public HTTPS callback shape.

Secret names can be overridden when a project does not use the defaults:

```bash
woml telegram doctor --token-secret PROJECT_TELEGRAM_TOKEN
woml discord doctor --token-secret PROJECT_DISCORD_TOKEN
woml whatsapp doctor \
  --access-token-secret PROJECT_WHATSAPP_TOKEN \
  --app-secret PROJECT_META_APP_SECRET \
  --verify-token-secret PROJECT_WHATSAPP_VERIFY \
  --phone-number-id 123456789012345
```

Raw provider responses, callback URLs returned by providers, tokens,
signatures, approval capabilities, message bodies, and recipient identities
are never printed by these commands.

## Resource and security boundaries

One WOML process accepts at most 64 credential connections and 1,000 trigger
subscribers per communication transport. Provider batches accept at most 100
items, inbound normalized message text is bounded to 40,000 UTF-8 bytes,
outbound provider requests are bounded to 64 KiB, and provider responses are
bounded to 1 MiB. Credentials are bounded to 16 KiB and reject control
characters. Diagnostic responses use a smaller 64 KiB limit.

WhatsApp callbacks accept at most 1 MiB, verify Meta's signature over the exact
raw bytes before JSON parsing, validate bounded identifiers and approval
capabilities, and reject oversized signed batches before durable admission.
Telegram and Discord normalize only reviewed message and callback shapes.

Provider credentials remain secret references. They are not copied into run
context, events, runtime logs, backups, or diagnostic output. A workflow's
bounded normalized message input may be retained with that run because the
script needs it; normal WOML retention and prune policy controls its lifetime.

## Recovery playbook

When authentication or permissions change:

1. replace the value with `woml secrets set <NAME>`;
2. run the provider's `doctor` command;
3. restart the active WOML runtime so its connection uses the new credential;
4. inspect background output with `woml <workflow-id> --logs`; and
5. retry only a new business operation when WOML reports an ambiguous send.

WOML does not automatically replay an outbound send whose provider outcome is
unknown. Confirmed rate limits retain provider retry guidance; revoked tokens
and permissions fail with actionable terminal codes. Telegram detects a
competing webhook/poller, Discord resumes recoverable Gateway sessions, and
WhatsApp relies on Meta's durable callback redelivery plus WOML occurrence
deduplication.

Use `woml backup` before maintenance, `woml restore` only into an inactive
runtime, and `woml prune` for reviewed retention cleanup. Provider events and
message identities follow the same backup, restore, inspection, and retention
authority as every other WOML run.
