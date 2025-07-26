import {
  cronflow,
  listPausedWorkflows,
  getPausedWorkflow,
} from '../sdk/src/cronflow';
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

// Simple workflow for framework testing with human approval
const frameworkWorkflow = cronflow.define({
  id: 'framework-test',
  name: 'Framework Integration Test',
  description:
    'Simple workflow for testing Express.js integration with human approval',
});

// Define workflow with human approval step
frameworkWorkflow
  .onWebhook('/api/webhooks/framework-test', {
    app: 'express', // Simple! Just specify the framework
    appInstance: app, // And pass the app instance
    method: 'POST',
    schema: z.object({
      message: z.string().min(1, 'Message cannot be empty'),
      userId: z.string().optional(),
      timestamp: z.string().optional(),
      requiresApproval: z.boolean().optional(),
    }),
    headers: {
      required: { 'content-type': 'application/json' },
    },
  })
  .onWebhook('/api/webhooks/trigger-step', {
    app: 'express',
    appInstance: app,
    method: 'POST',
    trigger: 'process-data', // Trigger only the 'process-data' step
    schema: z.object({
      message: z.string().min(1, 'Message cannot be empty'),
      userId: z.string().optional(),
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
      requiresApproval: ctx.payload.requiresApproval || false,
      timestamp: new Date().toISOString(),
    };
  })
  .humanInTheLoop({
    timeout: '5m', // 5 minute timeout for testing
    description: 'Approve message processing',
    onPause: (ctx, token) => {
      console.log(`🛑 Human approval required`);
      console.log(`🔑 Approval token: ${token}`);
      console.log(`📧 Context token: ${ctx.token}`);
      console.log('📧 Send this token to approver for manual review');
      console.log(`🔄 Use POST /api/resume-workflow with token to resume`);
    },
  })
  .step('process-data', async ctx => {
    console.log('📝 Step 2: Processing data');
    console.log('   Previous step result:', ctx.last);

    const processedData = {
      originalMessage: ctx.last.message,
      processedMessage: ctx.last.message.toUpperCase(),
      requiresApproval: ctx.last.requiresApproval,
      processedAt: new Date().toISOString(),
      stepCount: 2,
    };

    return processedData;
  })
  .pause(async ctx => {
    // This callback is executed when the pause step is reached
    console.log('⏸️  PAUSE STEP REACHED!');
    console.log('   - Current payload:', ctx.payload);
    console.log('   - Last step output:', ctx.last);
    console.log('   - Completed steps:', Object.keys(ctx.steps));
    console.log('   - Workflow ID:', ctx.run.workflowId);
    console.log('   - Run ID:', ctx.run.id);

    // You can perform any logic here before pausing
    console.log('   - Performing pre-pause operations...');
    await new Promise(resolve => setTimeout(resolve, 50)); // Simulate work

    console.log('   - Workflow will now pause for manual intervention');
    console.log('   - Use the resume functionality to continue execution');
  })
  .if('needs-approval', ctx => {
    console.log('🔍 Checking if approval is required');
    return ctx.last.requiresApproval;
  })
  .humanInTheLoop({
    timeout: '5m', // 5 minute timeout for testing
    description: 'Approve message processing',
    onPause: (ctx, token) => {
      console.log(`🛑 Human approval required`);
      console.log(`🔑 Approval token: ${token}`);
      console.log(`📧 Context token: ${ctx.token}`);
      console.log('📧 Send this token to approver for manual review');
      console.log(`🔄 Use POST /api/resume-workflow with token to resume`);
    },
  })
  .step('process-approval', async ctx => {
    console.log('✅ Processing approval result');

    if (ctx.last.timedOut) {
      console.log('⏰ Approval timed out');
      return {
        approved: false,
        reason: 'Timeout - no approval received within 5 minutes',
        status: 'timeout',
      };
    }

    console.log('✅ Manual approval received:', ctx.last);
    return {
      approved: ctx.last.approved,
      reason: ctx.last.reason,
      approvedBy: ctx.last.approvedBy,
      status: 'approved',
    };
  })
  .endIf()
  .step('finalize', async ctx => {
    console.log('🎯 Finalizing workflow');
    return {
      finalized: true,
      message: ctx.last.message || 'Workflow completed',
      timestamp: new Date().toISOString(),
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

// Resume workflow endpoint
app.post('/api/resume-workflow', async (req: any, res: any) => {
  try {
    const { token, payload } = req.body;

    if (!token) {
      return res.status(400).json({
        success: false,
        error: 'Token is required',
        timestamp: new Date().toISOString(),
      });
    }

    if (!payload) {
      return res.status(400).json({
        success: false,
        error: 'Payload is required',
        timestamp: new Date().toISOString(),
      });
    }

    console.log('🔄 Resuming workflow with token:', token);
    console.log('📋 Resume payload:', payload);

    await cronflow.resume(token, payload);

    res.json({
      success: true,
      message: 'Workflow resumed successfully',
      token,
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Resume failed:', error);
    res.status(500).json({
      success: false,
      error: error.message,
      timestamp: new Date().toISOString(),
    });
  }
});

// List paused workflows endpoint
app.get('/api/paused-workflows', async (req: any, res: any) => {
  try {
    const pausedWorkflows = listPausedWorkflows();

    console.log('📊 Listing paused workflows:', pausedWorkflows.length);

    res.json({
      success: true,
      count: pausedWorkflows.length,
      workflows: pausedWorkflows.map(wf => ({
        token: wf.token,
        workflowId: wf.workflowId,
        runId: wf.runId,
        description: wf.description,
        createdAt: new Date(wf.createdAt).toISOString(),
        expiresAt: wf.expiresAt ? new Date(wf.expiresAt).toISOString() : null,
        status: wf.status,
      })),
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Failed to list paused workflows:', error);
    res.status(500).json({
      success: false,
      error: error.message,
      timestamp: new Date().toISOString(),
    });
  }
});

// Get specific paused workflow endpoint
app.get('/api/paused-workflows/:token', async (req: any, res: any) => {
  try {
    const { token } = req.params;
    const workflow = getPausedWorkflow(token);

    if (!workflow) {
      return res.status(404).json({
        success: false,
        error: 'Paused workflow not found',
        token,
        timestamp: new Date().toISOString(),
      });
    }

    console.log('📊 Getting paused workflow details for token:', token);

    res.json({
      success: true,
      workflow: {
        token: workflow.token,
        workflowId: workflow.workflowId,
        runId: workflow.runId,
        description: workflow.description,
        createdAt: new Date(workflow.createdAt).toISOString(),
        expiresAt: workflow.expiresAt
          ? new Date(workflow.expiresAt).toISOString()
          : null,
        status: workflow.status,
        metadata: workflow.metadata,
      },
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Failed to get paused workflow:', error);
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
    `🎯 Step-specific webhook: POST http://localhost:${PORT}/api/webhooks/trigger-step`
  );
  console.log(
    `📊 Status check: GET http://localhost:${PORT}/api/workflows/:runId`
  );
  console.log(
    `🔄 Resume workflow: POST http://localhost:${PORT}/api/resume-workflow`
  );
  console.log(
    `📋 List paused workflows: GET http://localhost:${PORT}/api/paused-workflows`
  );
  console.log(
    `📊 Get paused workflow: GET http://localhost:${PORT}/api/paused-workflows/:token`
  );

  // Register workflow with Rust core BEFORE starting the server
  try {
    console.log('\n🔧 Starting cronflow engine...');
    await cronflow.start(); // This registers all workflows with the Rust core
    console.log('✅ Cronflow engine started successfully');
    console.log('✅ All workflows registered and ready to handle requests');
  } catch (error) {
    console.error('❌ Failed to start cronflow engine:', error);
    process.exit(1); // Exit if we can't start the engine
  }

  console.log('\n📝 Test Instructions:');
  console.log('1. Test workflow without approval:');
  console.log(
    '   curl -X POST http://localhost:3000/api/webhooks/framework-test \\'
  );
  console.log('     -H "Content-Type: application/json" \\');
  console.log(
    '     -d \'{"message": "Hello from webhook!", "requiresApproval": false}\''
  );
  console.log('');
  console.log('2. Test workflow with approval (will pause):');
  console.log(
    '   curl -X POST http://localhost:3000/api/webhooks/framework-test \\'
  );
  console.log('     -H "Content-Type: application/json" \\');
  console.log(
    '     -d \'{"message": "Hello from webhook!", "requiresApproval": true}\''
  );
  console.log('');
  console.log('3. Check paused workflows:');
  console.log('   curl http://localhost:3000/api/paused-workflows');
  console.log('');
  console.log('4. Resume a paused workflow (replace TOKEN with actual token):');
  console.log('   curl -X POST http://localhost:3000/api/resume-workflow \\');
  console.log('     -H "Content-Type: application/json" \\');
  console.log(
    '     -d \'{"token": "TOKEN", "payload": {"approved": true, "reason": "Looks good!"}}\''
  );
  console.log('');
  console.log('5. Test manual trigger:');
  console.log('   curl -X POST http://localhost:3000/api/trigger-workflow \\');
  console.log('     -H "Content-Type: application/json" \\');
  console.log('     -d \'{"message": "Hello from manual trigger!"}\'');
  console.log('');
  console.log(
    '6. Test step-specific webhook (triggers only process-data step):'
  );
  console.log(
    '   curl -X POST http://localhost:3000/api/webhooks/trigger-step \\'
  );
  console.log('     -H "Content-Type: application/json" \\');
  console.log('     -d \'{"message": "Hello from step-specific webhook!"}\'');
  console.log('');
  console.log('7. Check workflow status:');
  console.log('   curl http://localhost:3000/api/workflows/{runId}');
  console.log('');
  console.log('8. Test pause functionality:');
  console.log('   The workflow now includes a .pause() step that will:');
  console.log('   - Execute a callback function when reached');
  console.log('   - Log current context information');
  console.log('   - Pause workflow execution for manual intervention');
  console.log('   - Allow resuming with the existing resume functionality');
  console.log('');
  console.log(
    '✅ Framework integration with human approval and pause test ready!'
  );
});
