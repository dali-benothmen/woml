# Background Actions

Cronflow actions are designed to run in the background, allowing you to perform side effects without blocking the main workflow execution. Actions are perfect for logging, notifications, cleanup operations, and other non-critical tasks.

## Overview

Actions differ from steps in several key ways:

- **Output Ignored**: Action outputs are not stored or accessible by subsequent steps
- **Background Execution**: Actions run asynchronously without blocking the workflow
- **Side Effects**: Perfect for operations where you don't need the result
- **Non-Critical**: Actions don't affect the main workflow logic

## Basic Actions

### Simple Action

```typescript
const workflow = cronflow.define({
  id: 'action-workflow',
  name: 'Action Workflow',
});

workflow
  .step('create-user', async ctx => {
    return await createUser(ctx.payload);
  })
  .action('send-welcome-email', async ctx => {
    // This runs in the background
    await sendEmail(ctx.last.email, 'Welcome to our platform!');
    // Return value is ignored
  })
  .step('log-creation', async ctx => {
    // This step can access the user data from the previous step
    console.log('User created:', ctx.last);
    return { logged: true };
  });
```

### Action with Error Handling

```typescript
workflow
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  })
  .action('send-notification', async ctx => {
    try {
      await sendNotification(ctx.last.customerId, 'Order processed');
    } catch (error) {
      // Log error but don't fail the workflow
      console.error('Notification failed:', error);
    }
  })
  .action('update-analytics', async ctx => {
    try {
      await analytics.track('order_processed', {
        orderId: ctx.last.id,
        amount: ctx.last.amount,
      });
    } catch (error) {
      console.error('Analytics update failed:', error);
    }
  });
```

## Action Use Cases

### 1. Logging and Monitoring

```typescript
const loggingWorkflow = cronflow.define({
  id: 'logging-workflow',
  name: 'Logging Workflow',
});

loggingWorkflow
  .action('log-start', async ctx => {
    console.log('Workflow started:', {
      timestamp: new Date().toISOString(),
      payload: ctx.payload,
    });
  })
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action('log-success', async ctx => {
    console.log('Workflow completed successfully:', {
      timestamp: new Date().toISOString(),
      result: ctx.last,
    });
  })
  .action('send-metrics', async ctx => {
    await metrics.increment('workflow.success');
    await metrics.timing('workflow.duration', Date.now() - ctx.run.startTime);
  });
```

### 2. Notifications

```typescript
const notificationWorkflow = cronflow.define({
  id: 'notification-workflow',
  name: 'Notification Workflow',
});

notificationWorkflow
  .step('process-payment', async ctx => {
    return await processPayment(ctx.payload);
  })
  .action('send-payment-confirmation', async ctx => {
    await emailService.send({
      to: ctx.last.customerEmail,
      subject: 'Payment Confirmed',
      template: 'payment-confirmation',
      data: ctx.last,
    });
  })
  .action('send-sms-notification', async ctx => {
    await smsService.send({
      to: ctx.last.customerPhone,
      message: `Payment of $${ctx.last.amount} confirmed`,
    });
  })
  .action('update-customer-dashboard', async ctx => {
    await dashboardService.updatePaymentStatus(
      ctx.last.customerId,
      'confirmed'
    );
  });
```

### 3. Cleanup Operations

```typescript
const cleanupWorkflow = cronflow.define({
  id: 'cleanup-workflow',
  name: 'Cleanup Workflow',
});

cleanupWorkflow
  .step('process-file', async ctx => {
    return await processFile(ctx.payload.filePath);
  })
  .action('cleanup-temp-files', async ctx => {
    // Clean up temporary files in background
    await cleanupTempFiles(ctx.payload.filePath);
  })
  .action('archive-processed-file', async ctx => {
    // Archive the processed file
    await archiveFile(ctx.payload.filePath, 'processed');
  })
  .action('update-storage-metrics', async ctx => {
    // Update storage usage metrics
    await storageService.updateMetrics();
  });
```

### 4. Database Operations

```typescript
const databaseWorkflow = cronflow.define({
  id: 'database-workflow',
  name: 'Database Workflow',
});

databaseWorkflow
  .step('create-record', async ctx => {
    return await db.records.create({
      data: ctx.payload,
    });
  })
  .action('update-cache', async ctx => {
    // Update cache in background
    await cache.set(`record-${ctx.last.id}`, ctx.last);
  })
  .action('update-search-index', async ctx => {
    // Update search index
    await searchIndex.index(ctx.last);
  })
  .action('send-webhook', async ctx => {
    // Send webhook notification
    await webhookService.notify('record.created', ctx.last);
  });
```

## Advanced Action Patterns

### 1. Conditional Actions

```typescript
const conditionalActionWorkflow = cronflow.define({
  id: 'conditional-action-workflow',
  name: 'Conditional Action Workflow',
});

conditionalActionWorkflow
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  })
  .if('is-high-value', ctx => ctx.last.amount > 1000)
  .action('send-vip-notification', async ctx => {
    await sendVipNotification(ctx.last.customerId);
  })
  .else()
  .action('send-standard-notification', async ctx => {
    await sendStandardNotification(ctx.last.customerId);
  })
  .endIf()
  .action('log-order-processed', async ctx => {
    await logOrderProcessed(ctx.last);
  });
```

### 2. Parallel Actions

```typescript
const parallelActionWorkflow = cronflow.define({
  id: 'parallel-action-workflow',
  name: 'Parallel Action Workflow',
});

parallelActionWorkflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .parallel([
    async ctx => {
      await updateCache(ctx.last);
    },
    async ctx => {
      await updateAnalytics(ctx.last);
    },
    async ctx => {
      await sendNotification(ctx.last);
    },
  ])
  .action('final-cleanup', async ctx => {
    await finalCleanup();
  });
```

### 3. Action with Retry Logic

```typescript
const retryActionWorkflow = cronflow.define({
  id: 'retry-action-workflow',
  name: 'Retry Action Workflow',
});

retryActionWorkflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action('send-notification', async ctx => {
    let attempts = 0;
    const maxAttempts = 3;

    while (attempts < maxAttempts) {
      try {
        await sendNotification(ctx.last);
        break;
      } catch (error) {
        attempts++;
        if (attempts >= maxAttempts) {
          console.error('Notification failed after all attempts:', error);
        } else {
          await new Promise(resolve => setTimeout(resolve, 1000 * attempts));
        }
      }
    }
  });
```

### 4. Action with Circuit Breaker

```typescript
const circuitBreakerActionWorkflow = cronflow.define({
  id: 'circuit-breaker-action-workflow',
  name: 'Circuit Breaker Action Workflow',
});

circuitBreakerActionWorkflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action('external-api-call', async ctx => {
    const failureCount = await ctx.state.get('api-failures', 0);

    if (failureCount >= 5) {
      console.log('Circuit breaker open, skipping API call');
      return;
    }

    try {
      await externalApi.call(ctx.last);
      await ctx.state.set('api-failures', 0);
    } catch (error) {
      const currentFailures = await ctx.state.get('api-failures', 0);
      await ctx.state.set('api-failures', currentFailures + 1);
      console.error('API call failed:', error);
    }
  });
```

## Action Configuration

### Action with Options

```typescript
const configuredActionWorkflow = cronflow.define({
  id: 'configured-action-workflow',
  name: 'Configured Action Workflow',
});

configuredActionWorkflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action(
    'send-notification',
    async ctx => {
      await sendNotification(ctx.last);
    },
    {
      timeout: '30s', // Action timeout
      retry: {
        attempts: 3,
        backoff: {
          strategy: 'exponential',
          delay: '1s',
        },
      },
    }
  );
```

### Action with Custom Error Handling

```typescript
workflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action('risky-action', async ctx => {
    await riskyOperation(ctx.last);
  })
  .onError(ctx => {
    // Handle action errors
    console.error('Action failed:', ctx.error);
    return { actionError: ctx.error.message };
  });
```

## Best Practices

### 1. Use Actions for Side Effects

```typescript
// Good: Use actions for side effects
workflow
  .step('create-user', async ctx => {
    return await createUser(ctx.payload);
  })
  .action('send-welcome-email', async ctx => {
    await sendEmail(ctx.last.email, 'Welcome!');
  })
  .action('log-user-creation', async ctx => {
    await logUserCreation(ctx.last);
  });

// Avoid: Using steps for side effects when you don't need the output
workflow
  .step('create-user', async ctx => {
    return await createUser(ctx.payload);
  })
  .step('send-welcome-email', async ctx => {
    await sendEmail(ctx.last.email, 'Welcome!');
    return { emailSent: true }; // Unnecessary return
  });
```

### 2. Handle Action Errors Gracefully

```typescript
workflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action('send-notification', async ctx => {
    try {
      await sendNotification(ctx.last);
    } catch (error) {
      // Log error but don't fail the workflow
      console.error('Notification failed:', error);

      // Optionally retry or send to dead letter queue
      await deadLetterQueue.send({
        type: 'notification',
        data: ctx.last,
        error: error.message,
      });
    }
  });
```

### 3. Use Actions for Non-Critical Operations

```typescript
workflow
  .step('process-payment', async ctx => {
    // Critical operation - use step
    return await processPayment(ctx.payload);
  })
  .action('send-payment-notification', async ctx => {
    // Non-critical - use action
    await sendPaymentNotification(ctx.last);
  })
  .action('update-analytics', async ctx => {
    // Non-critical - use action
    await updatePaymentAnalytics(ctx.last);
  })
  .action('cleanup-temp-data', async ctx => {
    // Non-critical - use action
    await cleanupTempData();
  });
```

### 4. Monitor Action Performance

```typescript
workflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .action('monitored-action', async ctx => {
    const startTime = Date.now();

    try {
      await performAction(ctx.last);

      const duration = Date.now() - startTime;
      await metrics.timing('action.duration', duration);
      await metrics.increment('action.success');
    } catch (error) {
      await metrics.increment('action.failure');
      throw error;
    }
  });
```

### 5. Use Actions for Batch Operations

```typescript
workflow
  .step('fetch-users', async ctx => {
    return await fetchUsers(ctx.payload.criteria);
  })
  .action('batch-process-users', async ctx => {
    const users = ctx.last;

    // Process users in batches
    for (let i = 0; i < users.length; i += 100) {
      const batch = users.slice(i, i + 100);
      await processUserBatch(batch);

      // Small delay between batches
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  });
```

## Action vs Step Decision Guide

### Use Steps When:

- You need the output for subsequent steps
- The operation is critical to workflow success
- You want to handle errors in the workflow
- The operation affects the main business logic

### Use Actions When:

- You're performing side effects (logging, notifications)
- The operation is non-critical
- You don't need the output
- You want fire-and-forget behavior
- The operation is for monitoring or analytics

## Related Topics

- **[Error Handling](/guide/error-handling)** - Basic error handling and recovery
- **[Testing](/guide/testing)** - Testing workflows and components
- **[Performance](/guide/performance)** - Optimizing workflow performance
