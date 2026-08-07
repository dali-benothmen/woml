# WOML Slack Approval Setup

This is the one-time workspace setup for WOML's Slack approval provider. WOML
uses Socket Mode, so it does not need a public callback URL or `woml serve`.

## Install the app

1. In Slack's app management page, choose **Create New App → From an app
   manifest** and paste `manifest.json` from this directory.
2. Install the app to the workspace. The manifest requests only the scopes WOML
   currently uses: message send/update plus public/private channel lookup.
3. Under **Basic Information → App-Level Tokens**, generate an app token with
   the `connections:write` scope.
4. Copy the **Bot User OAuth Token** (`xoxb-...`) and the **App-Level Token**
   (`xapp-...`) into WOML's protected secret store:

   ```text
   woml secrets set SLACK_BOT_TOKEN
   woml secrets set SLACK_APP_TOKEN
   ```

   Enter each value only at the hidden prompt. Do not place either token in a
   `.woml` file, shell argument, environment committed to source control, or
   chat message.

After changing any Bot Token Scope, choose **Reinstall to Workspace** and copy
the Bot User OAuth Token from that same Slack app again. The installed token,
not merely the permissions displayed in the app editor, is what Slack checks.
Its Bot Token Scopes must include:

```text
chat:write
chat:write.public
channels:read
groups:read
```

`channels:history` does not replace `channels:read`. WOML uses
`conversations.list` to resolve a readable `#channel` alias. Supplying a stable
Slack conversation ID (`C...` or `G...`) bypasses that name lookup, but does not
replace the permission required to post or the requirement to invite the app
to a private channel.

## Choose channels

Public channel aliases such as `#woml-testing` work with the manifest as-is.
For a private channel, invite the WOML app to that channel first. Slack channel
IDs (`C...` or `G...`) are the most stable option and avoid name lookup.

```xml
<notify>
  <slack
    channels="#woml-testing"
    bot-token="{{secrets.SLACK_BOT_TOKEN}}"
    app-token="{{secrets.SLACK_APP_TOKEN}}"
  />
</notify>
```

Then run the workflow normally:

```text
woml run workflow.woml
```

The repository includes `examples/slackApprovalWorkflow.woml` as a live smoke
test. Change its `channels` value to a channel in your workspace before running
it.

The terminal remains attached while Socket Mode waits for a reviewer. Clicking
Approve or Reject in any delivered channel resolves the one shared approval,
updates all delivered messages, and continues only the selected WOML route.

## Common failures

- `WOML_SLACK_AUTH_FAILED`: replace a revoked, expired, or wrong token.
- `WOML_SLACK_PERMISSION_DENIED`: verify the scopes above under **Bot Token
  Scopes**, reinstall the app, refresh the stored `xoxb-...` token, and ensure
  the app belongs to a private destination.
- `WOML_SLACK_DESTINATION_INVALID`: verify the channel name/ID and app
  visibility.
- `WOML_SLACK_RATE_LIMITED`: WOML follows Slack's `Retry-After` value through
  its durable retry policy.
- `WOML_NOTIFICATION_DELIVERY_AMBIGUOUS`: the connection ended after a send
  may have reached Slack. WOML fails closed instead of creating a duplicate.
