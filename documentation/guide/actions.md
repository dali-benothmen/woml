# Actions

Actions are side-effect operations in Cronflow workflows that don't produce storable output. Unlike steps, which return data that can be used by subsequent steps, actions are designed for operations where the output is ignored or not needed.

## Key Differences: Steps vs Actions

| Feature       | Steps                            | Actions                              |
| ------------- | -------------------------------- | ------------------------------------ |
| **Output**    | Returns storable data            | Output is ignored                    |
| **Execution** | Synchronous by default           | Background execution                 |
| **Data Flow** | Can be referenced by other steps | Cannot be referenced                 |
| **Use Case**  | Data processing, calculations    | Notifications, logging, side effects |

## Creating Actions

Actions are created using the `.action()` method on workflow instances:

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'notification-workflow',
  name: 'Notification Workflow',
});

workflow
  .step('process-data', async ctx => {
    // This step returns data that can be used later
    return { processed: true, data: ctx.payload };
  })
  .action('send-notification', async ctx => {
    // This action performs a side effect, output is ignored
    await slack.sendMessage('#alerts', 'Data processed successfully');
  });
```

## Action Configuration

Actions support the same configuration options as steps:

```typescript
workflow
  .action('send-email', async ctx => {
    await emailService.send({
      to: ctx.payload.email,
      subject: 'Order Confirmation',
      body: `Order ${ctx.payload.orderId} has been processed.`,
    });
  })
  .retry({ attempts: 3, backoff: { delay: '1s' } })
  .timeout('30s');
```

## Background Execution

Actions are executed in the background by default, meaning the workflow continues to the next step immediately without waiting for the action to complete:

```typescript
workflow
  .step('process-order', async ctx => {
    return { orderId: '123', status: 'processed' };
  })
  .action('send-confirmation', async ctx => {
    // This runs in the background
    await emailService.sendConfirmation(ctx.last.orderId);
  })
  .step('update-inventory', async ctx => {
    // This step runs immediately, doesn't wait for the action
    return await inventoryService.update(ctx.last.orderId);
  });
```

## Common Action Use Cases

### 1. Notifications

```typescript
workflow
  .action('notify-slack', async ctx => {
    await slack.sendMessage(
      '#orders',
      `New order processed: ${ctx.last.orderId}`
    );
  })
  .action('send-email', async ctx => {
    await emailService.send({
      to: ctx.payload.customerEmail,
      subject: 'Order Confirmed',
      template: 'order-confirmation',
      data: ctx.last,
    });
  });
```

### 2. Logging

```typescript
workflow.action('log-event', async ctx => {
  console.log(`Workflow ${ctx.run.id} completed at ${new Date()}`);
  await logger.info('workflow.completed', {
    runId: ctx.run.id,
    workflowId: ctx.workflow_id,
    duration: Date.now() - ctx.run.started_at,
  });
});
```

### 3. External API Calls

```typescript
workflow.action('webhook-callback', async ctx => {
  await fetch('https://api.example.com/webhook', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      event: 'order.processed',
      data: ctx.last,
    }),
  });
});
```

### 4. Database Operations

```typescript
workflow.action('audit-log', async ctx => {
  await db.auditLogs.create({
    workflowId: ctx.workflow_id,
    runId: ctx.run.id,
    action: 'order.processed',
    data: ctx.last,
    timestamp: new Date(),
  });
});
```

## Action with Configuration

You can also create actions with configuration objects:

```typescript
workflow.action(
  {
    id: 'send-notification',
    title: 'Send Slack Notification',
    description: 'Sends a notification to the orders channel',
  },
  async ctx => {
    await slack.sendMessage(
      '#orders',
      `Order ${ctx.last.orderId} processed successfully`
    );
  }
);
```

## Error Handling in Actions

Actions can have their own error handling:

```typescript
workflow
  .action('send-notification', async ctx => {
    try {
      await slack.sendMessage('#orders', 'Order processed');
    } catch (error) {
      // Log the error but don't fail the workflow
      console.error('Failed to send notification:', error);
      await logger.error('notification.failed', { error: error.message });
    }
  })
  .retry({ attempts: 2, backoff: { delay: '5s' } });
```

## Action Hooks

Actions can trigger workflow hooks:

```typescript
const workflow = cronflow.define({
  id: 'notification-workflow',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (stepId === 'send-notification') {
        console.log('✅ Notification sent successfully');
      }
    },
    onFailure: (ctx, stepId) => {
      if (stepId === 'send-notification') {
        console.error('❌ Failed to send notification');
      }
    },
  },
});
```

## Best Practices

### 1. Use Actions for Side Effects

```typescript
// ✅ Good: Action for side effect
workflow.action('log-completion', async ctx => {
  await logger.info('workflow.completed', { runId: ctx.run.id });
});

// ❌ Avoid: Using action for data processing
workflow.action('calculate-total', async ctx => {
  return ctx.payload.items.reduce((sum, item) => sum + item.price, 0);
});
```

### 2. Handle Errors Gracefully

```typescript
workflow.action('send-notification', async ctx => {
  try {
    await notificationService.send(ctx.last);
  } catch (error) {
    // Don't throw - let the workflow continue
    console.error('Notification failed:', error);
  }
});
```

### 3. Use Descriptive Names

```typescript
// ✅ Good: Descriptive action names
workflow.action('send-order-confirmation-email', async ctx => {
  await emailService.sendOrderConfirmation(ctx.last);
});

// ❌ Avoid: Generic names
workflow.action('do-something', async ctx => {
  // Unclear what this does
});
```

### 4. Consider Performance

```typescript
// For heavy operations, consider using background actions
workflow.action(
  'generate-report',
  async ctx => {
    // This might take a while, but won't block the workflow
    await reportService.generate(ctx.last.orderId);
  },
  { background: true }
);
```

## Advanced Action Patterns

### Conditional Actions

```typescript
workflow
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .action('send-vip-notification', async ctx => {
    await slack.sendMessage(
      '#vip-orders',
      `VIP order: ${ctx.last.orderId} - $${ctx.last.amount}`
    );
  })
  .endIf();
```

### Parallel Actions

```typescript
workflow.parallel([
  async ctx => {
    await emailService.sendConfirmation(ctx.last);
  },
  async ctx => {
    await slack.sendNotification(ctx.last);
  },
  async ctx => {
    await smsService.sendUpdate(ctx.last);
  },
]);
```

## API Reference

### `workflow.action(name, handler, options?)`

Creates an action in the workflow.

**Parameters:**

- `name` (string | StepConfig): Action name or configuration
- `handler` (function): Action handler function
- `options` (StepOptions, optional): Action configuration

**Returns:** WorkflowInstance for chaining

**Example:**

```typescript
workflow.action(
  'send-notification',
  async ctx => {
    await notificationService.send(ctx.last);
  },
  {
    retry: { attempts: 3 },
    timeout: '30s',
  }
);
```

Actions are essential for building robust workflows that handle side effects without cluttering the main data flow. Use them for notifications, logging, external API calls, and other operations where the output isn't needed by subsequent steps.
