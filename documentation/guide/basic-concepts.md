# Basic Concepts

Learn the fundamental building blocks of Cronflow workflows.

## Define

The `cronflow.define()` method creates a new workflow with its configuration and settings.

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'my-workflow',
  name: 'My First Workflow',
  description: 'A simple workflow example',
  tags: ['example', 'basic'],
  concurrency: 5,
  timeout: '5m',
  hooks: {
    onSuccess: ctx => {
      console.log('✅ Workflow completed successfully');
    },
    onFailure: ctx => {
      console.error('❌ Workflow failed:', ctx.error);
    },
  },
});
```

### Configuration Options

| Option        | Type     | Default | Description                                   |
| ------------- | -------- | ------- | --------------------------------------------- |
| `id`          | string   | -       | Unique identifier for the workflow (required) |
| `name`        | string   | -       | Human-readable name                           |
| `description` | string   | -       | Detailed description                          |
| `tags`        | string[] | []      | Tags for organization                         |
| `concurrency` | number   | ∞       | Max concurrent runs                           |
| `timeout`     | string   | '30m'   | Max execution time                            |
| `hooks`       | object   | -       | Success/failure callbacks                     |

## Step

Steps are the primary units of work that produce output and can be referenced by other steps.

```typescript
workflow
  .step('fetch-data', async ctx => {
    // Fetch data from an API
    const response = await fetch('https://api.example.com/data');
    const data = await response.json();

    return { items: data, count: data.length };
  })
  .step('process-data', async ctx => {
    // Access previous step output
    const { items, count } = ctx.steps['fetch-data'].output;

    // Process the data
    const processed = items.map(item => ({
      ...item,
      processed: true,
      timestamp: new Date().toISOString(),
    }));

    return { processed, totalProcessed: processed.length };
  });
```

### Step Features

- **Output Storage**: Each step's return value is stored and accessible
- **Error Handling**: Steps can throw errors to trigger failure handling
- **Context Access**: Access to payload, previous steps, and workflow state
- **Async Support**: Full support for async/await operations

## Actions

Actions are similar to steps but their output is ignored. Use them for side effects like sending emails or logging.

```typescript
workflow
  .step('create-user', async ctx => {
    const user = await db.users.create({
      data: { email: ctx.payload.email, name: ctx.payload.name },
    });
    return user;
  })
  .action('send-welcome-email', async ctx => {
    // Send email (output ignored)
    await emailService.sendWelcomeEmail(ctx.last.email);
  })
  .action('log-creation', async ctx => {
    // Log to console (output ignored)
    console.log(`User created: ${ctx.last.email}`);
  });
```

### When to Use Actions

- **Side Effects**: Sending emails, notifications, logging
- **Database Updates**: When you don't need the result
- **External API Calls**: When response isn't needed for next steps
- **Cleanup Operations**: Finalizing processes

## Conditions

Conditions allow you to create dynamic workflows that branch based on data or logic.

```typescript
workflow
  .step('fetch-order', async ctx => {
    const order = await db.orders.findUnique({
      where: { id: ctx.payload.orderId },
    });
    return order;
  })
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .step('send-vip-notification', async ctx => {
    await slack.sendMessage('#vip', `VIP Order: $${ctx.last.amount}`);
    return { vipNotified: true };
  })
  .elseIf('is-medium-value', ctx => ctx.last.amount > 100)
  .step('send-standard-notification', async ctx => {
    await slack.sendMessage('#orders', `Order: $${ctx.last.amount}`);
    return { standardNotified: true };
  })
  .else()
  .step('log-basic-order', async ctx => {
    console.log(`Basic order: ${ctx.last.id}`);
    return { logged: true };
  })
  .endIf();
```

### Condition Types

- **`.if()`**: Primary condition
- **`.elseIf()`**: Alternative conditions
- **`.else()`**: Default case
- **`.endIf()`**: Required to close condition blocks

## Start

The `cronflow.start()` method initializes the engine and begins listening for triggers.

```typescript
// Basic startup
await cronflow.start();

// With custom configuration
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 3000,
    maxConnections: 1000,
  },
});

console.log('🚀 Cronflow engine started');
```

### Startup Options

| Option           | Type   | Default     | Description                |
| ---------------- | ------ | ----------- | -------------------------- |
| `host`           | string | '127.0.0.1' | Webhook server host        |
| `port`           | number | 3000        | Webhook server port        |
| `maxConnections` | number | 1000        | Max concurrent connections |

## Triggers

Triggers define how workflows are activated. Here are the basic trigger types:

### Webhook Trigger

```typescript
workflow.onWebhook('/webhooks/orders', {
  method: 'POST',
  schema: z.object({
    orderId: z.string(),
    amount: z.number().positive(),
  }),
});
```

### Schedule Trigger

```typescript
// Run daily at 2 AM
workflow.onSchedule('0 2 * * *');

// Run every 15 minutes
workflow.onInterval('15m');
```

### Event Trigger

```typescript
// Trigger on custom events
workflow.onEvent('user.registered');

// Publish events from other workflows
await cronflow.publishEvent('user.registered', { userId: '123' });
```

### Manual Trigger

```typescript
// Trigger workflow programmatically
await cronflow.trigger('my-workflow', {
  data: 'some payload',
});
```

## Complete Example

Here's a complete workflow that demonstrates all basic concepts:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Define the workflow
const userWorkflow = cronflow.define({
  id: 'user-onboarding',
  name: 'User Onboarding Workflow',
  description: 'Handles new user registration and setup',
  tags: ['auth', 'onboarding'],
  concurrency: 10,
  timeout: '5m',
  hooks: {
    onSuccess: ctx => {
      console.log(`✅ User ${ctx.payload.email} onboarded successfully`);
    },
    onFailure: ctx => {
      console.error(
        `❌ Onboarding failed for ${ctx.payload.email}:`,
        ctx.error
      );
    },
  },
});

// Define webhook trigger
userWorkflow.onWebhook('/webhooks/user-signup', {
  schema: z.object({
    email: z.string().email(),
    name: z.string().min(1),
    plan: z.enum(['free', 'pro', 'enterprise']),
  }),
});

// Define workflow steps
userWorkflow
  .step('validate-user', async ctx => {
    const { email, name, plan } = ctx.payload;

    // Check if user exists
    const existingUser = await db.users.findUnique({ where: { email } });
    if (existingUser) {
      throw new Error('User already exists');
    }

    return { email, name, plan, validated: true };
  })
  .step('create-user', async ctx => {
    const user = await db.users.create({
      data: {
        email: ctx.last.email,
        name: ctx.last.name,
        plan: ctx.last.plan,
        createdAt: new Date(),
      },
    });

    return { user, created: true };
  })
  .if('is-enterprise', ctx => ctx.last.user.plan === 'enterprise')
  .step('setup-enterprise', async ctx => {
    // Enterprise-specific setup
    await enterpriseService.setupAccount(ctx.last.user.id);
    return { enterpriseSetup: true };
  })
  .elseIf('is-pro', ctx => ctx.last.user.plan === 'pro')
  .step('setup-pro', async ctx => {
    // Pro-specific setup
    await proService.setupAccount(ctx.last.user.id);
    return { proSetup: true };
  })
  .else()
  .step('setup-free', async ctx => {
    // Free tier setup
    await freeService.setupAccount(ctx.last.user.id);
    return { freeSetup: true };
  })
  .endIf()
  .action('send-welcome-email', async ctx => {
    await emailService.sendWelcomeEmail(
      ctx.steps['create-user'].output.user.email
    );
  })
  .action('log-onboarding', async ctx => {
    console.log(`User ${ctx.steps['create-user'].output.user.email} onboarded`);
  });

// Start the engine
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 3000,
  },
});

console.log('🚀 User onboarding workflow ready');
```

## Context Object

The `ctx` object provides access to workflow data and utilities:

```typescript
async function stepHandler(ctx) {
  // Access trigger payload
  const { userId, data } = ctx.payload;

  // Access previous step outputs
  const userData = ctx.steps['fetch-user'].output;

  // Access run metadata
  console.log(`Run ID: ${ctx.run.id}`);

  // Use persistent state
  await ctx.state.set('last-processed', Date.now());
  const lastProcessed = await ctx.state.get('last-processed');

  // Convenience property for previous step
  const previousResult = ctx.last;

  return { processed: true };
}
```

## What's Next?

Now that you understand the basic concepts, explore:

- **[Quick Start](/guide/quick-start)** - Build your first workflow
- **[Workflows](/guide/workflows)** - Advanced workflow configuration
- **[Steps](/guide/steps)** - Detailed step configuration
- **[Conditions](/guide/conditions)** - Advanced conditional logic
- **[API Reference](/api/)** - Complete API documentation
