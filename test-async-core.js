#!/usr/bin/env node

/**
 * Test file for async core functions
 * Run with: node test-async-core.js
 */

const core = require('./local-builds/@cronflow/linux-x64-gnu');

// Sample workflow for testing
const sampleWorkflow = {
  id: 'test-async-workflow',
  name: 'Test Async Workflow',
  description: 'Testing async core functions',
  steps: [
    {
      id: 'step-1',
      name: 'First Step',
      title: 'First Step',
      description: 'Test step',
      action: 'console.log("Hello from step 1")',
      type: 'action',
      handler: 'console.log("Hello from step 1")',
      timeout: 30000,
      retry: { max_attempts: 1, backoff_ms: 1000 },
      depends_on: [],
      is_control_flow: false,
      condition_type: null,
      condition_expression: null,
      control_flow_block: null,
      parallel: null,
      parallel_group_id: null,
      parallel_step_count: null,
      race: null,
      for_each: null,
      pause: null,
    },
  ],
  triggers: ['Manual'],
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
};

const DB_PATH = './test-async-core.db';

async function testAsyncFunctions() {
  console.log('\n🧪 Testing Async Core Functions\n');
  console.log('='.repeat(50));

  try {
    // Test 1: Register workflow with async function
    console.log('\n📝 Test 1: Registering workflow (async)...');
    const registerResult = await core.registerWorkflowAsync(
      JSON.stringify(sampleWorkflow),
      DB_PATH
    );
    console.log('✅ Result:', registerResult);

    // Test 2: Create run with async function
    console.log('\n📝 Test 2: Creating workflow run (async)...');
    const createRunResult = await core.createRunAsync(
      'test-async-workflow',
      JSON.stringify({ test: 'data' }),
      DB_PATH
    );
    console.log('✅ Result:', createRunResult);

    // Test 3: Execute step with async function
    if (createRunResult.success && createRunResult.id) {
      console.log('\n📝 Test 3: Executing step (async)...');
      const executeStepResult = await core.executeStepAsync(
        createRunResult.id,
        'step-1',
        DB_PATH
      );
      console.log('✅ Result:', executeStepResult);
    }

    // Test 4: Compare with sync versions (for demonstration)
    console.log('\n📝 Test 4: Comparing sync vs async...');
    console.log('Sync version: core.registerWorkflow()');
    console.log(
      'Async version: core.registerWorkflowAsync() ← Returns Promise!'
    );

    console.log('\n✨ All async tests completed successfully!');
    console.log('\n💡 Key differences:');
    console.log('   • Async functions return Promises');
    console.log('   • Use await or .then() to handle results');
    console.log('   • Non-blocking - better performance under load');
    console.log('   • Function names end with "Async" suffix');
  } catch (error) {
    console.error('\n❌ Error:', error);
    process.exit(1);
  }
}

// Run tests
testAsyncFunctions()
  .then(() => {
    console.log('\n🎉 Test completed successfully!');
    process.exit(0);
  })
  .catch(error => {
    console.error('\n💥 Fatal error:', error);
    process.exit(1);
  });
