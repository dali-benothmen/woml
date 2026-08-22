# WOML Services Reference

Use this reference before generating a `services.*` call. Built-in services execute through WOML's supervised runtime. The optional final `{ name: "..." }` identifies one logical operation and is important when a step makes multiple effectful calls.

## Choosing a service

| Need | Use |
| --- | --- |
| Familiar Web API or streaming | Native `fetch()` |
| Managed HTTP parsing, status, timeout, and limits | `services.http.request()` |
| SQLite or PostgreSQL records/transactions | `services.db()` |
| Files and larger durable values | `services.storage` |
| Disposable expiring optimization data | `services.cache` |
| Small permanent workflow-owned memory | `services.state` |
| Fan out a named fact to subscribers | `services.events.emit()` |
| Wait for one child workflow's result | `services.workflows.call()` |
| Start independent child work | `services.workflows.start()` |
| Send Telegram/Discord/WhatsApp messages | Provider messaging services |

`services.queue` and document/NoSQL database adapters are not released.

## Managed HTTP

```js
const response = await services.http.request({
  url: "https://api.example.com/customers",
  method: "POST",
  headers: { authorization: `Bearer ${secrets.API_TOKEN}` },
  query: { region: "eu" },
  json: { customerId: context.payload.customerId },
  responseType: "json",
  timeout: "10s",
  acceptedStatus: { minimum: 200, maximum: 299 },
  redirect: "follow",
  maximumRedirects: 5,
  idempotency: {
    header: "Idempotency-Key",
    value: attempt.idempotencyKey
  }
}, { name: "create-customer" });

return response.data;
```

Request fields:

- required `url`;
- optional `method`, `headers`, and scalar `query`;
- exactly one optional body: `json`, `text`, or `bytesBase64`;
- `responseType`: `json` (default), `text`, `bytes`, or `storage`;
- `timeout` duration or `timeoutMs`;
- `acceptedStatus: { minimum, maximum }`;
- `redirect`: `follow`, `error`, or `manual` plus `maximumRedirects`;
- optional external `idempotency` configuration; and
- `storage` target when `responseType: "storage"`.

Returns `{ status, ok, headers, data, url, redirected }`. Managed failures throw `WomlServiceError`. Native `fetch()` remains preferable when code needs normal `Request`/`Response`/stream APIs.

## Database

```js
const db = services.db({
  driver: "sqlite",
  connection: "./.woml/customers.sqlite"
});
```

Or use PostgreSQL with `driver: "postgres"` and a secret connection string. Do not point SQLite at WOML's own state database.

### `query` and `execute`

```js
const selected = await db.query({
  text: "SELECT id, name FROM customers WHERE active = ?",
  values: [true]
});

const changed = await db.execute({
  text: "UPDATE customers SET active = ? WHERE id = ?",
  values: [false, 42]
}, { name: "deactivate-customer" });
```

SQLite parameters use `?`; PostgreSQL parameters use `$1`, `$2`, and so on. Reads return `{ rows, rowCount }`; mutations return `{ rowsAffected, lastInsertId }`.

### CRUD helpers

```js
await db.insert({ table: "customers", values: { name: "Ada", active: true } }, { name: "create-customer" });
await db.read({
  table: "customers",
  columns: ["id", "name"],
  where: { active: true },
  orderBy: [{ column: "id", direction: "asc" }],
  limit: 100
});
await db.update({ table: "customers", values: { active: false }, where: { id: 42 } }, { name: "deactivate-customer" });
await db.delete({ table: "customers", where: { id: 42 } }, { name: "delete-customer" });
```

`update` and `delete` require a non-empty `where`. Identifiers are strictly validated and values are parameterized.

### Transaction batch

```js
return db.transaction({
  operations: [
    { operation: "insert", table: "orders", values: context.payload.order },
    { operation: "execute", text: "UPDATE inventory SET quantity = quantity - ? WHERE sku = ?", values: [1, context.payload.sku] },
    { operation: "query", text: "SELECT quantity FROM inventory WHERE sku = ?", values: [context.payload.sku] }
  ]
}, { name: "create-order" });
```

The batch contains 1–100 operations and commits atomically. Never interpolate untrusted input into SQL.

## Object storage

```js
const object = await services.storage.put({
  key: "reports/daily.json",
  value: { customers: 42 }
}, { name: "store-daily-report" });

const saved = await services.storage.get({
  key: object.key,
  responseType: "json",
  ifVersion: object.version
});
```

Operations:

- `put({ key, value | text | bytesBase64, contentType?, overwrite?, ifVersion? }, options?)`;
- `get({ key, responseType?: "json" | "text" | "bytes", ifVersion? }, options?)` → `{ object, data }`;
- `head({ key }, options?)` → object reference or `null`;
- `list({ prefix?, limit?, cursor? }?, options?)` → ordered references and cursor;
- `delete({ key, ifVersion? }, options?)` → `{ deleted, object }`.

An ordinary put is create-only. Use `overwrite: true` for unconditional replacement or `ifVersion` for conditional replacement, never both. Keys are logical names, not filesystem paths.

For large downloads, set HTTP `responseType: "storage"` with `storage: { key, overwrite?, ifVersion?, contentType? }`; returned `data` is the object reference.

## Cache

```js
const cached = await services.cache.get("customer:42");
if (cached.hit) return cached.value;

await services.cache.set("customer:42", customer, {
  ttl: "15m",
  name: "cache-customer"
});
```

Operations:

- `get(key)` → `{ hit: true, value, expiresAt }` or `{ hit: false }`;
- `set(key, value, { ttl?, name? }?)` → `{ stored: true, expiresAt }`;
- `delete(key, { name? }?)` → `{ deleted }`;
- `has(key)` → `{ present }`;
- `increment(key, amount? = 1, { ttl?, name? }?)` → atomic counter result;
- `setIfAbsent(key, value, { ttl?, name? }?)` → winning or existing value.

TTL defaults to `5m` and accepts milliseconds or `ms`, `s`, `m`, `h`, `d`. Cache may expire or evict; never use it for correctness, financial counters, required checkpoints, or secrets.

## Durable state

```js
const previous = await services.state.get("visits");
const saved = await services.state.increment("visits", 1, {
  name: "count-visit",
  ifVersion: previous.found ? previous.version : 0
});
```

Operations:

- `get(key)` → `{ found: true, value, version, updatedAt }` or `{ found: false }`;
- `has(key)` → `{ present, version? }`;
- `set(key, value, { name, ifVersion? })` → `{ stored: true, version, updatedAt }`;
- `delete(key, { name, ifVersion? })` → `{ deleted }`;
- `increment(key, amount, { name, ifVersion? })` → `{ value, version, updatedAt }`;
- `setIfAbsent(key, value, { name })` → `{ stored, value, version, updatedAt }`.

Every mutation requires a stable `name`. Use `ifVersion` for compare-and-set when multiple runs may update the same key. State is small, permanent workflow-owned JSON; it is not encrypted secret storage, cache, a general database, or run context.

## Internal events

```js
const publication = await services.events.emit(
  "customer.updated",
  { customerId: context.payload.customerId },
  { name: "publish-customer-update" }
);
```

The payload must be a top-level JSON object. Every loaded subscriber with `<event name="customer.updated">` is considered; valid subscribers receive independent durable runs. No subscriber is a successful no-op. The result includes publication identity, status, counts, depth, and safe delivery outcomes.

## Workflow calls

Wait for one child result:

```js
const risk = await services.workflows.call(
  "calculate-risk",
  { customerId: context.payload.customerId },
  { name: "calculate-customer-risk", timeout: "30s" }
);
return { score: risk.score };
```

Start and continue after durable admission:

```js
const started = await services.workflows.start(
  "send-follow-up",
  { customerId: context.payload.customerId },
  { name: "start-follow-up" }
);
return started; // { workflowId, runId, duplicate }
```

Targets are exact loaded workflow IDs. Load parent and child in the same `woml run` command or run them against the same state database. `.call()` requires a final JSON result (including `null`; `undefined` is an error). Synchronous calls reject Human Approval targets; use `.start()` when appropriate. Child runs are independently durable and are not automatically cancelled with the parent.

## Communication messaging

```js
await services.telegram.send({
  botToken: secrets.TELEGRAM_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: "Hello",
  replyToMessageId: context.payload.messageId
}, { name: "telegram-reply" });

await services.discord.send({
  botToken: secrets.DISCORD_BOT_TOKEN,
  conversationId: context.payload.conversationId,
  text: "Hello",
  replyToMessageId: context.payload.messageId
}, { name: "discord-reply" });

await services.whatsapp.send({
  accessToken: secrets.WHATSAPP_ACCESS_TOKEN,
  phoneNumberId: "123456789012345",
  conversationId: context.payload.conversationId,
  template: {
    name: "approved_template",
    language: "en_US",
    parameters: ["Hello"]
  }
}, { name: "whatsapp-reply" });
```

Telegram and Discord accept `botToken`, `conversationId`, `text`, and optional `replyToMessageId`. WhatsApp accepts approved templates only. Successful sends return `{ provider, conversationId, messageId, acceptedAt, threadId? }`.

Slack has released trigger and notification tags but no `services.slack.send()` built-in. Do not invent one; use an imported module/custom integration when ordinary Slack messaging is required.

## Managed failures and reliability

Managed failures are catchable `WomlServiceError` values with stable `code`, `service`, `operation`, `retryable`, and `ambiguous` fields. Treat `ambiguous: true` as “the external effect may have happened.” Do not blindly retry it.

Credentials, bodies, SQL, object contents, cache/state values, and secret values are excluded from durable operation metadata unless the author deliberately returns them as context—which should not be done for secrets.
