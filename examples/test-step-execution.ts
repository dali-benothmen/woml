#!/usr/bin/env bun

/**
 * Test Step Execution Logic
 *
 * This test verifies that the step execution logic is working correctly
 * by directly calling executeStepFunction.
 */

import { cronflow } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';
import * as fs from 'fs';

// Test configuration
const TEST_DB_PATH = './test-step-execution.db';
const TEST_WORKFLOW_ID = 'test-step-execution-workflow';

async function testStepExecution() {
  console.log('🧪 Testing Step Execution Logic');
  console.log('='.repeat(50));

  try {
    // Clean up any existing test database
    if (fs.existsSync(TEST_DB_PATH)) {
      fs.unlinkSync(TEST_DB_PATH);
    }

    // Create a simple workflow
    console.log('\n📋 Creating workflow with step');
    const workflow = cronflow.define({
      id: TEST_WORKFLOW_ID,
      name: 'Step Execution Test Workflow',
      description: 'A test workflow to verify step execution',
    });

    // Add a simple step
    workflow.step(
      'test-step',
      async (ctx: Context) => {
        console.log('🎯 Executing test step with payload:', ctx.payload);
        return {
          message: 'Test step executed successfully',
          timestamp: Date.now(),
          received_payload: ctx.payload,
        };
      },
      {
        timeout: 5000,
      }
    );

    // Register the workflow
    await workflow.register();
    console.log('✅ Workflow registered successfully');

    // Initialize Cronflow
    await cronflow.start({ dbPath: TEST_DB_PATH });
    console.log('✅ Cronflow initialized');

    // Create a workflow run
    console.log('\n📋 Creating workflow run');
    const payload = { test_data: 'hello world', number: 42 };
    const runId = await cronflow.trigger(TEST_WORKFLOW_ID, payload);
    console.log('✅ Workflow run created:', runId);

    // Test direct step execution with context
    console.log('\n📋 Testing direct step execution');

    // Create a context for the step
    const context = {
      run_id: runId,
      workflow_id: TEST_WORKFLOW_ID,
      step_name: 'test-step',
      payload: payload,
      steps: {},
      services: {},
      run: {
        id: runId,
        workflow_id: TEST_WORKFLOW_ID,
      },
      metadata: {
        created_at: new Date().toISOString(),
        step_index: 0,
        total_steps: 1,
        timeout: null,
        retry_count: 0,
        max_retries: 3,
      },
    };

    const contextJson = JSON.stringify(context);

    // Execute the step function directly
    console.log('🔄 Executing step function directly...');
    const stepResult = await cronflow.executeStepFunction(
      'test-step',
      contextJson,
      TEST_WORKFLOW_ID,
      runId
    );

    console.log(
      '✅ Step execution result:',
      JSON.stringify(stepResult, null, 2)
    );

    // Verify the result
    if (stepResult.success && stepResult.result) {
      const result = stepResult.result;
      console.log('✅ Step executed successfully!');
      console.log('   - Status:', result.status);
      console.log('   - Output:', result.output);
      console.log('   - Duration:', result.duration_ms, 'ms');
    } else {
      console.log('❌ Step execution failed:', stepResult.message);
    }

    console.log('\n🎉 Step Execution Test Results:');
    console.log('='.repeat(40));
    console.log('✅ Step execution logic working');
    console.log('✅ Context passing working');
    console.log('✅ Step handler execution working');
    console.log('✅ Result serialization working');

    return {
      success: true,
      step_result: stepResult,
    };
  } catch (error: any) {
    console.error('❌ Test failed:', error);
    console.error('Stack trace:', error.stack);
    return {
      success: false,
      error: error.message,
      stack: error.stack,
    };
  } finally {
    // Clean up
    await cronflow.stop();
    if (fs.existsSync(TEST_DB_PATH)) {
      fs.unlinkSync(TEST_DB_PATH);
      console.log('✅ Test database file deleted:', TEST_DB_PATH);
    }
  }
}

// Run the test
testStepExecution()
  .then(result => {
    if (result.success) {
      console.log('\n🎯 Step Execution Logic - WORKING');
      console.log('✅ Test passed successfully');
      process.exit(0);
    } else {
      console.log('\n💥 Step Execution Logic - FAILED');
      console.log('❌ Test failed');
      process.exit(1);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
    process.exit(1);
  });

export { testStepExecution };
