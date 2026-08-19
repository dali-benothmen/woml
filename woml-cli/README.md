# WOML: Workflow Orchestration Markup Language

WOML is a declarative markup language for building and running production-grade workflow automation. A WOML workflow is a structured, HTML-inspired document that compiles into a typed, durable execution graph. Triggers, steps, control flow, lifecycle hooks, concurrency, and human approvals all live in one readable file with a Rust engine underneath.

When a step needs real logic, JavaScript runs inside `<script>`, so there is no ceiling on what a workflow can do. WOML handles everything around that code — execution order, retries, concurrency, lifecycle, human-in-the-loop, external services, and a durable, inspectable history of every run.

---

## Install

```bash
npm install -g woml-cli
```

Or with Bun:

```bash
bun add -g woml-cli
```

Or with pnpm:

```bash
pnpm add -g woml-cli
```

Verify:

```bash
woml --version
```

**Requirements:** macOS (x64, arm64), Linux (x64, glibc), or Windows (x64, arm64). No database to set up — the default state store is bundled.

---

## Document structure

A workflow is one `<woml>` document with a `<workflow>` root plus the standard containers: `<config>` for runtime policies, `<lifecycle>` for hooks, `<triggers>` for what starts a run, and `<steps>` for what runs.

```xml
<woml>
  <workflow id="..." name="..." version="1.0.0">
    <config concurrency="4" timeout="10m" />
    <lifecycle>
      <on-success><script>...</script></on-success>
      <on-error><script>...</script></on-error>
    </lifecycle>
    <triggers>...</triggers>
    <steps>...</steps>
  </workflow>
</woml>
```

Runtime bindings available inside every `<script>`:

- `context.payload` — trigger input
- `context.steps.<id>` — earlier step output
- `context.run` — durable run metadata
- `services.http`, `services.database`, `services.slack`, `services.storage`, `services.cache`, `services.event`, `services.messaging` — supervised capabilities (each must be declared by the workflow or its modules)
- `secrets.<NAME>` — only the secrets proven necessary at compile time

Scripts return JSON-compatible values. The Rust engine records every outcome durably.

---

## Triggers

Triggers decide **when** a workflow runs. Place one or more inside `<triggers>`.

### `<manual>` — run on demand

The simplest trigger. Starts a run when the operator calls `woml run` with a payload.

```xml
<manual id="start" />
```

### `<webhook>` — accept HTTP requests

Registers a static HTTP route that starts a run for every validated payload.

```xml
<webhook id="hook"
         path="/webhooks/orders"
         method="POST"
         auth="bearer"
         secret="{{secrets.ORDER_WEBHOOK_TOKEN}}">
  <schema>
    { "type": "object", "required": ["orderId"], "properties": { "orderId": { "type": "string" } } }
  </schema>
</webhook>
```

- `auth="bearer"` requires a `secret`; `auth="none"` is for deliberately public routes.
- The inline `<schema>` is JSON Schema Draft 2020-12; invalid payloads return `400 Bad Request` with the `WOML_TRIGGER_SCHEMA_INVALID` code and never start a run.

### `<schedule>` — cron expressions

Starts a run on a WOML Cron v1 schedule. Five numeric fields (`minute hour day-of-month month day-of-week`), wildcards, lists, inclusive ranges, and `/step` are supported. Seconds, names, and Quartz-only tokens are rejected.

```xml
<schedule id="daily"
          cron="0 9 * * MON-FRI"
          timezone="UTC"
          on-missed="skip" />
```

`timezone` defaults to UTC. `on-missed` chooses `skip` or `run-once` after a restart.

### `<interval>` — fixed cadence

Starts a run on a fixed interval. The compiler must not translate it into cron if semantics would change.

```xml
<interval id="heartbeat" every="30s" on-missed="skip" />
```

### `<event>` — react to internal events

Starts a run when another workflow emits a named event through the durable event bus.

```xml
<event id="created"
       name="order.created"
       secret="{{secrets.EVENT_CONTROL_TOKEN}}">
  <schema>
    { "type": "object", "required": ["orderId"], "properties": { "orderId": { "type": "string" } } }
  </schema>
</event>
```

### `<slack>` — Slack Socket Mode

Starts a run for Slack workspace events via a single Socket Mode connection per credential pair.

```xml
<slack id="msg"
       events="app-mention,direct-message"
       channels="ops,alerts"
       bot-token="{{secrets.SLACK_BOT_TOKEN}}"
       app-token="{{secrets.SLACK_APP_TOKEN}}" />
```

`events` accepts `app-mention` and `direct-message`. `channels` is optional and limits mentions to a comma-separated set.

### `<telegram>` — Telegram long polling

Starts a run for every incoming Telegram message via long polling and durable admission.

```xml
<telegram id="bot"
          events="message"
          bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
```

Telegram v1 supports the single `message` event.

### `<discord>` — Discord Gateway

Starts a run for Discord activity via a shared resumable Gateway connection.

```xml
<discord id="bot"
         events="app-mention,direct-message"
         bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />
```

`channels` is optional and accepts comma-separated numeric channel IDs (17–20 digits). Channel names are rejected because they are mutable display labels.

### `<whatsapp>` — WhatsApp Cloud API

Starts a run for inbound WhatsApp messages via signed Meta Cloud API callbacks.

```xml
<whatsapp id="bot"
          events="message"
          phone-number-id="123456789012345"
          verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
          app-secret="{{secrets.WHATSAPP_APP_SECRET}}" />
```

`phone-number-id` is Meta's durable Phone Number ID, not the display phone number.

---

## Steps

Steps run sequentially inside `<steps>`. Each `<step>` returns a value that becomes available at `context.steps.<stepId>`.

```xml
<step id="greet">
  <script>
    return { message: `Hello, ${context.payload.name}!` };
  </script>
</step>
```

---

## Control flow

Two compact routing primitives cover most branching needs.

### `<choose>` — mutually exclusive routes

`<choose id="...">` selects the first `<when>` whose `test` reference is true and publishes a merged result at `context.steps.<chooseId>`. The `test` attribute holds exactly one context reference — complex conditions belong in named steps.

```xml
<step id="needsReview">
  <script>
    return { value: context.steps.analysis.risk > 0.3 };
  </script>
</step>

<choose id="reviewRoute">
  <when test="{{context.steps.needsReview.value}}">
    <step id="humanDecision">
      <script>return { routed: 'review' };</script>
    </step>
    <result value="{{context.steps.humanDecision}}" />
  </when>
  <otherwise>
    <step id="automaticDecision">
      <script>return { routed: 'auto' };</script>
    </step>
    <result value="{{context.steps.automaticDecision}}" />
  </otherwise>
</choose>
```

### `<switch>` — exact-string routing

`<switch id="..." value="...">` compares one context reference against ordered string cases and runs exactly one route.

```xml
<switch id="route" value="{{context.steps.classify.intent}}">
  <case value="bug">
    <step id="sendBugs">
      <script>return { routedTo: 'bugs' };</script>
    </step>
    <result value="{{context.steps.sendBugs}}" />
  </case>
  <default>
    <step id="dropNoise">
      <script>return { dropped: true };</script>
    </step>
    <result value="{{context.steps.dropNoise}}" />
  </default>
</switch>
```

---

## Concurrent steps — `<parallel>`

`<parallel>` runs its direct child steps concurrently and joins after they finish. A one-step parallel is a valid degenerate fork/join.

```xml
<parallel id="fieldData" concurrency="2" on-error="wait-all">
  <step id="loadWeather">
    <script>return loadWeather(context.payload.fieldId);</script>
  </step>
  <step id="loadSoil">
    <script>return loadSoil(context.payload.fieldId);</script>
  </step>
</parallel>
```

- `concurrency` caps simultaneous child steps; defaults to the number of children.
- `on-error` is `fail-fast` (default) or `wait-all`. `fail-fast` stops scheduling new children; `wait-all` lets every child reach its terminal outcome first.
- All children see the same context view from immediately before the fork.
- A child cannot reference a sibling's output.

For multi-step concurrent routes (each branch holds its own sequence of steps), use `<fork>` and `<branch>` instead.

---

## Concurrent routes — `<fork>` and `<branch>`

`<fork>` runs multiple multi-step branches concurrently and joins on a chosen set. Each `<branch>` may contain steps, choices, switches, parallel groups, and approvals. Branches remain sequential internally while overlapping through the multiplexed Bun host.

```xml
<fork id="distribution" join="all">
  <branch id="tiktok">
    <step id="formatTikTok">
      <script>return { caption: `${context.steps.campaign.title} #automation` };</script>
    </step>
    <step id="publishTikTok">
      <script>return { platform: 'tiktok', caption: context.steps.formatTikTok.caption };</script>
    </step>
  </branch>
  <branch id="instagram">
    <step id="formatInstagram">
      <script>return { caption: `${context.steps.campaign.title}\n${context.steps.campaign.url}` };</script>
    </step>
    <step id="publishInstagram">
      <script>return { platform: 'instagram', caption: context.steps.formatInstagram.caption };</script>
    </step>
  </branch>
</fork>
```

- `join="all"` (or omitted) waits for every branch.
- `join="none"` waits for none.
- A whitespace-separated branch-ID list waits only for those branches.
- A branch can read context available before the fork and outputs created earlier in that same branch; it cannot read sibling-branch outputs.
- Nested forks inside a fork-owned branch are rejected.
- A workflow whose only terminal structure is a fork is rejected.

---

## Human approvals — `<approval>`

`<approval>` is a first-class durable control-flow item. It records that a run is waiting for a decision, optionally fires notifications, suspends the run, and selects exactly one continuation after the decision arrives.

```xml
<approval id="contentApproval"
          name="Content approval"
          description="Ask a moderator to approve or reject"
          timeout="24h"
          on-timeout="reject">
  <notify>
    <slack channels="moderators"
           bot-token="{{secrets.SLACK_BOT_TOKEN}}"
           app-token="{{secrets.SLACK_APP_TOKEN}}" />
  </notify>

  <step id="hold" />
  <when-approved>
    <step id="publish">
      <script>return { published: true };</script>
    </step>
    <result value="{{context.steps.publish}}" />
  </when-approved>
  <when-rejected>
    <step id="archive">
      <script>return { archived: true };</script>
    </step>
    <result value="{{context.steps.archive}}" />
  </when-rejected>
</approval>
```

- `timeout` caps how long the approval waits; `on-timeout` chooses `approve`, `reject`, or another arm.
- Optional `<notify>` fires built-in Slack/Telegram/Discord/WhatsApp notifications when the approval is armed.
- Exactly one `<when-approved>` or `<when-rejected>` is selected after the decision arrives.

---

## Notifications — `<notify>`

`<notify>` is a container for built-in Slack, Telegram, Discord, or WhatsApp deliveries. It is not a standalone step — it is attached to the parent that arms the notification.

### Inside lifecycle hooks

Fire a notification when the run finishes successfully or fails.

```xml
<lifecycle>
  <on-success>
    <notify>
      <slack channels="ops"
             bot-token="{{secrets.SLACK_BOT_TOKEN}}"
             app-token="{{secrets.SLACK_APP_TOKEN}}" />
    </notify>
    <script>
      console.log('Run completed successfully');
    </script>
  </on-success>
  <on-error>
    <notify>
      <slack channels="oncall"
             bot-token="{{secrets.SLACK_BOT_TOKEN}}"
             app-token="{{secrets.SLACK_APP_TOKEN}}" />
    </notify>
    <script>
      console.error('Run failed');
    </script>
  </on-error>
</lifecycle>
```

### Inside an approval

Fire a notification when the approval is armed so the right moderator sees the decision request (see the `<approval>` example above).

A `<notify>` contains one or more built-in provider tags — `<slack>`, `<telegram>`, `<discord>`, or `<whatsapp>` — and must not contain anything else.

---

## Real examples

### Local automation — organize a folder by file type

Run once, sorts every file into the right subfolder. Zero external services, zero API keys.

```xml
<woml>
  <workflow id="organize" name="Organize a folder by file type" version="1.0.0">
    <triggers><manual id="start" /></triggers>

    <steps>
      <step id="scan">
        <script>
          const { promises: fs } = await import('fs');
          const path = await import('path');
          const folder = context.payload.path ?? '.';
          const entries = await fs.readdir(folder, { withFileTypes: true });
          return {
            folder,
            files: entries
              .filter(e => e.isFile())
              .map(e => ({ name: e.name, ext: path.extname(e.name).toLowerCase() })),
          };
        </script>
      </step>

      <parallel id="moveAll" concurrency="4">
        <step id="moveImages">
          <script>
            const { promises: fs } = await import('fs');
            const path = await import('path');
            const { folder, files } = context.steps.scan;
            for (const f of files.filter(f => ['.jpg','.jpeg','.png','.gif','.webp','.svg'].includes(f.ext))) {
              const from = path.join(folder, f.name);
              const to = path.join(folder, 'Images', f.name);
              await fs.mkdir(path.dirname(to), { recursive: true });
              await fs.rename(from, to);
            }
            return { ok: true };
          </script>
        </step>
        <step id="moveDocs">
          <script>
            const { promises: fs } = await import('fs');
            const path = await import('path');
            const { folder, files } = context.steps.scan;
            for (const f of files.filter(f => ['.pdf','.doc','.docx','.txt','.md','.rtf'].includes(f.ext))) {
              const from = path.join(folder, f.name);
              const to = path.join(folder, 'Docs', f.name);
              await fs.mkdir(path.dirname(to), { recursive: true });
              await fs.rename(from, to);
            }
            return { ok: true };
          </script>
        </step>
        <step id="moveVideos">
          <script>
            const { promises: fs } = await import('fs');
            const path = await import('path');
            const { folder, files } = context.steps.scan;
            for (const f of files.filter(f => ['.mp4','.mov','.avi','.mkv','.webm'].includes(f.ext))) {
              const from = path.join(folder, f.name);
              const to = path.join(folder, 'Videos', f.name);
              await fs.mkdir(path.dirname(to), { recursive: true });
              await fs.rename(from, to);
            }
            return { ok: true };
          </script>
        </step>
        <step id="moveArchives">
          <script>
            const { promises: fs } = await import('fs');
            const path = await import('path');
            const { folder, files } = context.steps.scan;
            for (const f of files.filter(f => ['.zip','.tar','.gz','.7z','.rar'].includes(f.ext))) {
              const from = path.join(folder, f.name);
              const to = path.join(folder, 'Archives', f.name);
              await fs.mkdir(path.dirname(to), { recursive: true });
              await fs.rename(from, to);
            }
            return { ok: true };
          </script>
        </step>
      </parallel>

      <step id="summary">
        <script>
          return {
            message: `Organized ${context.steps.scan.files.length} file(s) into Images/, Docs/, Videos/, Archives/.`
          };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

```bash
woml run organize.woml --payload '{"path":"/path/to/Downloads"}'
```

---

### AI-powered — classify Slack messages and route them to the right channel

Send every incoming Slack message to an LLM, classify intent, and forward to a dedicated channel.

```xml
<woml>
  <workflow id="slack-router" version="1.0.0">
    <triggers>
      <slack id="incoming"
             events="app-mention,direct-message"
             channels="inbox"
             bot-token="{{secrets.SLACK_BOT_TOKEN}}"
             app-token="{{secrets.SLACK_APP_TOKEN}}" />
    </triggers>

    <steps>
      <step id="classify">
        <script>
          const response = await services.http.request({
            method: 'POST',
            url: 'https://api.openai.com/v1/chat/completions',
            headers: { authorization: `Bearer ${secrets.OPENAI_API_KEY}` },
            body: {
              model: 'gpt-4o-mini',
              messages: [
                {
                  role: 'system',
                  content: 'Classify the message into one of: bug, feature, question, noise. Reply with JSON { "intent": "...", "confidence": 0..1 }.'
                },
                { role: 'user', content: context.payload.text }
              ],
              response_format: { type: 'json_object' }
            },
            timeoutMs: 10000
          });
          return JSON.parse(response.body.choices[0].message.content);
        </script>
      </step>

      <switch id="route" value="{{context.steps.classify.intent}}">
        <case value="bug">
          <step id="sendBugs">
            <script>
              await services.slack.send({
                channel: '#bugs',
                text: `🐛 ${context.payload.text}\n> confidence ${context.steps.classify.confidence}`
              });
              return { routedTo: 'bugs' };
            </script>
          </step>
          <result value="{{context.steps.sendBugs}}" />
        </case>
        <case value="feature">
          <step id="sendFeatures">
            <script>
              await services.slack.send({
                channel: '#feature-requests',
                text: `💡 ${context.payload.text}`
              });
              return { routedTo: 'feature-requests' };
            </script>
          </step>
          <result value="{{context.steps.sendFeatures}}" />
        </case>
        <case value="question">
          <step id="sendQuestions">
            <script>
              await services.slack.send({
                channel: '#questions',
                text: `❓ ${context.payload.text}`
              });
              return { routedTo: 'questions' };
            </script>
          </step>
          <result value="{{context.steps.sendQuestions}}" />
        </case>
        <default>
          <step id="dropNoise">
            <script>return { dropped: true };</script>
          </step>
          <result value="{{context.steps.dropNoise}}" />
        </default>
      </switch>
    </steps>
  </workflow>
</woml>
```

---

### Webhook — flag risky orders, alert Slack

```xml
<woml>
  <workflow id="order-guard" version="1.0.0">
    <triggers>
      <webhook id="order"
               path="/webhooks/orders"
               method="POST"
               auth="bearer"
               secret="{{secrets.ORDER_WEBHOOK_TOKEN}}">
        <schema>
          {
            "type": "object",
            "required": ["orderId", "total", "customerId"],
            "properties": {
              "orderId":    { "type": "string" },
              "total":      { "type": "number" },
              "customerId": { "type": "string" }
            }
          }
        </schema>
      </webhook>
    </triggers>

    <steps>
      <step id="risk">
        <script>
          const customer = await services.http.request({
            method: 'GET',
            url: `https://internal.api/customers/${context.payload.customerId}`,
            timeoutMs: 5000
          });
          return {
            flagged: context.payload.total > 10000 || customer.body.disputes > 0
          };
        </script>
      </step>

      <step id="isFlagged">
        <script>return { value: context.steps.risk.flagged };</script>
      </step>

      <choose id="alertRoute">
        <when test="{{context.steps.isFlagged.value}}">
          <step id="alert">
            <script>
              await services.slack.send({
                channel: '#fraud',
                text: `High-risk order ${context.payload.orderId} ($${context.payload.total}) needs review.`
              });
              return { alerted: true };
            </script>
          </step>
          <result value="{{context.steps.alert}}" />
        </when>
        <otherwise>
          <step id="logOk">
            <script>return { logged: true };</script>
          </step>
          <result value="{{context.steps.logOk}}" />
        </otherwise>
      </choose>
    </steps>
  </workflow>
</woml>
```

---

### Schedule — daily sales report at 9am weekdays

```xml
<woml>
  <workflow id="daily-report" version="1.0.0">
    <triggers>
      <schedule id="weekdays" cron="0 9 * * MON-FRI" timezone="UTC" />
    </triggers>

    <steps>
      <step id="totals">
        <script>
          const rows = await services.database.query({
            sql: "SELECT COUNT(*) AS orders, COALESCE(SUM(total), 0) AS revenue FROM orders WHERE created_at >= date('now', '-1 day')",
            parameters: []
          });
          return rows[0];
        </script>
      </step>

      <step id="publish">
        <script>
          const { orders, revenue } = context.steps.totals;
          await services.slack.send({
            channel: '#sales',
            text: `Daily report — ${orders} orders, $${revenue.toFixed(2)} revenue.`
          });
          return { sent: true };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

---

### Telegram — answer mentions in your team chat

```xml
<woml>
  <workflow id="telegram-echo" version="1.0.0">
    <triggers>
      <telegram id="incoming"
                events="message"
                bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
    </triggers>

    <steps>
      <step id="reply">
        <script>
          await services.messaging.send({
            channel: 'telegram',
            conversationId: context.payload.conversationId,
            text: `You said: ${context.payload.text}`
          });
          return { ok: true };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

---

### Event — send a confirmation email when another workflow emits `order.created`

```xml
<woml>
  <workflow id="order-confirmation" version="1.0.0">
    <triggers>
      <event id="created"
             name="order.created"
             secret="{{secrets.EVENT_CONTROL_TOKEN}}">
        <schema>
          {
            "type": "object",
            "required": ["orderId", "email"],
            "properties": {
              "orderId": { "type": "string" },
              "email":   { "type": "string" }
            }
          }
        </schema>
      </event>
    </triggers>

    <steps>
      <step id="confirm">
        <script>
          await services.http.request({
            method: 'POST',
            url: 'https://api.emailprovider.com/v1/send',
            headers: { authorization: `Bearer ${secrets.EMAIL_API_KEY}` },
            body: {
              to: context.payload.email,
              template: 'order-confirmation',
              data: { orderId: context.payload.orderId }
            },
            timeoutMs: 5000
          });
          return { sentTo: context.payload.email };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

---

## Common commands

```bash
woml check workflows/                    # Validate workflows without running them
woml run workflows/                      # Run in the foreground (Ctrl+C to stop)
woml run workflows/ --background         # Run in the background, survives Ctrl+C
woml inspect                             # Show the current state of all runs
woml list                                # List known workflows and recent runs
woml get run_...                         # Print the full event history of a run
woml cancel run_...                      # Cancel a running or pending run
woml backup backups/latest               # Snapshot the durable state store to a file
woml prune --before 30d --dry-run        # Preview which old runs would be purged
```

---

## License

Apache-2.0.
