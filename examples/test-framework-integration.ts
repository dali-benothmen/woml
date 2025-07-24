import { cronflow } from '../sdk/src/cronflow';
import express from 'express';
import { z } from 'zod';

// Create Express app first
const app = express();
const PORT = 3000;

// Set up middleware BEFORE workflow definition
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

// Add request logging middleware
app.use((req, res, next) => {
  console.log(`🌐 ${req.method} ${req.path} - ${new Date().toISOString()}`);
  next();
});

// Simple workflow for framework testing (no hooks, actions, or conditions)
const frameworkWorkflow = cronflow.define({
  id: 'framework-test',
  name: 'Framework Integration Test',
  description:
    'Simple workflow for testing Express.js integration with URL webhooks',
});

// Define workflow with 3 simple steps
frameworkWorkflow
  .onWebhook('/api/webhooks/framework-test', {
    app: 'express', // Simple! Just specify the framework
    appInstance: app, // And pass the app instance
    method: 'POST',
    schema: z.object({
      message: z.string().min(1, 'Message cannot be empty'),
      userId: z.string().optional(),
      timestamp: z.string().optional(),
    }),
    headers: {
      required: { 'content-type': 'application/json' },
    },
  })
  .step('validate-input', async ctx => {
    console.log('📝 Step 1: Validating input');
    console.log('   Payload:', ctx.payload);

    if (!ctx.payload.message) {
      throw new Error('Message is required');
    }

    return {
      validated: true,
      message: ctx.payload.message,
      timestamp: new Date().toISOString(),
    };
  })
  .step('process-data', async ctx => {
    console.log('📝 Step 2: Processing data');
    console.log('   Previous step result:', ctx.last);

    const processedData = {
      originalMessage: ctx.last.message,
      processedMessage: ctx.last.message.toUpperCase(),
      processedAt: new Date().toISOString(),
      stepCount: 2,
    };

    return processedData;
  })
  .step('finalize', async ctx => {
    console.log('📝 Step 3: Finalizing');
    console.log('   Previous step result:', ctx.last);

    return {
      final: true,
      summary: {
        originalMessage: ctx.last.originalMessage,
        processedMessage: ctx.last.processedMessage,
        totalSteps: 3,
        completedAt: new Date().toISOString(),
      },
    };
  });

// ============================================================================
// EXAMPLES: How to use webhook integration with different frameworks
// ============================================================================

/*
// SIMPLE APPROACH (Pre-registered frameworks)
// ===========================================

// Express.js
workflow.onWebhook('/api/webhooks/test', {
  app: 'express',
  appInstance: app,
  method: 'POST'
});

// Fastify
workflow.onWebhook('/api/webhooks/test', {
  app: 'fastify',
  appInstance: fastify,
  method: 'POST'
});

// Koa
workflow.onWebhook('/api/webhooks/test', {
  app: 'koa',
  appInstance: app,
  method: 'POST'
});

// Hapi
workflow.onWebhook('/api/webhooks/test', {
  app: 'hapi',
  appInstance: server,
  method: 'POST'
});

// NestJS
workflow.onWebhook('/api/webhooks/test', {
  app: 'nestjs',
  appInstance: app,
  method: 'POST'
});

// Bun HTTP Server
workflow.onWebhook('/api/webhooks/test', {
  app: 'bun',
  appInstance: null, // Bun doesn't need app instance
  method: 'POST'
});

// Next.js
workflow.onWebhook('/api/webhooks/test', {
  app: 'nextjs',
  appInstance: app,
  method: 'POST'
});

// FLEXIBLE APPROACH (Custom frameworks)
// =====================================

// Any framework with custom registration
workflow.onWebhook('/api/webhooks/test', {
  registerRoute: (method, path, handler) => {
    // Your custom registration logic here
    myFramework[method.toLowerCase()](path, handler);
  },
  method: 'POST'
});

// Express with custom logic
workflow.onWebhook('/api/webhooks/test', {
  registerRoute: (method, path, handler) => {
    app[method.toLowerCase()](path, (req, res) => {
      // Custom middleware or logic
      console.log('Custom middleware');
      return handler(req, res);
    });
  },
  method: 'POST'
});

// Fastify with custom logic
workflow.onWebhook('/api/webhooks/test', {
  registerRoute: (method, path, handler) => {
    fastify[method.toLowerCase()](path, async (request, reply) => {
      // Custom Fastify-specific logic
      const result = await handler(request, reply);
      return result;
    });
  },
  method: 'POST'
});
*/

// Manual trigger endpoint
app.post('/api/trigger-workflow', async (req, res) => {
  try {
    console.log('🚀 Manual trigger endpoint called');
    console.log('   Payload:', req.body);

    const runId = await cronflow.trigger('framework-test', req.body);

    console.log('✅ Workflow triggered successfully');
    console.log('   Run ID:', runId);

    res.json({
      success: true,
      runId,
      message: 'Workflow triggered successfully',
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Manual trigger failed:', error);
    res.status(500).json({
      success: false,
      error: error.message,
      timestamp: new Date().toISOString(),
    });
  }
});

// Workflow status endpoint
app.get('/api/workflows/:runId', async (req, res) => {
  try {
    const { runId } = req.params;
    console.log('📊 Checking workflow status for run:', runId);

    const status = await cronflow.inspect(runId);

    res.json({
      success: true,
      runId,
      status,
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Status check failed:', error);
    res.status(500).json({
      success: false,
      error: error.message,
      timestamp: new Date().toISOString(),
    });
  }
});

// Start Express server
app.listen(PORT, async () => {
  console.log(`\n🚀 Express server started on port ${PORT}`);
  console.log(`🌐 Server URL: http://localhost:${PORT}`);
  console.log(`🔗 Health check: http://localhost:${PORT}/health`);
  console.log(
    `🚀 Manual trigger: POST http://localhost:${PORT}/api/trigger-workflow`
  );
  console.log(
    `🔗 Webhook endpoint: POST http://localhost:${PORT}/api/webhooks/framework-test`
  );
  console.log(
    `📊 Status check: GET http://localhost:${PORT}/api/workflows/:runId`
  );

  // Register workflow with Rust core
  try {
    console.log('\n🔧 Registering workflow with Rust core...');
    await cronflow.start(); // This registers all workflows with the Rust core (no webhook server)
    console.log('✅ Workflow registered successfully');
  } catch (error) {
    console.error('❌ Failed to register workflow:', error);
  }

  console.log('\n📝 Test Instructions:');
  console.log('1. Test manual trigger:');
  console.log('   curl -X POST http://localhost:3000/api/trigger-workflow \\');
  console.log('     -H "Content-Type: application/json" \\');
  console.log('     -d \'{"message": "Hello from manual trigger!"}\'');
  console.log('');
  console.log('2. Test webhook trigger:');
  console.log(
    '   curl -X POST http://localhost:3000/api/webhooks/framework-test \\'
  );
  console.log('     -H "Content-Type: application/json" \\');
  console.log('     -d \'{"message": "Hello from webhook!"}\'');
  console.log('');
  console.log('3. Check workflow status:');
  console.log('   curl http://localhost:3000/api/workflows/{runId}');
  console.log('');
  console.log('✅ Framework integration test ready!');
});
