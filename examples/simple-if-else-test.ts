import { cronflow } from '../sdk/src/cronflow';
import { z } from 'zod';

// Simple workflow with if condition only
const simpleIfWorkflow = cronflow.define({
  id: 'simple-if-test',
  name: 'Simple If Test',
  description:
    'Test basic if control flow with minimal steps and parallel execution',
  hooks: {
    // Enhanced hooks that support both workflow-level and step-level execution
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        // Workflow-level success
        console.log(
          '🎉 Workflow-level onSuccess: Simple If Test completed successfully!'
        );
        console.log('   Final output:', ctx.last);
        console.log('   Total steps completed:', Object.keys(ctx.steps).length);
        return;
      }

      // Step-level success
      if (Array.isArray(stepId)) {
        // Multiple steps specified
        if (stepId.includes(ctx.step_name || '')) {
          console.log(
            `✅ Step-level onSuccess for ${ctx.step_name}: Step completed successfully!`
          );
          console.log(`   Step result:`, ctx.step_result);
          console.log(`   Step status:`, ctx.step_status);
        }
      } else {
        // Single step specified
        if (stepId === ctx.step_name) {
          console.log(
            `✅ Step-level onSuccess for ${ctx.step_name}: Step completed successfully!`
          );
          console.log(`   Step result:`, ctx.step_result);
          console.log(`   Step status:`, ctx.step_status);
        }
      }
    },
    onFailure: (ctx, stepId) => {
      if (!stepId) {
        // Workflow-level failure
        console.log('💥 Workflow-level onFailure: Simple If Test failed!');
        console.log('   Error:', ctx.error);
        console.log('   Failed at step:', ctx.step_name);
        return;
      }

      // Step-level failure
      if (Array.isArray(stepId)) {
        // Multiple steps specified
        if (stepId.includes(ctx.step_name || '')) {
          console.log(
            `❌ Step-level onFailure for ${ctx.step_name}: Step failed!`
          );
          console.log(`   Step error:`, ctx.step_error);
          console.log(`   Step status:`, ctx.step_status);
        }
      } else {
        // Single step specified
        if (stepId === ctx.step_name) {
          console.log(
            `❌ Step-level onFailure for ${ctx.step_name}: Step failed!`
          );
          console.log(`   Step error:`, ctx.step_error);
          console.log(`   Step status:`, ctx.step_status);
        }
      }
    },
  },
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
  .action('background-notification', async ctx => {
    console.log(
      '🔄 Background Action: Sending notification (this should run in background)'
    );
    // Simulate sending a notification that takes time
    await new Promise(resolve => setTimeout(resolve, 2500));
    console.log(
      '✅ Background Action: Notification sent successfully after 2.5 seconds'
    );
    return {
      type: 'notification',
      message: 'High-value transaction processed',
      amount: ctx.last.amount,
      sent: true,
      timestamp: new Date().toISOString(),
    };
  })
  .endIf()
  .step('final-step', async ctx => {
    console.log('✅ Step 3: final-step executed');
    console.log('   Previous step result:', ctx.last);
    return {
      final: true,
      summary: ctx.last,
      parallelResults: ctx.last,
      executionCompleted: new Date().toISOString(),
      note: 'Background notification action may still be running',
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
