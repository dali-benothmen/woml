import { cronflow } from '../sdk/src/cronflow';

// Simple workflow with if condition only
const simpleIfWorkflow = cronflow.define({
  id: 'simple-if-test',
  name: 'Simple If Test',
  description: 'Test basic if control flow with minimal steps',
});
simpleIfWorkflow
  .onWebhook('/simple-if-test')
  .step('check-amount', async ctx => {
    console.log('✅ Step 1: check-amount executed');
    console.log('   Payload amount:', ctx.payload.amount);
    return { amount: ctx.payload.amount, checked: true };
  })
  .if('is-high-value', ctx => {
    console.log('🔍 Evaluating condition: is-high-value');
    console.log('   Condition: ctx.last.amount > 120');
    console.log('   Last step amount:', ctx.last.amount);
    const result = ctx.last.amount > 120;
    console.log('   Condition result:', result);
    return result;
  })
  .step('process-high-value', async ctx => {
    console.log('✅ Step 2: process-high-value executed (IF branch)');
    console.log('   Processing high value amount:', ctx.last.amount);
    return { type: 'high-value', processed: true, amount: ctx.last.amount };
  })
  .endIf()
  .step('final-step', async ctx => {
    console.log('✅ Step 3: final-step executed');
    console.log('   Previous step result:', ctx.last);
    return { final: true, summary: ctx.last };
  });

// Self-executing function to test if condition
(async () => {
  try {
    console.log('🚀 Starting simple workflow test...');

    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
        maxConnections: 1000,
      },
    });

    console.log('✅ Test completed successfully');
    console.log('🌐 Webhook server running at: http://127.0.0.1:3000');
    console.log(
      '📡 Webhook endpoint: http://127.0.0.1:3000/webhook/simple-if-test'
    );
  } catch (error) {
    console.error('❌ Test failed:', error);
  }
})();
