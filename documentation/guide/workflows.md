# Workflows

Workflows are the core building blocks of Cronflow. They define a sequence of steps that execute in order, with the ability to pass data between steps and handle errors gracefully.

## Defining Workflows

Use `cronflow.define()` to create a new workflow with its configuration and settings.

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'order-processor',
  name: 'Order Processing Workflow',
  description: 'Processes incoming orders with payment validation',
  tags: ['ecommerce', 'critical'],
  concurrency: 10,
  timeout: '5m',
  rateLimit: {
    count: 100,
    per: '1h',
  },
  queue: 'high-priority',
  version: 'v1.0.0',
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

## Configuration Options

| Option        | Type               | Default     | Description                                                                      |
| ------------- | ------------------ | ----------- | -------------------------------------------------------------------------------- |
| `id`          | `string`           | -           | A globally unique, kebab-case identifier (e.g., 'order-processor')               |
| `name`        | `string`           | -           | A human-readable name (e.g., "Order Processor")                                  |
| `description` | `string`           | -           | A longer description of the workflow's purpose                                   |
| `tags`        | `string[]`         | `[]`        | An array of tags for organization (e.g., ['ecommerce', 'critical'])              |
| `hooks`       | `object`           | -           | Global lifecycle hooks for every run of this workflow                            |
| `timeout`     | `string \| number` | `'30m'`     | Maximum duration for the entire workflow run (e.g., '10m')                       |
| `concurrency` | `number`           | `Infinity`  | Maximum number of concurrent runs allowed (1 ensures sequential execution)       |
| `rateLimit`   | `object`           | -           | Limits execution frequency: `{ count: number, per: string }`                     |
| `queue`       | `string`           | `'default'` | Assigns workflow to a specific execution queue for prioritization                |
| `version`     | `string`           | -           | Version string (e.g., 'v1.0.0') to manage multiple workflow versions             |
| `secrets`     | `object`           | -           | Configuration for fetching secrets from a vault instead of environment variables |

## Workflow Lifecycle

### 1. Definition

The workflow is defined with its configuration and steps.

```typescript
const orderWorkflow = cronflow.define({
  id: 'order-processing',
  name: 'Order Processing',
  concurrency: 5,
  timeout: '10m',
});

orderWorkflow
  .onWebhook('/webhooks/orders')
  .step('validate-order', async ctx => {
    // Validate the order
    return { validated: true };
  })
  .step('process-payment', async ctx => {
    // Process payment
    return { paymentId: 'pay_123' };
  })
  .action('send-confirmation', async ctx => {
    // Send confirmation email
  });
```

### 2. Registration

When you call `cronflow.start()`, all defined workflows are registered with the engine.

```typescript
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 3000,
  },
});
```

### 3. Execution

Workflows are triggered by their configured triggers (webhooks, schedules, events, etc.).

### 4. Completion

Workflows complete when all steps finish or when an error occurs.

## Workflow Hooks

Hooks allow you to respond to workflow lifecycle events:

```typescript
const workflow = cronflow.define({
  id: 'my-workflow',
  hooks: {
    onSuccess: ctx => {
      // Called when workflow completes successfully
      console.log(`Workflow ${ctx.run.id} succeeded`);
      await analytics.track('workflow_success', {
        workflowId: ctx.run.workflowId,
        runId: ctx.run.id,
        duration: ctx.run.duration,
      });
    },
    onFailure: ctx => {
      // Called when workflow fails or times out
      console.error(`Workflow ${ctx.run.id} failed:`, ctx.error);
      await slack.sendMessage(
        '#alerts',
        `Workflow failed: ${ctx.run.workflowId} - ${ctx.error.message}`
      );
    },
  },
});
```

## Concurrency Control

Control how many instances of a workflow can run simultaneously:

```typescript
// Allow only 1 instance at a time (sequential execution)
const sequentialWorkflow = cronflow.define({
  id: 'sequential-workflow',
  concurrency: 1,
});

// Allow up to 10 concurrent instances
const parallelWorkflow = cronflow.define({
  id: 'parallel-workflow',
  concurrency: 10,
});

// Unlimited concurrency (default)
const unlimitedWorkflow = cronflow.define({
  id: 'unlimited-workflow',
  concurrency: Infinity,
});
```

## Rate Limiting

Limit how frequently a workflow can be executed:

```typescript
const rateLimitedWorkflow = cronflow.define({
  id: 'rate-limited-workflow',
  rateLimit: {
    count: 100, // Maximum 100 executions
    per: '1h', // Per hour
  },
});
```

## Timeout Configuration

Set maximum execution time for workflows:

```typescript
const quickWorkflow = cronflow.define({
  id: 'quick-workflow',
  timeout: '30s', // 30 seconds
});

const longWorkflow = cronflow.define({
  id: 'long-workflow',
  timeout: '2h', // 2 hours
});

const unlimitedWorkflow = cronflow.define({
  id: 'unlimited-workflow',
  timeout: 0, // No timeout
});
```

## Queue Management

Assign workflows to specific queues for prioritization:

```typescript
const highPriorityWorkflow = cronflow.define({
  id: 'high-priority-workflow',
  queue: 'high-priority',
});

const normalWorkflow = cronflow.define({
  id: 'normal-workflow',
  queue: 'default',
});

const lowPriorityWorkflow = cronflow.define({
  id: 'low-priority-workflow',
  queue: 'low-priority',
});
```

## Version Management

Use version strings to manage multiple workflow versions:

```typescript
const v1Workflow = cronflow.define({
  id: 'order-processing',
  version: 'v1.0.0',
  // v1 implementation
});

const v2Workflow = cronflow.define({
  id: 'order-processing',
  version: 'v2.0.0',
  // v2 implementation with new features
});
```

## Workflow State

Workflows maintain state across runs using the context object:

```typescript
const statefulWorkflow = cronflow.define({
  id: 'stateful-workflow',
});

statefulWorkflow
  .step('increment-counter', async ctx => {
    // Get current counter value
    const currentCount = await ctx.state.get('counter', 0);

    // Increment and store
    const newCount = currentCount + 1;
    await ctx.state.set('counter', newCount);

    return { count: newCount };
  })
  .step('log-progress', async ctx => {
    const totalRuns = await ctx.state.get('counter', 0);
    console.log(`This workflow has run ${totalRuns} times`);
  });
```

## Workflow Inspection

Inspect running and completed workflows:

```typescript
// Get workflow status
const status = await cronflow.inspect('run-id-123');

// Cancel a running workflow
await cronflow.cancelRun('run-id-123');

// List paused workflows
const pausedWorkflows = cronflow.listPausedWorkflows();
```

## Best Practices

### 1. Use Descriptive IDs

```typescript
// Good
const workflow = cronflow.define({ id: 'user-onboarding-v2' });

// Avoid
const workflow = cronflow.define({ id: 'workflow1' });
```

### 2. Set Appropriate Timeouts

```typescript
// Quick operations
const quickWorkflow = cronflow.define({
  id: 'data-validation',
  timeout: '30s',
});

// Long-running operations
const longWorkflow = cronflow.define({
  id: 'data-processing',
  timeout: '1h',
});
```

### 3. Use Tags for Organization

```typescript
const workflow = cronflow.define({
  id: 'order-processing',
  tags: ['ecommerce', 'critical', 'payment'],
});
```

### 4. Handle Errors Gracefully

```typescript
const robustWorkflow = cronflow.define({
  id: 'robust-workflow',
  hooks: {
    onFailure: ctx => {
      // Log error details
      console.error('Workflow failed:', {
        runId: ctx.run.id,
        error: ctx.error.message,
        stack: ctx.error.stack,
      });

      // Send alert
      await slack.sendMessage(
        '#alerts',
        `Workflow ${ctx.run.workflowId} failed: ${ctx.error.message}`
      );
    },
  },
});
```

## What's Next?

- **[Steps](/guide/steps)** - Learn about individual workflow steps
- **[Conditions](/guide/conditions)** - Add dynamic logic to workflows
- **[Error Handling](/guide/error-handling)** - Build robust error handling
- **[Performance](/guide/performance)** - Optimize workflow performance
