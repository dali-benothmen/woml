# Workflow Instance Methods

Methods available on workflow instances returned by `cronflow.define()`.

## `.step(name, handlerFn, options?)`

Defines a primary unit of work that produces a storable output.

```typescript
workflow.step('fetch-user', async ctx => {
  const user = await db.users.findUnique({
    where: { id: ctx.payload.userId },
  });
  return user;
});
```

## `.action(name, handlerFn, options?)`

Defines a unit of work for side-effects where the output is ignored.

```typescript
workflow.action('send-notification', async ctx => {
  await slack.sendMessage('#alerts', 'Data processed successfully');
});
```

## `.retry(options)`

Attaches a retry policy to the preceding step.

```typescript
workflow
  .step('api-call', async () => {
    /* ... */
  })
  .retry({
    attempts: 5,
    backoff: { strategy: 'exponential', delay: '2s' },
  });
```

## `.timeout(duration)`

Sets a timeout for the preceding step.

## `.cache(config)`

Configures caching for the preceding step.

## `.delay(duration)`

Adds a delay before executing the preceding step.

## `.onError(handlerFn)`

Attaches a custom error handling function to the preceding step.

## `.log(message, level?)`

A dedicated step for structured logging within a workflow run.
