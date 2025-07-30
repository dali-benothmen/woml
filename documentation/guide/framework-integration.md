# Framework Integration

Cronflow seamlessly integrates with popular Node.js frameworks, allowing you to use your existing web server infrastructure for webhook handling and workflow management.

## Supported Frameworks

Cronflow supports the following frameworks:

- **Express.js** - Most popular Node.js web framework
- **Fastify** - High-performance web framework
- **Koa** - Lightweight and expressive framework
- **Hapi** - Enterprise-grade framework
- **NestJS** - Progressive Node.js framework
- **Bun** - All-in-one JavaScript runtime
- **Next.js** - React framework with API routes

## Express.js Integration

### Basic Integration

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();
app.use(express.json());

// Define workflow with Express integration
const orderWorkflow = cronflow.define({
  id: 'order-processing',
  name: 'Order Processing Workflow',
});

orderWorkflow
  .onWebhook('/webhooks/orders', {
    method: 'POST',
    app: 'express',
    appInstance: app,
    schema: z.object({
      orderId: z.string(),
      amount: z.number().positive(),
      customerId: z.string(),
    }),
  })
  .step('validate-order', async ctx => {
    return await validateOrder(ctx.payload);
  })
  .step('process-payment', async ctx => {
    return await processPayment(ctx.last);
  });

// Start the server
app.listen(3000, () => {
  console.log('Server running on port 3000');
});

// Start Cronflow engine
await cronflow.start();
```

### Advanced Express Integration

```typescript
import express from 'express';
import { cronflow } from 'cronflow';
import { z } from 'zod';

const app = express();

// Middleware
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

// Authentication middleware
const authenticateWebhook = (req: any, res: any, next: any) => {
  const token = req.headers['x-webhook-token'];
  if (token !== process.env.WEBHOOK_TOKEN) {
    return res.status(401).json({ error: 'Unauthorized' });
  }
  next();
};

// Define workflow with advanced options
const paymentWorkflow = cronflow.define({
  id: 'payment-processing',
  name: 'Payment Processing Workflow',
});

paymentWorkflow
  .onWebhook('/webhooks/stripe', {
    method: 'POST',
    app: 'express',
    appInstance: app,
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
    headers: {
      required: {
        'stripe-signature': process.env.STRIPE_SIGNATURE,
      },
      validate: headers => {
        // Custom header validation
        return headers['content-type'] === 'application/json';
      },
    },
    parseRawBody: true, // For Stripe signature validation
  })
  .step('validate-stripe-signature', async ctx => {
    // Validate Stripe signature
    return { validated: true };
  })
  .step('process-payment', async ctx => {
    return await processStripePayment(ctx.payload);
  });

app.listen(3000);
await cronflow.start();
```

## Fastify Integration

### Basic Fastify Setup

```typescript
import Fastify from 'fastify';
import { cronflow } from 'cronflow';

const fastify = Fastify({
  logger: true,
});

// Define workflow with Fastify
const userWorkflow = cronflow.define({
  id: 'user-registration',
  name: 'User Registration Workflow',
});

userWorkflow
  .onWebhook('/webhooks/users', {
    method: 'POST',
    app: 'fastify',
    appInstance: fastify,
    schema: z.object({
      email: z.string().email(),
      name: z.string().min(1),
      plan: z.enum(['free', 'pro', 'enterprise']),
    }),
  })
  .step('create-user', async ctx => {
    return await createUser(ctx.payload);
  })
  .step('send-welcome-email', async ctx => {
    return await sendWelcomeEmail(ctx.last.email);
  });

// Start Fastify server
await fastify.listen({ port: 3000 });
await cronflow.start();
```

### Advanced Fastify Features

```typescript
import Fastify from 'fastify';
import { cronflow } from 'cronflow';

const fastify = Fastify({
  logger: true,
  trustProxy: true,
});

// Add plugins
await fastify.register(require('@fastify/cors'));
await fastify.register(require('@fastify/helmet'));

// Custom webhook handler with Fastify
const analyticsWorkflow = cronflow.define({
  id: 'analytics-processing',
  name: 'Analytics Processing Workflow',
});

analyticsWorkflow
  .onWebhook('/webhooks/analytics', {
    method: 'POST',
    app: 'fastify',
    appInstance: fastify,
    schema: z.object({
      event: z.string(),
      userId: z.string(),
      properties: z.record(z.any()),
    }),
    headers: {
      required: {
        'x-api-key': process.env.ANALYTICS_API_KEY,
      },
    },
  })
  .step('process-analytics', async ctx => {
    return await processAnalyticsEvent(ctx.payload);
  });

await fastify.listen({ port: 3000, host: '0.0.0.0' });
await cronflow.start();
```

## Koa Integration

### Basic Koa Setup

```typescript
import Koa from 'koa';
import Router from '@koa/router';
import bodyParser from 'koa-bodyparser';
import { cronflow } from 'cronflow';

const app = new Koa();
const router = new Router();

app.use(bodyParser());

// Define workflow with Koa
const notificationWorkflow = cronflow.define({
  id: 'notification-sending',
  name: 'Notification Sending Workflow',
});

notificationWorkflow
  .onWebhook('/webhooks/notifications', {
    method: 'POST',
    app: 'koa',
    appInstance: app,
    schema: z.object({
      type: z.enum(['email', 'sms', 'push']),
      recipient: z.string(),
      message: z.string(),
    }),
  })
  .step('send-notification', async ctx => {
    return await sendNotification(ctx.payload);
  });

app.use(router.routes());
app.listen(3000);
await cronflow.start();
```

## Hapi Integration

### Basic Hapi Setup

```typescript
import Hapi from '@hapi/hapi';
import { cronflow } from 'cronflow';

const server = Hapi.server({
  port: 3000,
  host: 'localhost',
});

// Define workflow with Hapi
const inventoryWorkflow = cronflow.define({
  id: 'inventory-update',
  name: 'Inventory Update Workflow',
});

inventoryWorkflow
  .onWebhook('/webhooks/inventory', {
    method: 'POST',
    app: 'hapi',
    appInstance: server,
    schema: z.object({
      productId: z.string(),
      quantity: z.number().int().positive(),
      operation: z.enum(['add', 'subtract']),
    }),
  })
  .step('update-inventory', async ctx => {
    return await updateInventory(ctx.payload);
  });

await server.start();
await cronflow.start();
```

## NestJS Integration

### Basic NestJS Setup

```typescript
import { NestFactory } from '@nestjs/core';
import { AppModule } from './app.module';
import { cronflow } from 'cronflow';

async function bootstrap() {
  const app = await NestFactory.create(AppModule);

  // Define workflow with NestJS
  const orderWorkflow = cronflow.define({
    id: 'order-management',
    name: 'Order Management Workflow',
  });

  orderWorkflow
    .onWebhook('/webhooks/orders', {
      method: 'POST',
      app: 'nestjs',
      appInstance: app,
      schema: z.object({
        orderId: z.string(),
        items: z.array(
          z.object({
            productId: z.string(),
            quantity: z.number(),
          })
        ),
      }),
    })
    .step('process-order', async ctx => {
      return await processOrder(ctx.payload);
    });

  await app.listen(3000);
  await cronflow.start();
}

bootstrap();
```

## Bun Integration

### Basic Bun Setup

```typescript
import { cronflow } from 'cronflow';

// Define workflow with Bun
const dataProcessingWorkflow = cronflow.define({
  id: 'data-processing',
  name: 'Data Processing Workflow',
});

dataProcessingWorkflow
  .onWebhook('/webhooks/data', {
    method: 'POST',
    app: 'bun',
    appInstance: null, // Bun doesn't need app instance
    schema: z.object({
      data: z.array(z.any()),
      operation: z.string(),
    }),
  })
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  });

// Bun automatically handles the server
await cronflow.start();
```

## Next.js Integration

### API Routes Integration

```typescript
// pages/api/webhooks/orders.ts
import { NextApiRequest, NextApiResponse } from 'next';
import { cronflow } from 'cronflow';

const orderWorkflow = cronflow.define({
  id: 'order-processing',
  name: 'Order Processing Workflow',
});

orderWorkflow
  .onWebhook('/api/webhooks/orders', {
    method: 'POST',
    app: 'nextjs',
    appInstance: null,
    schema: z.object({
      orderId: z.string(),
      amount: z.number(),
    }),
  })
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  });

export default async function handler(
  req: NextApiRequest,
  res: NextApiResponse
) {
  if (req.method !== 'POST') {
    return res.status(405).json({ error: 'Method not allowed' });
  }

  try {
    const runId = await cronflow.trigger('order-processing', req.body);
    res.status(200).json({ success: true, runId });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
}

await cronflow.start();
```

## Advanced Framework Features

### 1. Custom Route Registration

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();

// Custom route registration
const customWorkflow = cronflow.define({
  id: 'custom-workflow',
  name: 'Custom Workflow',
});

customWorkflow
  .onWebhook('/custom-endpoint', {
    method: 'POST',
    registerRoute: (method, path, handler) => {
      // Custom route registration logic
      app[method.toLowerCase()](path, async (req, res) => {
        try {
          await handler(req, res);
        } catch (error) {
          res.status(500).json({ error: error.message });
        }
      });
    },
  })
  .step('custom-step', async ctx => {
    return { processed: true };
  });
```

### 2. Framework-Specific Middleware

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();

// Express-specific middleware
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

// Custom middleware for webhooks
const webhookMiddleware = (req: any, res: any, next: any) => {
  // Add webhook-specific headers
  req.headers['x-webhook-source'] = 'cronflow';
  next();
};

const workflow = cronflow.define({
  id: 'middleware-workflow',
  name: 'Middleware Workflow',
});

workflow
  .onWebhook('/webhooks/with-middleware', {
    method: 'POST',
    app: 'express',
    appInstance: app,
    // Custom middleware can be added here
  })
  .step('process-with-middleware', async ctx => {
    return { processed: true };
  });
```

### 3. Error Handling

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();

// Global error handler
app.use((error: any, req: any, res: any, next: any) => {
  console.error('Global error:', error);
  res.status(500).json({ error: 'Internal server error' });
});

const errorHandlingWorkflow = cronflow.define({
  id: 'error-handling-workflow',
  name: 'Error Handling Workflow',
});

errorHandlingWorkflow
  .onWebhook('/webhooks/error-test', {
    method: 'POST',
    app: 'express',
    appInstance: app,
    schema: z.object({
      shouldFail: z.boolean(),
    }),
  })
  .step('test-error', async ctx => {
    if (ctx.payload.shouldFail) {
      throw new Error('Intentional error for testing');
    }
    return { success: true };
  })
  .onError(ctx => {
    // Custom error handling
    console.error('Workflow error:', ctx.error);
    return { error: ctx.error.message, handled: true };
  });
```

## Best Practices

### 1. Framework-Specific Configuration

```typescript
// Express configuration
const expressConfig = {
  app: 'express',
  appInstance: expressApp,
  method: 'POST',
  schema: z.object({
    // Your schema
  }),
};

// Fastify configuration
const fastifyConfig = {
  app: 'fastify',
  appInstance: fastifyApp,
  method: 'POST',
  schema: z.object({
    // Your schema
  }),
};

// Use the appropriate config based on your framework
workflow.onWebhook('/webhooks/data', expressConfig);
```

### 2. Environment-Specific Setup

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();

// Development setup
if (process.env.NODE_ENV === 'development') {
  app.use((req, res, next) => {
    console.log(`${req.method} ${req.path}`);
    next();
  });
}

// Production setup
if (process.env.NODE_ENV === 'production') {
  app.use(require('helmet')());
  app.use(require('compression')());
}

const workflow = cronflow.define({
  id: 'environment-aware-workflow',
  name: 'Environment Aware Workflow',
});

workflow
  .onWebhook('/webhooks/data', {
    method: 'POST',
    app: 'express',
    appInstance: app,
  })
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  });
```

### 3. Security Considerations

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();

// Security middleware
app.use(require('helmet')());
app.use(
  require('rate-limiter-flexible').rateLimit({
    windowMs: 15 * 60 * 1000, // 15 minutes
    max: 100, // limit each IP to 100 requests per windowMs
  })
);

const secureWorkflow = cronflow.define({
  id: 'secure-workflow',
  name: 'Secure Workflow',
});

secureWorkflow
  .onWebhook('/webhooks/secure', {
    method: 'POST',
    app: 'express',
    appInstance: app,
    headers: {
      required: {
        'x-api-key': process.env.API_KEY,
        'x-signature': process.env.WEBHOOK_SIGNATURE,
      },
    },
  })
  .step('secure-processing', async ctx => {
    return await secureProcessing(ctx.payload);
  });
```

## What's Next?

- **[Advanced Triggers](/guide/advanced-triggers)** - Advanced webhook and trigger configurations
- **[State Management](/guide/state-management)** - Manage workflow state efficiently
- **[Testing](/guide/testing)** - Test framework integrations
- **[Background Actions](/guide/background-actions)** - How actions run in background
