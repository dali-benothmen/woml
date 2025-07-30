# Hooks

Hooks in Cronflow allow you to respond to workflow lifecycle events and step execution outcomes. They provide a way to add monitoring, logging, and custom behavior without modifying the core workflow logic.

## Overview

Hooks are functions that are called at specific points during workflow execution:

- **onSuccess** - Called when a step completes successfully
- **onFailure** - Called when a step fails
- **onStart** - Called when a workflow starts
- **onComplete** - Called when a workflow completes
- **onError** - Called when a workflow encounters an error

## Basic Hook Usage

### Workflow-Level Hooks

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'order-processor',
  name: 'Order Processing Workflow',
  hooks: {
    onSuccess: (ctx, stepId) => {
      console.log(`✅ Step ${stepId} completed successfully`);
      console.log(`Run ID: ${ctx.run.id}`);
      console.log(`Output:`, ctx.last);
    },
    onFailure: (ctx, stepId) => {
      console.error(`❌ Step ${stepId} failed`);
      console.error(`Run ID: ${ctx.run.id}`);
      console.error(`Error:`, ctx.error);
    },
  },
});

workflow
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  })
  .step('send-notification', async ctx => {
    return await sendNotification(ctx.last);
  });
```

### Step-Level Hooks

```typescript
workflow
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  })
  .onError(async ctx => {
    // Custom error handling for this specific step
    await notifyAdmin({
      step: 'process-order',
      error: ctx.error,
      payload: ctx.payload,
    });

    // Return fallback data
    return { status: 'fallback', processed: false };
  });
```

## Hook Context

The context object passed to hooks contains comprehensive information about the workflow execution:

```typescript
interface HookContext {
  run_id: string;
  workflow_id: string;
  step_name: string;
  payload: any;
  steps: Record<string, any>;
  run: {
    id: string;
    workflowId: string;
    status: string;
    started_at: string;
    completed_at?: string;
    error?: string;
  };
  last: any; // Output from the last step
  state: {
    get: (key: string, defaultValue?: any) => Promise<any>;
    set: (key: string, value: any, options?: any) => Promise<void>;
    incr: (key: string, amount?: number) => Promise<number>;
  };
  trigger: {
    headers: Record<string, string>;
  };
  cancel: (reason?: string) => void;
}
```

## Hook Types

### onSuccess Hook

Called when a step completes successfully:

```typescript
const workflow = cronflow.define({
  id: 'monitored-workflow',
  hooks: {
    onSuccess: async (ctx, stepId) => {
      // Log successful step completion
      await logger.info('step.completed', {
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        stepId,
        output: ctx.last,
        timestamp: new Date().toISOString(),
      });

      // Send metrics
      await metrics.increment('workflow.step.success', {
        workflow: ctx.workflow_id,
        step: stepId,
      });

      // Update dashboard
      await dashboard.updateStepStatus({
        runId: ctx.run.id,
        stepId,
        status: 'completed',
        output: ctx.last,
      });
    },
  },
});
```

### onFailure Hook

Called when a step fails:

```typescript
const workflow = cronflow.define({
  id: 'resilient-workflow',
  hooks: {
    onFailure: async (ctx, stepId) => {
      // Log the failure
      await logger.error('step.failed', {
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        stepId,
        error: ctx.error,
        payload: ctx.payload,
        timestamp: new Date().toISOString(),
      });

      // Send alert
      await alertService.send({
        level: 'error',
        title: `Step ${stepId} failed`,
        message: ctx.error.message,
        workflow: ctx.workflow_id,
        runId: ctx.run.id,
      });

      // Update retry count
      const retryCount = await ctx.state.get(`retry.${stepId}`, 0);
      await ctx.state.set(`retry.${stepId}`, retryCount + 1);

      // Circuit breaker logic
      if (retryCount >= 3) {
        await ctx.state.set(`circuit.${stepId}`, {
          open: true,
          openedAt: Date.now(),
        });
      }
    },
  },
});
```

### onStart Hook

Called when a workflow starts:

```typescript
const workflow = cronflow.define({
  id: 'tracked-workflow',
  hooks: {
    onStart: async ctx => {
      // Initialize workflow tracking
      await ctx.state.set('workflow.started_at', Date.now());
      await ctx.state.set('workflow.status', 'running');

      // Log workflow start
      await logger.info('workflow.started', {
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        payload: ctx.payload,
        timestamp: new Date().toISOString(),
      });

      // Send metrics
      await metrics.increment('workflow.started', {
        workflow: ctx.workflow_id,
      });

      // Notify monitoring systems
      await monitoring.notifyWorkflowStart({
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        payload: ctx.payload,
      });
    },
  },
});
```

### onComplete Hook

Called when a workflow completes successfully:

```typescript
const workflow = cronflow.define({
  id: 'completion-tracked-workflow',
  hooks: {
    onComplete: async ctx => {
      const startTime = await ctx.state.get('workflow.started_at');
      const duration = Date.now() - startTime;

      // Update workflow status
      await ctx.state.set('workflow.status', 'completed');
      await ctx.state.set('workflow.completed_at', Date.now());
      await ctx.state.set('workflow.duration', duration);

      // Log completion
      await logger.info('workflow.completed', {
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        duration,
        steps: Object.keys(ctx.steps),
        timestamp: new Date().toISOString(),
      });

      // Send completion metrics
      await metrics.timing('workflow.duration', duration, {
        workflow: ctx.workflow_id,
      });

      // Clean up temporary data
      await cleanup.temporaryData(ctx.run.id);
    },
  },
});
```

### onError Hook

Called when a workflow encounters an error:

```typescript
const workflow = cronflow.define({
  id: 'error-handled-workflow',
  hooks: {
    onError: async ctx => {
      const startTime = await ctx.state.get('workflow.started_at');
      const duration = Date.now() - startTime;

      // Update workflow status
      await ctx.state.set('workflow.status', 'failed');
      await ctx.state.set('workflow.failed_at', Date.now());
      await ctx.state.set('workflow.error', ctx.error.message);

      // Log the error
      await logger.error('workflow.failed', {
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        error: ctx.error.message,
        duration,
        payload: ctx.payload,
        timestamp: new Date().toISOString(),
      });

      // Send critical alert
      await alertService.send({
        level: 'critical',
        title: `Workflow ${ctx.workflow_id} failed`,
        message: ctx.error.message,
        runId: ctx.run.id,
        duration,
      });

      // Attempt recovery
      await recovery.attemptRecovery({
        workflowId: ctx.workflow_id,
        runId: ctx.run.id,
        error: ctx.error,
      });
    },
  },
});
```

## Advanced Hook Patterns

### Conditional Hooks

```typescript
const workflow = cronflow.define({
  id: 'conditional-hooks-workflow',
  hooks: {
    onSuccess: async (ctx, stepId) => {
      // Only log for specific steps
      if (['process-order', 'send-notification'].includes(stepId)) {
        await logger.info(`Step ${stepId} completed`, {
          output: ctx.last,
          runId: ctx.run.id,
        });
      }

      // Only send alerts for critical steps
      if (stepId === 'process-payment') {
        await alertService.send({
          level: 'info',
          title: 'Payment processed successfully',
          message: `Payment for order ${ctx.last.orderId} completed`,
        });
      }
    },
  },
});
```

### Hook with State Management

```typescript
const workflow = cronflow.define({
  id: 'state-managed-workflow',
  hooks: {
    onSuccess: async (ctx, stepId) => {
      // Track step execution count
      const count = await ctx.state.incr(`steps.${stepId}.count`, 1);

      // Track total execution time
      const stepStartTime = await ctx.state.get(`steps.${stepId}.start_time`);
      if (stepStartTime) {
        const duration = Date.now() - stepStartTime;
        const totalTime = await ctx.state.get(`steps.${stepId}.total_time`, 0);
        await ctx.state.set(`steps.${stepId}.total_time`, totalTime + duration);
        await ctx.state.set(
          `steps.${stepId}.avg_time`,
          (totalTime + duration) / count
        );
      }

      // Store step output for debugging
      await ctx.state.set(`steps.${stepId}.last_output`, ctx.last);
      await ctx.state.set(`steps.${stepId}.last_success`, Date.now());
    },
    onFailure: async (ctx, stepId) => {
      // Track failure count
      const failureCount = await ctx.state.incr(`steps.${stepId}.failures`, 1);

      // Store error details
      await ctx.state.set(`steps.${stepId}.last_error`, {
        message: ctx.error.message,
        timestamp: Date.now(),
        payload: ctx.payload,
      });

      // Alert if too many failures
      if (failureCount >= 5) {
        await alertService.send({
          level: 'warning',
          title: `Step ${stepId} has ${failureCount} failures`,
          message: 'Consider investigating this step',
        });
      }
    },
  },
});
```

### Hook with External Integrations

```typescript
const workflow = cronflow.define({
  id: 'integrated-workflow',
  hooks: {
    onSuccess: async (ctx, stepId) => {
      // Send to Slack
      await slack.sendMessage(
        '#workflows',
        `✅ Step ${stepId} completed in workflow ${ctx.workflow_id}`
      );

      // Update Jira ticket
      if (ctx.payload.jiraTicket) {
        await jira.addComment(
          ctx.payload.jiraTicket,
          `Workflow step ${stepId} completed successfully`
        );
      }

      // Send to DataDog
      await datadog.increment('workflow.step.success', {
        workflow: ctx.workflow_id,
        step: stepId,
      });

      // Send to PagerDuty (for critical steps)
      if (['process-payment', 'ship-order'].includes(stepId)) {
        await pagerduty.trigger({
          title: `Critical step ${stepId} completed`,
          description: `Workflow ${ctx.workflow_id} step ${stepId} completed successfully`,
          severity: 'info',
        });
      }
    },
    onFailure: async (ctx, stepId) => {
      // Send to Slack
      await slack.sendMessage(
        '#workflow-alerts',
        `❌ Step ${stepId} failed in workflow ${ctx.workflow_id}: ${ctx.error.message}`
      );

      // Create Jira issue
      await jira.createIssue({
        summary: `Workflow step ${stepId} failed`,
        description: `Workflow: ${ctx.workflow_id}\nRun: ${ctx.run.id}\nError: ${ctx.error.message}`,
        priority: 'High',
      });

      // Send to PagerDuty
      await pagerduty.trigger({
        title: `Workflow step ${stepId} failed`,
        description: `Workflow ${ctx.workflow_id} step ${stepId} failed: ${ctx.error.message}`,
        severity: 'critical',
      });
    },
  },
});
```

### Hook with Performance Monitoring

```typescript
const workflow = cronflow.define({
  id: 'performance-monitored-workflow',
  hooks: {
    onStart: async ctx => {
      // Start performance timer
      await ctx.state.set('performance.start_time', process.hrtime.bigint());
      await ctx.state.set('performance.memory_start', process.memoryUsage());
    },
    onSuccess: async (ctx, stepId) => {
      // Measure step performance
      const startTime = await ctx.state.get(`steps.${stepId}.start_time`);
      if (startTime) {
        const duration = process.hrtime.bigint() - startTime;
        const durationMs = Number(duration) / 1000000;

        // Track performance metrics
        await metrics.timing('step.duration', durationMs, {
          workflow: ctx.workflow_id,
          step: stepId,
        });

        // Alert on slow steps
        if (durationMs > 5000) {
          await alertService.send({
            level: 'warning',
            title: `Slow step detected: ${stepId}`,
            message: `Step took ${durationMs.toFixed(2)}ms to complete`,
          });
        }
      }
    },
    onComplete: async ctx => {
      // Calculate total workflow performance
      const startTime = await ctx.state.get('performance.start_time');
      const memoryStart = await ctx.state.get('performance.memory_start');

      if (startTime && memoryStart) {
        const totalDuration = process.hrtime.bigint() - startTime;
        const totalDurationMs = Number(totalDuration) / 1000000;
        const memoryEnd = process.memoryUsage();
        const memoryDelta = {
          rss: memoryEnd.rss - memoryStart.rss,
          heapUsed: memoryEnd.heapUsed - memoryStart.heapUsed,
        };

        // Log performance summary
        await logger.info('workflow.performance', {
          workflowId: ctx.workflow_id,
          runId: ctx.run.id,
          totalDuration: totalDurationMs,
          memoryDelta,
          steps: Object.keys(ctx.steps).length,
        });

        // Send performance metrics
        await metrics.timing('workflow.total_duration', totalDurationMs, {
          workflow: ctx.workflow_id,
        });
        await metrics.gauge('workflow.memory_delta', memoryDelta.heapUsed, {
          workflow: ctx.workflow_id,
        });
      }
    },
  },
});
```

## Hook Best Practices

### 1. Keep Hooks Lightweight

```typescript
// ✅ Good: Lightweight hook
hooks: {
  onSuccess: async (ctx, stepId) => {
    await logger.info(`Step ${stepId} completed`);
  },
}

// ❌ Avoid: Heavy operations in hooks
hooks: {
  onSuccess: async (ctx, stepId) => {
    // Don't do heavy processing in hooks
    await heavyDataProcessing(ctx.last);
    await complexAnalytics(ctx);
    await multipleApiCalls(ctx);
  },
}
```

### 2. Handle Hook Errors Gracefully

```typescript
hooks: {
  onSuccess: async (ctx, stepId) => {
    try {
      await logger.info(`Step ${stepId} completed`);
      await metrics.increment('step.success');
    } catch (error) {
      // Don't let hook errors affect workflow execution
      console.error('Hook error:', error);
    }
  },
}
```

### 3. Use Appropriate Log Levels

```typescript
hooks: {
  onSuccess: async (ctx, stepId) => {
    await logger.info('step.completed', { stepId, runId: ctx.run.id });
  },
  onFailure: async (ctx, stepId) => {
    await logger.error('step.failed', {
      stepId,
      runId: ctx.run.id,
      error: ctx.error.message
    });
  },
}
```

### 4. Avoid Side Effects in Hooks

```typescript
// ✅ Good: Observational hooks
hooks: {
  onSuccess: async (ctx, stepId) => {
    await logger.info(`Step ${stepId} completed`);
    await metrics.increment('step.success');
  },
}

// ❌ Avoid: Modifying workflow state in hooks
hooks: {
  onSuccess: async (ctx, stepId) => {
    // Don't modify workflow state in hooks
    await ctx.state.set('workflow.modified_by_hook', true);
  },
}
```

### 5. Use Hooks for Cross-Cutting Concerns

```typescript
// ✅ Good: Use hooks for logging, monitoring, alerts
hooks: {
  onSuccess: async (ctx, stepId) => {
    await logger.info(`Step ${stepId} completed`);
    await metrics.increment('step.success');
    await alertService.notifySuccess(ctx, stepId);
  },
}

// ❌ Avoid: Business logic in hooks
hooks: {
  onSuccess: async (ctx, stepId) => {
    // Don't put business logic in hooks
    if (stepId === 'process-order') {
      await orderService.updateStatus(ctx.last.orderId, 'completed');
    }
  },
}
```

## API Reference

### Hook Types

- `onSuccess(ctx, stepId)` - Called when a step completes successfully
- `onFailure(ctx, stepId)` - Called when a step fails
- `onStart(ctx)` - Called when a workflow starts
- `onComplete(ctx)` - Called when a workflow completes successfully
- `onError(ctx)` - Called when a workflow encounters an error

### Hook Context Properties

- `ctx.run_id` - Unique run identifier
- `ctx.workflow_id` - Workflow identifier
- `ctx.step_name` - Current step name
- `ctx.payload` - Original workflow payload
- `ctx.steps` - Outputs from all completed steps
- `ctx.last` - Output from the last step
- `ctx.run` - Run metadata
- `ctx.state` - State management interface
- `ctx.error` - Error object (in failure hooks)

### Step-Level Hooks

```typescript
workflow
  .step('process-data', async ctx => {
    return await processData(ctx.payload);
  })
  .onError(async ctx => {
    // Handle step-specific errors
    return { status: 'fallback' };
  });
```

Hooks are powerful tools for adding observability, monitoring, and cross-cutting concerns to your workflows without cluttering the main business logic.
