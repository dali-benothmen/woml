# WOML Internal Events

SC11 lets one running workflow start every workflow subscribed to a named
event. The publisher uses normal JavaScript:

```js
const publication = await services.events.emit('customer.updated', {
  customerId: 'customer-42'
});
```

The subscriber remains declarative WOML:

```xml
<event id="customerUpdated" name="customer.updated">
  <schema>
    { "type": "object", "required": ["customerId"] }
  </schema>
</event>
```

No HTTP request or control token sits between these workflows. Rust admits the
subscriber runs directly through the same durable authority used by the public
event endpoint.

## Result

`emit()` resolves to a JSON object containing:

- `publicationId`: stable identity derived from this step operation;
- `eventName` and `status` (`accepted`, `partial`, or `rejected`);
- accepted, duplicate, and rejected counts;
- `depth`, from zero for no subscribers through the maximum of 32; and
- one safe delivery result per subscriber, including accepted `runId` values.

No matching subscriber is a successful no-op. A subscriber whose schema
rejects the payload appears as a rejected delivery; valid subscribers are
still admitted.

## Idempotency and multiple emissions

One automatic `emit()` call per step receives a stable operation identity. If
a step contains multiple logical emissions, name each one:

```js
await services.events.emit('customer.updated', customer, {
  name: 'publish-customer-update'
});
```

A safe retry with the same name and payload returns the original subscriber
runs as duplicates. Reusing that identity with different event data is
rejected. This prevents WOML-owned retries from manufacturing duplicate runs;
it does not claim that arbitrary JavaScript outside this managed call is
exactly once.

## Internal-only and public events

Omit `secret` for an internal-only event. It stays available to
`services.events.emit()` but opens no HTTP endpoint.

Add a secret only when applications outside the WOML runtime must publish it:

```xml
<event
  id="customerUpdated"
  name="customer.updated"
  secret="{{secrets.EVENT_CONTROL_TOKEN}}"
/>
```

That additionally activates `POST /_woml/events/customer.updated`. Internal
publication still does not read or pass the secret.

## Safety and limits

Payloads must be top-level JSON objects no larger than 1 MiB. One event name
may have at most 1,000 loaded subscribers. Hidden durable lineage stops at 32
publications and rejects a cycle before it repeats the same workflow/event
trigger pair. Payload values never enter operation events, progress messages,
or diagnostics.

## Run the example

From the project root after rebuilding/linking the CLI:

```bash
woml run examples/internalEvents
```

The manual publisher runs once, emits `customer.updated`, and starts the
subscriber in the same long-lived automation runtime. No secret setup is
required for this internal-only example.
