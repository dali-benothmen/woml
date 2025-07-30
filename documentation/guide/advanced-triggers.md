# Advanced Triggers

Cronflow provides advanced trigger configurations for webhooks, schedules, events, and manual triggers with comprehensive validation and security features.

## Advanced Webhook Triggers

### Schema Validation

Use Zod schemas to validate incoming webhook payloads:

```typescript
import { z } from 'zod';
import { cronflow } from 'cronflow';

const orderWorkflow = cronflow.define({
  id: 'order-processing',
  name: 'Order Processing Workflow',
});

orderWorkflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  schema: z.object({
    orderId: z.string().uuid(),
    customerId: z.string().uuid(),
    amount: z.number().positive(),
    currency: z.enum(['USD', 'EUR', 'GBP']),
    items: z
      .array(
        z.object({
          productId: z.string(),
          quantity: z.number().int().positive(),
          price: z.number().positive(),
        })
      )
      .min(1),
    metadata: z.record(z.any()).optional(),
  }),
});
```

### Header Validation

Validate required headers and custom header logic:

```typescript
orderWorkflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  headers: {
    required: {
      'x-api-key': process.env.API_KEY,
      'x-signature': process.env.WEBHOOK_SIGNATURE,
      'content-type': 'application/json',
    },
    validate: headers => {
      // Custom header validation logic
      const timestamp = headers['x-timestamp'];
      const currentTime = Date.now();

      if (!timestamp || currentTime - parseInt(timestamp) > 300000) {
        return 'Request timestamp is too old or missing';
      }

      return true; // Validation passed
    },
  },
});
```

### Idempotency Keys

Prevent duplicate processing with idempotency keys:

```typescript
orderWorkflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  idempotencyKey: ctx => {
    // Extract idempotency key from headers or body
    return (
      ctx.trigger.headers['x-idempotency-key'] ||
      ctx.payload.idempotencyKey ||
      `${ctx.payload.orderId}-${Date.now()}`
    );
  },
});
```

### Raw Body Parsing

Parse raw body for signature validation:

```typescript
orderWorkflow.onWebhook('/webhooks/stripe', {
  method: 'POST',
  parseRawBody: true, // For Stripe signature validation
  schema: z.object({
    type: z.string(),
    data: z.object({
      object: z.object({
        id: z.string(),
        amount: z.number(),
        currency: z.string(),
      }),
    }),
  }),
});
```

## Advanced Schedule Triggers

### Complex Cron Expressions

```typescript
const reportWorkflow = cronflow.define({
  id: 'daily-report',
  name: 'Daily Report Generation',
});

// Run at 2 AM every day
reportWorkflow.onSchedule('0 2 * * *');

// Run every 15 minutes during business hours (9 AM - 5 PM)
reportWorkflow.onSchedule('*/15 9-17 * * 1-5');

// Run on the first day of every month at 3 AM
reportWorkflow.onSchedule('0 3 1 * *');

// Run every hour on weekdays
reportWorkflow.onSchedule('0 * * * 1-5');
```

### Interval-Based Scheduling

```typescript
// Run every 5 minutes
const healthCheckWorkflow = cronflow.define({
  id: 'health-check',
  name: 'Health Check Workflow',
});
healthCheckWorkflow.onInterval('5m');

// Run every 2 hours
const backupWorkflow = cronflow.define({
  id: 'backup-process',
  name: 'Backup Process Workflow',
});
backupWorkflow.onInterval('2h');

// Run every day
const dailyWorkflow = cronflow.define({
  id: 'daily-cleanup',
  name: 'Daily Cleanup Workflow',
});
dailyWorkflow.onInterval('1d');
```

## Advanced Event Triggers

### Custom Event Publishing

```typescript
// Publish events from workflows
const userWorkflow = cronflow.define({
  id: 'user-registration',
  name: 'User Registration Workflow',
});

userWorkflow
  .step('create-user', async ctx => {
    const user = await createUser(ctx.payload);
    return user;
  })
  .step('publish-event', async ctx => {
    // Publish event for other workflows to consume
    await cronflow.publishEvent('user.registered', {
      userId: ctx.last.id,
      email: ctx.last.email,
      timestamp: new Date().toISOString(),
    });
    return { eventPublished: true };
  });

// Listen for events
const notificationWorkflow = cronflow.define({
  id: 'notification-sender',
  name: 'Notification Sender Workflow',
});

notificationWorkflow.onEvent('user.registered');
```

### Event-Driven Architecture

```typescript
// Order processing workflow
const orderWorkflow = cronflow.define({
  id: 'order-processing',
  name: 'Order Processing Workflow',
});

orderWorkflow
  .step('process-order', async ctx => {
    const order = await processOrder(ctx.payload);
    return order;
  })
  .step('publish-order-events', async ctx => {
    const order = ctx.last;

    // Publish multiple events
    await Promise.all([
      cronflow.publishEvent('order.created', {
        orderId: order.id,
        amount: order.amount,
        customerId: order.customerId,
      }),
      cronflow.publishEvent('inventory.updated', {
        productId: order.productId,
        quantity: order.quantity,
      }),
    ]);

    return { eventsPublished: true };
  });

// Inventory workflow listens for inventory events
const inventoryWorkflow = cronflow.define({
  id: 'inventory-management',
  name: 'Inventory Management Workflow',
});

inventoryWorkflow.onEvent('inventory.updated');
```

## Advanced Manual Triggers

### Programmatic Triggering

```typescript
// Manual trigger with custom payload
const manualWorkflow = cronflow.define({
  id: 'manual-process',
  name: 'Manual Process Workflow',
});

manualWorkflow.step('process-manual-request', async ctx => {
  return await processManualRequest(ctx.payload);
});

// Trigger from your application
const runId = await cronflow.trigger('manual-process', {
  userId: 'user_123',
  action: 'data-export',
  parameters: {
    dateRange: '2024-01-01:2024-12-31',
    format: 'csv',
  },
});

console.log(`Manual workflow triggered with run ID: ${runId}`);
```

### Trigger with Metadata

```typescript
const dataProcessingWorkflow = cronflow.define({
  id: 'data-processing',
  name: 'Data Processing Workflow',
});

dataProcessingWorkflow.step('process-data', async ctx => {
  console.log('Processing data with metadata:', ctx.payload.metadata);
  return await processData(ctx.payload.data);
});

// Trigger with rich metadata
await cronflow.trigger('data-processing', {
  data: largeDataset,
  metadata: {
    source: 'manual-upload',
    uploadedBy: 'user_123',
    timestamp: new Date().toISOString(),
    priority: 'high',
  },
});
```

## Advanced Webhook Configurations

### Framework Integration

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();

const webhookWorkflow = cronflow.define({
  id: 'webhook-processor',
  name: 'Webhook Processor Workflow',
});

webhookWorkflow.onWebhook('/webhooks/data', {
  method: 'POST',
  app: 'express',
  appInstance: app,
  schema: z.object({
    event: z.string(),
    data: z.any(),
  }),
  headers: {
    required: {
      'x-api-key': process.env.API_KEY,
    },
  },
});
```

### Custom Route Registration

```typescript
const customWorkflow = cronflow.define({
  id: 'custom-webhook',
  name: 'Custom Webhook Workflow',
});

customWorkflow.onWebhook('/custom-endpoint', {
  method: 'POST',
  registerRoute: (method, path, handler) => {
    // Custom route registration logic
    app[method.toLowerCase()](path, async (req, res) => {
      try {
        const result = await handler(req, res);
        res.json(result);
      } catch (error) {
        res.status(500).json({ error: error.message });
      }
    });
  },
});
```

### Step-Specific Triggers

```typescript
const stepTriggerWorkflow = cronflow.define({
  id: 'step-trigger-workflow',
  name: 'Step Trigger Workflow',
});

stepTriggerWorkflow.onWebhook('/webhooks/step-trigger', {
  method: 'POST',
  trigger: 'validate-step', // Trigger specific step
  schema: z.object({
    data: z.any(),
  }),
});

stepTriggerWorkflow
  .step('validate-step', async ctx => {
    return await validateData(ctx.payload.data);
  })
  .step('process-step', async ctx => {
    return await processData(ctx.last);
  });
```

## Security and Validation

### Signature Validation

```typescript
const secureWorkflow = cronflow.define({
  id: 'secure-webhook',
  name: 'Secure Webhook Workflow',
});

secureWorkflow.onWebhook('/webhooks/secure', {
  method: 'POST',
  parseRawBody: true,
  headers: {
    required: {
      'x-signature': process.env.WEBHOOK_SIGNATURE,
    },
    validate: headers => {
      const signature = headers['x-signature'];
      const timestamp = headers['x-timestamp'];

      // Validate signature
      const expectedSignature = generateSignature(timestamp);
      return signature === expectedSignature;
    },
  },
});
```

### Rate Limiting

```typescript
const rateLimitedWorkflow = cronflow.define({
  id: 'rate-limited-workflow',
  name: 'Rate Limited Workflow',
  rateLimit: {
    count: 100, // Maximum 100 executions
    per: '1h', // Per hour
  },
});

rateLimitedWorkflow.onWebhook('/webhooks/rate-limited', {
  method: 'POST',
  schema: z.object({
    data: z.any(),
  }),
});
```

### IP Whitelisting

```typescript
const whitelistedWorkflow = cronflow.define({
  id: 'whitelisted-webhook',
  name: 'Whitelisted Webhook Workflow',
});

whitelistedWorkflow.onWebhook('/webhooks/whitelisted', {
  method: 'POST',
  headers: {
    validate: headers => {
      const clientIP = headers['x-forwarded-for'] || headers['x-real-ip'];
      const allowedIPs = process.env.ALLOWED_IPS?.split(',') || [];

      return allowedIPs.includes(clientIP);
    },
  },
});
```

## Advanced Trigger Patterns

### Multi-Step Validation

```typescript
const multiStepWorkflow = cronflow.define({
  id: 'multi-step-validation',
  name: 'Multi-Step Validation Workflow',
});

multiStepWorkflow.onWebhook('/webhooks/multi-step', {
  method: 'POST',
  schema: z.object({
    userId: z.string(),
    action: z.enum(['create', 'update', 'delete']),
    data: z.any(),
  }),
  headers: {
    required: {
      'x-api-key': process.env.API_KEY,
      'x-user-id': process.env.USER_ID,
    },
  },
});

multiStepWorkflow
  .step('validate-user', async ctx => {
    const user = await validateUser(ctx.payload.userId);
    return user;
  })
  .step('validate-permissions', async ctx => {
    const hasPermission = await checkPermissions(
      ctx.last.id,
      ctx.payload.action
    );
    if (!hasPermission) {
      throw new Error('Insufficient permissions');
    }
    return { permissionsValidated: true };
  })
  .step('execute-action', async ctx => {
    return await executeAction(ctx.payload);
  });
```

### Conditional Triggers

```typescript
const conditionalWorkflow = cronflow.define({
  id: 'conditional-trigger',
  name: 'Conditional Trigger Workflow',
});

conditionalWorkflow.onWebhook('/webhooks/conditional', {
  method: 'POST',
  schema: z.object({
    type: z.enum(['urgent', 'normal', 'low']),
    data: z.any(),
  }),
});

conditionalWorkflow
  .if('is-urgent', ctx => ctx.payload.type === 'urgent')
  .step('urgent-processing', async ctx => {
    return await processUrgent(ctx.payload);
  })
  .elseIf('is-normal', ctx => ctx.payload.type === 'normal')
  .step('normal-processing', async ctx => {
    return await processNormal(ctx.payload);
  })
  .else()
  .step('low-priority-processing', async ctx => {
    return await processLowPriority(ctx.payload);
  })
  .endIf();
```

## Best Practices

### 1. Comprehensive Validation

```typescript
const validatedWorkflow = cronflow.define({
  id: 'comprehensive-validation',
  name: 'Comprehensive Validation Workflow',
});

validatedWorkflow.onWebhook('/webhooks/validated', {
  method: 'POST',
  schema: z.object({
    // Comprehensive schema validation
    userId: z.string().uuid(),
    action: z.enum(['create', 'read', 'update', 'delete']),
    resource: z.string(),
    data: z.any(),
    metadata: z
      .object({
        source: z.string(),
        timestamp: z.string().datetime(),
        version: z.string(),
      })
      .optional(),
  }),
  headers: {
    required: {
      'x-api-key': process.env.API_KEY,
      'x-signature': process.env.SIGNATURE,
      'content-type': 'application/json',
    },
    validate: headers => {
      // Additional header validation
      const timestamp = headers['x-timestamp'];
      const currentTime = Date.now();

      if (!timestamp || currentTime - parseInt(timestamp) > 300000) {
        return 'Request timestamp is too old or missing';
      }

      return true;
    },
  },
  idempotencyKey: ctx => {
    return `${ctx.payload.userId}-${ctx.payload.action}-${ctx.payload.resource}`;
  },
});
```

### 2. Error Handling

```typescript
const errorHandlingWorkflow = cronflow.define({
  id: 'error-handling-trigger',
  name: 'Error Handling Trigger Workflow',
});

errorHandlingWorkflow.onWebhook('/webhooks/error-handling', {
  method: 'POST',
  schema: z.object({
    data: z.any(),
  }),
});

errorHandlingWorkflow
  .step('process-data', async ctx => {
    try {
      return await processData(ctx.payload.data);
    } catch (error) {
      // Log error and return fallback
      console.error('Processing failed:', error);
      return { error: error.message, fallback: true };
    }
  })
  .onError(ctx => {
    // Handle any remaining errors
    console.error('Workflow error:', ctx.error);
    return { status: 'failed', error: ctx.error.message };
  });
```

### 3. Monitoring and Logging

```typescript
const monitoredWorkflow = cronflow.define({
  id: 'monitored-trigger',
  name: 'Monitored Trigger Workflow',
});

monitoredWorkflow.onWebhook('/webhooks/monitored', {
  method: 'POST',
  schema: z.object({
    data: z.any(),
  }),
});

monitoredWorkflow
  .step('log-trigger', async ctx => {
    console.log('Webhook triggered:', {
      timestamp: new Date().toISOString(),
      payload: ctx.payload,
      headers: ctx.trigger.headers,
    });
    return { logged: true };
  })
  .step('process-data', async ctx => {
    return await processData(ctx.payload.data);
  })
  .step('log-completion', async ctx => {
    console.log('Processing completed:', {
      timestamp: new Date().toISOString(),
      result: ctx.last,
    });
    return { completed: true };
  });
```

## What's Next?

- **[State Management](/guide/state-management)** - Manage workflow state efficiently
- **[Testing](/guide/testing)** - Test advanced trigger configurations
- **[Background Actions](/guide/background-actions)** - How actions run in background
- **[Human-in-the-Loop](/guide/human-in-the-loop)** - Manual approval workflows
