# Conditions

Conditions allow you to create dynamic workflows that branch based on data or logic. They enable complex decision-making within your workflows.

## Basic Conditions

Use `.if()` to create conditional execution paths:

```typescript
workflow
  .step('fetch-order', async ctx => {
    const order = await db.orders.findUnique({
      where: { id: ctx.payload.orderId },
    });
    return order;
  })
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .step('send-vip-notification', async ctx => {
    await slack.sendMessage('#vip', `VIP Order: $${ctx.last.amount}`);
    return { vipNotified: true };
  })
  .elseIf('is-medium-value', ctx => ctx.last.amount > 100)
  .step('send-standard-notification', async ctx => {
    await slack.sendMessage('#orders', `Order: $${ctx.last.amount}`);
    return { standardNotified: true };
  })
  .else()
  .step('log-basic-order', async ctx => {
    console.log(`Basic order: ${ctx.last.id}`);
    return { logged: true };
  })
  .endIf();
```

## Condition Types

### Primary Condition (`.if()`)

```typescript
workflow
  .if('condition-name', ctx => {
    // Return true/false or Promise<boolean>
    return ctx.last.amount > 100;
  })
  .step('handle-true-case', async ctx => {
    // This step runs if condition is true
    return { handled: true };
  })
  .endIf();
```

### Alternative Conditions (`.elseIf()`)

```typescript
workflow
  .if('is-premium', ctx => ctx.last.user.tier === 'premium')
  .step('premium-handling', async ctx => {
    return { premiumHandled: true };
  })
  .elseIf('is-standard', ctx => ctx.last.user.tier === 'standard')
  .step('standard-handling', async ctx => {
    return { standardHandled: true };
  })
  .elseIf('is-basic', ctx => ctx.last.user.tier === 'basic')
  .step('basic-handling', async ctx => {
    return { basicHandled: true };
  })
  .endIf();
```

### Default Case (`.else()`)

```typescript
workflow
  .if('has-payment', ctx => ctx.last.paymentMethod)
  .step('process-payment', async ctx => {
    return { paymentProcessed: true };
  })
  .else()
  .step('handle-no-payment', async ctx => {
    return { noPaymentHandled: true };
  })
  .endIf();
```

## Complex Conditions

### Multiple Conditions

```typescript
workflow
  .if('is-high-value-and-vip', ctx => {
    return ctx.last.amount > 500 && ctx.last.user.tier === 'vip';
  })
  .step('vip-high-value-handling', async ctx => {
    return { vipHighValueHandled: true };
  })
  .elseIf('is-high-value', ctx => ctx.last.amount > 500)
  .step('high-value-handling', async ctx => {
    return { highValueHandled: true };
  })
  .else()
  .step('standard-handling', async ctx => {
    return { standardHandled: true };
  })
  .endIf();
```

### Async Conditions

```typescript
workflow
  .if('user-has-permission', async ctx => {
    const user = await db.users.findUnique({
      where: { id: ctx.payload.userId },
      include: { permissions: true },
    });
    return user.permissions.includes('admin');
  })
  .step('admin-action', async ctx => {
    return { adminActionPerformed: true };
  })
  .else()
  .step('unauthorized-handling', async ctx => {
    return { unauthorizedHandled: true };
  })
  .endIf();
```

### Array and Object Conditions

```typescript
workflow
  .if('has-multiple-items', ctx => {
    return ctx.last.items && ctx.last.items.length > 1;
  })
  .step('bulk-processing', async ctx => {
    return { bulkProcessed: true };
  })
  .elseIf('has-single-item', ctx => {
    return ctx.last.items && ctx.last.items.length === 1;
  })
  .step('single-item-processing', async ctx => {
    return { singleItemProcessed: true };
  })
  .else()
  .step('no-items-handling', async ctx => {
    return { noItemsHandled: true };
  })
  .endIf();
```

## Nested Conditions

You can nest conditions to create complex decision trees:

```typescript
workflow
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .if('is-vip-customer', ctx => ctx.last.user.tier === 'vip')
  .step('vip-high-value-handling', async ctx => {
    return { vipHighValueHandled: true };
  })
  .else()
  .step('high-value-handling', async ctx => {
    return { highValueHandled: true };
  })
  .endIf()
  .elseIf('is-medium-value', ctx => ctx.last.amount > 100)
  .step('medium-value-handling', async ctx => {
    return { mediumValueHandled: true };
  })
  .else()
  .step('basic-handling', async ctx => {
    return { basicHandled: true };
  })
  .endIf();
```

## Condition with Data Access

Access data from previous steps in conditions:

```typescript
workflow
  .step('fetch-user', async ctx => {
    return await db.users.findUnique({
      where: { id: ctx.payload.userId },
    });
  })
  .step('fetch-order', async ctx => {
    return await db.orders.findUnique({
      where: { id: ctx.payload.orderId },
    });
  })
  .if('user-can-afford', ctx => {
    const user = ctx.steps['fetch-user'].output;
    const order = ctx.steps['fetch-order'].output;
    return user.balance >= order.amount;
  })
  .step('process-payment', async ctx => {
    return { paymentProcessed: true };
  })
  .else()
  .step('insufficient-funds', async ctx => {
    return { insufficientFunds: true };
  })
  .endIf();
```

## Advanced Control Flow

### Parallel Execution with Conditions

```typescript
workflow
  .if('needs-validation', ctx => ctx.payload.requiresValidation)
  .parallel([
    async ctx => {
      // Validate in parallel
      return await validateOrder(ctx.last);
    },
    async ctx => {
      // Check inventory in parallel
      return await checkInventory(ctx.last);
    },
  ])
  .step('process-validated-order', async ctx => {
    const [validation, inventory] = ctx.last;
    return { processed: true, validation, inventory };
  })
  .else()
  .step('process-simple-order', async ctx => {
    return { processed: true };
  })
  .endIf();
```

### Race Conditions

```typescript
workflow
  .if('use-backup-api', ctx => ctx.payload.useBackup)
  .race([
    async ctx => await primaryApi.call(),
    async ctx => await backupApi.call(),
  ])
  .step('process-result', async ctx => {
    // ctx.last contains result from whichever API responded first
    return { processed: true, result: ctx.last };
  })
  .else()
  .step('use-primary-api', async ctx => {
    return await primaryApi.call();
  })
  .endIf();
```

## Human-in-the-Loop Conditions

Pause workflows for human approval:

```typescript
workflow
  .if('requires-approval', ctx => ctx.last.amount > 1000)
  .humanInTheLoop({
    timeout: '24h',
    description: 'High-value transaction requires approval',
    onPause: (ctx, token) => {
      // Send approval request
      sendApprovalEmail(token, {
        amount: ctx.last.amount,
        customer: ctx.last.customerEmail,
      });
    },
  })
  .step('process-approved-transaction', async ctx => {
    if (ctx.last.approved) {
      return { transactionProcessed: true };
    } else {
      return { transactionRejected: true };
    }
  })
  .else()
  .step('process-automatic-transaction', async ctx => {
    return { transactionProcessed: true };
  })
  .endIf();
```

## While Loops

Create loops that execute while a condition is true:

```typescript
workflow
  .while(
    'process-queue',
    ctx => ctx.state.get('queue-size', 0) > 0,
    async ctx => {
      // Process one item from the queue
      const item = await ctx.state.get('next-item');
      await processItem(item);

      // Update queue size
      const currentSize = await ctx.state.get('queue-size', 0);
      await ctx.state.set('queue-size', currentSize - 1);
    }
  )
  .step('queue-completed', async ctx => {
    return { queueProcessed: true };
  });
```

## ForEach Loops

Process arrays with parallel execution:

```typescript
workflow
  .step('fetch-users', async ctx => {
    return await db.users.findMany({ where: { active: true } });
  })
  .forEach(
    'process-user',
    ctx => ctx.steps['fetch-users'].output,
    (user, flow) => {
      // This sub-workflow runs for each user in parallel
      flow
        .step('send-welcome-email', async () => {
          return await emailService.sendWelcomeEmail(user.email);
        })
        .step('update-user-status', async () => {
          return await db.users.update({
            where: { id: user.id },
            data: { welcomed: true },
          });
        });
    }
  );
```

## Batch Processing

Process large arrays in smaller batches:

```typescript
workflow
  .step('fetch-all-users', async ctx => {
    return await db.users.findMany();
  })
  .batch(
    'process-users-in-batches',
    {
      items: ctx => ctx.steps['fetch-all-users'].output,
      size: 100,
    },
    (batch, flow) => {
      flow
        .step('process-batch', async () => {
          return await processUserBatch(batch);
        })
        .log(ctx => `Processed batch of ${batch.length} users`);
    }
  );
```

## Best Practices

### 1. Use Descriptive Condition Names

```typescript
// Good
.if('is-high-value-order', ctx => ctx.last.amount > 500)
.if('user-has-admin-permissions', ctx => ctx.last.user.role === 'admin')

// Avoid
.if('condition1', ctx => ctx.last.amount > 500)
.if('check', ctx => ctx.last.user.role === 'admin')
```

### 2. Keep Conditions Simple

```typescript
// Good: Simple, readable conditions
.if('is-vip', ctx => ctx.last.user.tier === 'vip')
.if('has-payment', ctx => ctx.last.paymentMethod)

// Avoid: Complex conditions
.if('complex-condition', ctx => {
  return ctx.last.amount > 500 &&
         ctx.last.user.tier === 'vip' &&
         ctx.last.paymentMethod &&
         ctx.last.items.length > 0;
})
```

### 3. Use Early Returns for Complex Logic

```typescript
workflow
  .step('validate-order', async ctx => {
    const order = ctx.payload;

    if (order.amount > 1000) {
      return { requiresApproval: true, reason: 'high-value' };
    }

    if (!order.paymentMethod) {
      return { requiresApproval: true, reason: 'no-payment' };
    }

    return { requiresApproval: false };
  })
  .if('requires-approval', ctx => ctx.last.requiresApproval)
  .step('send-for-approval', async ctx => {
    return { sentForApproval: true };
  })
  .else()
  .step('process-automatically', async ctx => {
    return { processedAutomatically: true };
  })
  .endIf();
```

### 4. Handle Edge Cases

```typescript
workflow
  .if('has-data', ctx => ctx.last && ctx.last.length > 0)
  .step('process-data', async ctx => {
    return { processed: true };
  })
  .else()
  .step('handle-empty-data', async ctx => {
    return { emptyDataHandled: true };
  })
  .endIf();
```

## What's Next?

- **[Error Handling](/guide/error-handling)** - Build robust error handling
- **[Performance](/guide/performance)** - Optimize workflow performance
- **[Parallel Execution](/guide/parallel-execution)** - Run steps concurrently
- **[State Management](/guide/state-management)** - Manage workflow state
