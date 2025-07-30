# Batch Processing

Cronflow provides powerful batch processing capabilities that allow you to efficiently handle large datasets by processing them in smaller, manageable chunks. This is perfect for data processing, bulk operations, and handling large arrays of items.

## Overview

Batch processing in Cronflow offers:

- **Efficient Memory Usage**: Process large datasets without overwhelming memory
- **Parallel Processing**: Handle multiple items concurrently
- **Error Isolation**: Failures in one batch don't affect others
- **Progress Tracking**: Monitor batch processing progress
- **Resumable Operations**: Continue from where you left off

## Basic Batch Processing

### Simple Batch Processing

```typescript
const batchWorkflow = cronflow.define({
  id: 'batch-workflow',
  name: 'Batch Processing Workflow',
});

batchWorkflow
  .step('fetch-all-users', async ctx => {
    return await fetchAllUsers(ctx.payload.criteria);
  })
  .batch(
    'process-users-in-batches',
    {
      items: ctx => ctx.steps['fetch-all-users'].output,
      size: 100,
    },
    (batch, flow) => {
      flow.step('process-batch', async () => {
        return await processUserBatch(batch);
      });
    }
  );
```

### Batch Processing with Progress

```typescript
const progressBatchWorkflow = cronflow.define({
  id: 'progress-batch-workflow',
  name: 'Progress Batch Workflow',
});

progressBatchWorkflow
  .step('get-large-dataset', async ctx => {
    return await getLargeDataset(ctx.payload);
  })
  .batch(
    'process-with-progress',
    {
      items: ctx => ctx.steps['get-large-dataset'].output,
      size: 50,
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          return await processBatch(batch);
        })
        .action('log-progress', async () => {
          const processed = await flow.state.get('processed-count', 0);
          const total = flow.steps['get-large-dataset'].output.length;
          console.log(`Processed ${processed + batch.length}/${total} items`);
          await flow.state.set('processed-count', processed + batch.length);
        });
    }
  );
```

## Advanced Batch Patterns

### 1. Batch Processing with Error Handling

```typescript
const errorHandlingBatchWorkflow = cronflow.define({
  id: 'error-handling-batch-workflow',
  name: 'Error Handling Batch Workflow',
});

errorHandlingBatchWorkflow
  .step('fetch-items', async ctx => {
    return await fetchItems(ctx.payload);
  })
  .batch(
    'process-with-error-handling',
    {
      items: ctx => ctx.steps['fetch-items'].output,
      size: 25,
    },
    (batch, flow) => {
      flow
        .step('validate-batch', async () => {
          return await validateBatch(batch);
        })
        .step('process-batch', async () => {
          try {
            return await processBatch(batch);
          } catch (error) {
            // Log failed batch but continue processing
            await logFailedBatch(batch, error);
            return { failed: true, error: error.message };
          }
        })
        .action('update-progress', async () => {
          const successCount = await flow.state.get('success-count', 0);
          const failureCount = await flow.state.get('failure-count', 0);

          if (flow.last.failed) {
            await flow.state.set('failure-count', failureCount + 1);
          } else {
            await flow.state.set('success-count', successCount + 1);
          }
        });
    }
  );
```

### 2. Parallel Batch Processing

```typescript
const parallelBatchWorkflow = cronflow.define({
  id: 'parallel-batch-workflow',
  name: 'Parallel Batch Workflow',
});

parallelBatchWorkflow
  .step('fetch-data', async ctx => {
    return await fetchData(ctx.payload);
  })
  .batch(
    'parallel-batch-processing',
    {
      items: ctx => ctx.steps['fetch-data'].output,
      size: 100,
      concurrency: 5, // Process 5 batches in parallel
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          return await processBatch(batch);
        })
        .action('update-cache', async () => {
          await updateCache(flow.last);
        });
    }
  );
```

### 3. Conditional Batch Processing

```typescript
const conditionalBatchWorkflow = cronflow.define({
  id: 'conditional-batch-workflow',
  name: 'Conditional Batch Workflow',
});

conditionalBatchWorkflow
  .step('fetch-users', async ctx => {
    return await fetchUsers(ctx.payload);
  })
  .batch(
    'conditional-processing',
    {
      items: ctx => ctx.steps['fetch-users'].output,
      size: 50,
    },
    (batch, flow) => {
      flow
        .step('analyze-batch', async () => {
          return await analyzeBatch(batch);
        })
        .if('batch-needs-processing', ctx => ctx.last.needsProcessing)
        .step('process-batch', async () => {
          return await processBatch(batch);
        })
        .else()
        .step('skip-batch', async () => {
          return { skipped: true, reason: 'No processing needed' };
        })
        .endIf()
        .action('log-batch-result', async () => {
          await logBatchResult(flow.last);
        });
    }
  );
```

### 4. Batch Processing with State Management

```typescript
const statefulBatchWorkflow = cronflow.define({
  id: 'stateful-batch-workflow',
  name: 'Stateful Batch Workflow',
});

statefulBatchWorkflow
  .step('initialize-processing', async ctx => {
    await ctx.state.set('total-items', 0);
    await ctx.state.set('processed-items', 0);
    await ctx.state.set('failed-items', 0);
    return await getInitialData(ctx.payload);
  })
  .batch(
    'stateful-batch-processing',
    {
      items: ctx => ctx.steps['initialize-processing'].output,
      size: 75,
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          const result = await processBatch(batch);

          // Update state
          const processed = await flow.state.get('processed-items', 0);
          await flow.state.set('processed-items', processed + batch.length);

          return result;
        })
        .action('log-progress', async () => {
          const processed = await flow.state.get('processed-items', 0);
          const total = await flow.state.get('total-items', 0);
          const percentage = Math.round((processed / total) * 100);

          console.log(`Progress: ${processed}/${total} (${percentage}%)`);
        });
    }
  );
```

## ForEach Processing

### Basic ForEach

```typescript
const forEachWorkflow = cronflow.define({
  id: 'foreach-workflow',
  name: 'ForEach Workflow',
});

forEachWorkflow
  .step('fetch-users', async ctx => {
    return await fetchUsers(ctx.payload);
  })
  .forEach(
    'process-user',
    ctx => ctx.steps['fetch-users'].output,
    (user, flow) => {
      flow
        .step('validate-user', async () => {
          return await validateUser(user);
        })
        .step('process-user', async () => {
          return await processUser(flow.last);
        })
        .action('send-notification', async () => {
          await sendUserNotification(user);
        });
    }
  );
```

### ForEach with Error Handling

```typescript
const forEachErrorWorkflow = cronflow.define({
  id: 'foreach-error-workflow',
  name: 'ForEach Error Workflow',
});

forEachErrorWorkflow
  .step('fetch-items', async ctx => {
    return await fetchItems(ctx.payload);
  })
  .forEach(
    'process-item',
    ctx => ctx.steps['fetch-items'].output,
    (item, flow) => {
      flow
        .step('process-item', async () => {
          try {
            return await processItem(item);
          } catch (error) {
            // Log error but continue processing other items
            await logItemError(item, error);
            return { failed: true, error: error.message };
          }
        })
        .action('update-progress', async () => {
          const processed = await flow.state.get('processed-count', 0);
          await flow.state.set('processed-count', processed + 1);
        });
    }
  );
```

### ForEach with Conditional Processing

```typescript
const conditionalForEachWorkflow = cronflow.define({
  id: 'conditional-foreach-workflow',
  name: 'Conditional ForEach Workflow',
});

conditionalForEachWorkflow
  .step('fetch-orders', async ctx => {
    return await fetchOrders(ctx.payload);
  })
  .forEach(
    'process-order',
    ctx => ctx.steps['fetch-orders'].output,
    (order, flow) => {
      flow
        .step('check-order-status', async () => {
          return await checkOrderStatus(order);
        })
        .if('order-needs-processing', ctx => ctx.last.needsProcessing)
        .step('process-order', async () => {
          return await processOrder(order);
        })
        .action('send-confirmation', async () => {
          await sendOrderConfirmation(order);
        })
        .else()
        .action('log-skipped-order', async () => {
          await logSkippedOrder(order);
        })
        .endIf();
    }
  );
```

## Advanced Batch Patterns

### 1. Batch Processing with Retry Logic

```typescript
const retryBatchWorkflow = cronflow.define({
  id: 'retry-batch-workflow',
  name: 'Retry Batch Workflow',
});

retryBatchWorkflow
  .step('fetch-data', async ctx => {
    return await fetchData(ctx.payload);
  })
  .batch(
    'retry-batch-processing',
    {
      items: ctx => ctx.steps['fetch-data'].output,
      size: 100,
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          let attempts = 0;
          const maxAttempts = 3;

          while (attempts < maxAttempts) {
            try {
              return await processBatch(batch);
            } catch (error) {
              attempts++;
              if (attempts >= maxAttempts) {
                throw error;
              }
              // Wait before retry
              await new Promise(resolve =>
                setTimeout(resolve, 1000 * attempts)
              );
            }
          }
        })
        .onError(ctx => {
          // Log failed batch
          await logFailedBatch(batch, ctx.error);
          return { failed: true, error: ctx.error.message };
        });
    }
  );
```

### 2. Batch Processing with Rate Limiting

```typescript
const rateLimitedBatchWorkflow = cronflow.define({
  id: 'rate-limited-batch-workflow',
  name: 'Rate Limited Batch Workflow',
});

rateLimitedBatchWorkflow
  .step('fetch-api-data', async ctx => {
    return await fetchApiData(ctx.payload);
  })
  .batch(
    'rate-limited-processing',
    {
      items: ctx => ctx.steps['fetch-api-data'].output,
      size: 10, // Small batches for rate limiting
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          return await processBatchWithRateLimit(batch);
        })
        .action('wait-between-batches', async () => {
          // Wait 1 second between batches to respect rate limits
          await new Promise(resolve => setTimeout(resolve, 1000));
        });
    }
  );
```

### 3. Batch Processing with Memory Management

```typescript
const memoryOptimizedBatchWorkflow = cronflow.define({
  id: 'memory-optimized-batch-workflow',
  name: 'Memory Optimized Batch Workflow',
});

memoryOptimizedBatchWorkflow
  .step('fetch-large-dataset', async ctx => {
    return await fetchLargeDataset(ctx.payload);
  })
  .batch(
    'memory-optimized-processing',
    {
      items: ctx => ctx.steps['fetch-large-dataset'].output,
      size: 25, // Small batches to manage memory
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          const result = await processBatch(batch);

          // Clear batch from memory after processing
          batch.length = 0;

          return result;
        })
        .action('garbage-collect', async () => {
          // Force garbage collection if available
          if (global.gc) {
            global.gc();
          }
        });
    }
  );
```

### 4. Batch Processing with Progress Persistence

```typescript
const persistentBatchWorkflow = cronflow.define({
  id: 'persistent-batch-workflow',
  name: 'Persistent Batch Workflow',
});

persistentBatchWorkflow
  .step('initialize-processing', async ctx => {
    const startTime = Date.now();
    await ctx.state.set('start-time', startTime);
    await ctx.state.set('processed-count', 0);
    await ctx.state.set('last-processed-index', -1);

    return await getDataToProcess(ctx.payload);
  })
  .batch(
    'persistent-batch-processing',
    {
      items: ctx => ctx.steps['initialize-processing'].output,
      size: 50,
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          const result = await processBatch(batch);

          // Update progress
          const processed = await flow.state.get('processed-count', 0);
          await flow.state.set('processed-count', processed + batch.length);

          // Update last processed index
          const lastIndex = await flow.state.get('last-processed-index', -1);
          await flow.state.set(
            'last-processed-index',
            lastIndex + batch.length
          );

          return result;
        })
        .action('save-progress', async () => {
          const progress = {
            processed: await flow.state.get('processed-count', 0),
            lastIndex: await flow.state.get('last-processed-index', -1),
            startTime: await flow.state.get('start-time', 0),
            currentTime: Date.now(),
          };

          await saveProgress(progress);
        });
    }
  );
```

## Best Practices

### 1. Choose Appropriate Batch Sizes

```typescript
// Small batches for memory-intensive operations
.batch('memory-intensive', {
  items: ctx => ctx.steps['fetch-data'].output,
  size: 25
}, (batch, flow) => { /* ... */ });

// Medium batches for balanced processing
.batch('balanced-processing', {
  items: ctx => ctx.steps['fetch-data'].output,
  size: 100
}, (batch, flow) => { /* ... */ });

// Large batches for simple operations
.batch('simple-processing', {
  items: ctx => ctx.steps['fetch-data'].output,
  size: 500
}, (batch, flow) => { /* ... */ });
```

### 2. Handle Errors Gracefully

```typescript
.batch('error-handling', {
  items: ctx => ctx.steps['fetch-data'].output,
  size: 100
}, (batch, flow) => {
  flow
    .step('process-batch', async () => {
      try {
        return await processBatch(batch);
      } catch (error) {
        // Log error but don't fail the entire workflow
        await logBatchError(batch, error);
        return { failed: true, error: error.message };
      }
    })
    .action('update-error-count', async () => {
      if (flow.last.failed) {
        const errorCount = await flow.state.get('error-count', 0);
        await flow.state.set('error-count', errorCount + 1);
      }
    });
});
```

### 3. Monitor Performance

```typescript
.batch('performance-monitoring', {
  items: ctx => ctx.steps['fetch-data'].output,
  size: 100
}, (batch, flow) => {
  flow
    .step('start-timer', async () => {
      return { startTime: Date.now() };
    })
    .step('process-batch', async () => {
      return await processBatch(batch);
    })
    .action('log-performance', async () => {
      const startTime = flow.steps['start-timer'].output.startTime;
      const duration = Date.now() - startTime;

      console.log(`Batch processed in ${duration}ms`);
      await logPerformance(duration, batch.length);
    });
});
```

### 4. Use State for Progress Tracking

```typescript
.batch('progress-tracking', {
  items: ctx => ctx.steps['fetch-data'].output,
  size: 50
}, (batch, flow) => {
  flow
    .step('process-batch', async () => {
      return await processBatch(batch);
    })
    .action('update-progress', async () => {
      const processed = await flow.state.get('processed-count', 0);
      const total = flow.steps['fetch-data'].output.length;
      const newProcessed = processed + batch.length;

      await flow.state.set('processed-count', newProcessed);

      const percentage = Math.round((newProcessed / total) * 100);
      console.log(`Progress: ${newProcessed}/${total} (${percentage}%)`);
    });
});
```

## Related Topics

- **[Error Handling](/guide/error-handling)** - Basic error handling and recovery
- **[Testing](/guide/testing)** - Testing workflows and components
- **[Performance](/guide/performance)** - Optimizing workflow performance
