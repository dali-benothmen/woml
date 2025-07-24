import { cronflow } from '../sdk/src/cronflow';
import { z } from 'zod';

// Simple workflow with if condition only
const simpleIfWorkflow = cronflow.define({
  id: 'simple-if-test',
  name: 'Simple If Test',
  description:
    'Test basic if control flow with minimal steps and parallel execution',
});

simpleIfWorkflow
  .onWebhook('/simple-if-test', {
    method: 'POST',
    schema: z.object({
      amount: z.number().positive(),
      description: z.string().optional(),
    }),
    parseRawBody: false,
    headers: {
      required: {
        'content-type': 'application/json',
      },
    },
  })
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
  .parallel([
    async ctx => {
      console.log('🔄 Parallel step 1: validate-data executing...');
      await new Promise(resolve => setTimeout(resolve, 200));
      console.log('✅ Parallel step 1: validate-data completed');
      return {
        step: 'validate-data',
        result: 'success',
        validation: {
          amount: ctx.last.amount,
          isValid: ctx.last.amount > 0,
          timestamp: new Date().toISOString(),
        },
      };
    },
    async ctx => {
      console.log('🔄 Parallel step 2: log-transaction executing...');
      await new Promise(resolve => setTimeout(resolve, 150));
      console.log('✅ Parallel step 2: log-transaction completed');
      return {
        step: 'log-transaction',
        result: 'success',
        log: {
          transactionId: `txn_${Date.now()}`,
          amount: ctx.last.amount,
          type: ctx.last.type,
          logged: true,
        },
      };
    },
  ])
  .endIf()
  .step('final-step', async ctx => {
    console.log('✅ Step 3: final-step executed');
    console.log('   Previous step result:', ctx.last);
    return {
      final: true,
      summary: ctx.last,
      parallelResults: ctx.last,
      executionCompleted: new Date().toISOString(),
    };
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
    console.log('📋 Webhook configuration:');
    console.log('   - Method: POST');
    console.log(
      '   - Schema validation: amount (positive number), description (optional)'
    );
    console.log('   - Parse raw body: false');
  } catch (error) {
    console.error('❌ Test failed:', error);
  }
})();
