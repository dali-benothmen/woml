# WOML Notifications

WOML supports built-in Slack and local custom notification providers for two
deliberately separate products: actionable Human Approval messages and
informational lifecycle notifications. Sharing transport code does not share
authority.

## Approval notifications

An `<approval>` may send one or more Slack messages. Every delivery receives a
separate capability token, but all tokens resolve the same durable approval.
The first valid approve or reject decision wins; later clicks cannot change it.

Approval messages may contain interactive buttons and are handled by
Notification Provider Host v1. See the Human Approval implementation plan and
the notification protocol documents for provider contracts and diagnostics.

## Informational lifecycle notifications

Lifecycle hooks may send Slack messages without creating an approval:

```xml
<lifecycle>
  <on-failure>
    <notify>
      <slack
        channels="#incidents"
        message="Workflow {{lifecycle.workflow.id}} failed: {{lifecycle.failure.code}}"
        bot-token="{{secrets.SLACK_BOT_TOKEN}}"
        app-token="{{secrets.SLACK_APP_TOKEN}}"
      />
    </notify>
  </on-failure>
</lifecycle>
```

Informational delivery uses Notification Provider Host v2. It has no approval
token, approve/reject callback, or interactive decision authority. A provider
failure records a bounded lifecycle warning and does not rewrite the workflow's
business outcome.

Multiple channels may be supplied as a space-separated `channels` attribute.
WOML resolves symbolic channel names, caches IDs, and uses the existing shared
Socket Mode/Web API transport. The app requires the same Slack permissions and
channel membership described by `woml slack doctor` diagnostics.

## Secrets and message safety

Store credentials with:

```bash
woml secrets set SLACK_BOT_TOKEN
woml secrets set SLACK_APP_TOKEN
```

Messages may interpolate bounded scalar values from `context.*` and
`lifecycle.*`. Secret interpolation in `message` is rejected. Durable events,
progress, inspection, and diagnostics contain provider/destination identities
and safe error codes—not credentials or full message bodies.

## Custom providers

A project can import a reusable `.woml` file containing
`<provider kind="notification">` and use its import name directly under
`<notify>`. The provider receives explicit props and the bounded
`notification` binding; Rust still owns durable delivery intent, safe retries,
approval capabilities, and exactly-one shared decisions. Definition-owned
`on-success`/`on-error` and `on-complete` scripts execute after the delivery
outcome is committed and can only add lifecycle warnings.

See [Reusable WOML Steps and Notification Providers](woml-reusable-definitions.md)
for the complete authoring and security contract.

## Failure and cancellation behavior

- Each delivery has a stable identity so restart does not intentionally create
  a second logical message.
- Started delivery with no terminal event after a crash is ambiguous and fails
  closed rather than being replayed automatically.
- Informational delivery failure becomes a lifecycle warning.
- Cancelling a waiting approval invalidates its decision capability.
- The frozen Provider Host v2 resolution vocabulary has no `cancelled`
  approval-message state, so WOML does not falsely update cancellation as
  rejection. A visible cancelled update requires a later protocol version.

Built-in Discord, WhatsApp, Telegram, custom webhooks, and additional adapters
remain roadmap items. A user-authored local provider can integrate those APIs
today, but it must preserve the same separation between transport and WOML's
decision authority.
