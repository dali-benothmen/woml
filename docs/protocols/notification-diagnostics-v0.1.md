# WOML Notification and Secret Diagnostics v0.1

Status: frozen and implemented through the completed N6 Slack publication
milestone. Later providers must reuse or explicitly version these codes.

Diagnostics never include resolved secret values, decision capabilities,
provider payloads, or credential-store implementation errors.

## Secret Reference and Store

| Code | Surface | Meaning |
| --- | --- | --- |
| `WOML_SECRET_REFERENCE_INVALID` | frontend | A value resembles a secret reference but is not exact `{{secrets.NAME}}`. |
| `WOML_SECRET_LITERAL_FORBIDDEN` | frontend | A secret-bearing attribute contains a literal or another reference kind. |
| `WOML_SECRET_NAME_INVALID` | CLI/frontend | A name violates `[A-Z][A-Z0-9_]*`. |
| `WOML_SECRET_VALUE_EMPTY` | CLI | Hidden input contained no value. |
| `WOML_SECRET_VALUE_TOO_LARGE` | CLI | The value exceeds 2048 UTF-8 bytes. |
| `WOML_SECRET_NOT_FOUND` | CLI/runtime | A required symbolic name is absent from the selected provider. |
| `WOML_SECRET_STORE_UNAVAILABLE` | CLI/runtime | The OS credential service is unsupported, unavailable, or locked. |
| `WOML_SECRET_INDEX_INVALID` | CLI | The protected metadata index cannot be validated. |
| `WOML_SECRET_PROVIDER_INVALID` | CLI | `WOML_SECRETS_PROVIDER` is not `os` or `env`. |
| `WOML_SECRET_PROVIDER_READ_ONLY` | CLI | A mutation was requested from the CI environment provider. |
| `WOML_SECRET_PROMPT_REQUIRES_TTY` | CLI | Hidden input was requested without an interactive terminal. |
| `WOML_SECRET_PROMPT_CANCELLED` | CLI | The user cancelled hidden input. |

## Notification Source Validation Implemented in N2

| Code | Meaning |
| --- | --- |
| `WOML_NOTIFY_INVALID_ORDER` | `<notify>` is not immediately before the two approval arms. |
| `WOML_NOTIFY_EMPTY` | `<notify>` has no provider child. |
| `WOML_NOTIFY_UNSUPPORTED_PROVIDER` | The first profile received anything other than `<slack>`. |
| `WOML_SLACK_ATTRIBUTE_REQUIRED` | A required Slack attribute is missing. |
| `WOML_SLACK_UNKNOWN_ATTRIBUTE` | Slack markup contains an unreviewed attribute. |
| `WOML_SLACK_CHANNELS_EMPTY` | `channels` has no destination token. |
| `WOML_SLACK_CHANNEL_INVALID` | A token is neither a canonical alias nor conversation ID. |
| `WOML_SLACK_CHANNEL_DUPLICATE` | One Slack credential set repeats a normalized destination. |
| `WOML_SECRET_SINK_UNSUPPORTED` | A secret reference appears outside a reviewed secret-bearing attribute. |

## Durable Runtime Codes Implemented in N3

| Code | Meaning |
| --- | --- |
| `WOML_NOTIFICATION_RUNTIME_UNAVAILABLE` | Reserved for installations that do not contain the Model v5 provider runtime. |
| `WOML_NOTIFICATION_DELIVERY_AMBIGUOUS` | Recovery found a send whose external effect is uncertain and refused to replay it. |
| `WOML_NOTIFICATION_DELIVERY_FAILED` | Every configured notification delivery reached a final failure. |
| `WOML_NOTIFICATION_UPDATE_INTERRUPTED` | Recovery converted an interrupted message update into durable retryable work. |

## Provider Codes Implemented in N4

| Code | Meaning |
| --- | --- |
| `WOML_NOTIFICATION_INTERACTION_TIMEOUT` | No provider action arrived before the local provider-wait deadline; the durable approval remains waiting. |
| `WOML_NOTIFICATION_RESPONSE_INVALID` | The provider host returned a malformed or semantically incompatible result. |
| `WOML_NOTIFICATION_HOST_CRASHED` | The provider host stopped during uncertain delivery and Rust failed the attempt closed. |
| `WOML_NOTIFICATION_SIZE_LIMIT_EXCEEDED` | A provider protocol frame exceeded the frozen byte limit. |
| `WOML_SLACK_RESPONSE_INVALID` | The Slack adapter returned an invalid provider message identity. |
| `WOML_SLACK_UNAVAILABLE` | The adapter could not safely reach its Slack transport. |

## Additional Provider Codes Implemented in N5

| Code | Meaning |
| --- | --- |
| `WOML_SLACK_AUTH_FAILED` | Slack rejected or expired the bot/app credential. |
| `WOML_SLACK_PERMISSION_DENIED` | The installed app lacks a required scope or channel permission. |
| `WOML_SLACK_DESTINATION_INVALID` | The channel is absent, archived, or invisible to the app. |
| `WOML_SLACK_RATE_LIMITED` | Slack returned HTTP 429; the durable retry uses `Retry-After`. |
| `WOML_SLACK_UPDATE_FAILED` | Slack could not safely update a delivered approval message. |
| `WOML_NOTIFICATION_DELIVERY_AMBIGUOUS` | A send may have reached Slack but no definitive response arrived, so WOML refuses blind replay. |

## N6 Hardening Result

N6 required no new failure code or event shape. Provider failures retain one
structured safe kind and stable code, and the release gate verifies that
diagnostics contain neither resolved secret values nor decision capabilities.
Human-readable CLI presentation may improve without changing this frozen
failure vocabulary.
