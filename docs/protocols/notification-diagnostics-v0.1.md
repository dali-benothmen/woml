# WOML Notification and Secret Diagnostics v0.1

Status: frozen and implemented through the N2 frontend. N3–N6 must reuse or
explicitly version the runtime codes.

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

## Runtime and Provider Codes Reserved for N3–N6

The provider returns a structured safe failure kind plus a stable code. Initial
codes include `WOML_SLACK_AUTH_FAILED`, `WOML_SLACK_DESTINATION_INVALID`,
`WOML_SLACK_RATE_LIMITED`, `WOML_SLACK_UNAVAILABLE`,
`WOML_NOTIFICATION_DELIVERY_AMBIGUOUS`, `WOML_NOTIFICATION_HOST_CRASHED`,
`WOML_NOTIFICATION_SIZE_LIMIT_EXCEEDED`, and
`WOML_NOTIFICATION_DELIVERY_FAILED`.
