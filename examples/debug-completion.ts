#!/usr/bin/env bun

/**
 * Debug Workflow Completion Logic
 *
 * This script helps debug why workflow completion logic isn't working properly.
 */

import { cronflow } from '../sdk/src/index';

const TEST_WORKFLOW_ID = 'debug-completion-workflow';

async function debugCompletion() {
  console.log('🔍 Debugging Workflow Completion Logic');
  console.log('='.repeat(50));

  try {
    // Create a simple workflow with one step
    const workflow = cronflow.define({
      id: TEST_WORKFLOW_ID,
      name: 'Debug Completion Workflow',
      description: 'A simple workflow to debug completion logic',
      hooks: {
        onSuccess: (ctx: any) => {
          console.log('🎉 SUCCESS HOOK EXECUTED!');
          console.log('   - Context:', JSON.stringify(ctx, null, 2));
        },
        onFailure: (ctx: any) => {
          console.log('💥 FAILURE HOOK EXECUTED!');
          console.log('   - Context:', JSON.stringify(ctx, null, 2));
        },
      },
    });

    // Add a single step
    workflow.step('simple', async ctx => {
      console.log('🚀 Executing simple step');
      console.log('   - Payload:', ctx.payload);
      return { message: 'Simple step completed', timestamp: Date.now() };
    });

    // Initialize Cronflow
    await cronflow.start();
    console.log('✅ Cronflow initialized');

    // Execute workflow
    const runId = await cronflow.trigger(TEST_WORKFLOW_ID, { test: 'debug' });
    console.log('✅ Workflow triggered:', runId);

    // Wait a bit for execution
    await new Promise(resolve => setTimeout(resolve, 1000));

    // Check status
    const status = await cronflow.inspect(runId);
    console.log('📊 Workflow status:', status);

    // Wait a bit more
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Check status again
    const finalStatus = await cronflow.inspect(runId);
    console.log('📊 Final workflow status:', finalStatus);

    return { success: true, runId, status: finalStatus };
  } catch (error: any) {
    console.error('❌ Debug failed:', error);
    return { success: false, error: error.message };
  } finally {
    await cronflow.stop();
  }
}

// Run the debug
debugCompletion()
  .then(result => {
    if (result.success) {
      console.log('\n🎯 Debug completed successfully');
      console.log('✅ Run ID:', result.runId);
      console.log('✅ Final Status:', result.status);
    } else {
      console.log('\n💥 Debug failed');
      console.log('❌ Error:', result.error);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
  });
