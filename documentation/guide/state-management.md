# State Management

Cronflow provides powerful state management capabilities that allow you to store and retrieve data across workflow runs. State is persistent and can be shared between different workflows or isolated to specific workflows.

## State Types

Cronflow supports two types of state:

- **Global State** - Shared across all workflows
- **Workflow State** - Isolated to specific workflows

## Global State

Global state is shared across all workflows and is useful for storing application-wide data.

### Setting Global State

```typescript
import { cronflow } from 'cronflow';

// Set a simple value
await cronflow.setGlobalState('app.version', '1.0.0');

// Set with TTL (Time To Live)
await cronflow.setGlobalState(
  'temp.data',
  { value: 'temporary' },
  {
    ttl: '1h', // Expires in 1 hour
  }
);

// Set complex objects
await cronflow.setGlobalState('user.preferences', {
  theme: 'dark',
  language: 'en',
  notifications: true,
});
```

### Getting Global State

```typescript
// Get with default value
const version = await cronflow.getGlobalState('app.version', '0.0.0');

// Get complex data
const preferences = await cronflow.getGlobalState('user.preferences', {
  theme: 'light',
  language: 'en',
  notifications: false,
});

// Get without default (returns null if not found)
const data = await cronflow.getGlobalState('some.key');
```

### Incrementing Global State

```typescript
// Increment by 1
const count = await cronflow.incrGlobalState('counter', 1);

// Increment by custom amount
const total = await cronflow.incrGlobalState('total.orders', 5);
```

### Deleting Global State

```typescript
// Delete a specific key
const deleted = await cronflow.deleteGlobalState('temp.data');

// Check if deletion was successful
if (deleted) {
  console.log('Key deleted successfully');
}
```

## Workflow State

Workflow state is isolated to specific workflows and is useful for storing workflow-specific data.

### Setting Workflow State

```typescript
const workflow = cronflow.define({
  id: 'order-processor',
  name: 'Order Processing Workflow',
});

workflow.step('process-order', async ctx => {
  // Set workflow-specific state
  await cronflow.setWorkflowState(
    ctx.workflow_id,
    'last-processed-order',
    ctx.payload.orderId
  );

  // Set with TTL
  await cronflow.setWorkflowState(
    ctx.workflow_id,
    'temp.cache',
    { data: 'cached' },
    { ttl: '30m' }
  );

  return { processed: true };
});
```

### Getting Workflow State

```typescript
workflow.step('check-previous', async ctx => {
  // Get workflow state with default
  const lastOrder = await cronflow.getWorkflowState(
    ctx.workflow_id,
    'last-processed-order',
    'none'
  );

  // Get complex workflow state
  const config = await cronflow.getWorkflowState(
    ctx.workflow_id,
    'workflow.config',
    { retries: 3, timeout: '30s' }
  );

  return { lastOrder, config };
});
```

### Incrementing Workflow State

```typescript
workflow.step('track-processing', async ctx => {
  // Increment workflow counter
  const processedCount = await cronflow.incrWorkflowState(
    ctx.workflow_id,
    'processed.count',
    1
  );

  // Increment by custom amount
  const totalAmount = await cronflow.incrWorkflowState(
    ctx.workflow_id,
    'total.amount',
    ctx.payload.amount
  );

  return { processedCount, totalAmount };
});
```

### Deleting Workflow State

```typescript
workflow.step('cleanup', async ctx => {
  // Delete specific workflow state
  const deleted = await cronflow.deleteWorkflowState(
    ctx.workflow_id,
    'temp.cache'
  );

  if (deleted) {
    console.log('Workflow state cleaned up');
  }
});
```

## State in Context

You can also access state directly through the context object:

```typescript
workflow.step('use-state', async ctx => {
  // Set state
  await ctx.state.set('key', 'value');

  // Get state
  const value = await ctx.state.get('key', 'default');

  // Increment state
  const count = await ctx.state.incr('counter', 1);

  return { value, count };
});
```

## State with TTL (Time To Live)

State can be set with expiration times:

```typescript
// Global state with TTL
await cronflow.setGlobalState('session.data', sessionData, {
  ttl: '2h', // Expires in 2 hours
});

// Workflow state with TTL
await cronflow.setWorkflowState(
  workflowId,
  'temp.result',
  result,
  { ttl: '30m' } // Expires in 30 minutes
);
```

### TTL Formats

```typescript
// Time-based TTL
await cronflow.setGlobalState('key', 'value', { ttl: '30s' }); // 30 seconds
await cronflow.setGlobalState('key', 'value', { ttl: '5m' }); // 5 minutes
await cronflow.setGlobalState('key', 'value', { ttl: '2h' }); // 2 hours
await cronflow.setGlobalState('key', 'value', { ttl: '1d' }); // 1 day
await cronflow.setGlobalState('key', 'value', { ttl: '1w' }); // 1 week
```

## State Statistics

Get information about your state usage:

```typescript
// Get state statistics
const stats = await cronflow.getStateStats();

console.log('Global state stats:', stats.global);
console.log('Workflow state stats:', stats.workflows);

// Example output:
// {
//   global: {
//     totalKeys: 150,
//     expiredKeys: 25,
//     namespace: 'global',
//     dbPath: './cronflow.db'
//   },
//   workflows: {
//     'order-processor': {
//       totalKeys: 45,
//       expiredKeys: 10,
//       namespace: 'order-processor',
//       dbPath: './cronflow.db'
//     }
//   }
// }
```

## State Cleanup

Clean up expired state entries:

```typescript
// Clean up expired state
const cleanup = await cronflow.cleanupExpiredState();

console.log('Global cleanup:', cleanup.global); // Number of cleaned keys
console.log('Workflow cleanup:', cleanup.workflows); // Per-workflow cleanup counts
```

## State Patterns

### 1. Caching

```typescript
workflow.step('get-cached-data', async ctx => {
  // Try to get from cache first
  const cached = await cronflow.getWorkflowState(
    ctx.workflow_id,
    'cache.user-data',
    null
  );

  if (cached) {
    return cached; // Return cached data
  }

  // Fetch fresh data
  const freshData = await fetchUserData(ctx.payload.userId);

  // Cache for 1 hour
  await cronflow.setWorkflowState(
    ctx.workflow_id,
    'cache.user-data',
    freshData,
    { ttl: '1h' }
  );

  return freshData;
});
```

### 2. Rate Limiting

```typescript
workflow.step('check-rate-limit', async ctx => {
  const key = `rate-limit:${ctx.payload.userId}`;
  const current = await cronflow.getGlobalState(key, 0);

  if (current >= 100) {
    throw new Error('Rate limit exceeded');
  }

  // Increment counter
  await cronflow.incrGlobalState(key, 1);

  // Set expiration for rate limit window
  await cronflow.setGlobalState(key, current + 1, { ttl: '1h' });

  return { allowed: true };
});
```

### 3. Workflow Progress Tracking

```typescript
workflow.step('track-progress', async ctx => {
  const progress = await cronflow.getWorkflowState(
    ctx.workflow_id,
    'progress',
    { completed: 0, total: 0 }
  );

  progress.completed += 1;

  await cronflow.setWorkflowState(ctx.workflow_id, 'progress', progress);

  return { progress };
});
```

### 4. Cross-Workflow Communication

```typescript
// Workflow A: Producer
const producerWorkflow = cronflow.define({
  id: 'data-producer',
});

producerWorkflow.step('generate-data', async ctx => {
  const data = { timestamp: Date.now(), value: Math.random() };

  // Store in global state for other workflows
  await cronflow.setGlobalState('shared.data', data);

  return data;
});

// Workflow B: Consumer
const consumerWorkflow = cronflow.define({
  id: 'data-consumer',
});

consumerWorkflow.step('consume-data', async ctx => {
  // Read from global state
  const data = await cronflow.getGlobalState('shared.data');

  if (!data) {
    throw new Error('No data available');
  }

  return { consumed: data };
});
```

### 5. Configuration Management

```typescript
// Set application configuration
await cronflow.setGlobalState('app.config', {
  api: {
    baseUrl: 'https://api.example.com',
    timeout: 30000,
  },
  features: {
    notifications: true,
    analytics: false,
  },
  limits: {
    maxRetries: 3,
    rateLimit: 100,
  },
});

// Use in workflows
workflow.step('use-config', async ctx => {
  const config = await cronflow.getGlobalState('app.config');

  const response = await fetch(config.api.baseUrl + '/data', {
    timeout: config.api.timeout,
  });

  return { response: await response.json() };
});
```

## Best Practices

### 1. Use Appropriate State Types

```typescript
// ✅ Good: Use global state for shared data
await cronflow.setGlobalState('app.config', config);

// ✅ Good: Use workflow state for workflow-specific data
await cronflow.setWorkflowState(workflowId, 'progress', progress);

// ❌ Avoid: Don't use global state for temporary data
await cronflow.setGlobalState('temp.data', tempData);
```

### 2. Set TTL for Temporary Data

```typescript
// ✅ Good: Set TTL for temporary data
await cronflow.setGlobalState('session.data', sessionData, { ttl: '2h' });

// ❌ Avoid: Don't leave temporary data without expiration
await cronflow.setGlobalState('temp.data', tempData);
```

### 3. Use Descriptive Keys

```typescript
// ✅ Good: Descriptive keys
await cronflow.setGlobalState('user.preferences.theme', 'dark');
await cronflow.setWorkflowState(
  workflowId,
  'order.processing.status',
  'completed'
);

// ❌ Avoid: Generic keys
await cronflow.setGlobalState('data', value);
await cronflow.setWorkflowState(workflowId, 'status', status);
```

### 4. Handle State Errors

```typescript
workflow.step('safe-state-access', async ctx => {
  try {
    const data = await cronflow.getWorkflowState(
      ctx.workflow_id,
      'important.data',
      null
    );

    if (!data) {
      // Handle missing data
      return { status: 'no-data' };
    }

    return { status: 'success', data };
  } catch (error) {
    console.error('State access failed:', error);
    return { status: 'error', error: error.message };
  }
});
```

### 5. Clean Up Regularly

```typescript
// Set up periodic cleanup
setInterval(
  async () => {
    try {
      const cleanup = await cronflow.cleanupExpiredState();
      console.log('State cleanup completed:', cleanup);
    } catch (error) {
      console.error('State cleanup failed:', error);
    }
  },
  60 * 60 * 1000
); // Every hour
```

## API Reference

### Global State

- `cronflow.setGlobalState(key, value, options?)` - Set global state
- `cronflow.getGlobalState(key, defaultValue?)` - Get global state
- `cronflow.incrGlobalState(key, amount?)` - Increment global state
- `cronflow.deleteGlobalState(key)` - Delete global state

### Workflow State

- `cronflow.setWorkflowState(workflowId, key, value, options?)` - Set workflow state
- `cronflow.getWorkflowState(workflowId, key, defaultValue?)` - Get workflow state
- `cronflow.incrWorkflowState(workflowId, key, amount?)` - Increment workflow state
- `cronflow.deleteWorkflowState(workflowId, key)` - Delete workflow state

### State Management

- `cronflow.getStateStats()` - Get state statistics
- `cronflow.cleanupExpiredState()` - Clean up expired state

### Context State

- `ctx.state.set(key, value, options?)` - Set state in context
- `ctx.state.get(key, defaultValue?)` - Get state in context
- `ctx.state.incr(key, amount?)` - Increment state in context

State management is essential for building complex workflows that need to persist data across runs and share information between different parts of your application.
