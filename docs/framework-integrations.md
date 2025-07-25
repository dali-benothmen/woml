# Cronflow Framework Integrations

Cronflow is designed to work seamlessly with any Node.js/Bun.js web framework. This document shows how to integrate it with popular frameworks.

## 🚀 **Framework Compatibility**

Cronflow works with:
- ✅ **Express.js** - Most popular Node.js framework
- ✅ **Fastify** - High-performance framework
- ✅ **Koa** - Lightweight framework by Express team
- ✅ **Hapi** - Enterprise-grade framework
- ✅ **Bun.js HTTP Server** - Built-in Bun server
- ✅ **Next.js** - React framework
- ✅ **NestJS** - Enterprise Node.js framework
- ✅ **Any custom HTTP server**

## 📋 **Integration Patterns**

### **1. Basic Integration Pattern**

```typescript
import { cronflow } from 'cronflow';

// 1. Define workflows during app initialization
async function initializeWorkflows() {
  const workflow = cronflow.define({
    id: 'my-workflow',
    name: 'My Workflow',
    description: 'Description',
  });

  workflow
    .step('step1', async (ctx) => {
      // Step logic
      return { result: 'success' };
    })
    .step('step2', async (ctx) => {
      // Step logic using ctx.last from previous step
      return { processed: ctx.last.result };
    });

  // Start the engine
  await cronflow.start();
}

// 2. Create API endpoints to trigger workflows
app.post('/api/trigger-workflow', async (req, res) => {
  try {
    const runId = await cronflow.trigger('my-workflow', req.body);
    res.json({ success: true, runId });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// 3. Create endpoints to check workflow status
app.get('/api/workflows/:runId', async (req, res) => {
  try {
    const status = await cronflow.inspect(req.params.runId);
    res.json({ success: true, status });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});
```

## 🔧 **Framework-Specific Examples**

### **Express.js Integration**

```typescript
import express from 'express';
import { cronflow } from 'cronflow';

const app = express();
app.use(express.json());

// Initialize workflows
async function initializeWorkflows() {
  const userWorkflow = cronflow.define({
    id: 'user-registration',
    name: 'User Registration',
  });

  userWorkflow
    .step('validate', async (ctx) => {
      // Validation logic
      return { validated: true };
    })
    .step('create', async (ctx) => {
      // User creation logic
      return { userId: 'user_123' };
    });

  await cronflow.start();
}

// API routes
app.post('/api/users', async (req, res) => {
  try {
    const runId = await cronflow.trigger('user-registration', req.body);
    res.json({ success: true, runId });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

app.get('/api/workflows/:runId', async (req, res) => {
  try {
    const status = await cronflow.inspect(req.params.runId);
    res.json({ success: true, status });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});

// Start server
initializeWorkflows().then(() => {
  app.listen(3000, () => {
    console.log('Server running on port 3000');
  });
});
```

### **Fastify Integration**

```typescript
import Fastify from 'fastify';
import { cronflow } from 'cronflow';

const fastify = Fastify({ logger: true });

// Initialize workflows
async function initializeWorkflows() {
  const orderWorkflow = cronflow.define({
    id: 'order-processing',
    name: 'Order Processing',
  });

  orderWorkflow
    .step('validate-order', async (ctx) => {
      return { orderId: 'order_123' };
    })
    .step('process-payment', async (ctx) => {
      return { paymentId: 'payment_456' };
    });

  await cronflow.start();
}

// API routes with schema validation
fastify.post('/api/orders', {
  schema: {
    body: {
      type: 'object',
      properties: {
        items: { type: 'array' },
        customerId: { type: 'string' },
      },
      required: ['items', 'customerId'],
    },
  },
}, async (request, reply) => {
  try {
    const runId = await cronflow.trigger('order-processing', request.body);
    return { success: true, runId };
  } catch (error) {
    reply.status(500);
    return { error: error.message };
  }
});

// Start server
initializeWorkflows().then(async () => {
  await fastify.listen({ port: 3000 });
});
```

### **Koa Integration**

```typescript
import Koa from 'koa';
import Router from '@koa/router';
import bodyParser from 'koa-bodyparser';
import { cronflow } from 'cronflow';

const app = new Koa();
const router = new Router();

app.use(bodyParser());

// Initialize workflows
async function initializeWorkflows() {
  const emailWorkflow = cronflow.define({
    id: 'email-campaign',
    name: 'Email Campaign',
  });

  emailWorkflow
    .step('prepare-content', async (ctx) => {
      return { content: 'Email content' };
    })
    .step('send-emails', async (ctx) => {
      return { sent: 1000 };
    });

  await cronflow.start();
}

// API routes
router.post('/api/campaigns', async (ctx) => {
  try {
    const runId = await cronflow.trigger('email-campaign', ctx.request.body);
    ctx.body = { success: true, runId };
  } catch (error) {
    ctx.status = 500;
    ctx.body = { error: error.message };
  }
});

router.get('/api/workflows/:runId', async (ctx) => {
  try {
    const status = await cronflow.inspect(ctx.params.runId);
    ctx.body = { success: true, status };
  } catch (error) {
    ctx.status = 500;
    ctx.body = { error: error.message };
  }
});

app.use(router.routes());

// Start server
initializeWorkflows().then(() => {
  app.listen(3000, () => {
    console.log('Server running on port 3000');
  });
});
```

### **Bun.js HTTP Server Integration**

```typescript
import { cronflow } from 'cronflow';

// Initialize workflows
async function initializeWorkflows() {
  const dataWorkflow = cronflow.define({
    id: 'data-processing',
    name: 'Data Processing',
  });

  dataWorkflow
    .step('extract', async (ctx) => {
      return { data: 'extracted' };
    })
    .step('transform', async (ctx) => {
      return { transformed: ctx.last.data };
    })
    .step('load', async (ctx) => {
      return { loaded: true };
    });

  await cronflow.start();
}

// HTTP server handler
async function handleRequest(request: Request): Promise<Response> {
  const url = new URL(request.url);
  const path = url.pathname;

  if (path === '/api/process-data' && request.method === 'POST') {
    try {
      const body = await request.json();
      const runId = await cronflow.trigger('data-processing', body);
      
      return new Response(JSON.stringify({ success: true, runId }), {
        headers: { 'Content-Type': 'application/json' },
      });
    } catch (error) {
      return new Response(JSON.stringify({ error: error.message }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      });
    }
  }

  return new Response('Not found', { status: 404 });
}

// Start server
initializeWorkflows().then(() => {
  Bun.serve({
    port: 3000,
    fetch: handleRequest,
  });
  console.log('Server running on port 3000');
});
```

### **Next.js API Routes Integration**

```typescript
// pages/api/workflows/trigger.ts
import { NextApiRequest, NextApiResponse } from 'next';
import { cronflow } from 'cronflow';

// Initialize workflows (run once)
let initialized = false;
async function initializeWorkflows() {
  if (initialized) return;
  
  const workflow = cronflow.define({
    id: 'nextjs-workflow',
    name: 'Next.js Workflow',
  });

  workflow
    .step('process', async (ctx) => {
      return { processed: true };
    });

  await cronflow.start();
  initialized = true;
}

export default async function handler(req: NextApiRequest, res: NextApiResponse) {
  if (req.method !== 'POST') {
    return res.status(405).json({ error: 'Method not allowed' });
  }

  try {
    await initializeWorkflows();
    const runId = await cronflow.trigger('nextjs-workflow', req.body);
    res.json({ success: true, runId });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
}
```

### **NestJS Integration**

```typescript
// workflow.service.ts
import { Injectable, OnModuleInit } from '@nestjs/common';
import { cronflow } from 'cronflow';

@Injectable()
export class WorkflowService implements OnModuleInit {
  async onModuleInit() {
    await this.initializeWorkflows();
  }

  private async initializeWorkflows() {
    const workflow = cronflow.define({
      id: 'nestjs-workflow',
      name: 'NestJS Workflow',
    });

    workflow
      .step('process', async (ctx) => {
        return { processed: true };
      });

    await cronflow.start();
  }

  async triggerWorkflow(payload: any) {
    return await cronflow.trigger('nestjs-workflow', payload);
  }

  async getWorkflowStatus(runId: string) {
    return await cronflow.inspect(runId);
  }
}

// workflow.controller.ts
import { Controller, Post, Get, Body, Param } from '@nestjs/common';
import { WorkflowService } from './workflow.service';

@Controller('workflows')
export class WorkflowController {
  constructor(private workflowService: WorkflowService) {}

  @Post('trigger')
  async triggerWorkflow(@Body() payload: any) {
    const runId = await this.workflowService.triggerWorkflow(payload);
    return { success: true, runId };
  }

  @Get(':runId')
  async getWorkflowStatus(@Param('runId') runId: string) {
    const status = await this.workflowService.getWorkflowStatus(runId);
    return { success: true, status };
  }
}
```

## 🎯 **Common Integration Patterns**

### **1. Workflow Initialization**

```typescript
// Always initialize workflows before starting your server
async function initializeWorkflows() {
  // Define your workflows
  const workflow = cronflow.define({ id: 'my-workflow' });
  
  // Add steps
  workflow.step('step1', async (ctx) => {
    // Step logic
  });
  
  // Start the engine
  await cronflow.start();
}

// Call this before starting your web server
await initializeWorkflows();
```

### **2. API Endpoint Pattern**

```typescript
// Standard pattern for workflow trigger endpoints
app.post('/api/workflows/:workflowId/trigger', async (req, res) => {
  try {
    const { workflowId } = req.params;
    const payload = req.body;
    
    const runId = await cronflow.trigger(workflowId, payload);
    
    res.json({
      success: true,
      runId,
      message: 'Workflow triggered successfully',
    });
  } catch (error) {
    res.status(500).json({
      success: false,
      error: error.message,
    });
  }
});
```

### **3. Status Checking Pattern**

```typescript
// Standard pattern for checking workflow status
app.get('/api/workflows/runs/:runId', async (req, res) => {
  try {
    const { runId } = req.params;
    
    const status = await cronflow.inspect(runId);
    
    res.json({
      success: true,
      runId,
      status,
    });
  } catch (error) {
    res.status(500).json({
      success: false,
      error: error.message,
    });
  }
});
```

### **4. Error Handling Pattern**

```typescript
// Consistent error handling across all endpoints
function handleWorkflowError(error: any, res: any) {
  console.error('Workflow error:', error);
  
  if (error.message.includes('not found')) {
    return res.status(404).json({
      success: false,
      error: 'Workflow not found',
    });
  }
  
  return res.status(500).json({
    success: false,
    error: error.message || 'Internal server error',
  });
}
```

## 🔒 **Security Considerations**

### **1. Input Validation**

```typescript
// Always validate input before triggering workflows
app.post('/api/workflows/trigger', async (req, res) => {
  try {
    // Validate input
    const { workflowId, payload } = req.body;
    
    if (!workflowId) {
      return res.status(400).json({ error: 'workflowId is required' });
    }
    
    if (!payload || typeof payload !== 'object') {
      return res.status(400).json({ error: 'payload must be an object' });
    }
    
    // Trigger workflow
    const runId = await cronflow.trigger(workflowId, payload);
    res.json({ success: true, runId });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});
```

### **2. Authentication & Authorization**

```typescript
// Add authentication middleware
app.use('/api/workflows', authenticateUser);

// Add authorization checks
app.post('/api/workflows/:workflowId/trigger', async (req, res) => {
  const { workflowId } = req.params;
  const user = req.user; // From authentication middleware
  
  // Check if user has permission to trigger this workflow
  if (!user.canTriggerWorkflow(workflowId)) {
    return res.status(403).json({ error: 'Insufficient permissions' });
  }
  
  // Trigger workflow
  const runId = await cronflow.trigger(workflowId, req.body);
  res.json({ success: true, runId });
});
```

## 📊 **Monitoring & Observability**

### **1. Health Check Endpoint**

```typescript
app.get('/health', (req, res) => {
  res.json({
    status: 'healthy',
    service: 'my-app',
    cronflow: {
      state: cronflow.getEngineState(),
      workflows: cronflow.getWorkflows().length,
    },
    timestamp: new Date().toISOString(),
  });
});
```

### **2. Metrics Endpoint**

```typescript
app.get('/metrics', async (req, res) => {
  try {
    const workflows = cronflow.getWorkflows();
    const stats = await cronflow.getStateStats();
    
    res.json({
      workflows: workflows.length,
      stateStats: stats,
      engineState: cronflow.getEngineState(),
    });
  } catch (error) {
    res.status(500).json({ error: error.message });
  }
});
```

## 🚀 **Performance Tips**

### **1. Lazy Initialization**

```typescript
// Initialize workflows only when needed
let workflowsInitialized = false;

async function ensureWorkflowsInitialized() {
  if (!workflowsInitialized) {
    await initializeWorkflows();
    workflowsInitialized = true;
  }
}

app.post('/api/workflows/trigger', async (req, res) => {
  await ensureWorkflowsInitialized();
  // ... rest of the logic
});
```

### **2. Connection Pooling**

```typescript
// Use connection pooling for database operations in workflows
const workflow = cronflow.define({ id: 'db-workflow' });

workflow.step('database-operation', async (ctx) => {
  // Use connection pool instead of creating new connections
  const result = await dbPool.query('SELECT * FROM users');
  return result;
});
```

## 📝 **Best Practices**

1. **Initialize workflows before starting your web server**
2. **Use consistent error handling patterns**
3. **Validate all inputs before triggering workflows**
4. **Implement proper authentication and authorization**
5. **Add health checks and monitoring endpoints**
6. **Use environment variables for configuration**
7. **Implement graceful shutdown**
8. **Add comprehensive logging**
9. **Use TypeScript for better type safety**
10. **Test your integrations thoroughly**

## 🔗 **Next Steps**

- Check out the [API Reference](../api-reference.md) for detailed method documentation
- Explore [Workflow Examples](../examples/) for more complex use cases
- Read about [Performance Optimization](../performance.md) for production deployments
- Learn about [Error Handling](../error-handling.md) best practices 