# Core Namespace

The main entry point for the Cronflow framework.

## `cronflow.define(options, setupFn?)`

Defines a new, isolated Workflow instance with its configuration, services, and hooks.

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'order-processor',
  name: 'Order Processing Workflow',
  description: 'Processes incoming orders with payment validation',
  tags: ['ecommerce', 'critical'],
  concurrency: 10,
  timeout: '5m',
  hooks: {
    onSuccess: ctx => {
      console.log(`✅ Workflow ${ctx.run.id} completed successfully`);
    },
    onFailure: ctx => {
      console.error(`❌ Run ${ctx.run.id} failed!`, ctx.error);
    },
  },
});
```

## `cronflow.start(options?)`

Boots the engine, registers all defined workflows, and begins listening for triggers.

```typescript
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 3000,
    maxConnections: 1000,
  },
});
```

## `cronflow.stop()`

Gracefully shuts down the engine, allowing in-progress tasks to complete.

## `cronflow.trigger(workflowId, payload)`

Manually starts a run of a workflow by its ID with a given payload.

```typescript
await cronflow.trigger('order-processor', {
  orderId: 'ord_123',
  amount: 99.99,
});
```

## `cronflow.inspect(runId)`

Retrieves the status and history of a specific workflow run for debugging and monitoring.

## `cronflow.cancelRun(runId)`

Programmatically finds and cancels a specific, currently running workflow instance.

## `cronflow.publishEvent(name, payload)`

Publishes a global event that can be used to trigger workflows listening via `onEvent()`.

## `cronflow.getWorkflows()`

Returns an array of all defined workflows.

## `cronflow.getWorkflow(id)`

Retrieves a specific workflow by its ID.

## `cronflow.getEngineState()`

Returns the current state of the engine ('STOPPED', 'STARTING', 'STARTED').

## `cronflow.VERSION`

The current version of Cronflow.
