# Cronflow API Reference

A comprehensive guide to the cronflow API for building reliable, scalable workflow automation.

## Table of Contents

1. [Core Namespace: cronflow](#core-namespace-cronflow)
2. [Workflow Definition API](#workflow-definition-api)
3. [Trigger Methods](#trigger-methods)
4. [Step & Action Methods](#step--action-methods)
5. [Control Flow Methods](#control-flow-methods)
6. [Advanced Control Flow](#advanced-control-flow)
7. [Testing API](#testing-api)
8. [Context (ctx) Object](#context-ctx-object)
9. [Engine API](#engine-api)
10. [Configuration Options](#configuration-options)
11. [Examples](#examples)

---

## Core Namespace: cronflow

The main entry point for the cronflow framework.

```typescript
import { cronflow } from "cronflow";
```

### `cronflow.define(options, setupFn?)`

Defines a new, isolated Workflow instance with its configuration, services, and hooks.

#### Parameters

**`options` (object)**: The primary configuration for the workflow.

| Property      | Type                  | Required | Description                                                                      |
| ------------- | --------------------- | -------- | -------------------------------------------------------------------------------- |
| `id`          | `string`              | ✅       | A globally unique, kebab-case identifier (e.g., 'order-processor')               |
| `name`        | `string`              | ❌       | A human-readable name (e.g., "Order Processor")                                  |
| `description` | `string`              | ❌       | A longer description of the workflow's purpose                                   |
| `tags`        | `string[]`            | ❌       | An array of tags for organization (e.g., ['ecommerce', 'critical'])              |
| `hooks`       | `object`              | ❌       | Global lifecycle hooks for every run of this workflow                            |
| `timeout`     | `string \| number`    | ❌       | Maximum duration for the entire workflow run (e.g., '10m')                       |
| `concurrency` | `number`              | ❌       | Maximum number of concurrent runs allowed (1 ensures sequential execution)       |
| `rateLimit`   | `object`              | ❌       | Limits execution frequency: `{ count: number, per: string }`                     |
| `queue`       | `string`              | ❌       | Assigns workflow to a specific execution queue for prioritization                |
| `version`     | `string`              | ❌       | Version string (e.g., 'v1.0.0') to manage multiple workflow versions             |
| `secrets`     | `object`              | ❌       | Configuration for fetching secrets from a vault instead of environment variables |

**`setupFn` ((workflow) => void)**: Optional callback function that receives the newly created workflow instance.

#### Hook Options

| Hook        | Type            | Description                                                              |
| ----------- | --------------- | ------------------------------------------------------------------------ |
| `onSuccess` | `(ctx) => void` | Called when a run completes successfully                                 |
| `onFailure` | `(ctx) => void` | Called when a run fails or times out. `ctx` contains an `error` property |

#### Rate Limit Configuration

The `rateLimit` option accepts an object with the following properties:

| Property | Type     | Required | Description                                    |
| -------- | -------- | -------- | ---------------------------------------------- |
| `count`  | `number` | ✅       | Maximum number of executions allowed           |
| `per`    | `string` | ✅       | Time period for the limit (e.g., '1m', '1h') |

#### Secrets Configuration

The `secrets` option allows you to configure secret management for the workflow:

```typescript
{
  secrets: {
    // Configuration for fetching secrets from a vault
    // Implementation details depend on your secret management system
  }
}
```

#### Example

```typescript
const orderWorkflow = cronflow.define({
  id: "v1-order-processor",
  name: "Order Processor",
  description: "Processes incoming orders with payment validation",
  tags: ["ecommerce", "finance", "critical"],
  concurrency: 10,
  timeout: "5m",
  rateLimit: {
    count: 100,
    per: "1h"
  },
  queue: "high-priority",
  version: "v1.0.0",
  secrets: {
    // Secret management configuration
  },
  hooks: {
    onSuccess: (ctx) => {
      console.log(`✅ Workflow ${ctx.run.id} completed successfully`);
    },
    onFailure: (ctx) => {
      console.error(`❌ Run ${ctx.run.id} failed!`, ctx.error);
    },
  },
});
```

### `cronflow.start(options?)`

Boots the engine, registers all defined workflows, and begins listening for triggers.

#### Parameters

- `options` (object): Optional configuration for the engine startup
  - `webhookServer` (object): Configuration for the webhook HTTP server
    - `host` (string): Host address to bind to (default: '127.0.0.1')
    - `port` (number): Port number to listen on (default: 3000)
    - `maxConnections` (number): Maximum concurrent connections (default: 1000)

#### Examples

**Basic startup (localhost only):**
```typescript
await cronflow.start();
```

**External access configuration:**
```typescript
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',        // Allow external connections
    port: 3000,              // Standard HTTP port
    maxConnections: 1000,    // Handle concurrent requests
  },
});
```

**Custom port configuration:**
```typescript
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 8080,              // Custom port
  },
});
```

### `cronflow.stop()`

Gracefully shuts down the engine, allowing in-progress tasks to complete.

### `cronflow.trigger(workflowId, payload)`

Manually starts a run of a workflow by its ID with a given payload.

#### Parameters

- `workflowId` (string): The ID of the workflow to trigger
- `payload` (any): The data to pass to the workflow

#### Example

```typescript
await cronflow.trigger("order-processor", {
  orderId: "ord_123",
  amount: 99.99,
});
```

### `cronflow.inspect(runId)`

Retrieves the status and history of a specific workflow run for debugging and monitoring.

#### Parameters

- `runId` (string): The ID of the run to inspect

#### Returns

An object containing the run's status, history, and metadata.

### `cronflow.cancelRun(runId)`

Programmatically finds and cancels a specific, currently running workflow instance.

### `cronflow.publishEvent(name, payload)`

Publishes a global event that can be used to trigger workflows listening via `onEvent()`.

---

## Workflow Definition API

Methods available on the object returned by `cronflow.define()`.

---

## Trigger Methods

### `.onWebhook(path, options?)`

Registers a webhook endpoint to trigger the workflow on an HTTP request.

#### Parameters

- `path` (string): The URL path to listen on (e.g., '/webhooks/stripe')
- `options` (object): Optional configuration

#### Options

| Property         | Type                     | Default  | Description                                                          |
| ---------------- | ------------------------ | -------- | -------------------------------------------------------------------- |
| `method`         | `'POST' \| 'GET' \| ...` | `'POST'` | The HTTP method to accept                                            |
| `schema`         | `z.ZodObject`            | -        | Zod schema to validate incoming request body                         |
| `validate`       | `Function`               | -        | Custom validation function (payload) => boolean \| string            |
| `idempotencyKey` | `(ctx) => string`        | -        | Function to extract key from request to prevent duplicate processing |
| `parseRawBody`   | `boolean`                | `false`  | Whether to parse the raw body for signature validation               |
| `app`            | `string`                 | -        | Framework name for integration (e.g., 'express', 'fastify')          |
| `appInstance`    | `any`                    | -        | Framework app instance for integration                               |
| `trigger`        | `string`                 | -        | ID or name of specific step to trigger instead of full workflow      |
| `condition`      | `Function`               | -        | Function to evaluate whether webhook should execute (req) => boolean  |
| `middleware`     | `Array<Function>`        | -        | Array of middleware functions to execute before the webhook handler  |
| `onSuccess`      | `Function`               | -        | Callback executed when webhook processing succeeds                   |
| `onError`        | `Function`               | -        | Callback executed when webhook processing fails                      |
| `retry`          | `RetryConfig`            | -        | Retry configuration for failed webhook executions                    |

#### Example

```typescript
import { z } from "zod";

// Basic webhook with schema validation
orderWorkflow.onWebhook("/v1/orders/create", {
  schema: z.object({
    orderId: z.string().uuid(),
    amount: z.number().positive(),
  }),
  idempotencyKey: (ctx) => ctx.trigger.headers["x-idempotency-key"],
  parseRawBody: true, // For Stripe signature validation
});

// Webhook with custom validation
orderWorkflow.onWebhook("/webhooks/orders/validate", {
  method: "POST",
  validate: (payload) => {
    // Custom business rules
    if (payload.amount > 10000) {
      return "Orders above $10,000 require manual review";
    }
    
    if (payload.items && payload.items.length > 50) {
      return "Orders with more than 50 items are not supported";
    }
    
    // Validation passed
    return true;
  },
  schema: z.object({
    orderId: z.string(),
    amount: z.number().positive(),
    items: z.array(z.object({ id: z.string(), quantity: z.number() })).optional(),
  }),
});

// Webhook with async custom validation
orderWorkflow.onWebhook("/webhooks/orders/async-validate", {
  method: "POST",
  validate: async (payload) => {
    // Async validation (e.g., database checks)
    const customer = await customerService.getCustomer(payload.customerId);
    
    if (!customer.isActive) {
      return "Customer account is inactive";
    }
    
    if (customer.creditLimit < payload.amount) {
      return `Order amount exceeds credit limit of $${customer.creditLimit}`;
    }
    
    return true;
  },
  schema: z.object({
    customerId: z.string(),
    amount: z.number().positive(),
  }),
});

// Webhook with both schema and custom validation
orderWorkflow.onWebhook("/webhooks/orders/comprehensive", {
  method: "POST",
  // Schema validation runs first
  schema: z.object({
    orderId: z.string().min(1),
    amount: z.number().positive(),
    customerId: z.string().min(1),
    priority: z.enum(['low', 'medium', 'high']).optional(),
  }),
  // Custom validation runs after schema validation
  validate: (payload) => {
    // Business rule: high priority orders must be above $100
    if (payload.priority === 'high' && payload.amount < 100) {
      return "High priority orders must be at least $100";
    }
    
    // Business rule: weekend orders have amount limits
    const isWeekend = new Date().getDay() === 0 || new Date().getDay() === 6;
    if (isWeekend && payload.amount > 5000) {
      return "Weekend orders are limited to $5,000";
    }
    
    return true;
  },
});

// Webhook with middleware for authentication and rate limiting
orderWorkflow.onWebhook("/webhooks/orders", {
  method: "POST",
  middleware: [
    async (req, res, next) => {
      // Rate limiting
      const clientId = req.headers["x-client-id"];
      if (!clientId) {
        return res.status(401).json({ error: "Missing client ID" });
      }
      next();
    },
    async (req, res, next) => {
      // Authentication
      const token = req.headers.authorization;
      if (!token || !isValidToken(token)) {
        return res.status(401).json({ error: "Invalid token" });
      }
      next();
    },
  ],
  schema: z.object({
    orderId: z.string(),
    amount: z.number(),
  }),
});

// Webhook with success/error callbacks
orderWorkflow.onWebhook("/webhooks/orders", {
  method: "POST",
  trigger: "process-order", // Trigger specific step
  onSuccess: (ctx, result) => {
    console.log("✅ Webhook processed successfully", {
      workflowId: ctx.workflow_id,
      result: result
    });
    
    // Send notification, log to analytics, etc.
    analyticsService.track('webhook_success', {
      workflow: ctx.workflow_id,
      step: ctx.step_name,
      timestamp: new Date()
    });
  },
  onError: (ctx, error) => {
    console.error("❌ Webhook processing failed:", error);
    
    // Send alert, log error details, etc.
    alertService.sendAlert({
      type: 'webhook_failure',
      workflow: ctx.workflow_id,
      error: error.message,
      context: ctx
    });
  },
  schema: z.object({
    orderId: z.string(),
    amount: z.number(),
  }),
});

// Webhook with conditional execution
orderWorkflow.onWebhook("/webhooks/high-value-orders", {
  method: "POST",
  condition: (req) => {
    // Only trigger for orders above $100
    return req.body.amount > 100;
  },
  onSuccess: (ctx, result) => {
    console.log("✅ High-value order processed:", ctx.payload.amount);
  },
  schema: z.object({
    orderId: z.string(),
    amount: z.number().positive(),
  }),
});

// Webhook with async condition
orderWorkflow.onWebhook("/webhooks/premium-customers", {
  method: "POST",
  condition: async (req) => {
    // Async condition check
    const customer = await customerService.getCustomer(req.body.customerId);
    return customer.tier === 'premium';
  },
  trigger: "premium-processing",
  schema: z.object({
    customerId: z.string(),
    orderId: z.string(),
  }),
});

// Webhook with complex conditions
orderWorkflow.onWebhook("/webhooks/business-hours", {
  method: "POST",
  condition: (req) => {
    const now = new Date();
    const hour = now.getHours();
    const isWeekend = now.getDay() === 0 || now.getDay() === 6;
    
    // Only process during business hours (9 AM - 5 PM, weekdays)
    return !isWeekend && hour >= 9 && hour < 17;
  },
  onSuccess: (ctx, result) => {
    console.log("✅ Business hours order processed");
  },
});

// Webhook with retry configuration
orderWorkflow.onWebhook("/webhooks/orders/retry", {
  method: "POST",
  retry: {
    attempts: 3,
    backoff: { 
      strategy: "exponential",
      delay: "1s",
      maxDelay: "30s",
      multiplier: 2
    },
    onRetry: (error, attempt, nextDelay) => {
      console.log(`🔄 Retry attempt ${attempt}, next delay: ${nextDelay}ms`);
      console.log(`Error: ${error.message}`);
    }
  },
  trigger: "process-order",
  onSuccess: (ctx, result) => {
    console.log("✅ Order processed successfully after retries");
  },
  onError: (ctx, error) => {
    console.error("❌ Order processing failed after all retries:", error.message);
  }
});

// Webhook with custom retry conditions
orderWorkflow.onWebhook("/webhooks/orders/selective-retry", {
  method: "POST",
  retry: {
    attempts: 5,
    backoff: { 
      strategy: "linear",
      delay: "2s",
      multiplier: 1.5
    },
    retryOn: {
      // Only retry on specific errors
      errors: ["timeout", "network error", "service unavailable"],
      // Only retry on specific status codes
      statusCodes: [500, 502, 503, 504],
      // Custom retry condition
      conditions: (error, attempt) => {
        // Don't retry validation errors
        if (error.message.includes("validation")) {
          return false;
        }
        // Retry network issues up to 3 times
        if (error.message.includes("network") && attempt < 3) {
          return true;
        }
        return false;
      }
    }
  },
  validate: (payload) => {
    if (!payload.orderId) {
      throw new Error("validation: orderId is required");
    }
    return true;
  }
});

// Webhook with different backoff strategies
orderWorkflow.onWebhook("/webhooks/orders/backoff-strategies", {
  method: "POST",
  // Fixed delay: 5s between each retry
  retry: {
    attempts: 3,
    backoff: { 
      strategy: "fixed",
      delay: "5s"
    }
  }
});

orderWorkflow.onWebhook("/webhooks/orders/linear-backoff", {
  method: "POST",
  // Linear backoff: 1s, 2s, 3s, 4s...
  retry: {
    attempts: 4,
    backoff: { 
      strategy: "linear",
      delay: "1s",
      multiplier: 1 // Each retry adds 1 * multiplier * delay
    }
  }
});

orderWorkflow.onWebhook("/webhooks/orders/exponential-backoff", {
  method: "POST",
  // Exponential backoff: 1s, 2s, 4s, 8s... (capped at 60s)
  retry: {
    attempts: 6,
    backoff: { 
      strategy: "exponential",
      delay: "1s",
      maxDelay: "60s",
      multiplier: 2
    }
  }
});
```

### `.onSchedule(cronString)`

Triggers the workflow based on a CRON string.

#### Example

```typescript
// Run at 2 AM every day
const reportWorkflow = cronflow.define({ id: "daily-report" });
reportWorkflow.onSchedule("0 2 * * *");
```

### `.onInterval(interval)`

Triggers the workflow at a fixed, human-readable interval.

#### Example

```typescript
// Run every 15 minutes
const healthCheckWorkflow = cronflow.define({ id: "health-check" });
healthCheckWorkflow.onInterval("15m");
```

### `.onEvent(eventName)`

Triggers the workflow when a custom event is published via `cronflow.publishEvent()`.

#### Example

```typescript
const notificationWorkflow = cronflow.define({ id: "user-notification" });
notificationWorkflow.onEvent("user.registered");
```

### `.onPoll(pollFn, options?)`

Triggers the workflow for each new item found by a polling function.

#### Parameters

- `pollFn` ((ctx) => Promise<Array<{id: string, payload: any}>>): Function that returns new items
- `options` (object): Optional configuration

#### Example

```typescript
const emailWorkflow = cronflow.define({ id: "email-processor" });
emailWorkflow.onPoll(
  async (ctx) => {
    const newEmails = await fetchNewEmails();
    return newEmails.map((email) => ({
      id: email.id,
      payload: email,
    }));
  },
  {
    interval: "5m", // Poll every 5 minutes
  }
);
```

---

## Step & Action Methods

### `.step(name, handlerFn, options?)`

Defines a primary unit of work that produces a storable output.

#### Parameters

- `name` (string): A unique name for the step (e.g., 'fetch-user')
- `handlerFn` ((ctx) => any | Promise<any>): The function to execute
- `options` (object): Optional step-specific overrides for retry, timeout, delay, cache

#### Example

```typescript
.step('fetchSalesData', async (ctx) => {
  return await db.query('SELECT * FROM sales WHERE date = ?', [new Date()]);
}, {
  timeout: '5m',
  cache: {
    key: (ctx) => `sales-${new Date().toISOString().split('T')[0]}`,
    ttl: '12h'
  }
})
```

### `.action(name, handlerFn, options?)`

Defines a unit of work for side-effects where the output is ignored.

#### Example

```typescript
.action('sendToSlack', async (ctx) => {
  const message = ctx.steps.formatMessage.output;
  await slack.sendMessage('#sales', message);
}, {
  retry: {
    attempts: 3,
    backoff: { strategy: 'fixed', delay: '10s' }
  }
})
```

### `.retry(options)`

Attaches a retry policy to the preceding step.

#### Options

| Property           | Type                       | Default         | Description                    |
| ------------------ | -------------------------- | --------------- | ------------------------------ |
| `attempts`         | `number`                   | `3`             | Number of retry attempts       |
| `backoff`          | `object`                   | -               | Backoff strategy configuration |
| `backoff.strategy` | `'exponential' \| 'fixed'` | `'exponential'` | Backoff strategy               |
| `backoff.delay`    | `string \| number`         | `'1s'`          | Initial delay                  |

#### Example

```typescript
.step('api-call', async () => { /* ... */ })
.retry({
  attempts: 5,
  backoff: { strategy: 'exponential', delay: '2s' }
})
```

### `.onError(handlerFn)`

Attaches a custom error handling function to the preceding step.

#### Example

```typescript
.step('risky-operation', async () => { /* ... */ })
.onError((ctx) => {
  console.error('Step failed:', ctx.error);
  // Return a fallback value
  return { status: 'fallback' };
})
```

### `.log(message, level?)`

A dedicated step for structured logging within a workflow run.

#### Parameters

- `message` (string | (ctx) => string): The log message
- `level` (`'info' \| 'warn' \| 'error'`): Log level (default: 'info')

#### Example

```typescript
.log('Processing order', 'info')
.step('process-order', async (ctx) => { /* ... */ })
.log((ctx) => `Order ${ctx.last.id} processed successfully`)
```

---

## Control Flow Methods

### `.if(name, condition, options?)`

Defines conditional execution paths based on the current context.

#### Parameters

- `name` (string): A unique name for this conditional block
- `condition` ((ctx) => boolean | Promise<boolean>): The condition to evaluate
- `options` (object): Optional configuration

#### Example

```typescript
.if('is-high-value', (ctx) => ctx.last.amount > 500)
  .step('send-vip-notification', async (ctx) => {
    // This step only runs if the condition is true
    return await sendVIPNotification(ctx.last);
  })
.elseIf('is-medium-value', (ctx) => ctx.last.amount > 100)
  .step('send-standard-notification', async (ctx) => {
    return await sendStandardNotification(ctx.last);
  })
.else()
  .step('send-basic-notification', async (ctx) => {
    return await sendBasicNotification(ctx.last);
  })
.endIf()
```

### `.parallel(steps)`

Executes a set of steps concurrently and waits for all to complete.

#### Parameters

- `steps` (Array<(ctx) => any | Promise<any>>): Array of functions to execute in parallel

#### Example

```typescript
.parallel([
  (ctx) => ctx.services.db.fetchSalesData(),
  (ctx) => ctx.services.db.fetchUserData(),
])
.step('generate-report', (ctx) => {
  const [salesData, userData] = ctx.last; // ctx.last holds the array of results
  return generateReport(salesData, userData);
})
```

### `.race(steps)`

Executes multiple branches concurrently and proceeds with the one that finishes first.

#### Parameters

- `steps` (Array<(ctx) => any | Promise<any>>): Array of functions to race

#### Example

```typescript
.race([
  (ctx) => fetchFromPrimaryAPI(),
  (ctx) => fetchFromBackupAPI(),
])
.step('process-result', (ctx) => {
  // ctx.last contains the result from whichever function finished first
  return processResult(ctx.last);
})
```

### `.while(name, condition, iterationFn)`

Creates a durable loop that executes as long as a condition is met.

#### Parameters

- `name` (string): A unique name for this loop
- `condition` ((ctx) => boolean | Promise<boolean>): The condition to check before each iteration
- `iterationFn` ((ctx) => void): The function to execute in each iteration

#### Example

```typescript
.while('process-queue',
  (ctx) => ctx.state.get('queue-size', 0) > 0,
  (ctx) => {
    // Process one item from the queue
    const item = ctx.state.get('next-item');
    processItem(item);
    ctx.state.set('queue-size', ctx.state.get('queue-size') - 1);
  }
)
```

### `.cancel(reason?)`

Gracefully stops the execution of the current workflow path.

#### Parameters

- `reason` (string): Optional reason for cancellation

#### Example

```typescript
.if('should-stop', (ctx) => ctx.last.status === 'cancelled')
  .cancel('Order was cancelled by user')
.endIf()
```

### `.sleep(duration)`

Pauses the workflow for a specified duration.

#### Parameters

- `duration` (string | number): Duration to sleep (e.g., '5s', '1m', 5000)

#### Example

```typescript
.step('send-notification', async (ctx) => {
  await sendNotification(ctx.last);
})
.sleep('30s') // Wait 30 seconds
.step('send-reminder', async (ctx) => {
  await sendReminder(ctx.last);
})
```

### `.subflow(name, workflowId, input?)`

Executes another Workflow as a child process, enabling modularity.

#### Parameters

- `name` (string): A unique name for this subflow
- `workflowId` (string): The ID of the workflow to execute
- `input` (any): Optional input data to pass to the subflow

#### Example

```typescript
.subflow('cleanup-temp-files', 'cleanup-workflow', {
  directory: '/tmp/workflow-data',
  olderThan: '24h'
})
```

---

## Advanced Control Flow

### `.forEach(name, items, iterationFn)`

Dynamically executes a sub-workflow in parallel for each item in an array.

#### Parameters

- `name` (string): A unique name for this loop block
- `items` ((ctx) => any[] | Promise<any[]>): A function that returns an array of items to iterate over
- `iterationFn` ((item, flow) => void): A function that defines the sub-workflow to be run for each item

#### Example

```typescript
.step('get-new-users', async () => db.users.findMany({ where: { onboarded: false } }))
.forEach('onboard-user-loop',
  (ctx) => ctx.steps['get-new-users'].output,
  (user, flow) => {
    // This sub-workflow runs for each user in parallel
    flow
      .step('send-welcome-email', async () => {
        return await ctx.services.resend.send({ to: user.email, ... });
      })
      .step('update-user-status', async () => {
        return await db.users.update({ where: { id: user.id }, data: { onboarded: true } });
      });
  }
)
```

### `.batch(name, options, batchFn)`

Processes a large array of items in smaller, sequential batches.

#### Parameters

- `name` (string): A unique name for the batching block
- `options` (object):
  - `items` ((ctx) => any[] | Promise<any[]>): A function returning the large array of items
  - `size` (number): The desired size for each batch
- `batchFn` ((batch, flow) => void): Function that processes each batch

#### Example

```typescript
.step('get-all-users', async () => db.users.findMany())
.batch('process-users-in-batches',
  {
    items: (ctx) => ctx.steps['get-all-users'].output,
    size: 100
  },
  (batch, flow) => {
    flow
      .step('process-batch', async () => {
        return await processUserBatch(batch);
      })
      .log((ctx) => `Processed batch of ${batch.length} users`);
  }
)
```

### `.humanInTheLoop(options)`

Pauses the workflow to wait for external human input via an API call.

#### Parameters

- `options` (object):
  - `timeout` (string, optional): Maximum time to wait for human input. If not provided, workflow pauses indefinitely until manually resumed.
  - `onPause` ((ctx, token) => void): Function called when workflow pauses, receives context and token
  - `description` (string): Description of what human input is needed

#### Example

```typescript
.humanInTheLoop({
  timeout: '3d', // Optional: wait up to 3 days
  description: 'Approve high-value transaction',
  onPause: (ctx, token) => {
    // Access previous step data from context
    const transactionAmount = ctx.last.amount;
    const customerId = ctx.steps['validate-transaction'].output.customerId;
    
    sendApprovalEmail(token, {
      amount: transactionAmount,
      customerId: customerId,
      approvalUrl: `https://approvals.example.com/approve?token=${token}`
    });
  }
})
.step('process-approval', (ctx) => {
  // This step runs after human approval or timeout
  if (ctx.last.timedOut) {
    return handleTimeoutScenario(ctx.last);
  }
  return processApprovedTransaction(ctx.last);
})
```

#### Behavior

- **With timeout**: Workflow waits for the specified duration, then automatically times out if no human approval is received
- **Without timeout**: Workflow pauses indefinitely until manually resumed via `cronflow.resume(token, payload)`
- **Timeout result**: Returns `{ approved: false, status: 'timeout', timedOut: true }`
- **Manual resume**: Returns the payload provided via `cronflow.resume(token, payload)`

### `.waitForEvent(eventName, timeout?)`

Pauses the workflow until a specific event is emitted.

#### Parameters

- `eventName` (string): The name of the event to wait for
- `timeout` (string): Optional timeout for waiting

#### Example

```typescript
.waitForEvent('payment.confirmed', '1h')
.step('process-confirmed-payment', (ctx) => {
  return processPayment(ctx.last);
})
```

---

## Testing API

A dedicated API for writing unit and integration tests for your workflows.

### `workflow.test()`

The entry point to the testing harness for a specific workflow.

#### Example

```typescript
const orderWorkflow = cronflow.define({ id: "order-processor" });
// ... define workflow steps

// In your test file
describe("Order Processing Workflow", () => {
  it("should process a valid order", async () => {
    const testRun = await orderWorkflow
      .test()
      .trigger({
        orderId: "ord_123",
        amount: 99.99,
      })
      .expectStep("validate-order")
      .toSucceed()
      .expectStep("process-payment")
      .toSucceed()
      .expectStep("send-confirmation")
      .toSucceed()
      .run();

    expect(testRun.status).toBe("completed");
  });
});
```

### `.trigger(payload)`

Simulates a trigger event to start a test run.

### `.mockStep(stepName, mockFn)`

Mocks the implementation of a specific step during testing.

#### Example

```typescript
await orderWorkflow
  .test()
  .trigger({ orderId: "ord_123" })
  .mockStep("process-payment", async (ctx) => {
    return { status: "succeeded", transactionId: "txn_test_123" };
  })
  .run();
```

### `.expectStep(stepName)`

Asserts expectations about a step's execution.

#### Example

```typescript
await orderWorkflow
  .test()
  .trigger({ orderId: "ord_123" })
  .expectStep("validate-order")
  .toSucceed()
  .expectStep("process-payment")
  .toFailWith("Insufficient funds")
  .run();
```

---

## Context (ctx) Object

The context object is passed to every step function and contains all the data and utilities needed for the step.

### Properties

| Property       | Type     | Description                                            |
| -------------- | -------- | ------------------------------------------------------ |
| `ctx.payload`  | `any`    | Data from the trigger that started the workflow        |
| `ctx.steps`    | `object` | Outputs from all previously completed steps            |
| `ctx.run`      | `object` | Metadata about the current run (`runId`, `workflowId`) |
| `ctx.state`    | `object` | Persistent state shared across workflow runs           |
| `ctx.last`     | `any`    | Output from the previous step (convenience property)   |
| `ctx.trigger`  | `object` | Information about what triggered this workflow         |

### State Management

```typescript
// Set a value
await ctx.state.set("user-count", 42);

// Get a value
const count = await ctx.state.get("user-count", 0); // 0 is default

// Increment a value
const newCount = await ctx.state.incr("user-count", 1);

// Set with TTL
await ctx.state.set("temp-data", data, { ttl: "1h" });
```

### Step Outputs

```typescript
// Access previous step outputs
const userData = ctx.steps["fetch-user"].output;
const orderData = ctx.steps["fetch-order"].output;

// Or use the convenience property
const lastStepOutput = ctx.last;
```

---

## Engine API

### `cronflow.replay(runId, options?)`

Re-runs a previously executed workflow from its recorded history.

#### Parameters

- `runId` (string): The ID of the failed or completed run to replay
- `options` (object):
  - `overridePayload` (object): Optional. Use a different initial trigger payload for the replay
  - `mockStep(stepName, mockFn)`: Optional. Override the implementation of specific steps during the replay

#### Example

```typescript
// In a separate script or test file
await cronflow.replay('run_id_of_failed_payment', {
  // Let's pretend the API call to the payment gateway now succeeds
  mockStep('process-payment-api', async (ctx) => {
    console.log('Replaying with mocked successful payment...');
    return { status: 'succeeded', transactionId: 'txn_mock_123' };
  })
});
```

### `cronflow.resume(token, payload)`

Resumes a workflow paused by `.humanInTheLoop()`.

#### Parameters

- `token` (string): The token provided by the human-in-the-loop step
- `payload` (any): The human's response/decision

#### Example

```typescript
// Called by your approval API endpoint
await cronflow.resume("approval_token_123", {
  approved: true,
  reason: "Looks good to me",
});
```

### `cronflow.listPausedWorkflows()`

Returns an array of all currently paused workflows waiting for human approval.

#### Returns

Array of paused workflow objects with the following structure:
```typescript
{
  token: string;
  workflowId: string;
  runId: string;
  stepId: string;
  description: string;
  metadata?: Record<string, any>;
  createdAt: number;
  expiresAt?: number;
  status: 'waiting' | 'resumed' | 'timeout';
  payload: any;
  lastStepOutput: any;
}
```

#### Example

```typescript
const pausedWorkflows = cronflow.listPausedWorkflows();
console.log('Paused workflows:', pausedWorkflows);
```

### `cronflow.getPausedWorkflow(token)`

Retrieves information about a specific paused workflow by its token.

#### Parameters

- `token` (string): The token of the paused workflow

#### Returns

The paused workflow object or `undefined` if not found.

#### Example

```typescript
const workflow = cronflow.getPausedWorkflow("approval_token_123");
if (workflow) {
  console.log('Workflow details:', workflow);
}
```

---

## Configuration Options

### Workflow Options

| Option        | Type               | Default     | Description                                                  |
| ------------- | ------------------ | ----------- | ------------------------------------------------------------ |
| `timeout`     | `string \| number` | `'30m'`     | Maximum duration for the entire workflow run                 |
| `concurrency` | `number`           | `Infinity`  | Maximum number of concurrent runs allowed                    |
| `rateLimit`   | `object`           | -           | Limits execution frequency: `{ count: number, per: string }` |
| `queue`       | `string`           | `'default'` | Assigns workflow to a specific execution queue               |
| `version`     | `string`           | -           | Version string to manage multiple workflow versions          |
| `secrets`     | `object`           | -           | Configuration for fetching secrets from a vault              |

### Step Options

| Option    | Type               | Default                 | Description                                                    |
| --------- | ------------------ | ----------------------- | -------------------------------------------------------------- |
| `timeout` | `string \| number` | Inherited from workflow | Maximum duration for this step                                 |
| `retry`   | `object`           | Inherited from workflow | Retry configuration for this step                              |
| `cache`   | `object`           | -                       | Caching configuration: `{ key: (ctx) => string, ttl: string }` |
| `delay`   | `string \| number` | -                       | Delay before executing this step                               |

### Retry Configuration

```typescript
{
  attempts: 3,
  backoff: {
    strategy: 'exponential', // or 'fixed'
    delay: '1s'
  }
}
```

### Cache Configuration

```typescript
{
  key: (ctx) => `user-${ctx.payload.userId}`,
  ttl: '1h'
}
```

---

## Examples

### Complete Order Processing Workflow

```typescript
import { cronflow } from "cronflow";
import { z } from "zod";
import { db } from "../../lib/db";
import {
  stripeService,
  slackService,
  jiraService,
  resendService,
} from "./services";

// Define the workflow
export const orderProcessingWorkflow = cronflow.define({
  id: "v1-order-processing",
  name: "Order Fulfillment Workflow",
  tags: ["ecommerce", "critical"],
  concurrency: 20,
  hooks: {
    onFailure: (ctx) => {
      console.error(`[Workflow Failed] Run ID: ${ctx.run.id}`, ctx.error);
    },
  },
});

// Define the trigger
orderProcessingWorkflow.onWebhook("/webhooks/stripe", {
  parseRawBody: true,
});

// Define the workflow steps
orderProcessingWorkflow
  .step("validate-stripe-signature", (ctx) => {
    const signature = ctx.trigger.headers["stripe-signature"];
    if (!signature) {
      throw new Error("Missing Stripe signature.");
    }

    // In a real implementation, you would validate the signature here
    return { validated: true, eventType: "checkout.session.completed" };
  })

  .step("fetch-order-and-user", async (ctx) => {
    // In a real implementation, you would fetch from database here
    return {
      id: "ord_123",
      totalAmount: 600,
      user: {
        email: "customer@example.com",
        stripeCustomerId: "cus_123",
      },
    };
  })

  .if("is-high-value-order", (ctx) => ctx.last.totalAmount > 500)
  .step("send-vip-notification", async (ctx) => {
    // In a real implementation, you would send VIP notification here
    return { notificationSent: true, type: "vip" };
  })
  .endIf()

  .step("create-fulfillment-task", async (ctx) => {
    // In a real implementation, you would create JIRA issue here
    return { taskCreated: true, taskId: "TASK-123" };
  })

  .step("send-confirmation-email", async (ctx) => {
    // In a real implementation, you would send email here
    return { emailSent: true, emailId: "email_123" };
  })

  .action("log-completion", (ctx) => {
    console.log(
      `Workflow completed. Task ${ctx.steps["create-fulfillment-task"].output.taskId} created. Email sent with ID ${ctx.steps["send-confirmation-email"].output.emailId}.`
    );
  });
```

### Testing the Workflow

```typescript
describe("Order Processing Workflow", () => {
  it("should process a high-value order", async () => {
    const testRun = await orderProcessingWorkflow
      .test()
      .trigger({
        type: "checkout.session.completed",
        data: {
          object: {
            id: "cs_test_123",
            metadata: { orderId: "ord_123" },
            amount_total: 60000, // $600
          },
        },
      })
      .expectStep("validate-stripe-signature")
      .toSucceed()
      .expectStep("fetch-order-and-user")
      .toSucceed()
      .expectStep("is-high-value-order")
      .toSucceed()
      .run();

    expect(testRun.status).toBe("completed");
    expect(testRun.steps["log-completion"]).toBeDefined();
  });
});
```

This comprehensive API reference provides everything you need to build powerful, reliable workflows with cronflow. The examples demonstrate real-world usage patterns and best practices for building production-ready automation systems.
