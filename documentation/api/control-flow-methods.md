# Control Flow Methods

## `.if(name, condition, options?)`

Defines conditional execution paths.

```typescript
workflow
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .step('send-vip-notification', async ctx => {
    return await sendVIPNotification(ctx.last);
  })
  .endIf();
```

## `.elseIf(name, condition)`

Defines alternative conditions.

## `.else()`

Defines the default condition.

## `.endIf()`

Ends a conditional block.

## `.parallel(steps)`

Executes a set of steps concurrently.

```typescript
workflow.parallel([
  ctx => ctx.services.db.fetchSalesData(),
  ctx => ctx.services.db.fetchUserData(),
]);
```

## `.race(steps)`

Executes multiple branches concurrently and proceeds with the first to finish.

## `.while(name, condition, iterationFn)`

Creates a durable loop that executes as long as a condition is met.

## `.cancel(reason?)`

Gracefully stops the execution of the current workflow path.

## `.sleep(duration)`

Pauses the workflow for a specified duration.

## `.subflow(name, workflowId, input?)`

Executes another Workflow as a child process.
