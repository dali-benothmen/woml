import { cronflow } from '../sdk/src/cronflow';
import { z } from 'zod';
import { Context } from '../sdk/src/workflow/types';

const simpleIfWorkflow = cronflow.define({
  id: 'simple-if-else-test',
  name: 'Simple If/Else Test',
  description:
    'A simple workflow to test if/else logic with scheduled execution',
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
  .onSchedule('0 * * * * *') // Run every 1 minute (changed from 30 seconds)
  .step(
    {
      id: 'check-amount',
      title: 'Check Amount',
      description: 'Check the amount',
    },
    async (ctx: Context) => {
      // Generate a random amount for testing
      const amount = Math.floor(Math.random() * 1000) + 1;
      console.log('🔍 Checking amount:', amount);
      return { amount, checked: true };
    }
  )
  .step(
    {
      id: 'process-high-value',
      title: 'Process High Value',
      description: 'Process the high value transaction',
    },
    async (ctx: Context) => {
      console.log('💎 Processing high-value transaction');
      return { type: 'high-value', processed: true, amount: ctx.last.amount };
    }
  )
  .step(
    {
      id: 'final-summary',
      title: 'Final Summary',
      description: 'Create the final summary',
    },
    async (ctx: Context) => {
      console.log('📋 Creating final summary');
      return {
        final: true,
        summary: ctx.last,
        completedAt: new Date().toISOString(),
      };
    }
  );

// Self-executing function to start the workflow
(async () => {
  try {
    console.log('🚀 Starting simple if/else workflow with schedule trigger...');

    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Simple if/else workflow started successfully');
    console.log('⏰ Schedule: Every 30 seconds (CRON: */30 * * * * *)');
    console.log('🌐 Webhook server still running on: http://127.0.0.1:3000');
    console.log('');
    console.log('📊 The workflow will automatically trigger every 30 seconds');
    console.log('🔍 Watch the console for execution logs');
    console.log('');
    console.log('🛑 To stop the workflow, press Ctrl+C');
    console.log('');
    console.log('📈 To monitor workflow runs, you can use:');
    console.log('   cronflow.inspect("run_id_here")');
    console.log('');
    console.log('📋 To list all workflows:');
    console.log('   cronflow.getWorkflows()');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
