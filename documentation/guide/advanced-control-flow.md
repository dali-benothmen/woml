# Advanced Control Flow

Cronflow provides advanced control flow capabilities that allow you to create complex, dynamic workflows with loops, subflows, and sophisticated branching logic.

## Overview

Advanced control flow features include:

- **While Loops**: Execute steps repeatedly until a condition is met
- **Subflows**: Call other workflows as child processes
- **Sleep**: Pause workflow execution for a specified duration
- **Wait for Events**: Pause until specific events occur
- **Complex Branching**: Advanced conditional logic

## While Loops

### Basic While Loop

```typescript
const whileWorkflow = cronflow.define({
  id: 'while-workflow',
  name: 'While Loop Workflow',
});

whileWorkflow
  .step('initialize-counter', async ctx => {
    await ctx.state.set('counter', 0);
    return { initialized: true };
  })
  .while(
    'process-until-complete',
    ctx => {
      const counter = ctx.state.get('counter', 0);
      return counter < 10; // Continue while counter is less than 10
    },
    async ctx => {
      const currentCounter = await ctx.state.get('counter', 0);
      const newCounter = currentCounter + 1;
      await ctx.state.set('counter', newCounter);

      console.log(`Processing iteration ${newCounter}`);
      return { iteration: newCounter };
    }
  )
  .step('finalize', async ctx => {
    const finalCounter = await ctx.state.get('counter', 0);
    return { completed: true, totalIterations: finalCounter };
  });
```

### While Loop with External Condition

```typescript
const externalWhileWorkflow = cronflow.define({
  id: 'external-while-workflow',
  name: 'External While Workflow',
});

externalWhileWorkflow
  .step('fetch-queue', async ctx => {
    return await fetchQueue(ctx.payload.queueId);
  })
  .while(
    'process-queue',
    ctx => {
      const queue = ctx.steps['fetch-queue'].output;
      return queue.length > 0; // Continue while queue has items
    },
    async ctx => {
      const queue = ctx.steps['fetch-queue'].output;
      const item = queue.shift(); // Process first item

      await processQueueItem(item);
      await updateQueue(ctx.payload.queueId, queue);

      return { processed: item.id };
    }
  )
  .step('queue-completed', async ctx => {
    return { status: 'completed', message: 'All items processed' };
  });
```

### While Loop with State Management

```typescript
const statefulWhileWorkflow = cronflow.define({
  id: 'stateful-while-workflow',
  name: 'Stateful While Workflow',
});

statefulWhileWorkflow
  .step('initialize-processing', async ctx => {
    await ctx.state.set('processed-count', 0);
    await ctx.state.set('failed-count', 0);
    await ctx.state.set('total-items', ctx.payload.items.length);

    return { items: ctx.payload.items };
  })
  .while(
    'process-all-items',
    ctx => {
      const processed = ctx.state.get('processed-count', 0);
      const total = ctx.state.get('total-items', 0);
      return processed < total;
    },
    async ctx => {
      const items = ctx.steps['initialize-processing'].output.items;
      const processed = await ctx.state.get('processed-count', 0);
      const currentItem = items[processed];

      try {
        await processItem(currentItem);
        await ctx.state.set('processed-count', processed + 1);
        return { success: true, itemId: currentItem.id };
      } catch (error) {
        const failed = await ctx.state.get('failed-count', 0);
        await ctx.state.set('failed-count', failed + 1);
        return { success: false, error: error.message };
      }
    }
  )
  .step('finalize-processing', async ctx => {
    const processed = await ctx.state.get('processed-count', 0);
    const failed = await ctx.state.get('failed-count', 0);

    return {
      totalProcessed: processed,
      totalFailed: failed,
      successRate: ((processed - failed) / processed) * 100,
    };
  });
```

## Subflows

### Basic Subflow

```typescript
const parentWorkflow = cronflow.define({
  id: 'parent-workflow',
  name: 'Parent Workflow',
});

const childWorkflow = cronflow.define({
  id: 'child-workflow',
  name: 'Child Workflow',
});

childWorkflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .step('validate-result', async ctx => {
    return await validateResult(ctx.last);
  });

parentWorkflow
  .step('prepare-data', async ctx => {
    return await prepareData(ctx.payload);
  })
  .subflow('process-with-child', 'child-workflow', ctx => ({
    data: ctx.last,
    metadata: ctx.payload.metadata,
  }))
  .step('finalize', async ctx => {
    const childResult = ctx.last;
    return await finalizeProcess(childResult);
  });
```

### Subflow with Error Handling

```typescript
const robustParentWorkflow = cronflow.define({
  id: 'robust-parent-workflow',
  name: 'Robust Parent Workflow',
});

const robustChildWorkflow = cronflow.define({
  id: 'robust-child-workflow',
  name: 'Robust Child Workflow',
});

robustChildWorkflow
  .step('risky-operation', async ctx => {
    if (ctx.payload.shouldFail) {
      throw new Error('Child workflow failed');
    }
    return await performOperation(ctx.payload);
  })
  .onError(ctx => {
    return { error: ctx.error.message, handled: true };
  });

robustParentWorkflow
  .step('prepare-operation', async ctx => {
    return await prepareOperation(ctx.payload);
  })
  .subflow('execute-child', 'robust-child-workflow', ctx => ({
    data: ctx.last,
    shouldFail: ctx.payload.shouldFail,
  }))
  .if('child-succeeded', ctx => !ctx.last.error)
  .step('process-success', async ctx => {
    return await processSuccess(ctx.last);
  })
  .else()
  .step('handle-child-failure', async ctx => {
    return await handleChildFailure(ctx.last);
  })
  .endIf();
```

### Subflow with State Sharing

```typescript
const statefulParentWorkflow = cronflow.define({
  id: 'stateful-parent-workflow',
  name: 'Stateful Parent Workflow',
});

const statefulChildWorkflow = cronflow.define({
  id: 'stateful-child-workflow',
  name: 'Stateful Child Workflow',
});

statefulChildWorkflow.step('process-with-state', async ctx => {
  const parentState = await ctx.state.get('parent-data');
  const result = await processWithState(ctx.payload, parentState);

  // Share state back to parent
  await ctx.state.set('child-result', result);

  return result;
});

statefulParentWorkflow
  .step('initialize-state', async ctx => {
    await ctx.state.set('parent-data', ctx.payload);
    return { initialized: true };
  })
  .subflow('execute-child', 'stateful-child-workflow', ctx => ({
    data: ctx.last,
    parentState: ctx.state.get('parent-data'),
  }))
  .step('process-child-result', async ctx => {
    const childResult = ctx.last;
    const childState = await ctx.state.get('child-result');

    return await processChildResult(childResult, childState);
  });
```

## Sleep and Delays

### Basic Sleep

```typescript
const sleepWorkflow = cronflow.define({
  id: 'sleep-workflow',
  name: 'Sleep Workflow',
});

sleepWorkflow
  .step('start-process', async ctx => {
    return await startProcess(ctx.payload);
  })
  .sleep('5s') // Sleep for 5 seconds
  .step('check-status', async ctx => {
    return await checkStatus(ctx.last.id);
  })
  .if('still-processing', ctx => ctx.last.status === 'processing')
  .sleep('10s') // Sleep for 10 more seconds
  .step('check-status-again', async ctx => {
    return await checkStatus(ctx.steps['start-process'].output.id);
  })
  .endIf();
```

### Dynamic Sleep

```typescript
const dynamicSleepWorkflow = cronflow.define({
  id: 'dynamic-sleep-workflow',
  name: 'Dynamic Sleep Workflow',
});

dynamicSleepWorkflow
  .step('calculate-delay', async ctx => {
    const delay = ctx.payload.priority === 'high' ? '2s' : '10s';
    return { delay };
  })
  .sleep(ctx => ctx.last.delay)
  .step('process-after-delay', async ctx => {
    return await processAfterDelay(ctx.payload);
  });
```

### Sleep with Retry Logic

```typescript
const retrySleepWorkflow = cronflow.define({
  id: 'retry-sleep-workflow',
  name: 'Retry Sleep Workflow',
});

retrySleepWorkflow
  .step('attempt-operation', async ctx => {
    return await attemptOperation(ctx.payload);
  })
  .if('operation-failed', ctx => ctx.last.status === 'failed')
  .sleep('5s')
  .step('retry-operation', async ctx => {
    return await retryOperation(ctx.steps['attempt-operation'].output);
  })
  .if('retry-failed', ctx => ctx.last.status === 'failed')
  .sleep('15s')
  .step('final-retry', async ctx => {
    return await finalRetry(ctx.steps['attempt-operation'].output);
  })
  .endIf()
  .endIf();
```

## Wait for Events

### Basic Event Waiting

```typescript
const eventWorkflow = cronflow.define({
  id: 'event-workflow',
  name: 'Event Workflow',
});

eventWorkflow
  .step('start-process', async ctx => {
    return await startProcess(ctx.payload);
  })
  .waitForEvent('process.completed', '30s') // Wait for event with timeout
  .step('process-completed', async ctx => {
    const eventData = ctx.last;
    return await handleProcessCompletion(eventData);
  });
```

### Event Waiting with Custom Logic

```typescript
const customEventWorkflow = cronflow.define({
  id: 'custom-event-workflow',
  name: 'Custom Event Workflow',
});

customEventWorkflow
  .step('submit-job', async ctx => {
    return await submitJob(ctx.payload);
  })
  .waitForEvent('job.completed', '5m', {
    filter: event => event.jobId === ctx.steps['submit-job'].output.id,
    validate: event => event.status === 'success',
  })
  .step('handle-job-completion', async ctx => {
    return await handleJobCompletion(ctx.last);
  });
```

## Advanced Control Flow Patterns

### 1. Complex Conditional Loops

```typescript
const complexLoopWorkflow = cronflow.define({
  id: 'complex-loop-workflow',
  name: 'Complex Loop Workflow',
});

complexLoopWorkflow
  .step('initialize', async ctx => {
    await ctx.state.set('attempts', 0);
    await ctx.state.set('backoff-delay', 1000);
    return { initialized: true };
  })
  .while(
    'retry-with-backoff',
    ctx => {
      const attempts = ctx.state.get('attempts', 0);
      const maxAttempts = 5;
      return attempts < maxAttempts;
    },
    async ctx => {
      const attempts = await ctx.state.get('attempts', 0);
      const backoffDelay = await ctx.state.get('backoff-delay', 1000);

      try {
        const result = await performOperation(ctx.payload);
        return { success: true, result };
      } catch (error) {
        await ctx.state.set('attempts', attempts + 1);
        await ctx.state.set('backoff-delay', backoffDelay * 2);

        if (attempts < 4) {
          await ctx.sleep(`${backoffDelay}ms`);
        }

        return { success: false, error: error.message };
      }
    }
  )
  .step('finalize', async ctx => {
    const attempts = await ctx.state.get('attempts', 0);
    return { totalAttempts: attempts, finalResult: ctx.last };
  });
```

### 2. Nested Subflows

```typescript
const nestedWorkflow = cronflow.define({
  id: 'nested-workflow',
  name: 'Nested Workflow',
});

const level1Workflow = cronflow.define({
  id: 'level1-workflow',
  name: 'Level 1 Workflow',
});

const level2Workflow = cronflow.define({
  id: 'level2-workflow',
  name: 'Level 2 Workflow',
});

level2Workflow.step('deep-process', async ctx => {
  return await deepProcess(ctx.payload);
});

level1Workflow
  .step('level1-process', async ctx => {
    return await level1Process(ctx.payload);
  })
  .subflow('call-level2', 'level2-workflow', ctx => ({
    data: ctx.last,
  }));

nestedWorkflow
  .step('prepare-data', async ctx => {
    return await prepareData(ctx.payload);
  })
  .subflow('call-level1', 'level1-workflow', ctx => ({
    data: ctx.last,
  }))
  .step('finalize-nested', async ctx => {
    return await finalizeNested(ctx.last);
  });
```

### 3. Event-Driven Loops

```typescript
const eventDrivenWorkflow = cronflow.define({
  id: 'event-driven-workflow',
  name: 'Event-Driven Workflow',
});

eventDrivenWorkflow
  .step('start-monitoring', async ctx => {
    await ctx.state.set('monitoring', true);
    return { started: true };
  })
  .while(
    'monitor-events',
    ctx => {
      return ctx.state.get('monitoring', false);
    },
    async ctx => {
      // Wait for events and process them
      const event = await ctx.waitForEvent('data.available', '1m');

      if (event) {
        await processEvent(event);
        return { processed: event.id };
      } else {
        // No event received, continue monitoring
        return { noEvent: true };
      }
    }
  )
  .step('stop-monitoring', async ctx => {
    await ctx.state.set('monitoring', false);
    return { stopped: true };
  });
```

### 4. State Machine Pattern

```typescript
const stateMachineWorkflow = cronflow.define({
  id: 'state-machine-workflow',
  name: 'State Machine Workflow',
});

stateMachineWorkflow
  .step('initialize', async ctx => {
    await ctx.state.set('current-state', 'initialized');
    return { status: 'initialized' };
  })
  .while(
    'state-machine-loop',
    ctx => {
      const currentState = ctx.state.get('current-state');
      return currentState !== 'completed' && currentState !== 'failed';
    },
    async ctx => {
      const currentState = await ctx.state.get('current-state');

      switch (currentState) {
        case 'initialized':
          await ctx.state.set('current-state', 'processing');
          return await startProcessing(ctx.payload);

        case 'processing':
          const result = await checkProcessingStatus();
          if (result.completed) {
            await ctx.state.set('current-state', 'completed');
          } else if (result.failed) {
            await ctx.state.set('current-state', 'failed');
          }
          return result;

        default:
          await ctx.sleep('1s');
          return { waiting: true };
      }
    }
  )
  .step('finalize-state-machine', async ctx => {
    const finalState = await ctx.state.get('current-state');
    return { finalState, completed: true };
  });
```

## Best Practices

### 1. Avoid Infinite Loops

```typescript
// Good: Always have a termination condition
.while('safe-loop', ctx => {
  const attempts = ctx.state.get('attempts', 0);
  return attempts < 10; // Clear termination condition
}, async ctx => {
  // Loop logic
});

// Avoid: No clear termination condition
.while('dangerous-loop', ctx => {
  return true; // Could run forever
}, async ctx => {
  // Loop logic
});
```

### 2. Use State for Loop Control

```typescript
.while('state-controlled-loop', ctx => {
  const processed = ctx.state.get('processed-count', 0);
  const total = ctx.state.get('total-items', 0);
  return processed < total;
}, async ctx => {
  // Process items
  const processed = await ctx.state.get('processed-count', 0);
  await ctx.state.set('processed-count', processed + 1);
});
```

### 3. Handle Subflow Errors

```typescript
.subflow('safe-subflow', 'child-workflow', ctx => ({
  data: ctx.last
}))
.onError(ctx => {
  // Handle subflow errors gracefully
  return { subflowError: ctx.error.message, fallback: true };
});
```

### 4. Use Appropriate Timeouts

```typescript
// Short timeout for quick operations
.waitForEvent('quick.event', '30s')

// Medium timeout for normal operations
.waitForEvent('normal.event', '5m')

// Long timeout for slow operations
.waitForEvent('slow.event', '1h')
```

## Related Topics

- **[Error Handling](/guide/error-handling)** - Basic error handling and recovery
- **[Testing](/guide/testing)** - Testing workflows and components
- **[Performance](/guide/performance)** - Optimizing workflow performance
