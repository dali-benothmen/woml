# Trigger Methods

## `.onWebhook(path, options?)`

Registers a webhook endpoint to trigger the workflow.

```typescript
workflow.onWebhook('/webhooks/stripe', {
  method: 'POST',
  schema: z.object({
    orderId: z.string().uuid(),
    amount: z.number().positive(),
  }),
});
```

## `.onSchedule(cronString)`

Triggers the workflow based on a CRON string.

```typescript
workflow.onSchedule('0 2 * * *'); // Run at 2 AM every day
```

## `.onInterval(interval)`

Triggers the workflow at a fixed, human-readable interval.

```typescript
workflow.onInterval('15m'); // Run every 15 minutes
```

## `.onEvent(eventName)`

Triggers the workflow when a custom event is published.

```typescript
workflow.onEvent('user.registered');
```

## `.onPoll(pollFn, options?)`

Triggers the workflow for each new item found by a polling function.

```typescript
workflow.onPoll(async ctx => {
  const newEmails = await fetchNewEmails();
  return newEmails.map(email => ({
    id: email.id,
    payload: email,
  }));
});
```

## `.manual()`

Registers a manual trigger for the workflow.
