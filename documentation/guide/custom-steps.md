# Custom Steps

Custom steps allow you to create reusable, composable workflow components that encapsulate common functionality. They promote code reuse, maintainability, and consistency across your workflows.

## Creating Custom Steps

Custom steps are functions that return step handlers. They can accept configuration parameters and return a step function that follows the standard Cronflow step signature.

### Basic Custom Step

```typescript
import { Context } from 'cronflow';

// Define a custom step
function createEmailStep(config: {
  template: string;
  subject: string;
  to: string | ((ctx: Context) => string);
}) {
  return async (ctx: Context) => {
    const recipient =
      typeof config.to === 'function' ? config.to(ctx) : config.to;

    const emailData = {
      to: recipient,
      subject: config.subject,
      template: config.template,
      data: ctx.payload,
    };

    const result = await emailService.send(emailData);

    return {
      sent: true,
      messageId: result.messageId,
      recipient,
    };
  };
}

// Use the custom step
const workflow = cronflow.define({
  id: 'notification-workflow',
});

workflow.step(
  'send-welcome-email',
  createEmailStep({
    template: 'welcome',
    subject: 'Welcome to our platform!',
    to: ctx => ctx.payload.email,
  })
);
```

### Custom Step with Validation

```typescript
import { z } from 'zod';

function createOrderStep(config: {
  validateSchema?: z.ZodSchema;
  retryAttempts?: number;
  timeout?: string;
}) {
  return async (ctx: Context) => {
    // Validate input if schema provided
    if (config.validateSchema) {
      config.validateSchema.parse(ctx.payload);
    }

    const order = await orderService.create({
      customerId: ctx.payload.customerId,
      items: ctx.payload.items,
      total: ctx.payload.total,
    });

    return {
      orderId: order.id,
      status: order.status,
      createdAt: order.createdAt,
    };
  };
}

// Use with validation
const orderSchema = z.object({
  customerId: z.string(),
  items: z.array(
    z.object({
      id: z.string(),
      quantity: z.number().positive(),
    })
  ),
  total: z.number().positive(),
});

workflow.step(
  'create-order',
  createOrderStep({
    validateSchema: orderSchema,
    retryAttempts: 3,
    timeout: '30s',
  })
);
```

### Custom Step with Dependencies

```typescript
function createUserProfileStep(config: {
  includeOrders?: boolean;
  includePreferences?: boolean;
}) {
  return async (ctx: Context) => {
    const userId = ctx.payload.userId;

    const profile = await userService.getProfile(userId);

    const result: any = {
      id: profile.id,
      name: profile.name,
      email: profile.email,
    };

    if (config.includeOrders) {
      result.orders = await orderService.getUserOrders(userId);
    }

    if (config.includePreferences) {
      result.preferences = await preferenceService.getUserPreferences(userId);
    }

    return result;
  };
}

workflow.step(
  'get-user-profile',
  createUserProfileStep({
    includeOrders: true,
    includePreferences: true,
  })
);
```

## Advanced Custom Steps

### Custom Step with State Management

```typescript
function createCachedStep<T>(
  key: string,
  fetchFn: (ctx: Context) => Promise<T>,
  options: {
    ttl?: string;
    cacheKey?: (ctx: Context) => string;
  } = {}
) {
  return async (ctx: Context) => {
    const cacheKey = options.cacheKey
      ? options.cacheKey(ctx)
      : `${key}:${ctx.payload.id}`;

    // Try to get from cache
    const cached = await cronflow.getWorkflowState(
      ctx.workflow_id,
      cacheKey,
      null
    );

    if (cached) {
      return { ...cached, fromCache: true };
    }

    // Fetch fresh data
    const data = await fetchFn(ctx);

    // Cache the result
    await cronflow.setWorkflowState(
      ctx.workflow_id,
      cacheKey,
      data,
      options.ttl ? { ttl: options.ttl } : undefined
    );

    return { ...data, fromCache: false };
  };
}

// Use the cached step
workflow.step(
  'get-user-data',
  createCachedStep(
    'user-data',
    async ctx => await userService.getUser(ctx.payload.userId),
    { ttl: '1h' }
  )
);
```

### Custom Step with Error Handling

```typescript
function createResilientStep<T>(
  stepFn: (ctx: Context) => Promise<T>,
  options: {
    fallback?: (ctx: Context) => Promise<T>;
    maxRetries?: number;
    onError?: (error: Error, ctx: Context) => void;
  } = {}
) {
  return async (ctx: Context) => {
    let lastError: Error;

    for (let attempt = 1; attempt <= (options.maxRetries || 3); attempt++) {
      try {
        return await stepFn(ctx);
      } catch (error) {
        lastError = error as Error;

        if (options.onError) {
          options.onError(lastError, ctx);
        }

        if (attempt === options.maxRetries) {
          if (options.fallback) {
            return await options.fallback(ctx);
          }
          throw lastError;
        }

        // Wait before retry
        await new Promise(resolve => setTimeout(resolve, 1000 * attempt));
      }
    }

    throw lastError!;
  };
}

// Use the resilient step
workflow.step(
  'fetch-external-data',
  createResilientStep(async ctx => await externalApi.fetch(ctx.payload.id), {
    maxRetries: 3,
    fallback: async ctx => ({ id: ctx.payload.id, status: 'fallback' }),
    onError: (error, ctx) =>
      console.error(`Attempt failed for ${ctx.payload.id}:`, error),
  })
);
```

### Custom Step with Conditional Logic

```typescript
function createConditionalStep<T>(
  condition: (ctx: Context) => boolean | Promise<boolean>,
  trueStep: (ctx: Context) => Promise<T>,
  falseStep?: (ctx: Context) => Promise<T>
) {
  return async (ctx: Context) => {
    const shouldExecute = await condition(ctx);

    if (shouldExecute) {
      return await trueStep(ctx);
    } else if (falseStep) {
      return await falseStep(ctx);
    } else {
      return { skipped: true, reason: 'Condition not met' };
    }
  };
}

// Use the conditional step
workflow.step(
  'process-vip-order',
  createConditionalStep(
    ctx => ctx.payload.amount > 500,
    async ctx => await vipOrderService.process(ctx.payload),
    async ctx => await regularOrderService.process(ctx.payload)
  )
);
```

## Custom Step Libraries

### HTTP Request Step

```typescript
function createHttpStep(config: {
  method: 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH';
  url: string | ((ctx: Context) => string);
  headers?: Record<string, string> | ((ctx: Context) => Record<string, string>);
  body?: any | ((ctx: Context) => any);
  timeout?: number;
}) {
  return async (ctx: Context) => {
    const url = typeof config.url === 'function' ? config.url(ctx) : config.url;

    const headers =
      typeof config.headers === 'function'
        ? config.headers(ctx)
        : config.headers || {};

    const body =
      typeof config.body === 'function' ? config.body(ctx) : config.body;

    const response = await fetch(url, {
      method: config.method,
      headers: {
        'Content-Type': 'application/json',
        ...headers,
      },
      body: body ? JSON.stringify(body) : undefined,
      signal: config.timeout ? AbortSignal.timeout(config.timeout) : undefined,
    });

    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }

    return await response.json();
  };
}

// Use the HTTP step
workflow
  .step(
    'fetch-user-data',
    createHttpStep({
      method: 'GET',
      url: ctx => `https://api.example.com/users/${ctx.payload.userId}`,
      timeout: 5000,
    })
  )
  .step(
    'update-user',
    createHttpStep({
      method: 'PUT',
      url: ctx => `https://api.example.com/users/${ctx.payload.userId}`,
      body: ctx => ({ name: ctx.payload.name, email: ctx.payload.email }),
    })
  );
```

### Database Step

```typescript
function createDbStep<T>(
  operation: 'find' | 'create' | 'update' | 'delete',
  config: {
    table: string;
    where?: (ctx: Context) => any;
    data?: (ctx: Context) => any;
    select?: string[];
  }
) {
  return async (ctx: Context) => {
    const { table, where, data, select } = config;

    switch (operation) {
      case 'find':
        return await db[table].findMany({
          where: where ? where(ctx) : undefined,
          select: select ? { [select.join(',')]: true } : undefined,
        });

      case 'create':
        return await db[table].create({
          data: data ? data(ctx) : ctx.payload,
        });

      case 'update':
        return await db[table].update({
          where: where ? where(ctx) : { id: ctx.payload.id },
          data: data ? data(ctx) : ctx.payload,
        });

      case 'delete':
        return await db[table].delete({
          where: where ? where(ctx) : { id: ctx.payload.id },
        });

      default:
        throw new Error(`Unsupported operation: ${operation}`);
    }
  };
}

// Use the database step
workflow
  .step(
    'find-user',
    createDbStep('find', {
      table: 'users',
      where: ctx => ({ id: ctx.payload.userId }),
      select: ['id', 'name', 'email'],
    })
  )
  .step(
    'create-order',
    createDbStep('create', {
      table: 'orders',
      data: ctx => ({
        userId: ctx.payload.userId,
        items: ctx.payload.items,
        total: ctx.payload.total,
      }),
    })
  );
```

### Notification Step

```typescript
function createNotificationStep(config: {
  type: 'email' | 'slack' | 'sms' | 'webhook';
  template?: string;
  channel?: string;
  recipients: string | string[] | ((ctx: Context) => string | string[]);
  message: string | ((ctx: Context) => string);
}) {
  return async (ctx: Context) => {
    const recipients =
      typeof config.recipients === 'function'
        ? config.recipients(ctx)
        : config.recipients;

    const message =
      typeof config.message === 'function'
        ? config.message(ctx)
        : config.message;

    switch (config.type) {
      case 'email':
        return await emailService.send({
          to: recipients,
          subject: config.template || 'Notification',
          body: message,
        });

      case 'slack':
        return await slackService.sendMessage(
          config.channel || '#general',
          message
        );

      case 'sms':
        return await smsService.send({
          to: recipients,
          message,
        });

      case 'webhook':
        return await webhookService.send({
          url: config.channel!,
          data: { message, recipients },
        });

      default:
        throw new Error(`Unsupported notification type: ${config.type}`);
    }
  };
}

// Use the notification step
workflow
  .step(
    'notify-admin',
    createNotificationStep({
      type: 'slack',
      channel: '#alerts',
      recipients: ['admin@example.com'],
      message: ctx => `New order received: ${ctx.payload.orderId}`,
    })
  )
  .step(
    'send-confirmation',
    createNotificationStep({
      type: 'email',
      template: 'order-confirmation',
      recipients: ctx => ctx.payload.customerEmail,
      message: ctx => `Your order ${ctx.payload.orderId} has been confirmed.`,
    })
  );
```

## Custom Step Composition

### Combining Custom Steps

```typescript
function createOrderWorkflow() {
  return {
    validateOrder: createValidationStep(orderSchema),
    processPayment: createPaymentStep({ retryAttempts: 3 }),
    updateInventory: createInventoryStep(),
    sendNotification: createNotificationStep({
      type: 'email',
      template: 'order-confirmation',
      recipients: ctx => ctx.payload.customerEmail,
      message: ctx => `Order ${ctx.payload.orderId} confirmed!`,
    }),
  };
}

// Use the composed workflow
const orderSteps = createOrderWorkflow();

workflow
  .step('validate', orderSteps.validateOrder)
  .step('pay', orderSteps.processPayment)
  .step('inventory', orderSteps.updateInventory)
  .step('notify', orderSteps.sendNotification);
```

### Custom Step with Middleware

```typescript
function createStepWithMiddleware<T>(
  stepFn: (ctx: Context) => Promise<T>,
  middleware: Array<(ctx: Context) => Promise<void>>
) {
  return async (ctx: Context) => {
    // Execute middleware before step
    for (const mw of middleware) {
      await mw(ctx);
    }

    // Execute the step
    const result = await stepFn(ctx);

    return result;
  };
}

// Use with middleware
const loggingMiddleware = async (ctx: Context) => {
  console.log(`Executing step: ${ctx.step_name}`);
};

const timingMiddleware = async (ctx: Context) => {
  ctx.startTime = Date.now();
};

workflow.step(
  'process-data',
  createStepWithMiddleware(
    async ctx => await processData(ctx.payload),
    [loggingMiddleware, timingMiddleware]
  )
);
```

## Best Practices

### 1. Keep Custom Steps Focused

```typescript
// ✅ Good: Single responsibility
function createEmailStep(config: EmailConfig) {
  return async (ctx: Context) => {
    // Only handles email sending
    return await emailService.send(config);
  };
}

// ❌ Avoid: Multiple responsibilities
function createComplexStep(config: any) {
  return async (ctx: Context) => {
    // Don't mix validation, processing, and notification
    const validated = await validate(ctx.payload);
    const processed = await process(validated);
    const notified = await notify(processed);
    return notified;
  };
}
```

### 2. Use TypeScript for Type Safety

```typescript
interface EmailStepConfig {
  template: string;
  subject: string;
  to: string | ((ctx: Context) => string);
  from?: string;
}

function createEmailStep(config: EmailStepConfig) {
  return async (ctx: Context): Promise<EmailResult> => {
    // Type-safe implementation
  };
}
```

### 3. Provide Sensible Defaults

```typescript
function createHttpStep(config: HttpStepConfig) {
  const defaults = {
    timeout: 5000,
    headers: {},
    retries: 1,
  };

  const finalConfig = { ...defaults, ...config };

  return async (ctx: Context) => {
    // Use finalConfig with defaults applied
  };
}
```

### 4. Handle Errors Gracefully

```typescript
function createResilientStep<T>(
  stepFn: (ctx: Context) => Promise<T>,
  options: { fallback?: T; logErrors?: boolean } = {}
) {
  return async (ctx: Context): Promise<T> => {
    try {
      return await stepFn(ctx);
    } catch (error) {
      if (options.logErrors) {
        console.error(`Step failed: ${error.message}`);
      }

      if (options.fallback !== undefined) {
        return options.fallback;
      }

      throw error;
    }
  };
}
```

### 5. Document Your Custom Steps

````typescript
/**
 * Creates a step that sends emails using a template
 * @param config - Email configuration
 * @returns Step function that sends emails
 * @example
 * ```typescript
 * workflow.step('send-welcome', createEmailStep({
 *   template: 'welcome',
 *   subject: 'Welcome!',
 *   to: ctx => ctx.payload.email
 * }));
 * ```
 */
function createEmailStep(config: EmailStepConfig) {
  return async (ctx: Context) => {
    // Implementation
  };
}
````

## API Reference

### Custom Step Function Signature

```typescript
type CustomStep<T = any> = (ctx: Context) => Promise<T> | T;
```

### Creating Custom Steps

- `createCustomStep(config)` - Create a custom step with configuration
- `createResilientStep(stepFn, options)` - Create a step with error handling
- `createConditionalStep(condition, trueStep, falseStep?)` - Create a conditional step
- `createCachedStep(key, fetchFn, options)` - Create a cached step

Custom steps are powerful tools for building maintainable, reusable workflow components. Use them to encapsulate common patterns and promote consistency across your workflows.
