const { cronflow } = require('./sdk/src/cronflow');
const express = require('express');
const { z } = require('zod');

const app = express();
app.use(express.json());

// Simple workflow for testing
const testWorkflow = cronflow.define({
  id: 'test-framework-fix',
  name: 'Test Framework Fix',
});

// Define steps FIRST
testWorkflow
  .step('validate-input', async ctx => {
    console.log('✅ Step: validate-input');
    return { validated: true, message: ctx.payload.message };
  })
  .step('process-data', async ctx => {
    console.log('✅ Step: process-data');
    return { processed: true, result: ctx.last.message.toUpperCase() };
  })
  .humanInTheLoop({
    timeout: '1m',
    description: 'Test approval',
    onPause: (ctx, token) => {
      console.log(`🛑 Approval required for: ${ctx.last.result}`);
      console.log(`🔑 Token: ${token}`);
      console.log(`📧 Context token: ${ctx.token}`);
    },
  })
  .step('finalize', async ctx => {
    console.log('✅ Step: finalize');
    return { finalized: true, approved: ctx.last.approved };
  })
  // Register webhook AFTER steps
  .onWebhook('/test-webhook', {
    app: 'express',
    appInstance: app,
    method: 'POST',
    schema: z.object({
      message: z.string().min(1),
    }),
  });

// Start server
app.listen(3001, async () => {
  console.log('🚀 Starting test server on port 3001');

  try {
    await cronflow.start();
    console.log('✅ Cronflow engine started');
    console.log('✅ Test ready! Run:');
    console.log('curl -X POST http://localhost:3001/test-webhook \\');
    console.log('  -H "Content-Type: application/json" \\');
    console.log('  -d \'{"message": "Hello from test!"}\'');
  } catch (error) {
    console.error('❌ Failed to start:', error);
    process.exit(1);
  }
});
