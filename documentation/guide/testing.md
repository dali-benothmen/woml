# Testing

Cronflow provides comprehensive testing capabilities that allow you to test workflows in isolation, mock external dependencies, and validate workflow behavior.

## Overview

Cronflow offers two main testing approaches:

- **Unit Testing**: Test individual workflows in isolation
- **Integration Testing**: Test workflows with real dependencies

## Basic Testing

### Simple Workflow Testing

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'test-workflow',
  name: 'Test Workflow',
});

workflow
  .step('fetch-data', async ctx => {
    return await fetchData(ctx.payload.id);
  })
  .step('process-data', async ctx => {
    return await processData(ctx.last);
  });

// Test the workflow
const result = await workflow.test({
  payload: { id: 'test-123' },
});

console.log('Test result:', result);
// Output: { success: true, steps: { 'fetch-data': {...}, 'process-data': {...} } }
```

### Testing with Mock Data

```typescript
const testWorkflow = cronflow.define({
  id: 'mock-test-workflow',
  name: 'Mock Test Workflow',
});

testWorkflow
  .step('fetch-user', async ctx => {
    return await fetchUser(ctx.payload.userId);
  })
  .step('send-email', async ctx => {
    return await sendEmail(ctx.last.email, 'Welcome!');
  });

// Test with mocked step
const result = await testWorkflow.test({
  payload: { userId: 'user-123' },
  mockStep: {
    'fetch-user': {
      id: 'user-123',
      email: 'test@example.com',
      name: 'Test User',
    },
    'send-email': { sent: true, messageId: 'msg-456' },
  },
});

console.log('Mock test result:', result);
```

## Advanced Testing

### Advanced Test Configuration

```typescript
const advancedWorkflow = cronflow.define({
  id: 'advanced-test-workflow',
  name: 'Advanced Test Workflow',
});

advancedWorkflow
  .step('validate-input', async ctx => {
    if (!ctx.payload.email) {
      throw new Error('Email is required');
    }
    return { validated: true };
  })
  .step('process-user', async ctx => {
    return await processUser(ctx.payload);
  })
  .step('send-notification', async ctx => {
    return await sendNotification(ctx.last);
  });

// Advanced test with expectations
const result = await advancedWorkflow.advancedTest({
  payload: { email: 'test@example.com', name: 'Test User' },
  mockStep: {
    'process-user': { id: 'user-123', processed: true },
    'send-notification': { sent: true },
  },
  expectStep: {
    'validate-input': output => {
      if (!output.validated) {
        throw new Error('Validation step failed');
      }
    },
    'process-user': output => {
      if (!output.processed) {
        throw new Error('User processing failed');
      }
    },
  },
});

console.log('Advanced test result:', result);
```

### Testing Error Scenarios

```typescript
const errorWorkflow = cronflow.define({
  id: 'error-test-workflow',
  name: 'Error Test Workflow',
});

errorWorkflow
  .step('risky-operation', async ctx => {
    if (ctx.payload.shouldFail) {
      throw new Error('Intentional failure');
    }
    return { success: true };
  })
  .onError(ctx => {
    return { error: ctx.error.message, handled: true };
  });

// Test error handling
const errorResult = await errorWorkflow.test({
  payload: { shouldFail: true },
});

console.log('Error test result:', errorResult);
// Output: { success: true, steps: { 'risky-operation': { error: 'Intentional failure', handled: true } } }
```

## Testing Patterns

### 1. Testing Conditional Logic

```typescript
const conditionalWorkflow = cronflow.define({
  id: 'conditional-test-workflow',
  name: 'Conditional Test Workflow',
});

conditionalWorkflow
  .step('check-status', async ctx => {
    return { status: ctx.payload.status };
  })
  .if('is-active', ctx => ctx.last.status === 'active')
  .step('process-active', async ctx => {
    return { processed: true, type: 'active' };
  })
  .elseIf('is-pending', ctx => ctx.last.status === 'pending')
  .step('process-pending', async ctx => {
    return { processed: true, type: 'pending' };
  })
  .else()
  .step('process-inactive', async ctx => {
    return { processed: true, type: 'inactive' };
  })
  .endIf();

// Test different conditions
const activeResult = await conditionalWorkflow.test({
  payload: { status: 'active' },
});

const pendingResult = await conditionalWorkflow.test({
  payload: { status: 'pending' },
});

const inactiveResult = await conditionalWorkflow.test({
  payload: { status: 'inactive' },
});

console.log('Active test:', activeResult.steps['process-active']);
console.log('Pending test:', pendingResult.steps['process-pending']);
console.log('Inactive test:', inactiveResult.steps['process-inactive']);
```

### 2. Testing Parallel Execution

```typescript
const parallelWorkflow = cronflow.define({
  id: 'parallel-test-workflow',
  name: 'Parallel Test Workflow',
});

parallelWorkflow
  .parallel([
    async ctx => await fetchUserData(ctx.payload.userId),
    async ctx => await fetchOrderData(ctx.payload.orderId),
    async ctx => await fetchProductData(ctx.payload.productIds),
  ])
  .step('combine-results', async ctx => {
    const [user, order, products] = ctx.last;
    return { combined: true, user, order, products };
  });

// Test parallel execution
const parallelResult = await parallelWorkflow.test({
  payload: {
    userId: 'user-123',
    orderId: 'order-456',
    productIds: ['prod-1', 'prod-2'],
  },
  mockStep: {
    'parallel-0': { id: 'user-123', name: 'Test User' },
    'parallel-1': { id: 'order-456', amount: 100 },
    'parallel-2': [{ id: 'prod-1' }, { id: 'prod-2' }],
  },
});

console.log('Parallel test result:', parallelResult);
```

### 3. Testing State Management

```typescript
const stateWorkflow = cronflow.define({
  id: 'state-test-workflow',
  name: 'State Test Workflow',
});

stateWorkflow
  .step('increment-counter', async ctx => {
    const currentCount = await ctx.state.get('counter', 0);
    const newCount = currentCount + 1;
    await ctx.state.set('counter', newCount);
    return { count: newCount };
  })
  .step('check-counter', async ctx => {
    const totalCount = await ctx.state.get('counter', 0);
    return { totalCount };
  });

// Test state persistence
const result1 = await stateWorkflow.test({
  payload: { increment: 1 },
});

const result2 = await stateWorkflow.test({
  payload: { increment: 1 },
});

console.log('First run counter:', result1.steps['check-counter'].totalCount); // 1
console.log('Second run counter:', result2.steps['check-counter'].totalCount); // 2
```

### 4. Testing External Dependencies

```typescript
const externalWorkflow = cronflow.define({
  id: 'external-test-workflow',
  name: 'External Test Workflow',
});

externalWorkflow
  .step('call-external-api', async ctx => {
    return await externalApi.getUser(ctx.payload.userId);
  })
  .step('process-api-response', async ctx => {
    return await processApiResponse(ctx.last);
  });

// Mock external API calls
const externalResult = await externalWorkflow.test({
  payload: { userId: 'user-123' },
  mockStep: {
    'call-external-api': {
      id: 'user-123',
      name: 'Mock User',
      email: 'mock@example.com',
    },
    'process-api-response': {
      processed: true,
      user: { id: 'user-123', name: 'Mock User' },
    },
  },
});

console.log('External API test result:', externalResult);
```

## Integration Testing

### Testing with Real Dependencies

```typescript
const integrationWorkflow = cronflow.define({
  id: 'integration-test-workflow',
  name: 'Integration Test Workflow',
});

integrationWorkflow
  .step('create-user', async ctx => {
    return await db.users.create({
      data: ctx.payload,
    });
  })
  .step('send-welcome-email', async ctx => {
    return await emailService.sendWelcome(ctx.last.email);
  });

// Integration test with real database
const integrationResult = await integrationWorkflow.test({
  payload: {
    email: 'test@example.com',
    name: 'Integration Test User',
  },
});

console.log('Integration test result:', integrationResult);
```

### Testing Webhook Triggers

```typescript
const webhookWorkflow = cronflow.define({
  id: 'webhook-test-workflow',
  name: 'Webhook Test Workflow',
});

webhookWorkflow
  .onWebhook('/webhooks/test', {
    method: 'POST',
    schema: z.object({
      userId: z.string(),
      action: z.string(),
    }),
  })
  .step('process-webhook', async ctx => {
    return await processWebhook(ctx.payload);
  });

// Test webhook trigger
const webhookResult = await webhookWorkflow.test({
  payload: {
    userId: 'user-123',
    action: 'test-action',
  },
});

console.log('Webhook test result:', webhookResult);
```

## Testing Best Practices

### 1. Test Different Scenarios

```typescript
const comprehensiveWorkflow = cronflow.define({
  id: 'comprehensive-test-workflow',
  name: 'Comprehensive Test Workflow',
});

comprehensiveWorkflow
  .step('validate-input', async ctx => {
    if (!ctx.payload.required) {
      throw new Error('Required field missing');
    }
    return { validated: true };
  })
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  });

// Test success scenario
const successResult = await comprehensiveWorkflow.test({
  payload: { required: true, data: 'test' },
});

// Test failure scenario
const failureResult = await comprehensiveWorkflow.test({
  payload: { data: 'test' }, // Missing required field
});

console.log('Success test:', successResult.success); // true
console.log('Failure test:', failureResult.success); // false
```

### 2. Test Performance

```typescript
const performanceWorkflow = cronflow.define({
  id: 'performance-test-workflow',
  name: 'Performance Test Workflow',
});

performanceWorkflow.step('heavy-operation', async ctx => {
  const startTime = Date.now();
  const result = await heavyOperation(ctx.payload);
  const duration = Date.now() - startTime;

  if (duration > 5000) {
    throw new Error('Operation took too long');
  }

  return { result, duration };
});

// Test performance
const perfResult = await performanceWorkflow.test({
  payload: { data: 'performance-test' },
  mockStep: {
    'heavy-operation': { result: 'success', duration: 3000 },
  },
});

console.log('Performance test result:', perfResult);
```

### 3. Test Error Recovery

```typescript
const recoveryWorkflow = cronflow.define({
  id: 'recovery-test-workflow',
  name: 'Recovery Test Workflow',
});

recoveryWorkflow
  .step('risky-operation', async ctx => {
    if (ctx.payload.shouldFail) {
      throw new Error('Operation failed');
    }
    return { success: true };
  })
  .onError(ctx => {
    // Recovery logic
    return { recovered: true, error: ctx.error.message };
  });

// Test error recovery
const recoveryResult = await recoveryWorkflow.test({
  payload: { shouldFail: true },
});

console.log('Recovery test result:', recoveryResult);
```

### 4. Test State Transitions

```typescript
const stateTransitionWorkflow = cronflow.define({
  id: 'state-transition-test-workflow',
  name: 'State Transition Test Workflow',
});

stateTransitionWorkflow
  .step('initialize-state', async ctx => {
    await ctx.state.set('status', 'initialized');
    return { status: 'initialized' };
  })
  .step('process-state', async ctx => {
    const currentStatus = await ctx.state.get('status');
    await ctx.state.set('status', 'processing');
    return { previousStatus: currentStatus, currentStatus: 'processing' };
  })
  .step('finalize-state', async ctx => {
    await ctx.state.set('status', 'completed');
    return { status: 'completed' };
  });

// Test state transitions
const stateResult = await stateTransitionWorkflow.test({
  payload: { data: 'test' },
});

console.log('State transition test:', stateResult);
```

## Testing Utilities

### Benchmark Testing

```typescript
// Benchmark workflow performance
const benchmarkResult = await cronflow.benchmark('test-workflow', {
  iterations: 100,
  payload: { data: 'benchmark-test' },
});

console.log('Benchmark results:', benchmarkResult);
// Output: { averageTime: 150, minTime: 120, maxTime: 200, totalRuns: 100 }
```

### Test Coverage

```typescript
// Get test coverage information
const coverage = await workflow.getTestCoverage();

console.log('Test coverage:', coverage);
// Output: { steps: 5, tested: 4, coverage: 80 }
```

## What's Next?

- **[Background Actions](/guide/background-actions)** - How actions run in background
- **[Human-in-the-Loop](/guide/human-in-the-loop)** - Manual approval workflows
- **[Batch Processing](/guide/batch-processing)** - Process large datasets efficiently
- **[Advanced Control Flow](/guide/advanced-control-flow)** - Advanced workflow patterns
