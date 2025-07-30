# Human-in-the-Loop

Functions for managing workflows that require human approval.

## `cronflow.resume(token, payload)`

Resumes a workflow paused by `.humanInTheLoop()`.

```typescript
await cronflow.resume('approval_token_123', {
  approved: true,
  reason: 'Looks good to me',
});
```

## `cronflow.storePausedWorkflow(token, pauseInfo)`

Stores information about a paused workflow.

## `cronflow.getPausedWorkflow(token)`

Retrieves information about a specific paused workflow by its token.

## `cronflow.listPausedWorkflows()`

Returns an array of all currently paused workflows waiting for human approval.
