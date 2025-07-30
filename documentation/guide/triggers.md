# Triggers

Triggers are the entry points that start your Cronflow workflows. They define when and how workflows are executed, supporting multiple trigger types for different use cases.

## Trigger Types

Cronflow supports several trigger types:

- **Webhook Triggers** - HTTP endpoints for external integrations
- **Schedule Triggers** - Time-based execution using cron expressions
- **Event Triggers** - Custom events for decoupled workflows
- **Manual Triggers** - Manual execution via API calls
- **Polling Triggers** - Automatic data checking

## Webhook Triggers

Webhook triggers create HTTP endpoints that can receive external requests and start workflows.

### Basic Webhook

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'order-processor',
  name: 'Order Processing Workflow',
});

workflow
  .onWebhook('/webhooks/orders', {
    method: 'POST',
    schema: z.object({
      orderId: z.string().uuid(),
      customerId: z.string(),
      amount: z.number().positive(),
    }),
  })
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  });
```

### Webhook with Headers Validation

```typescript
workflow.onWebhook('/webhooks/stripe', {
  method: 'POST',
  headers: {
    required: {
      'stripe-signature': 'whsec_...',
      'content-type': 'application/json',
    },
    validate: headers => {
      // Custom header validation
      if (!headers['user-agent']?.includes('Stripe')) {
        return 'Invalid user agent';
      }
      return true;
    },
  },
  schema: z.object({
    id: z.string(),
    type: z.string(),
    data: z.object({
      object: z.object({
        id: z.string(),
        amount: z.number(),
      }),
    }),
  }),
});
```

### Webhook with Framework Integration

```typescript
import express from 'express';

const app = express();

const workflow = cronflow.define({
  id: 'api-workflow',
});

workflow.onWebhook('/api/orders', {
  app: 'express',
  appInstance: app,
  method: 'POST',
  schema: z.object({
    orderId: z.string(),
    items: z.array(
      z.object({
        id: z.string(),
        quantity: z.number(),
      })
    ),
  }),
});
```

### Webhook Response Handling

```typescript
workflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  trigger: 'process-order', // Trigger specific step
  onSuccess: ctx => {
    console.log('✅ Webhook processed successfully');
  },
  onError: (ctx, error) => {
    console.error('❌ Webhook processing failed:', error);
  },
});
```

## Schedule Triggers

Schedule triggers execute workflows at specified times using cron expressions.

### Basic Schedule

```typescript
workflow.onSchedule('0 2 * * *'); // Run at 2 AM daily
```

### Common Schedule Patterns

```typescript
// Every minute
workflow.onSchedule('* * * * *');

// Every hour
workflow.onSchedule('0 * * * *');

// Every day at midnight
workflow.onSchedule('0 0 * * *');

// Every Monday at 9 AM
workflow.onSchedule('0 9 * * 1');

// Every 15 minutes
workflow.onSchedule('*/15 * * * *');

// First day of every month
workflow.onSchedule('0 0 1 * *');
```

### Interval-based Scheduling

```typescript
// Every 5 minutes
workflow.onInterval('5m');

// Every 2 hours
workflow.onInterval('2h');

// Every day
workflow.onInterval('1d');
```

### Schedule with Timezone

```typescript
workflow.onSchedule('0 9 * * 1', {
  timezone: 'America/New_York',
});
```

## Event Triggers

Event triggers allow workflows to respond to custom events published by your application.

### Basic Event Trigger

```typescript
workflow.onEvent('user.registered');
```

### Publishing Events

```typescript
import { cronflow } from 'cronflow';

// Publish an event
await cronflow.publishEvent('user.registered', {
  userId: '123',
  email: 'user@example.com',
  timestamp: new Date(),
});

// Publish with additional data
await cronflow.publishEvent('order.completed', {
  orderId: '456',
  amount: 99.99,
  customerId: '789',
});
```

### Event with Payload Validation

```typescript
workflow.onEvent('order.created', {
  schema: z.object({
    orderId: z.string(),
    customerId: z.string(),
    amount: z.number().positive(),
  }),
});
```

## Manual Triggers

Manual triggers allow workflows to be started programmatically.

### Basic Manual Trigger

```typescript
workflow.manual();
```

### Triggering Manually

```typescript
// Trigger the workflow
const runId = await cronflow.trigger('order-processor', {
  orderId: '123',
  customerId: '456',
  items: [
    { id: 'item1', quantity: 2 },
    { id: 'item2', quantity: 1 },
  ],
});

console.log('Workflow started with run ID:', runId);
```

## Polling Triggers

Polling triggers automatically check for new data and trigger workflows when conditions are met.

### Basic Polling

```typescript
workflow.onPoll(async ctx => {
  const newOrders = await db.orders.findMany({
    where: { status: 'pending' },
    take: 10,
  });

  return newOrders.map(order => ({
    id: order.id,
    payload: order,
  }));
});
```

### Polling with Conditions

```typescript
workflow.onPoll(async ctx => {
  const pendingOrders = await db.orders.findMany({
    where: {
      status: 'pending',
      createdAt: {
        gte: new Date(Date.now() - 24 * 60 * 60 * 1000), // Last 24 hours
      },
    },
  });

  if (pendingOrders.length === 0) {
    return []; // No triggers
  }

  return pendingOrders.map(order => ({
    id: `order_${order.id}`,
    payload: {
      orderId: order.id,
      customerId: order.customerId,
      amount: order.amount,
    },
  }));
});
```

## Advanced Trigger Configurations

### Multiple Triggers

Workflows can have multiple triggers:

```typescript
workflow
  .onWebhook('/webhooks/orders')
  .onSchedule('0 2 * * *') // Also run daily at 2 AM
  .onEvent('order.urgent')
  .manual();
```

### Trigger with Middleware

```typescript
workflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  middleware: [
    async (req, res, next) => {
      // Rate limiting
      const clientId = req.headers['x-client-id'];
      if (!clientId) {
        return res.status(401).json({ error: 'Missing client ID' });
      }
      next();
    },
    async (req, res, next) => {
      // Authentication
      const token = req.headers.authorization;
      if (!token || !isValidToken(token)) {
        return res.status(401).json({ error: 'Invalid token' });
      }
      next();
    },
  ],
});
```

### Conditional Triggers

```typescript
workflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  condition: req => {
    // Only trigger for orders above $100
    return req.body.amount > 100;
  },
});
```

## Trigger Validation

### Schema Validation

```typescript
import { z } from 'zod';

const orderSchema = z.object({
  orderId: z.string().uuid(),
  customerId: z.string(),
  items: z.array(
    z.object({
      id: z.string(),
      quantity: z.number().positive(),
      price: z.number().positive(),
    })
  ),
  shippingAddress: z.object({
    street: z.string(),
    city: z.string(),
    zipCode: z.string(),
  }),
});

workflow.onWebhook('/webhooks/orders', {
  schema: orderSchema,
});
```

### Custom Validation

```typescript
workflow.onWebhook('/webhooks/orders', {
  validate: payload => {
    if (payload.amount > 10000) {
      return 'Orders above $10,000 require manual review';
    }
    return true;
  },
});
```

## Trigger Error Handling

### Webhook Error Responses

```typescript
workflow.onWebhook('/webhooks/orders', {
  onError: (ctx, error) => {
    console.error('Webhook processing failed:', error);

    // Return custom error response
    return {
      status: 400,
      body: {
        error: 'Invalid order data',
        details: error.message,
      },
    };
  },
});
```

### Retry Configuration

```typescript
workflow.onWebhook('/webhooks/orders', {
  retry: {
    attempts: 3,
    backoff: { delay: '5s' },
  },
});
```

## Trigger Monitoring

### Trigger Statistics

```typescript
// Get trigger statistics
const stats = await cronflow.getTriggerStats();
console.log('Trigger stats:', stats);

// Get workflow triggers
const triggers = await cronflow.getWorkflowTriggers('order-processor');
console.log('Workflow triggers:', triggers);
```

### Schedule Triggers

```typescript
// Get all schedule triggers
const schedules = await cronflow.getScheduleTriggers();
console.log('Schedule triggers:', schedules);
```

## Best Practices

### 1. Use Descriptive Trigger Paths

```typescript
// ✅ Good: Descriptive paths
workflow.onWebhook('/webhooks/stripe/payment-succeeded');
workflow.onWebhook('/webhooks/shopify/order-created');

// ❌ Avoid: Generic paths
workflow.onWebhook('/webhook');
workflow.onWebhook('/api');
```

### 2. Validate Input Data

```typescript
workflow.onWebhook('/webhooks/orders', {
  schema: z.object({
    orderId: z.string().uuid(),
    amount: z.number().positive(),
    customerEmail: z.string().email(),
  }),
});
```

### 3. Handle Errors Gracefully

```typescript
workflow.onWebhook('/webhooks/orders', {
  onError: (ctx, error) => {
    console.error('Webhook error:', error);
    // Don't throw - return error response
    return {
      status: 400,
      body: { error: 'Invalid request' },
    };
  },
});
```

### 4. Use Appropriate HTTP Methods

```typescript
// For creating resources
workflow.onWebhook('/webhooks/orders', { method: 'POST' });

// For updates
workflow.onWebhook('/webhooks/orders/:id', { method: 'PUT' });

// For deletions
workflow.onWebhook('/webhooks/orders/:id', { method: 'DELETE' });
```

### 5. Implement Rate Limiting

```typescript
workflow.onWebhook('/webhooks/orders', {
  middleware: [
    rateLimit({
      windowMs: 15 * 60 * 1000, // 15 minutes
      max: 100, // limit each IP to 100 requests per windowMs
    }),
  ],
});
```

## API Reference

### `workflow.onWebhook(path, options?)`

Creates a webhook trigger.

**Parameters:**

- `path` (string): Webhook endpoint path
- `options` (WebhookOptions, optional): Webhook configuration

### `workflow.onSchedule(cronExpression, options?)`

Creates a schedule trigger.

**Parameters:**

- `cronExpression` (string): Cron expression
- `options` (ScheduleOptions, optional): Schedule configuration

### `workflow.onEvent(eventName, options?)`

Creates an event trigger.

**Parameters:**

- `eventName` (string): Event name
- `options` (EventOptions, optional): Event configuration

### `workflow.manual()`

Creates a manual trigger.

### `workflow.onPoll(pollFunction, options?)`

Creates a polling trigger.

**Parameters:**

- `pollFunction` (function): Polling function
- `options` (PollOptions, optional): Polling configuration

Triggers are the foundation of workflow automation in Cronflow. Choose the right trigger type for your use case and configure them properly to build robust, reliable workflows.
