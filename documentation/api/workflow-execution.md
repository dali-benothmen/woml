# Workflow Execution

Functions for executing and managing workflow runs.

## `cronflow.executeStep(runId, stepId, contextJson?)`

Executes a specific step within a workflow run.

## `cronflow.executeStepFunction(stepName, contextJson, workflowId, runId)`

Executes a step function with the given context.

## `cronflow.executeJobFunction(jobJson, servicesJson)`

Executes a job function with the given parameters.

## `cronflow.replay(runId, options?)`

Re-runs a previously executed workflow from its recorded history.

```typescript
await cronflow.replay('run_id_of_failed_payment', {
  mockStep('process-payment-api', async ctx => {
    return { status: 'succeeded', transactionId: 'txn_mock_123' };
  })
});
```
