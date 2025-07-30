# Error Handling

Cronflow provides comprehensive error handling mechanisms to build robust, fault-tolerant workflows. Learn how to handle errors gracefully and implement recovery strategies.

## Basic Error Handling

### Throwing Errors

Steps can throw errors to indicate failure:

```typescript
workflow.step('validate-user', async ctx => {
  const user = await db.users.findUnique({
    where: { id: ctx.payload.userId },
  });

  if (!user) {
    throw new Error('User not found');
  }

  if (user.status === 'suspended') {
    throw new Error('User account is suspended');
  }

  return user;
});
```

### Catching Errors with `.onError()`

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

## Retry Mechanisms

### Automatic Retries

Configure automatic retries for failed steps:

```typescript
workflow.step(
  'api-call',
  async ctx => {
    return await externalApi.call();
  },
  {
    retry: {
      attempts: 3,
      backoff: { strategy: 'exponential', delay: '1s' },
    },
  }
);
```

### Retry Configuration Options

| Option             | Type                       | Default         | Description                    |
| ------------------ | -------------------------- | --------------- | ------------------------------ |
| `attempts`         | `number`                   | `3`             | Number of retry attempts       |
| `backoff`          | `object`                   | -               | Backoff strategy configuration |
| `backoff.strategy` | `'exponential' \| 'fixed'` | `'exponential'` | Backoff strategy               |
| `backoff.delay`    | `string \| number`         | `'1s'`          | Initial delay                  |

### Exponential Backoff

```typescript
workflow.step(
  'api-call',
  async ctx => {
    return await apiCall();
  },
  {
    retry: {
      attempts: 5,
      backoff: { strategy: 'exponential', delay: '1s' },
      // Retries at: 1s, 2s, 4s, 8s, 16s
    },
  }
);
```

### Fixed Backoff

```typescript
workflow.step(
  'api-call',
  async ctx => {
    return await apiCall();
  },
  {
    retry: {
      attempts: 3,
      backoff: { strategy: 'fixed', delay: '2s' },
      // Retries at: 2s, 2s, 2s
    },
  }
);
```

## Circuit Breaker Pattern

Implement circuit breaker pattern for external dependencies:

```typescript
workflow
  .step('external-api-call', async ctx => {
    try {
      return await externalApi.call();
    } catch (error) {
      // Check if circuit breaker should be opened
      const failureCount = await ctx.state.get('api-failures', 0);
      if (failureCount >= 5) {
        throw new Error('Circuit breaker open - too many failures');
      }

      // Increment failure count
      await ctx.state.set('api-failures', failureCount + 1);
      throw error;
    }
  })
  .onError(ctx => {
    if (ctx.error.message.includes('Circuit breaker')) {
      // Use fallback when circuit breaker is open
      return { status: 'fallback', data: getFallbackData() };
    }

    // Re-throw other errors
    throw ctx.error;
  });
```

## Fallback Strategies

### Step-Level Fallbacks

```typescript
workflow
  .step('primary-api', async ctx => {
    return await primaryApi.call();
  })
  .onError(ctx => {
    // Try backup API
    return await backupApi.call();
  })
  .onError(ctx => {
    // Use cached data as final fallback
    return getCachedData();
  });
```

### Workflow-Level Fallbacks

```typescript
const workflow = cronflow.define({
  id: 'robust-workflow',
  hooks: {
    onFailure: ctx => {
      // Global fallback when workflow fails
      console.error('Workflow failed, using fallback:', ctx.error);

      // Send alert
      slack.sendMessage(
        '#alerts',
        `Workflow ${ctx.run.workflowId} failed: ${ctx.error.message}`
      );

      // Store failure for analysis
      db.workflowFailures.create({
        data: {
          workflowId: ctx.run.workflowId,
          runId: ctx.run.id,
          error: ctx.error.message,
          timestamp: new Date(),
        },
      });
    },
  },
});
```

## Error Classification

### Handle Different Error Types

```typescript
workflow
  .step('process-payment', async ctx => {
    return await paymentProcessor.charge(ctx.payload);
  })
  .onError(ctx => {
    const error = ctx.error;

    if (error.code === 'INSUFFICIENT_FUNDS') {
      return { status: 'failed', reason: 'insufficient_funds' };
    }

    if (error.code === 'CARD_DECLINED') {
      return { status: 'failed', reason: 'card_declined' };
    }

    if (error.code === 'NETWORK_ERROR') {
      // Retry network errors
      throw error;
    }

    // Unknown error
    return { status: 'failed', reason: 'unknown' };
  });
```

## Timeout Handling

### Step Timeouts

```typescript
workflow
  .step(
    'long-running-operation',
    async ctx => {
      return await longRunningProcess();
    },
    {
      timeout: '5m', // 5 minute timeout
    }
  )
  .onError(ctx => {
    if (ctx.error.message.includes('timeout')) {
      return { status: 'timeout', fallback: true };
    }
    throw ctx.error;
  });
```

### Workflow Timeouts

```typescript
const workflow = cronflow.define({
  id: 'timeout-workflow',
  timeout: '10m', // 10 minute workflow timeout
  hooks: {
    onFailure: ctx => {
      if (ctx.error.message.includes('timeout')) {
        console.log('Workflow timed out, cleaning up...');
        // Perform cleanup operations
      }
    },
  },
});
```

## Error Recovery

### Manual Recovery

```typescript
// Resume a paused workflow
await cronflow.resume('approval-token-123', {
  approved: true,
  reason: 'Looks good to me'
});

// Replay a failed workflow
await cronflow.replay('failed-run-id', {
  mockStep('problematic-step', async ctx => {
    // Provide fixed implementation
    return { fixed: true };
  })
});
```

### Automatic Recovery

```typescript
workflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .onError(ctx => {
    // Attempt automatic recovery
    return await attemptRecovery(ctx.error);
  })
  .step('validate-recovery', async ctx => {
    // Validate that recovery was successful
    if (!ctx.last.recovered) {
      throw new Error('Recovery failed');
    }
    return ctx.last;
  });
```

## Error Monitoring

### Structured Error Logging

```typescript
workflow
  .step('critical-operation', async ctx => {
    return await criticalApiCall();
  })
  .onError(ctx => {
    // Log structured error information
    console.error('Critical operation failed:', {
      step: 'critical-operation',
      runId: ctx.run.id,
      workflowId: ctx.run.workflowId,
      error: {
        message: ctx.error.message,
        stack: ctx.error.stack,
        code: ctx.error.code,
      },
      payload: ctx.payload,
      timestamp: new Date().toISOString(),
    });

    // Send to monitoring service
    monitoringService.trackError({
      type: 'critical_operation_failure',
      runId: ctx.run.id,
      error: ctx.error.message,
    });

    throw ctx.error; // Re-throw for workflow-level handling
  });
```

### Error Metrics

```typescript
const workflow = cronflow.define({
  id: 'monitored-workflow',
  hooks: {
    onFailure: ctx => {
      // Track error metrics
      metrics.increment('workflow.failure', {
        workflowId: ctx.run.workflowId,
        errorType: ctx.error.constructor.name,
      });

      // Alert on repeated failures
      const failureCount = metrics.get('workflow.failure', {
        workflowId: ctx.run.workflowId,
        timeWindow: '1h',
      });

      if (failureCount > 10) {
        slack.sendMessage(
          '#alerts',
          `High failure rate for workflow ${ctx.run.workflowId}: ${failureCount} failures in 1h`
        );
      }
    },
  },
});
```

## Best Practices

### 1. Use Specific Error Types

```typescript
// Good: Specific error handling
workflow.step('api-call', async ctx => {
  try {
    return await api.call();
  } catch (error) {
    if (error.code === 'RATE_LIMIT') {
      throw new Error('Rate limit exceeded');
    }
    if (error.code === 'AUTH_FAILED') {
      throw new Error('Authentication failed');
    }
    throw error;
  }
});

// Avoid: Generic error handling
workflow.step('api-call', async ctx => {
  try {
    return await api.call();
  } catch (error) {
    throw new Error('API call failed'); // Loses original error context
  }
});
```

### 2. Implement Graceful Degradation

```typescript
workflow
  .step('fetch-user-data', async ctx => {
    return await userService.getUser(ctx.payload.userId);
  })
  .onError(ctx => {
    // Return minimal user data if full profile fails
    return {
      id: ctx.payload.userId,
      name: 'Unknown User',
      fallback: true,
    };
  });
```

### 3. Use Appropriate Retry Strategies

```typescript
// For transient failures (network, temporary unavailability)
workflow.step(
  'api-call',
  async ctx => {
    return await api.call();
  },
  {
    retry: {
      attempts: 3,
      backoff: { strategy: 'exponential', delay: '1s' },
    },
  }
);

// For permanent failures (validation errors, auth failures)
workflow.step('validate-input', async ctx => {
  if (!ctx.payload.email) {
    throw new Error('Email is required');
  }
  return ctx.payload;
});
// No retry for validation errors
```

### 4. Implement Circuit Breakers

```typescript
workflow.step('external-service', async ctx => {
  const failureCount = await ctx.state.get('service-failures', 0);
  const lastFailure = await ctx.state.get('last-failure-time', 0);

  // Check if circuit breaker should be reset (after 5 minutes)
  if (Date.now() - lastFailure > 5 * 60 * 1000) {
    await ctx.state.set('service-failures', 0);
  }

  // Check if circuit breaker is open
  if (failureCount >= 5) {
    throw new Error('Circuit breaker open');
  }

  try {
    const result = await externalService.call();
    // Reset failure count on success
    await ctx.state.set('service-failures', 0);
    return result;
  } catch (error) {
    // Increment failure count
    await ctx.state.set('service-failures', failureCount + 1);
    await ctx.state.set('last-failure-time', Date.now());
    throw error;
  }
});
```

### 5. Provide Meaningful Error Messages

```typescript
workflow
  .step('process-order', async ctx => {
    const order = ctx.payload;

    if (!order.items || order.items.length === 0) {
      throw new Error('Order must contain at least one item');
    }

    if (order.amount <= 0) {
      throw new Error('Order amount must be greater than zero');
    }

    return await processOrder(order);
  })
  .onError(ctx => {
    // Provide user-friendly error messages
    const errorMessage = ctx.error.message;

    if (errorMessage.includes('must contain at least one item')) {
      return { status: 'failed', reason: 'empty_order' };
    }

    if (errorMessage.includes('must be greater than zero')) {
      return { status: 'failed', reason: 'invalid_amount' };
    }

    return { status: 'failed', reason: 'unknown' };
  });
```

## What's Next?

- **[Performance](/guide/performance)** - Optimize workflow performance
- **[Parallel Execution](/guide/parallel-execution)** - Run steps concurrently
- **[State Management](/guide/state-management)** - Manage workflow state
- **[Testing](/guide/testing)** - Test error scenarios
