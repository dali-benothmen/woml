#!/usr/bin/env bun

/**
 * Debug State Machine Execution
 *
 * This script helps debug the workflow state machine execution
 * to understand why the workflow status is not being updated.
 */

import { cronflow } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';

const TEST_WORKFLOW_ID = 'debug-state-machine-workflow';

async function debugStateMachine() {
  console.log('🔍 Debugging State Machine Execution');
  console.log('='.repeat(50));

  try {
    // Create a simple workflow with no dependencies
    const workflow = cronflow.define({
      id: TEST_WORKFLOW_ID,
      name: 'Debug State Machine Workflow',
      description: 'A simple workflow to debug state machine execution',
    });

    // Add a single step with no dependencies
    workflow.step('simple-step', async (ctx: Context) => {
      console.log('🚀 Simple step executed!');
      console.log('   - Payload:', ctx.payload);
      console.log('   - Run ID:', ctx.run.id);

      return {
        message: 'Simple step completed successfully',
        timestamp: Date.now(),
      };
    });

    // Initialize Cronflow
    await cronflow.start();
    console.log('✅ Cronflow initialized');

    // Create a workflow run
    const payload = {
      message: 'Debug test',
      timestamp: Date.now(),
    };

    console.log('🔄 Creating workflow run...');
    const runId = await cronflow.trigger(TEST_WORKFLOW_ID, payload);
    console.log('✅ Workflow run created:', runId);

    // Check initial status
    console.log('📋 Checking initial status...');
    const initialStatus = await cronflow.inspect(runId);
    console.log('   - Initial status:', initialStatus.status);

    // Wait a moment for execution
    console.log('⏳ Waiting for execution...');
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Check final status
    console.log('📋 Checking final status...');
    const finalStatus = await cronflow.inspect(runId);
    console.log('   - Final status:', finalStatus.status);
    console.log(
      '   - Final status details:',
      JSON.stringify(finalStatus, null, 2)
    );

    return {
      success: true,
      run_id: runId,
      initial_status: initialStatus.status,
      final_status: finalStatus.status,
    };
  } catch (error: any) {
    console.error('❌ Debug failed:', error);
    return {
      success: false,
      error: error.message,
    };
  } finally {
    await cronflow.stop();
  }
}

// Run the debug
debugStateMachine()
  .then(result => {
    if (result.success) {
      console.log('\n🎯 Debug Results:');
      console.log('   - Run ID:', result.run_id);
      console.log('   - Initial Status:', result.initial_status);
      console.log('   - Final Status:', result.final_status);
      console.log(
        '   - Status Changed:',
        result.initial_status !== result.final_status
      );
    } else {
      console.log('\n💥 Debug failed:', result.error);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
  });
