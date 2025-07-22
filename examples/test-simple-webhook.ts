#!/usr/bin/env bun

import { cronflow } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';

// Example 1: Simple workflow with webhook trigger
const simpleWorkflow = cronflow.define({
  id: 'simple-webhook-workflow',
  name: 'Simple Webhook Workflow',
  description: 'A basic workflow triggered by webhook',
});

simpleWorkflow
  .onWebhook('/webhooks/simple')
  .step('process-webhook', async (ctx: Context) => {
    console.log('📥 Received webhook payload:', ctx.payload);
    return { processed: true, timestamp: new Date().toISOString() };
  })
  .action('log-success', (ctx: Context) => {
    console.log('✅ Webhook processed successfully');
  });

console.log('✅ Simple Webhook Workflow created successfully!');
console.log('📋 Workflow ID:', simpleWorkflow.getId());
console.log('📋 Steps:', simpleWorkflow.getSteps().length);
console.log('📋 Triggers:', simpleWorkflow.getTriggers().length);

// Start the Cronflow engine
async function startCronflow() {
  try {
    console.log('🚀 Starting Cronflow engine...');
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
        maxConnections: 1000,
      },
    });
    console.log('✅ Cronflow engine started successfully!');
    console.log('🌐 Webhook server running on: http://127.0.0.1:3000');
    console.log(
      '📡 Webhook endpoint: http://127.0.0.1:3000/webhook/webhooks/simple'
    );
    console.log('🏥 Health check: http://127.0.0.1:3000/health');

    // Keep the process running
    console.log('\n🔄 Server is running. Press Ctrl+C to stop.');
    console.log('💡 Test the webhook with:');
    console.log(
      'curl -X POST http://127.0.0.1:3000/webhook/webhooks/simple \\'
    );
    console.log('  -H "Content-Type: application/json" \\');
    console.log('  -d \'{"test": "data", "message": "Hello Cronflow!"}\'');

    // Keep the process alive
    process.on('SIGINT', async () => {
      console.log('\n🛑 Shutting down Cronflow engine...');
      await cronflow.stop();
      console.log('✅ Cronflow engine stopped successfully!');
      process.exit(0);
    });
  } catch (error) {
    console.error('❌ Failed to start Cronflow engine:', error);
    process.exit(1);
  }
}

// Run the test
startCronflow();
