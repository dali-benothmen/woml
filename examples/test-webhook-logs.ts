#!/usr/bin/env bun

import { cronflow } from '../sdk/src/index';

// Create a simple webhook workflow
const webhookWorkflow = cronflow.define({
  id: 'test-webhook-logs',
  name: 'Test Webhook Logs',
});

webhookWorkflow
  .onWebhook('/webhooks/test')
  .step('process-webhook', async ctx => {
    console.log('📥 Received webhook payload:', ctx.payload);
    return { processed: true, timestamp: new Date().toISOString() };
  })
  .action('log-success', ctx => {
    console.log('✅ Webhook processed successfully');
  });

async function testWebhookLogs() {
  console.log('🚀 Testing webhook logs...');

  await cronflow.start({
    webhookServer: {
      host: '127.0.0.1',
      port: 3000,
    },
  });

  console.log('✅ Engine started!');
  console.log(
    '📡 Webhook endpoint: http://127.0.0.1:3000/webhook/webhooks/test'
  );

  // Wait for server to be ready
  await new Promise(resolve => setTimeout(resolve, 1000));

  console.log('\n📤 Making webhook request...');

  const response = await fetch('http://127.0.0.1:3000/webhook/webhooks/test', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      message: 'Hello from webhook!',
      test: true,
    }),
  });

  console.log(`📥 HTTP Response: ${response.status} ${response.statusText}`);

  // Wait for workflow to complete
  await new Promise(resolve => setTimeout(resolve, 2000));

  console.log('\n🛑 Stopping engine...');
  await cronflow.stop();
  console.log('✅ Test completed!');
}

testWebhookLogs().catch(console.error);
