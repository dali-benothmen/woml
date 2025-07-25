import { cronflow } from '../sdk/src/cronflow';
import { z } from 'zod';
import { Context } from '../sdk/src/workflow/types';

const simpleIfWorkflow = cronflow.define({
  id: 'simple-if-else-test',
  name: 'Simple If/Else Test',
  description: 'A simple workflow to test if/else logic with human approval',
  timeout: '10m',
  hooks: {
    onSuccess: (ctx: Context) => {
      console.log('🎉 Simple if/else workflow completed successfully!');
    },
    onFailure: (ctx: Context) => {
      console.error('💥 Simple if/else workflow failed:', ctx.error);
    },
  },
});

simpleIfWorkflow
  .onWebhook('/simple-if-test', {
    schema: z.object({
      amount: z.number().positive(),
      description: z.string().optional(),
    }),
  })
  .step('check-amount', async (ctx: Context) => {
    console.log('🔍 Checking amount:', ctx.payload.amount);
    return { amount: ctx.payload.amount, checked: true };
  })
  .if('is-high-value', (ctx: Context) => {
    console.log('🔍 Evaluating high-value condition');
    return ctx.last.amount > 100;
  })
  .step('process-high-value', async (ctx: Context) => {
    console.log('💎 Processing high-value transaction');
    return { type: 'high-value', processed: true, amount: ctx.last.amount };
  })
  .humanInTheLoop({
    timeout: '1h',
    description: 'Approve high-value transaction',
    onPause: (token: string) => {
      console.log(`🛑 Human approval required. Token: ${token}`);
      console.log('📧 Send this token to approver for manual review');
      console.log(
        '🔄 Use cronflow.resume(token, {approved: true, reason: "Approved"}) to resume'
      );
    },
  })
  .step('after-approval', async (ctx: Context) => {
    console.log('✅ Human approval received');
    console.log('   Approval result:', ctx.last);
    return { approved: ctx.last.approved, approvedBy: ctx.last.approvedBy };
  })
  .parallel([
    async (ctx: Context) => {
      console.log('🔄 Parallel step 1: Validate data');
      await new Promise(resolve => setTimeout(resolve, 200));
      return { validation: 'success', amount: ctx.last.amount };
    },
    async (ctx: Context) => {
      console.log('🔄 Parallel step 2: Log transaction');
      await new Promise(resolve => setTimeout(resolve, 150));
      return { logged: true, transactionId: `txn_${Date.now()}` };
    },
  ])
  .action('background-notification', async (ctx: Context) => {
    console.log('🔄 Background action: Sending notification');
    await new Promise(resolve => setTimeout(resolve, 1000));
    console.log('✅ Background notification sent');
  })
  .endIf()
  .else()
  .step('process-low-value', async (ctx: Context) => {
    console.log('📝 Processing low-value transaction');
    return { type: 'low-value', processed: true, amount: ctx.last.amount };
  })
  .endIf()
  .step('final-summary', async (ctx: Context) => {
    console.log('📋 Creating final summary');
    return {
      final: true,
      summary: ctx.last,
      completedAt: new Date().toISOString(),
    };
  });

// Self-executing function to start the workflow
(async () => {
  try {
    console.log('🚀 Starting simple if/else workflow...');

    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Simple if/else workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/simple-if-test');
    console.log('�� Test examples:');
    console.log(
      '  High value (requires approval): curl -X POST http://127.0.0.1:3000/simple-if-test \\'
    );
    console.log('    -H "Content-Type: application/json" \\');
    console.log(
      '    -d \'{"amount": 500, "description": "High value order"}\''
    );
    console.log(
      '  Low value (no approval): curl -X POST http://127.0.0.1:3000/simple-if-test \\'
    );
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"amount": 50, "description": "Low value order"}\'');
    console.log('');
    console.log('🔄 To resume a paused workflow, use:');
    console.log(
      '   cronflow.resume("token_here", {approved: true, reason: "Approved"})'
    );
    console.log('');
    console.log('📊 To list paused workflows, use:');
    console.log('   cronflow.listPausedWorkflows()');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
