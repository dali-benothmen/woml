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
- `WOML_SLACK_PERMISSION_DENIED`: reinstall the app after applying the
  manifest, or invite it to the private channel.
- `WOML_SLACK_DESTINATION_INVALID`: verify the channel name/ID and app
  visibility.
- `WOML_SLACK_RATE_LIMITED`: WOML follows Slack's `Retry-After` value through
  its durable retry policy.
- `WOML_NOTIFICATION_DELIVERY_AMBIGUOUS`: the connection ended after a send
  may have reached Slack. WOML fails closed instead of creating a duplicate.
