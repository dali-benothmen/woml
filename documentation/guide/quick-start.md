# Quick Start

Get up and running with Cronflow in minutes! This guide will walk you through building your first workflow.

## 🚀 Your First Workflow

Let's create a simple workflow that processes user registrations with validation and welcome emails.

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Define the workflow
const userWorkflow = cronflow.define({
  id: 'user-registration',
  name: 'User Registration Workflow',
  description: 'Handles new user registration with validation',
  tags: ['auth', 'critical'],
  concurrency: 10,
  timeout: '2m',
  hooks: {
    onSuccess: ctx => {
      console.log(`✅ User registration completed: ${ctx.run.id}`);
    },
    onFailure: ctx => {
      console.error(`❌ Registration failed: ${ctx.error}`);
    },
  },
});

// Define webhook trigger
userWorkflow.onWebhook('/webhooks/user-signup', {
  schema: z.object({
    email: z.string().email(),
    name: z.string().min(1),
    company: z.string().optional(),
  }),
});

// Define workflow steps
userWorkflow
  .step('validate-user', async ctx => {
    const { email, name } = ctx.payload;

    // Check if user already exists
    const existingUser = await db.users.findUnique({
      where: { email },
    });

    if (existingUser) {
      throw new Error('User already exists');
    }

    return { email, name, validated: true };
  })
  .step('create-user', async ctx => {
    const user = await db.users.create({
      data: {
        email: ctx.last.email,
        name: ctx.last.name,
        createdAt: new Date(),
      },
    });

    return { user, created: true };
  })
  .action('send-welcome-email', async ctx => {
    await emailService.sendWelcomeEmail(ctx.last.user.email);
  });

// Start the engine
await cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 3000,
    maxConnections: 1000,
  },
});

console.log('🚀 Cronflow engine started on port 3000');
```

## 🔄 Adding Conditions

Let's add conditional logic to handle different user types:

```typescript
const orderWorkflow = cronflow.define({
  id: 'order-processing',
  name: 'Order Processing Workflow',
});

orderWorkflow
  .onWebhook('/webhooks/orders')
  .step('fetch-order', async ctx => {
    const order = await db.orders.findUnique({
      where: { id: ctx.payload.orderId },
    });
    return order;
  })
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .step('send-vip-notification', async ctx => {
    await slack.sendMessage(
      '#vip-orders',
      `VIP Order: ${ctx.last.id} - $${ctx.last.amount}`
    );
    return { vipNotified: true };
  })
  .elseIf('is-medium-value', ctx => ctx.last.amount > 100)
  .step('send-standard-notification', async ctx => {
    await slack.sendMessage(
      '#orders',
      `Order: ${ctx.last.id} - $${ctx.last.amount}`
    );
    return { standardNotified: true };
  })
  .else()
  .step('log-basic-order', async ctx => {
    console.log(`Basic order: ${ctx.last.id}`);
    return { logged: true };
  })
  .endIf();
```

## ⚡ Parallel Execution

Run multiple operations simultaneously for better performance:

```typescript
const dataProcessingWorkflow = cronflow.define({
  id: 'data-processing',
  name: 'Data Processing Workflow',
});

dataProcessingWorkflow
  .onWebhook('/webhooks/data-sync')
  .parallel([
    async ctx => {
      // Fetch user data
      const users = await db.users.findMany();
      return { users: users.length };
    },
    async ctx => {
      // Fetch order data
      const orders = await db.orders.findMany();
      return { orders: orders.length };
    },
    async ctx => {
      // Fetch product data
      const products = await db.products.findMany();
      return { products: products.length };
    },
  ])
  .step('generate-report', async ctx => {
    // ctx.last contains array of results from parallel steps
    const [userResult, orderResult, productResult] = ctx.last;

    const report = {
      totalUsers: userResult.users,
      totalOrders: orderResult.orders,
      totalProducts: productResult.products,
      generatedAt: new Date().toISOString(),
    };

    return report;
  });
```

## 🛡️ Error Handling

Add robust error handling with retries and fallbacks:

```typescript
const paymentWorkflow = cronflow.define({
  id: 'payment-processing',
  name: 'Payment Processing Workflow',
});

paymentWorkflow
  .onWebhook('/webhooks/payments')
  .step('process-payment', async ctx => {
    try {
      const payment = await stripe.charges.create({
        amount: ctx.payload.amount * 100,
        currency: 'usd',
        source: ctx.payload.paymentToken,
      });

      return { paymentId: payment.id, status: 'succeeded' };
    } catch (error) {
      throw new Error(`Payment failed: ${error.message}`);
    }
  })
  .retry({
    attempts: 3,
    backoff: { strategy: 'exponential', delay: '2s' },
  })
  .onError(ctx => {
    // Custom error handling
    console.error('Payment processing failed:', ctx.error);

    // Send alert to team
    slack.sendMessage(
      '#alerts',
      `Payment failed for order ${ctx.payload.orderId}: ${ctx.error}`
    );

    // Return fallback response
    return { status: 'failed', fallback: true };
  })
  .step('send-confirmation', async ctx => {
    if (ctx.last.status === 'succeeded') {
      await emailService.sendPaymentConfirmation(ctx.payload.customerEmail);
      return { confirmationSent: true };
    } else {
      await emailService.sendPaymentFailure(ctx.payload.customerEmail);
      return { failureNotified: true };
    }
  });
```

## 📊 Advanced Example

Here's a more complex workflow that demonstrates multiple features:

```typescript
const ecommerceWorkflow = cronflow.define({
  id: 'ecommerce-order-processing',
  name: 'E-commerce Order Processing',
  description: 'Complete order processing with inventory management',
  tags: ['ecommerce', 'critical'],
  concurrency: 20,
  timeout: '10m',
  hooks: {
    onSuccess: ctx => {
      console.log(`✅ Order ${ctx.payload.orderId} processed successfully`);
    },
    onFailure: ctx => {
      console.error(`❌ Order ${ctx.payload.orderId} failed:`, ctx.error);
    },
  },
});

ecommerceWorkflow
  .onWebhook('/webhooks/orders/create', {
    schema: z.object({
      orderId: z.string().uuid(),
      customerEmail: z.string().email(),
      items: z.array(
        z.object({
          productId: z.string(),
          quantity: z.number().positive(),
        })
      ),
      totalAmount: z.number().positive(),
    }),
  })
  .step('validate-order', async ctx => {
    const { orderId, items, totalAmount } = ctx.payload;

    // Check if order already exists
    const existingOrder = await db.orders.findUnique({
      where: { id: orderId },
    });

    if (existingOrder) {
      throw new Error('Order already exists');
    }

    // Validate items
    for (const item of items) {
      const product = await db.products.findUnique({
        where: { id: item.productId },
      });

      if (!product) {
        throw new Error(`Product ${item.productId} not found`);
      }
    }

    return { validated: true, items, totalAmount };
  })
  .parallel([
    async ctx => {
      // Check inventory
      const inventoryChecks = await Promise.all(
        ctx.last.items.map(async item => {
          const product = await db.products.findUnique({
            where: { id: item.productId },
          });
          return {
            productId: item.productId,
            available: product.stock >= item.quantity,
            stock: product.stock,
          };
        })
      );

      return { inventoryChecks };
    },
    async ctx => {
      // Process payment
      const payment = await stripe.charges.create({
        amount: ctx.last.totalAmount * 100,
        currency: 'usd',
        source: ctx.payload.paymentToken,
        metadata: { orderId: ctx.payload.orderId },
      });

      return { paymentId: payment.id, charged: true };
    },
  ])
  .if('all-items-available', ctx => {
    const [inventoryResult] = ctx.last;
    return inventoryResult.inventoryChecks.every(check => check.available);
  })
  .step('create-order', async ctx => {
    const order = await db.orders.create({
      data: {
        id: ctx.payload.orderId,
        customerEmail: ctx.payload.customerEmail,
        totalAmount: ctx.last.totalAmount,
        status: 'confirmed',
        createdAt: new Date(),
      },
    });

    return { order, created: true };
  })
  .step('update-inventory', async ctx => {
    await Promise.all(
      ctx.steps['validate-order'].output.items.map(async item => {
        await db.products.update({
          where: { id: item.productId },
          data: { stock: { decrement: item.quantity } },
        });
      })
    );

    return { inventoryUpdated: true };
  })
  .action('send-confirmation', async ctx => {
    await emailService.sendOrderConfirmation(
      ctx.payload.customerEmail,
      ctx.steps['create-order'].output.order
    );
  })
  .else()
  .step('handle-out-of-stock', async ctx => {
    const [inventoryResult] = ctx.last;
    const outOfStockItems = inventoryResult.inventoryChecks
      .filter(check => !check.available)
      .map(check => check.productId);

    await emailService.sendOutOfStockNotification(
      ctx.payload.customerEmail,
      outOfStockItems
    );

    return { outOfStock: true, items: outOfStockItems };
  })
  .endIf();
```

## 🎯 What's Next?

Now that you've built your first workflow, explore these topics:

- **[Basic Concepts](/guide/basic-concepts)** - Learn about workflows, steps, and context
- **[Workflows](/guide/workflows)** - Deep dive into workflow configuration
- **[Steps](/guide/steps)** - Master step creation and configuration
- **[Conditions](/guide/conditions)** - Add dynamic logic to your workflows
- **[Error Handling](/guide/error-handling)** - Build robust, fault-tolerant workflows
- **[API Reference](/api/)** - Complete API documentation
- **[Examples](/examples/)** - Real-world workflow examples
