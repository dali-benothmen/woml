import { cronflow, listPausedWorkflows } from '../sdk/src/cronflow';
import { z } from 'zod';
import { Context } from '../sdk/src/workflow/types';

const resumeTestWorkflow = cronflow.define({
  id: 'resume-test-workflow',
  name: 'Resume Test Workflow',
  description: 'A simple workflow to test the resume functionality',
  timeout: '5m',
  hooks: {
    onSuccess: (ctx: Context) => {
      console.log('🎉 Resume test workflow completed successfully!');
      console.log('Final result:', ctx.last);
    },
    onFailure: (ctx: Context) => {
      console.error('💥 Resume test workflow failed:', ctx.error);
    },
  },
});

resumeTestWorkflow
  .onWebhook('/test-resume', {
    schema: z.object({
      message: z.string(),
      requiresApproval: z.boolean().optional(),
    }),
  })
  .step('process-input', async (ctx: Context) => {
    console.log('📝 Processing input:', ctx.payload);
    return {
      message: ctx.payload.message,
      requiresApproval: ctx.payload.requiresApproval || false,
      processed: true,
    };
  })
  .if('needs-approval', (ctx: Context) => {
    return ctx.last.requiresApproval;
  })
  .humanInTheLoop({
    timeout: '30s', // Short timeout for testing
    description: 'Approve the message processing',
    onPause: (token: string) => {
      console.log(`🛑 Human approval required`);
      console.log(`🔑 Approval token: ${token}`);
      console.log(
        '📧 This workflow will wait for manual resume or timeout in 30 seconds'
      );
    },
  })
  .step('process-approval', async (ctx: Context) => {
    console.log('✅ Processing approval result');

    if (ctx.last.timedOut) {
      console.log('⏰ Approval timed out');
      return {
        approved: false,
        reason: 'Timeout',
        status: 'timeout',
      };
    }

    console.log('✅ Manual approval received:', ctx.last);
    return {
      approved: ctx.last.approved,
      reason: ctx.last.reason,
      status: 'approved',
    };
  })
  .endIf()
  .step('final-result', async (ctx: Context) => {
    console.log('📋 Creating final result');
    return {
      success: true,
      message: ctx.steps['process-input'].output.message,
      approvalStatus: ctx.last.status || 'no-approval-needed',
      completedAt: new Date().toISOString(),
    };
  });

// Self-executing function to start the workflow
(async () => {
  try {
    console.log('🚀 Starting resume test workflow...');

    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Resume test workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/test-resume');
    console.log('');
    console.log('📋 Test Instructions:');
    console.log('1. Trigger a workflow that requires approval:');
    console.log('   curl -X POST http://127.0.0.1:3000/test-resume \\');
    console.log('     -H "Content-Type: application/json" \\');
    console.log(
      '     -d \'{"message": "Hello World", "requiresApproval": true}\''
    );
    console.log('');
    console.log('2. Check paused workflows:');
    console.log('   console.log(listPausedWorkflows());');
    console.log('');
    console.log('3. Resume the workflow (replace TOKEN with actual token):');
    console.log(
      '   await cronflow.resume("TOKEN", {approved: true, reason: "Looks good!"});'
    );
    console.log('');
    console.log('4. Or test timeout by waiting 30 seconds');
    console.log('');
    console.log('5. Test without approval:');
    console.log('   curl -X POST http://127.0.0.1:3000/test-resume \\');
    console.log('     -H "Content-Type: application/json" \\');
    console.log('     -d \'{"message": "No approval needed"}\'');
    console.log('');
    console.log('🔄 Resume functionality is now fully implemented!');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
