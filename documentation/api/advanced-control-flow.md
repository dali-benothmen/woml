# Advanced Control Flow

## `.forEach(name, items, iterationFn)`

Dynamically executes a sub-workflow in parallel for each item in an array.

## `.batch(name, options, batchFn)`

Processes a large array of items in smaller, sequential batches.

## `.humanInTheLoop(options)`

Pauses the workflow to wait for external human input.

```typescript
workflow.humanInTheLoop({
  timeout: '3d',
  description: 'Approve high-value transaction',
  onPause: (ctx, token) => {
    sendApprovalEmail(token, {
      amount: ctx.last.amount,
      approvalUrl: `https://approvals.example.com/approve?token=${token}`,
    });
  },
});
```

## `.waitForEvent(eventName, timeout?)`

Pauses the workflow until a specific event is emitted.
