# Parallel Execution

Cronflow provides powerful parallel execution capabilities that allow you to run multiple operations simultaneously, significantly improving performance and throughput.

## Parallel Execution

Use `.parallel()` to execute multiple steps concurrently and wait for all to complete:

```typescript
workflow
  .parallel([
    async ctx => {
      // Fetch user data
      return await db.users.findUnique({
        where: { id: ctx.payload.userId },
      });
    },
    async ctx => {
      // Fetch order data
      return await db.orders.findUnique({
        where: { id: ctx.payload.orderId },
      });
    },
    async ctx => {
      // Fetch product data
      return await db.products.findMany({
        where: { id: { in: ctx.payload.productIds } },
      });
    },
  ])
  .step('process-combined-data', async ctx => {
    // All three operations completed in parallel
    const [user, order, products] = ctx.last;
    return { processed: true, user, order, products };
  });
```

### Parallel Execution Benefits

- **Performance**: Execute multiple operations simultaneously
- **Efficiency**: Reduce total execution time
- **Resource Utilization**: Better use of system resources
- **Scalability**: Handle multiple concurrent operations

### Parallel Execution Patterns

#### 1. Data Fetching

```typescript
workflow
  .parallel([
    async ctx => await fetchUserData(ctx.payload.userId),
    async ctx => await fetchOrderData(ctx.payload.orderId),
    async ctx => await fetchProductData(ctx.payload.productIds),
    async ctx => await fetchPaymentData(ctx.payload.paymentId),
  ])
  .step('combine-data', async ctx => {
    const [user, order, products, payment] = ctx.last;
    return { combined: true, user, order, products, payment };
  });
```

#### 2. API Calls

```typescript
workflow
  .parallel([
    async ctx => await externalApi.getUser(ctx.payload.userId),
    async ctx => await externalApi.getOrder(ctx.payload.orderId),
    async ctx => await externalApi.getInventory(ctx.payload.productIds),
  ])
  .step('validate-data', async ctx => {
    const [user, order, inventory] = ctx.last;
    return validateAllData(user, order, inventory);
  });
```

#### 3. Database Operations

```typescript
workflow
  .parallel([
    async ctx =>
      await db.users.update({
        where: { id: ctx.payload.userId },
        data: { lastActive: new Date() },
      }),
    async ctx =>
      await db.orders.update({
        where: { id: ctx.payload.orderId },
        data: { status: 'processing' },
      }),
    async ctx =>
      await db.analytics.create({
        data: { event: 'order_processed', timestamp: new Date() },
      }),
  ])
  .step('log-completion', async ctx => {
    console.log('All database operations completed');
  });
```

## Race Conditions

Use `.race()` to execute multiple operations and proceed with the first one that completes:

```typescript
workflow
  .race([
    async ctx => await primaryApi.call(),
    async ctx => await backupApi.call(),
  ])
  .step('process-result', async ctx => {
    // ctx.last contains the result from whichever API responded first
    return processResult(ctx.last);
  });
```

### Race Condition Use Cases

#### 1. API Fallback

```typescript
workflow
  .race([
    async ctx => await primaryPaymentGateway.process(ctx.payload),
    async ctx => await backupPaymentGateway.process(ctx.payload),
  ])
  .step('handle-payment-result', async ctx => {
    const result = ctx.last;
    if (result.success) {
      return { paymentProcessed: true, gateway: result.gateway };
    } else {
      throw new Error('All payment gateways failed');
    }
  });
```

#### 2. Data Source Selection

```typescript
workflow
  .race([
    async ctx => await cache.get(ctx.payload.key),
    async ctx => await database.get(ctx.payload.key),
    async ctx => await externalApi.get(ctx.payload.key),
  ])
  .step('use-fastest-result', async ctx => {
    return { data: ctx.last, source: 'fastest' };
  });
```

#### 3. Health Checks

```typescript
workflow
  .race([
    async ctx => await healthCheck.primary(),
    async ctx => await healthCheck.backup(),
    async ctx => await healthCheck.fallback(),
  ])
  .step('report-health', async ctx => {
    const health = ctx.last;
    return { status: health.status, responseTime: health.responseTime };
  });
```

## Advanced Parallel Patterns

### 1. Conditional Parallel Execution

```typescript
workflow
  .if('needs-validation', ctx => ctx.payload.requiresValidation)
  .parallel([
    async ctx => await validateOrder(ctx.last),
    async ctx => await checkInventory(ctx.last),
    async ctx => await verifyPayment(ctx.last),
  ])
  .step('process-validated-order', async ctx => {
    const [validation, inventory, payment] = ctx.last;
    return { processed: true, validation, inventory, payment };
  })
  .else()
  .step('process-simple-order', async ctx => {
    return { processed: true };
  })
  .endIf();
```

### 2. Parallel with Error Handling

```typescript
workflow
  .parallel([
    async ctx => {
      try {
        return await riskyApiCall();
      } catch (error) {
        return { error: error.message, fallback: true };
      }
    },
    async ctx => {
      try {
        return await anotherRiskyApiCall();
      } catch (error) {
        return { error: error.message, fallback: true };
      }
    },
  ])
  .step('handle-results', async ctx => {
    const [result1, result2] = ctx.last;

    if (result1.error && result2.error) {
      throw new Error('All operations failed');
    }

    return {
      success: true,
      results: [result1, result2],
    };
  });
```

### 3. Parallel with Timeout

```typescript
workflow.step('start-parallel-operations', async ctx => {
  const promises = [
    externalApi.call1(),
    externalApi.call2(),
    externalApi.call3(),
  ];

  const timeout = new Promise((_, reject) =>
    setTimeout(() => reject(new Error('Timeout')), 5000)
  );

  try {
    const results = await Promise.race([Promise.all(promises), timeout]);

    return results;
  } catch (error) {
    return { error: error.message, partial: true };
  }
});
```

## Performance Considerations

### 1. Memory Usage

```typescript
// Good: Process in smaller batches
workflow
  .step('get-large-dataset', async ctx => {
    return await db.users.findMany({ where: { active: true } });
  })
  .forEach(
    'process-users',
    ctx => ctx.steps['get-large-dataset'].output,
    (user, flow) => {
      flow.step('process-user', async () => {
        return await processUser(user);
      });
    }
  );
```

### 2. Resource Limits

```typescript
workflow.parallel(
  [
    async ctx => await apiCall1(),
    async ctx => await apiCall2(),
    async ctx => await apiCall3(),
  ],
  {
    maxConcurrency: 3, // Limit concurrent operations
    timeout: '30s', // Overall timeout
  }
);
```

### 3. Error Isolation

```typescript
workflow
  .parallel([
    async ctx => {
      try {
        return await criticalOperation();
      } catch (error) {
        // Log error but don't fail the entire parallel block
        console.error('Critical operation failed:', error);
        return { error: error.message };
      }
    },
    async ctx => await nonCriticalOperation(),
  ])
  .step('handle-results', async ctx => {
    const [critical, nonCritical] = ctx.last;

    if (critical.error) {
      // Handle critical failure
      return { status: 'degraded', nonCritical };
    }

    return { status: 'success', critical, nonCritical };
  });
```

## Best Practices

### 1. Use Parallel for Independent Operations

```typescript
// Good: Independent operations
workflow.parallel([
  async ctx => await fetchUserData(ctx.payload.userId),
  async ctx => await fetchOrderData(ctx.payload.orderId),
  async ctx => await fetchProductData(ctx.payload.productIds),
]);

// Avoid: Dependent operations
workflow.parallel([
  async ctx => await createUser(ctx.payload),
  async ctx => await createOrder(ctx.payload.userId), // Depends on user creation
]);
```

### 2. Handle Errors Gracefully

```typescript
workflow
  .parallel([
    async ctx => await operation1(),
    async ctx => await operation2(),
    async ctx => await operation3(),
  ])
  .onError(ctx => {
    // Handle parallel execution errors
    console.error('Parallel execution failed:', ctx.error);
    return { status: 'failed', fallback: true };
  });
```

### 3. Monitor Performance

```typescript
workflow
  .step('start-timer', async ctx => {
    return { startTime: Date.now() };
  })
  .parallel([
    async ctx => await operation1(),
    async ctx => await operation2(),
    async ctx => await operation3(),
  ])
  .step('measure-performance', async ctx => {
    const startTime = ctx.steps['start-timer'].output.startTime;
    const duration = Date.now() - startTime;

    console.log(`Parallel execution took ${duration}ms`);
    return { duration, results: ctx.last };
  });
```

### 4. Use Race for Fallback Scenarios

```typescript
workflow
  .race([
    async ctx => await primaryService.call(),
    async ctx => await backupService.call(),
  ])
  .step('handle-service-response', async ctx => {
    const result = ctx.last;
    return {
      success: true,
      service: result.service,
      data: result.data,
    };
  });
```

## What's Next?

- **[Framework Integration](/guide/framework-integration)** - Integrate with Express, Fastify, and other frameworks
- **[Advanced Triggers](/guide/advanced-triggers)** - Advanced webhook and trigger configurations
- **[State Management](/guide/state-management)** - Manage workflow state efficiently
- **[Testing](/guide/testing)** - Test parallel execution scenarios
