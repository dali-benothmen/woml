# WOML

**Workflow Orchestration Markup Language.** Markup for automation that scales with you.

WOML is a markup language for building and running workflow automation. A workflow written in WOML is a document you can read top to bottom — triggers, steps, control flow, approvals, and lifecycle all expressed as clear, HTML-inspired structure instead of tangled code or an unreadable diagram.

When a step needs real logic, JavaScript is always available inside `<script>`, so there is no ceiling on what a workflow can do. WOML handles everything around that code: execution order, retries, concurrency, human approvals, external services, and a durable, inspectable history of every run.

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

## The basic API

WOML has a small set of tags that compose into anything. If you know HTML, you can read a WOML workflow on sight.

### Triggers

What starts the workflow.

```xml
<manual id="start" />                            <!-- run on demand -->
<webhook id="hook" path="/orders" method="POST" secret="{{secrets.TOKEN}}" />
<schedule id="daily" cron="0 9 * * MON-FRI" timezone="UTC" />
<interval id="every-30s" seconds="30" />
<event id="created" name="order.created" secret="{{secrets.EVENT_TOKEN}}" />
<slack id="msg" events="app-mention" channels="ops" bot-token="{{secrets.SLACK_BOT}}" app-token="{{secrets.SLACK_APP}}" />
<telegram id="bot" events="message" commands="/run" bot-token="{{secrets.TELEGRAM_BOT}}" />
<discord id="bot" events="message" commands="/run" bot-token="{{secrets.DISCORD_BOT}}" />
<whatsapp id="bot" events="message" phone="+15555550100" token="{{secrets.WA_TOKEN}}" />
```

### Steps

What runs.

```xml
<step id="greet">
  <script>
    return { message: `Hello, ${context.payload.name}!` };
  </script>
</step>
```

### Control flow

```xml
<choose>
  <when test="context.steps.score.value > 80">
    <step id="escalate"><script>...</script></step>
  </when>
  <otherwise>
    <step id="log"><script>...</script></step>
  </otherwise>
</choose>

<switch id="route">
  <case test="context.payload.tier === 'gold'">...</case>
  <case test="context.payload.tier === 'silver'">...</case>
  <default>...</default>
</switch>

<if test="context.payload.priority === 'high'">
  <step id="fast-track"><script>...</script></step>
</if>

<for-each id="loop" source="{{context.steps.list.items}}">
  <step id="process"><script>...</script></step>
</for-each>

<fork>
  <step id="branch-a"><script>...</script></step>
  <step id="branch-b"><script>...</script></step>
  <join mode="all" />
</fork>
```

### Composition

```xml
<workflow-call id="send" workflow="notify-customer" input="{{context.payload}}" />
```

### Runtime bindings inside `<script>`

- `context.payload` — trigger input
- `context.steps.<id>` — earlier step output
- `context.item` — current `<for-each>` item
- `services.http`, `services.database`, `services.slack`, `services.storage`, `services.cache`, `services.event` — supervised capabilities
- `secrets.<NAME>` — only the secrets proven necessary at compile time

Scripts return JSON-compatible values. The Rust engine records every outcome durably.

---

## Real examples

### Local automation — organize a folder by file type

Run once, sorts every file into the right subfolder. Zero external services, zero API keys.

```xml
<woml>
  <workflow id="organize" name="Organize a folder by file type">
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

      <for-each id="organize" source="{{context.steps.scan.files}}">
        <choose>
          <when test="context.item.ext in ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.svg']">
            <step id="move-image">
              <script>
                const { promises: fs } = await import('fs');
                const path = await import('path');
                const from = path.join(context.steps.scan.folder, context.item.name);
                const to   = path.join(context.steps.scan.folder, 'Images', context.item.name);
                await fs.mkdir(path.dirname(to), { recursive: true });
                await fs.rename(from, to);
                return { movedTo: 'Images' };
              </script>
            </step>
          </when>
          <when test="context.item.ext in ['.pdf', '.doc', '.docx', '.txt', '.md', '.rtf']">
            <step id="move-doc"><script>...</script></step>
          </when>
          <when test="context.item.ext in ['.mp4', '.mov', '.avi', '.mkv', '.webm']">
            <step id="move-video"><script>...</script></step>
          </when>
          <when test="context.item.ext in ['.zip', '.tar', '.gz', '.7z', '.rar']">
            <step id="move-archive"><script>...</script></step>
          </when>
          <otherwise>
            <step id="move-misc"><script>...</script></step>
          </otherwise>
        </choose>
      </for-each>

      <step id="summary">
        <script>
          const total = context.steps.scan.files.length;
          return { message: `Organized ${total} file(s) into Images/, Docs/, Videos/, Archives/, Misc/.` };
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
  <workflow id="slack-router">
    <triggers>
      <slack id="incoming" events="message" channels="inbox"
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

      <switch id="route">
        <case test="context.steps.classify.intent === 'bug'">
          <step id="send-bugs">
            <script>
              await services.slack.send({
                channel: '#bugs',
                text: `🐛 ${context.payload.text}\n> confidence ${context.steps.classify.confidence}`
              });
              return { routedTo: 'bugs' };
            </script>
          </step>
        </case>
        <case test="context.steps.classify.intent === 'feature'">
          <step id="send-features">
            <script>
              await services.slack.send({
                channel: '#feature-requests',
                text: `� ${context.payload.text}`
              });
              return { routedTo: 'feature-requests' };
            </script>
          </step>
        </case>
        <case test="context.steps.classify.intent === 'question'">
          <step id="send-questions">
            <script>
              await services.slack.send({
                channel: '#questions',
                text: `❓ ${context.payload.text}`
              });
              return { routedTo: 'questions' };
            </script>
          </step>
        </case>
        <default>
          <step id="drop-noise"><script>return { dropped: true };</script></step>
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
  <workflow id="order-guard">
    <triggers>
      <webhook id="order" path="/webhooks/orders" method="POST"
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
          return { flagged: context.payload.total > 10000 || customer.body.disputes > 0 };
        </script>
      </step>

      <choose>
        <when test="context.steps.risk.flagged">
          <step id="alert">
            <script>
              await services.slack.send({
                channel: '#fraud',
                text: `High-risk order ${context.payload.orderId} ($${context.payload.total}) needs review.`
              });
              return { alerted: true };
            </script>
          </step>
        </when>
      </choose>
    </steps>
  </workflow>
</woml>
```

---

### Schedule — daily sales report at 9am weekdays

```xml
<woml>
  <workflow id="daily-report">
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

### Telegram — answer `/sales` from your team chat

```xml
<woml>
  <workflow id="telegram-sales">
    <triggers>
      <telegram id="ask" events="message" commands="/sales"
                bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
    </triggers>

    <steps>
      <step id="lookup">
        <script>
          const rows = await services.database.query({
            sql: "SELECT COUNT(*) AS today FROM orders WHERE created_at >= date('now')",
            parameters: []
          });
          return { today: rows[0].today };
        </script>
      </step>

      <step id="notify">
        <script>
          await services.slack.send({
            channel: '#sales',
            text: `Today's numbers were requested on Telegram: ${context.steps.lookup.today} orders so far.`
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
  <workflow id="order-confirmation">
    <triggers>
      <event id="created" name="order.created" secret="{{secrets.EVENT_CONTROL_TOKEN}}">
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
