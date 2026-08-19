# WOML: Workflow Orchestration Markup Language

If you can read HTML, you can use WOML to automate anything, literally anything.

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

A workflow is one `<woml>` document with a `<workflow>` root, plus `<config>`, `<lifecycle>`, `<triggers>`, and `<steps>` containers. Triggers decide *when* a workflow runs; steps decide *what* it does. Everything inside a `<script>` is plain JavaScript.

### Document skeleton

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

### Triggers

One or more of these inside `<triggers>`.

```xml
<manual id="start" />

<webhook id="hook"
         path="/webhooks/orders"
         method="POST"
         auth="bearer"
         secret="{{secrets.ORDER_WEBHOOK_TOKEN}}">
  <schema>
    { "type": "object", "required": ["orderId"], "properties": { "orderId": { "type": "string" } } }
  </schema>
</webhook>

<schedule id="daily" cron="0 9 * * MON-FRI" timezone="UTC" />

<interval id="heartbeat" every="30s" on-missed="skip" />

<event id="created" name="order.created" secret="{{secrets.EVENT_CONTROL_TOKEN}}" />

<slack id="msg"
       events="app-mention,direct-message"
       channels="ops,alerts"
       bot-token="{{secrets.SLACK_BOT_TOKEN}}"
       app-token="{{secrets.SLACK_APP_TOKEN}}" />

<telegram id="bot" events="message" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />

<discord id="bot"
         events="app-mention,direct-message"
         bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />

<whatsapp id="bot"
          events="message"
          phone-number-id="123456789012345"
          verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}"
          app-secret="{{secrets.WHATSAPP_APP_SECRET}}" />
```

### Steps

Steps are sequential inside `<steps>`. Each `<step>` returns a value that becomes available at `context.steps.<stepId>`.

```xml
<step id="greet">
  <script>
    return { message: `Hello, ${context.payload.name}!` };
  </script>
</step>
```

### Control flow

- **`<choose>`** — mutually exclusive conditional routing. The `id` is required. Each `<when>` and the `<otherwise>` must end with a `<result>` that selects which value to publish at `context.steps.<chooseId>`. The `test` attribute holds exactly one context reference.
- **`<switch>`** — exact-string routing against one `value` reference. Each `<case>` and the `<default>` must end with a `<result>`.
- **`<parallel>`** — run direct child steps concurrently, then join. Use `concurrency` to cap simultaneous children and `on-error="fail-fast"` (default) or `wait-all` for the failure policy.
- **`<fork>`** + **`<branch>`** — concurrent multi-step routes with an explicit `join` mode (`all`, `none`, or a whitespace-separated list of branch IDs).
- **`<approval>`** — durable human-in-the-loop. Records an approval decision and pauses the run until one of the configured arms resolves.
- **`<notify>`** — attach a built-in Slack/Telegram/Discord/WhatsApp notification to the step, lifecycle hook, or approval that contains it.

### Runtime bindings inside `<script>`

- `context.payload` — trigger input
- `context.steps.<id>` — earlier step output
- `context.run` — durable run metadata
- `services.http`, `services.database`, `services.slack`, `services.storage`, `services.cache`, `services.event`, `services.messaging` — supervised capabilities (each must be declared by the workflow or its modules)
- `secrets.<NAME>` — only the secrets proven necessary at compile time

Scripts return JSON-compatible values. The Rust engine records every outcome durably.

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

      <step id="imageCount">
        <script>
          return {
            value: context.steps.scan.files.filter(f =>
              ['.jpg','.jpeg','.png','.gif','.webp','.svg'].includes(f.ext)
            ).length
          };
        </script>
      </step>

      <step id="docCount">
        <script>
          return {
            value: context.steps.scan.files.filter(f =>
              ['.pdf','.doc','.docx','.txt','.md','.rtf'].includes(f.ext)
            ).length
          };
        </script>
      </step>

      <step id="videoCount">
        <script>
          return {
            value: context.steps.scan.files.filter(f =>
              ['.mp4','.mov','.avi','.mkv','.webm'].includes(f.ext)
            ).length
          };
        </script>
      </step>

      <step id="archiveCount">
        <script>
          return {
            value: context.steps.scan.files.filter(f =>
              ['.zip','.tar','.gz','.7z','.rar'].includes(f.ext)
            ).length
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
          const { imageCount, docCount, videoCount, archiveCount } = context.steps;
          return {
            message: `Organized ${context.steps.scan.files.length} file(s): ${imageCount.value} Images, ${docCount.value} Docs, ${videoCount.value} Videos, ${archiveCount.value} Archives.`
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
        <script>
          return { value: context.steps.risk.flagged };
        </script>
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
