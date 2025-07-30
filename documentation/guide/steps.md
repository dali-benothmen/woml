# Steps

Steps are the primary units of work in Cronflow workflows. They define individual operations that can be executed, retried, and monitored.

## Defining Steps

Use `.step()` to define a step that produces output and can be referenced by other steps.

```typescript
workflow
  .step('fetch-user', async ctx => {
    const user = await db.users.findUnique({
      where: { id: ctx.payload.userId },
    });
    return user;
  })
  .step('process-order', async ctx => {
    // Access previous step output
    const user = ctx.steps['fetch-user'].output;
    const order = await createOrder(user, ctx.payload);
    return order;
  });
```

## Step Configuration

Steps can be configured with various options for retry, timeout, caching, and more:

```typescript
workflow.step(
  'api-call',
  async ctx => {
    const response = await fetch('https://api.example.com/data');
    return await response.json();
  },
  {
    timeout: '30s',
    retry: {
      attempts: 3,
      backoff: { strategy: 'exponential', delay: '2s' },
    },
    cache: {
      key: ctx => `api-data-${ctx.payload.userId}`,
      ttl: '1h',
    },
  }
);
```

## Step Options

| Option    | Type               | Default                 | Description                                                    |
| --------- | ------------------ | ----------------------- | -------------------------------------------------------------- |
| `timeout` | `string \| number` | Inherited from workflow | Maximum duration for this step                                 |
| `retry`   | `object`           | Inherited from workflow | Retry configuration for this step                              |
| `cache`   | `object`           | -                       | Caching configuration: `{ key: (ctx) => string, ttl: string }` |
| `delay`   | `string \| number` | -                       | Delay before executing this step                               |

## Retry Configuration

Configure automatic retries for failed steps:

```typescript
workflow.step(
  'risky-operation',
  async ctx => {
    // This step will be retried if it fails
    return await riskyApiCall();
  },
  {
    retry: {
      attempts: 5,
      backoff: { strategy: 'exponential', delay: '1s' },
    },
  }
);
```

### Retry Options

| Option             | Type                       | Default         | Description                    |
| ------------------ | -------------------------- | --------------- | ------------------------------ |
| `attempts`         | `number`                   | `3`             | Number of retry attempts       |
| `backoff`          | `object`                   | -               | Backoff strategy configuration |
| `backoff.strategy` | `'exponential' \| 'fixed'` | `'exponential'` | Backoff strategy               |
| `backoff.delay`    | `string \| number`         | `'1s'`          | Initial delay                  |

### Backoff Strategies

```typescript
// Exponential backoff (1s, 2s, 4s, 8s...)
workflow.step(
  'api-call',
  async ctx => {
    return await apiCall();
  },
  {
    retry: {
      attempts: 3,
      backoff: { strategy: 'exponential', delay: '1s' },
    },
  }
);

// Fixed backoff (always 2s)
workflow.step(
  'api-call',
  async ctx => {
    return await apiCall();
  },
  {
    retry: {
      attempts: 3,
      backoff: { strategy: 'fixed', delay: '2s' },
    },
  }
);
```

## Caching

Cache step results to avoid redundant operations:

```typescript
workflow.step(
  'fetch-user-profile',
  async ctx => {
    const user = await db.users.findUnique({
      where: { id: ctx.payload.userId },
      include: { profile: true },
    });
    return user;
  },
  {
    cache: {
      key: ctx => `user-profile-${ctx.payload.userId}`,
      ttl: '1h', // Cache for 1 hour
    },
  }
);
```

### Cache Configuration

| Option | Type              | Description                               |
| ------ | ----------------- | ----------------------------------------- |
| `key`  | `(ctx) => string` | Function that generates cache key         |
| `ttl`  | `string`          | Time-to-live for cached data (e.g., '1h') |

## Timeout Configuration

Set maximum execution time for individual steps:

```typescript
workflow
  .step(
    'quick-operation',
    async ctx => {
      return await quickApiCall();
    },
    {
      timeout: '5s', // 5 seconds
    }
  )
  .step(
    'long-operation',
    async ctx => {
      return await longRunningProcess();
    },
    {
      timeout: '10m', // 10 minutes
    }
  );
```

## Delay Configuration

Add delays before step execution:

```typescript
workflow
  .step('send-notification', async ctx => {
    await sendEmail(ctx.payload.email);
  })
  .step(
    'send-reminder',
    async ctx => {
      // Wait 24 hours before sending reminder
      await sendReminder(ctx.payload.email);
    },
    {
      delay: '24h',
    }
  );
```

## Error Handling

Handle errors at the step level:

```typescript
workflow
  .step('risky-operation', async ctx => {
    return await riskyApiCall();
  })
  .onError(ctx => {
    // Custom error handling
    console.error('Step failed:', ctx.error);

    // Return fallback value
    return { status: 'fallback', data: null };
  });
```

## Step Context

The context object provides access to workflow data and utilities:

```typescript
workflow.step('process-data', async ctx => {
  // Access trigger payload
  const { userId, data } = ctx.payload;

  // Access previous step outputs
  const userData = ctx.steps['fetch-user'].output;
  const orderData = ctx.steps['fetch-order'].output;

  // Access run metadata
  console.log(`Processing run: ${ctx.run.id}`);

  // Use persistent state
  await ctx.state.set('last-processed', Date.now());
  const lastProcessed = await ctx.state.get('last-processed');

  // Convenience property for previous step
  const previousResult = ctx.last;

  return { processed: true, timestamp: new Date().toISOString() };
});
```

## Step Outputs

Steps can return any data type, which is stored and accessible by subsequent steps:

```typescript
workflow
  .step('fetch-data', async ctx => {
    return {
      users: await db.users.findMany(),
      orders: await db.orders.findMany(),
      timestamp: new Date().toISOString(),
    };
  })
  .step('process-data', async ctx => {
    // Access the output from fetch-data step
    const { users, orders, timestamp } = ctx.steps['fetch-data'].output;

    return {
      userCount: users.length,
      orderCount: orders.length,
      processedAt: timestamp,
    };
  });
```

## Actions vs Steps

Actions are similar to steps but their output is ignored:

```typescript
workflow
  .step('create-user', async ctx => {
    const user = await db.users.create({
      data: { email: ctx.payload.email },
    });
    return user; // Output is stored
  })
  .action('send-welcome-email', async ctx => {
    await emailService.sendWelcomeEmail(ctx.last.email);
    // No return value needed (output ignored)
  });
```

## Logging

Use the `.log()` method for structured logging within workflows:

```typescript
workflow
  .log('Starting user processing', 'info')
  .step('fetch-user', async ctx => {
    const user = await db.users.findUnique({
      where: { id: ctx.payload.userId },
    });
    return user;
  })
  .log(ctx => `User ${ctx.last.email} fetched successfully`)
  .step('process-user', async ctx => {
    // Process user data
    return { processed: true };
  })
  .log('User processing completed', 'info');
```

### Log Levels

- `'info'` (default): General information
- `'warn'`: Warnings
- `'error'`: Errors

## Step Validation

Validate step inputs and outputs:

```typescript
import { z } from 'zod';

const userSchema = z.object({
  id: z.string(),
  email: z.string().email(),
  name: z.string(),
});

workflow
  .step('fetch-user', async ctx => {
    const user = await db.users.findUnique({
      where: { id: ctx.payload.userId },
    });

    // Validate output
    return userSchema.parse(user);
  })
  .step('process-user', async ctx => {
    // Input is guaranteed to be valid
    const user = ctx.steps['fetch-user'].output;
    return { processed: true, userId: user.id };
  });
```

## Best Practices

### 1. Keep Steps Focused

```typescript
// Good: Single responsibility
workflow
  .step('fetch-user', async ctx => {
    return await db.users.findUnique({ where: { id: ctx.payload.userId } });
  })
  .step('validate-user', async ctx => {
    const user = ctx.last;
    if (!user) throw new Error('User not found');
    return user;
  });

// Avoid: Multiple responsibilities
workflow.step('fetch-and-validate-user', async ctx => {
  const user = await db.users.findUnique({ where: { id: ctx.payload.userId } });
  if (!user) throw new Error('User not found');
  return user;
});
```

### 2. Use Descriptive Names

```typescript
// Good
workflow.step('fetch-user-profile', async ctx => {
  /* ... */
});
workflow.step('validate-payment-method', async ctx => {
  /* ... */
});

// Avoid
workflow.step('step1', async ctx => {
  /* ... */
});
workflow.step('process', async ctx => {
  /* ... */
});
```

### 3. Handle Errors Appropriately

```typescript
workflow
  .step(
    'api-call',
    async ctx => {
      try {
        return await externalApi.call();
      } catch (error) {
        // Log error and re-throw for retry mechanism
        console.error('API call failed:', error);
        throw error;
      }
    },
    {
      retry: { attempts: 3, backoff: { strategy: 'exponential', delay: '1s' } },
    }
  )
  .onError(ctx => {
    // Handle final failure
    return { status: 'failed', fallback: true };
  });
```

### 4. Use Caching for Expensive Operations

```typescript
workflow.step(
  'fetch-user-profile',
  async ctx => {
    return await expensiveApiCall(ctx.payload.userId);
  },
  {
    cache: {
      key: ctx => `user-profile-${ctx.payload.userId}`,
      ttl: '30m', // Cache for 30 minutes
    },
  }
);
```

## What's Next?

- **[Conditions](/guide/conditions)** - Add dynamic logic to workflows
- **[Error Handling](/guide/error-handling)** - Build robust error handling
- **[Performance](/guide/performance)** - Optimize workflow performance
- **[Custom Steps](/guide/custom-steps)** - Create reusable step components
