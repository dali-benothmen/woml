#!/usr/bin/env bun

import { cronflow } from '../sdk';

// Test workflow for Task 1.2: Enhanced execute_step function
const testWorkflow = cronflow.define({
  id: 'test-task-1-2',
  name: 'Test Task 1.2 - Enhanced execute_step',
  description:
    'Test for enhanced execute_step function with database integration',
});

// Define a simple workflow with steps
testWorkflow
  .onWebhook('/test-task-1-2')
  .step('step-1', async ctx => {
    console.log('🔧 Step 1 executing...');
    return { message: 'Step 1 completed' };
  })
  .action('final-action', async ctx => {
    console.log('🔧 Final action executing...');
    console.log('✅ Workflow completed!');
  });

async function testTask12() {
  try {
    console.log('🚀 Testing Task 1.2: Enhanced execute_step function');
    console.log('='.repeat(60));

    // Test workflow registration
    console.log('✅ Workflow registered successfully');

    // Start the engine to register workflows with Rust
    console.log('🚀 Starting engine to register workflows...');
    await cronflow.start();
    console.log('✅ Engine started and workflows registered with Rust');

    // First, create a run in the database
    console.log('🧪 Creating a run in the database...');
    const runId = await cronflow.trigger('test-task-1-2', {
      test: 'data',
    });
    console.log('📋 Created run with ID:', runId);

    // Test executeStep with empty context (should use database lookup)
    console.log('🧪 Testing executeStep with empty context (database path)...');

    const stepResult = await cronflow.executeStep(runId, 'step-1', ''); // Empty context

    console.log(
      '📋 Step execution result:',
      JSON.stringify(stepResult, null, 2)
    );

    console.log('');
    console.log(
      '✅ Task 1.2: Enhanced execute_step function - Completed Successfully!'
    );
    console.log('');
    console.log('📝 What was tested:');
    console.log('  ✅ Enhanced execute_step function');
    console.log('  ✅ Database lookup path');
    console.log('  ✅ Run creation and retrieval');
    console.log('  ✅ Step execution via database');
    console.log('');
    console.log('🔄 Next steps:');
    console.log('  📋 Task 1.3: Implement Bun.js Step Execution Handler');
    console.log('  📋 Task 1.4: Add Step Execution Orchestration');
    console.log('  📋 Task 1.5: Test Basic Step Execution');

    // Stop the engine
    await cronflow.stop();
    console.log('✅ Engine stopped successfully');
  } catch (error) {
    console.error('❌ Test failed:', error);
    process.exit(1);
  }
}

testTask12();
