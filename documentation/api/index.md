# API Reference

Welcome to the comprehensive Cronflow API reference. This documentation covers all the classes, methods, and types available in the Cronflow library, organized by functionality.

## API Structure

The Cronflow API is organized into the following categories:

### Core Functions

- **[Core Namespace](/api/core-namespace)** - Main entry point functions (define, start, stop, trigger, etc.)
- **[State Management](/api/state-management)** - Global and workflow-specific state management
- **[Workflow Execution](/api/workflow-execution)** - Step execution and replay functions
- **[Human-in-the-Loop](/api/human-in-the-loop)** - Approval and pause management
- **[Trigger Management](/api/trigger-management)** - Webhook, schedule, and manual triggers
- **[Event Management](/api/event-management)** - Custom event handling
- **[Hook Management](/api/hook-management)** - Workflow and step hooks
- **[Context Management](/api/context-management)** - Context object creation
- **[Core Status & Performance](/api/core-status-performance)** - Monitoring and benchmarking

### Workflow Instance Methods

- **[Workflow Instance Methods](/api/workflow-instance-methods)** - Step, action, retry, timeout, etc.
- **[Trigger Methods](/api/trigger-methods)** - onWebhook, onSchedule, onEvent, etc.
- **[Control Flow Methods](/api/control-flow-methods)** - if, parallel, race, while, etc.
- **[Advanced Control Flow](/api/advanced-control-flow)** - forEach, batch, humanInTheLoop, etc.
- **[Workflow Management](/api/workflow-management)** - Validation, metadata, registration
- **[Testing API](/api/testing-api)** - Test harnesses

### Reference

- **[Context Object](/api/context-object)** - Properties and state management
- **[Configuration](/api/configuration)** - Workflow and step options

## Quick Start

```typescript
import { cronflow } from 'cronflow';

// Define a workflow
const workflow = cronflow.define({
  id: 'my-workflow',
  name: 'My Workflow',
});

// Add steps
workflow
  .step('fetch-data', async ctx => {
    return await fetchData(ctx.payload);
  })
  .action('send-notification', async ctx => {
    await sendNotification(ctx.last);
  });

// Start the engine
await cronflow.start();

// Trigger the workflow
await cronflow.trigger('my-workflow', { data: 'example' });
```

## What's Next?

- **[Core Features](/guide/)** - Learn about workflows, steps, and triggers
- **[Examples](/examples/)** - Real-world workflow examples
- **[Installation](/guide/installation)** - Set up Cronflow in your project
