# Introduction

## What is Cronflow?

**Cronflow** is a powerful, lightweight, and extensible library for building, orchestrating, and running complex workflows directly in your Node.js and TypeScript applications. It's designed for developers who want the power of platforms like n8n, Zapier, or Temporal.io, but with the flexibility, version control, and expressiveness of a code-first environment.

Built with **Rust + Bun** for unparalleled performance, Cronflow delivers enterprise-grade workflow automation with minimal infrastructure requirements.

## 🎯 Why Cronflow?

### Performance First

- **94x faster** than traditional workflow tools
- **0.5ms** average response time vs 27ms
- **True concurrency** with parallel execution
- **0.49MB** memory per step vs 5MB+

### Developer Experience

- **Code-first approach** - version control your workflows with Git
- **Full TypeScript support** - type-safe workflow development
- **Simple API** - intuitive and expressive
- **Zero infrastructure** - no servers, databases, or complex setup

### Production Ready

- **Built with Rust** for reliability and performance
- **Bun integration** for lightning-fast execution
- **Comprehensive error handling** and retry mechanisms
- **Extensible architecture** for custom steps and conditions

## Core Concepts

### Workflow Definition

A workflow is defined using the `cronflow.define()` method, which creates an isolated workflow instance with its own configuration, services, and hooks.

```typescript
import { cronflow } from 'cronflow';

const workflow = cronflow.define({
  id: 'order-processor',
  name: 'Order Processing Workflow',
  description: 'Processes incoming orders with payment validation',
  tags: ['ecommerce', 'critical'],
  concurrency: 10,
  timeout: '5m',
  hooks: {
    onSuccess: ctx => {
      console.log(`✅ Workflow ${ctx.run.id} completed successfully`);
    },
    onFailure: ctx => {
      console.error(`❌ Run ${ctx.run.id} failed!`, ctx.error);
    },
  },
});
```

### Triggers

Workflows can be triggered by various events:

```typescript
// Webhook trigger
workflow.onWebhook('/webhooks/stripe', {
  schema: z.object({
    orderId: z.string().uuid(),
    amount: z.number().positive(),
  }),
});

// Schedule trigger
workflow.onSchedule('0 2 * * *'); // Run at 2 AM daily

// Event trigger
workflow.onEvent('user.registered');

// Polling trigger
workflow.onPoll(async ctx => {
  const newEmails = await fetchNewEmails();
  return newEmails.map(email => ({
    id: email.id,
    payload: email,
  }));
});
```

### Steps and Actions

Steps are the primary units of work that produce storable output:

```typescript
workflow
  .step('fetch-user', async ctx => {
    const user = await db.users.findUnique({
      where: { id: ctx.payload.userId },
    });
    return user;
  })
  .step('process-order', async ctx => {
    const user = ctx.steps['fetch-user'].output;
    const order = await createOrder(user, ctx.payload);
    return order;
  });
```

Actions are for side-effects where output is ignored:

```typescript
workflow.action('send-notification', async ctx => {
  await slack.sendMessage(
    '#orders',
    `New order: ${ctx.steps['process-order'].output.id}`
  );
});
```

### Context Object

The context object (`ctx`) provides access to all workflow data and utilities:

```typescript
async function stepHandler(ctx) {
  // Access trigger payload
  const { userId, amount } = ctx.payload;

  // Access previous step outputs
  const userData = ctx.steps['fetch-user'].output;

  // Access run metadata
  console.log(`Run ID: ${ctx.run.id}`);

  // Use persistent state
  await ctx.state.set('last-processed', Date.now());
  const lastProcessed = await ctx.state.get('last-processed');

  // Convenience property for previous step
  const previousResult = ctx.last;

  return { processed: true };
}
```

### Control Flow

Cronflow provides powerful control flow mechanisms:

```typescript
workflow
  .if('is-high-value', ctx => ctx.last.amount > 500)
  .step('send-vip-notification', async ctx => {
    return await sendVIPNotification(ctx.last);
  })
  .elseIf('is-medium-value', ctx => ctx.last.amount > 100)
  .step('send-standard-notification', async ctx => {
    return await sendStandardNotification(ctx.last);
  })
  .else()
  .step('send-basic-notification', async ctx => {
    return await sendBasicNotification(ctx.last);
  })
  .endIf()
  .parallel([
    async ctx => await updateInventory(ctx.last),
    async ctx => await sendConfirmation(ctx.last),
  ]);
```

## Architecture

Cronflow is built with a modular, high-performance architecture:

```
┌─────────────────┐
│   Workflow      │  ← Main orchestrator
├─────────────────┤
│   Triggers      │  ← Webhooks, schedules, events
├─────────────────┤
│   Steps         │  ← Individual units of work
├─────────────────┤
│   Control Flow  │  ← Conditions, loops, parallel execution
├─────────────────┤
│   Context       │  ← State and data management
├─────────────────┤
│   Rust Core     │  ← High-performance engine
└─────────────────┘
```

## Key Features

### 🚀 Performance

- **Rust-powered core** for maximum performance
- **Bun integration** for fast JavaScript execution
- **Memory efficient** - minimal memory footprint
- **True concurrency** - parallel step execution

### 🔧 Developer Experience

- **TypeScript first** - full type safety
- **Intuitive API** - chainable workflow builder
- **Comprehensive error handling** - retry, fallback, and recovery
- **Rich logging** - structured logging with context

### 🎯 Production Ready

- **Zero dependencies** - minimal runtime overhead
- **Cross-platform** - works on Linux, macOS, Windows
- **Extensible** - custom steps and conditions
- **Testable** - easy to unit test workflows

### 🤖 Advanced Features

- **Human-in-the-Loop** - pause workflows for human approval
- **Event-driven** - trigger workflows on custom events
- **Polling** - automatically check for new data
- **Subflows** - modular workflow composition
- **Replay** - debug and fix failed workflows

## Performance Comparison

### 🏆 Cronflow vs Industry Leaders

| Feature            | Cronflow               | n8n                 | Make.com            | Zapier           | Temporal   |
| ------------------ | ---------------------- | ------------------- | ------------------- | ---------------- | ---------- |
| **Performance**    | ⚡ **98% faster**      | 🐌 Slow             | 🐌 Slow             | 🐌 Slow          | 🐌 Slow    |
| **Memory Usage**   | 💚 **90% less**        | ❌ High             | ❌ High             | ❌ High          | ❌ High    |
| **Type Safety**    | ✅ **Full TypeScript** | ❌ None             | ❌ None             | ❌ None          | ⚠️ Partial |
| **Code-First**     | ✅ **Native**          | ❌ Visual only      | ❌ Visual only      | ❌ Visual only   | ✅ Native  |
| **Testing**        | ✅ **Comprehensive**   | ❌ Limited          | ❌ Limited          | ❌ Limited       | ✅ Good    |
| **Deployment**     | ✅ **Single package**  | ❌ Complex          | ❌ Complex          | ❌ Cloud only    | ⚠️ Complex |
| **Hot Reload**     | ✅ **Instant**         | ❌ Restart required | ❌ Restart required | ❌ Not available | ⚠️ Limited |
| **Error Handling** | ✅ **Circuit Breaker** | ❌ Basic            | ❌ Basic            | ❌ Basic         | ✅ Good    |
| **Monitoring**     | ✅ **Built-in**        | ❌ External         | ❌ External         | ❌ External      | ✅ Good    |

## Getting Started

Ready to build your first workflow? Check out the [Installation Guide](/guide/installation) to get Cronflow up and running, or jump straight to the [Quick Start](/guide/quick-start) to build your first workflow in minutes.

## What's Next?

- **[Installation](/guide/installation)** - Set up Cronflow in your project
- **[Quick Start](/guide/quick-start)** - Build your first workflow
- **[Basic Concepts](/guide/basic-concepts)** - Learn about workflows, steps, and context
- **[API Reference](/api/)** - Complete API documentation
- **[Examples](/examples/)** - Real-world workflow examples
